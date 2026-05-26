//! Phase 4 M8 — per-element cut-plane clip against a volumetric mesh.
//!
//! Implements `planning/mili-viz/phase-4-m8.md` Decisions 75–77: a
//! closed clipped hull (kept-side faces ∪ tessellated cap),
//! per-element parallel-for-each compute (no global remesh, no spatial
//! index), and a sentinel material id for cap triangles.
//!
//! The corpus is what `MeshTopology` already cached; we walk
//! `elem_classes`, sign-classify each element's corners against the
//! plane, and emit the kept portion. A future bandwidth optimization
//! (rayon-parallel + a per-state BVH) can layer on top of this seam.

use rayon::prelude::*;

use crate::geometry::{element_edges_table, faces_table, MeshTopology, Superclass};

/// Reserved per-triangle material id for cut-plane cap triangles
/// (phase-4-m8.md). Distinct from [`crate::geometry::INTERIOR_SENTINEL`]
/// (`u32::MAX`).
pub const CAP_MATERIAL: u32 = u32::MAX - 1;

/// Reserved per-triangle material id for slice-plane cap triangles
/// (phase-4-m9.md Decision 80). Distinct from [`CAP_MATERIAL`] so the
/// client can render cut vs slice caps differently without needing
/// out-of-band intent.
pub const SLICE_MATERIAL: u32 = u32::MAX - 2;

/// Emission policy for [`clip_topology`] (phase-4-m9.md Decision 79).
/// `Cut` reproduces M8's closed clipped hull; `Slice` emits only the
/// plane-element intersection (the cap, tagged with [`SLICE_MATERIAL`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ClipMode {
    Cut,
    Slice,
}

/// Plane equation `(p - origin) · normal >= 0` defines the keep side.
/// When `relative` is set, the kept half-space is flipped (the
/// griz-style `cutrpln` reversal of the cut direction).
#[derive(Clone, Copy, Debug)]
pub struct Plane {
    pub origin: [f64; 3],
    pub normal: [f64; 3],
}

impl Plane {
    /// Build a plane from the frozen proto field; returns `None` for
    /// a zero-length normal (the doc's "clear the cut" sentinel).
    #[must_use]
    pub fn from_proto(pb: &mili_viz_proto::v1::CutPlane) -> Option<Self> {
        let mut n = [pb.nx, pb.ny, pb.nz];
        let mag = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
        if mag < 1e-12 {
            return None;
        }
        n = [n[0] / mag, n[1] / mag, n[2] / mag];
        if pb.relative {
            n = [-n[0], -n[1], -n[2]];
        }
        Some(Plane {
            origin: [pb.ox, pb.oy, pb.oz],
            normal: n,
        })
    }

    /// Signed distance — positive on the keep side.
    #[inline]
    pub fn signed_distance(&self, p: &[f32; 3]) -> f64 {
        let dx = f64::from(p[0]) - self.origin[0];
        let dy = f64::from(p[1]) - self.origin[1];
        let dz = f64::from(p[2]) - self.origin[2];
        dx * self.normal[0] + dy * self.normal[1] + dz * self.normal[2]
    }
}

/// Output buffers for an `MVG3` clipped emit. `verts` carries the
/// state's coords concatenated with any new intersection vertices
/// (intersections are appended in element-encounter order).
///
/// `scalar` is `Some` iff the caller supplied per-vertex scalars; cap
/// vertices get linear interpolation along their straddled edge
/// (`phase-4-m9.md` Decision 79); cap centroid scalars are the mean
/// of the polygon vertices.
pub struct ClipBuffers {
    pub verts: Vec<f32>,
    pub indices: Vec<u32>,
    pub tri_material: Vec<u32>,
    pub tri_flags: Vec<u32>,
    pub edges: Vec<u32>,
    pub scalar: Option<Vec<f32>>,
    /// Packed `(class_idx, elem_row)` per triangle (wireframe-parity
    /// #6 path (a)). Kept triangles inherit the source element's
    /// member id; cap triangles carry [`crate::geometry::TRI_MEMBER_NONE`]
    /// since the cap is a geometric intersection without a single
    /// owning element.
    pub tri_member_id: Vec<u32>,
}

/// Per-element output, accumulated by the parallel pass and merged
/// sequentially so vertex ids stay deterministic.
struct PerElem {
    new_verts: Vec<[f32; 3]>,
    /// Per-`new_verts` entry: linear interpolation parameters
    /// `(corner_a, corner_b, t)` so the merge step can resolve the
    /// scalar at this new vertex given the global per-vertex scalar
    /// array. Centroid entries use `corner_a == u32::MAX` and stash
    /// the precomputed mean of the polygon scalars as `t`.
    new_vert_scalar: Vec<NewVertScalar>,
    // Kept-side triangles, vertex refs (Local(corner) or New(idx into
    // new_verts)) — resolved to global indices at merge time.
    kept_tris: Vec<[VertexRef; 3]>,
    kept_mat: Vec<u32>,
    cap_tris: Vec<[VertexRef; 3]>,
    cap_mat: u32,
    // Edge refs (corner-corner or corner-new or new-new) for the
    // wireframe pass: kept portions of original element edges + cap
    // boundary edges.
    edges: Vec<[VertexRef; 2]>,
}

#[derive(Clone, Copy)]
enum NewVertScalar {
    /// Linear blend of `scalar[a] (1-t) + scalar[b] t`. Used at
    /// straddled-edge intersection points (Decision 79).
    Edge { a: u32, b: u32, t: f32 },
    /// Cap centroid — mean of the polygon-vertex scalars; filled in
    /// at merge time once the polygon scalars are resolved.
    Centroid,
}

#[derive(Clone, Copy)]
enum VertexRef {
    /// Existing global vertex (the state's node id).
    Existing(u32),
    /// New vertex local to the element (offset into `new_verts`).
    New(u32),
}

#[inline]
fn lerp(a: &[f32; 3], b: &[f32; 3], t: f64) -> [f32; 3] {
    let t = t as f32;
    [
        a[0] + (b[0] - a[0]) * t,
        a[1] + (b[1] - a[1]) * t,
        a[2] + (b[2] - a[2]) * t,
    ]
}

/// Clip a single element against the plane (phase-4-m8.md Decisions
/// 75–76; phase-4-m9.md Decision 79 adds the `slice_only` branch).
/// Per-element only — no global state. `cap_mat` is the per-tri
/// material id used for cap output (CAP_MATERIAL for cut,
/// SLICE_MATERIAL for slice).
fn clip_element(
    sc: Superclass,
    corner_global: &[u32],
    corner_pos: &[[f32; 3]],
    material: u32,
    plane: &Plane,
    mode: ClipMode,
) -> Option<PerElem> {
    let cap_mat = match mode {
        ClipMode::Cut => CAP_MATERIAL,
        ClipMode::Slice => SLICE_MATERIAL,
    };
    let n = corner_global.len();
    let dists: Vec<f64> = corner_pos
        .iter()
        .map(|p| plane.signed_distance(p))
        .collect();
    let eps = 1e-9_f64;
    let all_keep = dists.iter().all(|&d| d >= -eps);
    let all_drop = dists.iter().all(|&d| d <= eps);

    let faces = faces_table(sc);
    let elem_edges = element_edges_table(sc);

    if all_drop {
        return None;
    }
    if all_keep {
        // All-keep: in Cut mode, emit element faces verbatim; in
        // Slice mode, contribute nothing (the plane misses).
        if mode == ClipMode::Slice {
            return None;
        }
        let mut kept_tris: Vec<[VertexRef; 3]> = Vec::new();
        for face in faces {
            if face.iter().any(|&li| li >= n) {
                continue;
            }
            for i in 1..face.len() - 1 {
                kept_tris.push([
                    VertexRef::Existing(corner_global[face[0]]),
                    VertexRef::Existing(corner_global[face[i]]),
                    VertexRef::Existing(corner_global[face[i + 1]]),
                ]);
            }
        }
        let kept_mat = vec![material; kept_tris.len()];
        let mut edges_out: Vec<[VertexRef; 2]> = Vec::with_capacity(elem_edges.len());
        for [a, b] in elem_edges {
            if *a < n && *b < n {
                edges_out.push([
                    VertexRef::Existing(corner_global[*a]),
                    VertexRef::Existing(corner_global[*b]),
                ]);
            }
        }
        return Some(PerElem {
            new_verts: Vec::new(),
            new_vert_scalar: Vec::new(),
            kept_tris,
            kept_mat,
            cap_tris: Vec::new(),
            cap_mat,
            edges: edges_out,
        });
    }

    // Straddle: build per-edge intersections (deduped within the
    // element by sorted-corner pair).
    let mut new_verts: Vec<[f32; 3]> = Vec::new();
    let mut new_vert_scalar: Vec<NewVertScalar> = Vec::new();
    let mut isect: std::collections::HashMap<(usize, usize), u32> =
        std::collections::HashMap::new();
    let mut intersect_for = |a: usize, b: usize| -> u32 {
        let key = (a.min(b), a.max(b));
        if let Some(&idx) = isect.get(&key) {
            return idx;
        }
        let da = dists[a];
        let db = dists[b];
        // Interpolation factor along edge (a → b), parameter at which
        // signed distance hits zero. Always represent the t-param
        // along the (key.0 → key.1) edge so the merger can do the
        // scalar blend regardless of which side was "a".
        let (lo, hi) = (key.0, key.1);
        let (d_lo, d_hi, p_lo, p_hi) = (dists[lo], dists[hi], &corner_pos[lo], &corner_pos[hi]);
        let t = d_lo / (d_lo - d_hi);
        let p = lerp(p_lo, p_hi, t);
        let idx = new_verts.len() as u32;
        new_verts.push(p);
        new_vert_scalar.push(NewVertScalar::Edge {
            a: corner_global[lo],
            b: corner_global[hi],
            t: t as f32,
        });
        let _ = (da, db);
        isect.insert(key, idx);
        idx
    };

    let mut kept_tris: Vec<[VertexRef; 3]> = Vec::new();
    let mut edges_out: Vec<[VertexRef; 2]> = Vec::new();

    // Kept-side polygons via Sutherland–Hodgman per face. In Slice
    // mode we still walk the faces (to compute intersection points
    // and the cap), but discard the kept polygons.
    for face in faces {
        if face.iter().any(|&li| li >= n) {
            continue;
        }
        let m = face.len();
        let mut poly: Vec<VertexRef> = Vec::with_capacity(m + 2);
        for i in 0..m {
            let a = face[i];
            let b = face[(i + 1) % m];
            let da = dists[a];
            let db = dists[b];
            let a_kept = da >= -eps;
            let b_kept = db >= -eps;
            if a_kept {
                poly.push(VertexRef::Existing(corner_global[a]));
            }
            // Straddle along edge a→b → add the intersection point.
            if (a_kept && !b_kept) || (!a_kept && b_kept) {
                poly.push(VertexRef::New(intersect_for(a, b)));
            }
        }
        if mode == ClipMode::Cut && poly.len() >= 3 {
            for i in 1..poly.len() - 1 {
                kept_tris.push([poly[0], poly[i], poly[i + 1]]);
            }
        }
    }
    let kept_mat = vec![material; kept_tris.len()];

    // Kept-side portions of the original element edges (Cut only;
    // Slice emits no element edges — its only wireframe is the cap
    // boundary).
    if mode == ClipMode::Cut {
        for [a, b] in elem_edges {
            if *a >= n || *b >= n {
                continue;
            }
            let da = dists[*a];
            let db = dists[*b];
            let a_kept = da >= -eps;
            let b_kept = db >= -eps;
            if a_kept && b_kept {
                edges_out.push([
                    VertexRef::Existing(corner_global[*a]),
                    VertexRef::Existing(corner_global[*b]),
                ]);
            } else if a_kept && !b_kept {
                edges_out.push([
                    VertexRef::Existing(corner_global[*a]),
                    VertexRef::New(intersect_for(*a, *b)),
                ]);
            } else if !a_kept && b_kept {
                edges_out.push([
                    VertexRef::New(intersect_for(*a, *b)),
                    VertexRef::Existing(corner_global[*b]),
                ]);
            }
        }
    }

    // Cap polygon: the closed curve traced by intersection points.
    // For each face, the *two* corner-pair keys where the face is
    // entered/exited give one cap edge. Collected per face, the
    // union forms the cap boundary; fan-triangulate from the
    // centroid.
    let mut cap_edges: Vec<(u32, u32)> = Vec::new();
    for face in faces {
        if face.iter().any(|&li| li >= n) {
            continue;
        }
        let m = face.len();
        let mut crossings: Vec<u32> = Vec::new();
        for i in 0..m {
            let a = face[i];
            let b = face[(i + 1) % m];
            let da = dists[a];
            let db = dists[b];
            let a_kept = da >= -eps;
            let b_kept = db >= -eps;
            if a_kept != b_kept {
                let key = (a.min(b), a.max(b));
                crossings.push(*isect.get(&key).expect("intersection cached"));
            }
        }
        // A convex face crossed by a plane straddles on exactly two
        // edges. Defensive — skip otherwise.
        if crossings.len() == 2 {
            cap_edges.push((crossings[0], crossings[1]));
        }
    }
    let mut cap_tris: Vec<[VertexRef; 3]> = Vec::new();
    if cap_edges.len() >= 3 && !new_verts.is_empty() {
        // Walk the cap edges into an ordered polygon: pick the first
        // edge, follow shared endpoints.
        let mut ordered: Vec<u32> = Vec::new();
        let mut used = vec![false; cap_edges.len()];
        ordered.push(cap_edges[0].0);
        ordered.push(cap_edges[0].1);
        used[0] = true;
        loop {
            let last = *ordered.last().unwrap();
            let mut found = false;
            for (i, &(a, b)) in cap_edges.iter().enumerate() {
                if used[i] {
                    continue;
                }
                if a == last {
                    ordered.push(b);
                    used[i] = true;
                    found = true;
                    break;
                }
                if b == last {
                    ordered.push(a);
                    used[i] = true;
                    found = true;
                    break;
                }
            }
            if !found {
                break;
            }
        }
        // Closed loop check: the last entry should match the first.
        if ordered.len() >= 3 && *ordered.last().unwrap() == ordered[0] {
            ordered.pop();
        }
        if ordered.len() >= 3 {
            // Centroid of the cap polygon in 3D.
            let mut cx = 0.0_f64;
            let mut cy = 0.0_f64;
            let mut cz = 0.0_f64;
            for &v in &ordered {
                let p = new_verts[v as usize];
                cx += f64::from(p[0]);
                cy += f64::from(p[1]);
                cz += f64::from(p[2]);
            }
            let k = ordered.len() as f64;
            let centroid = [(cx / k) as f32, (cy / k) as f32, (cz / k) as f32];
            let centroid_idx = new_verts.len() as u32;
            new_verts.push(centroid);
            // Centroid scalar = mean of cap polygon vertex scalars;
            // we can't compute it here (no scalar in scope), so the
            // merger fills it in by averaging the resolved scalars.
            // Stash a placeholder; the merge resolves it.
            new_vert_scalar.push(NewVertScalar::Centroid);
            for i in 0..ordered.len() {
                let a = ordered[i];
                let b = ordered[(i + 1) % ordered.len()];
                cap_tris.push([
                    VertexRef::New(centroid_idx),
                    VertexRef::New(a),
                    VertexRef::New(b),
                ]);
                edges_out.push([VertexRef::New(a), VertexRef::New(b)]);
            }
        }
    }

    Some(PerElem {
        new_verts,
        new_vert_scalar,
        kept_tris,
        kept_mat,
        cap_tris,
        cap_mat,
        edges: edges_out,
    })
}

/// Build the clipped volumetric output for the entire mesh
/// (phase-4-m8.md Decision 76 / phase-4-m9.md Decisions 79–80). The
/// `verts` buffer starts as the state's coords and grows by the
/// concatenated per-element intersection points; per-element vertex
/// refs are resolved sequentially after the parallel pass so the
/// global vertex ids are deterministic across runs.
///
/// When `base_scalar` is `Some`, the output `scalar` carries
/// per-vertex values: existing nodes get the input scalar; new
/// intersection vertices get the linear blend along the straddled
/// element-edge (Decision 79); cap centroids get the mean of the
/// polygon vertex scalars.
#[must_use]
pub fn clip_topology(
    topo: &MeshTopology,
    coords: &[f32],
    base_scalar: Option<&[f32]>,
    plane: &Plane,
    mode: ClipMode,
) -> ClipBuffers {
    let node_count = coords.len() / 3;
    let work: Vec<(usize, usize)> = topo
        .elem_class_summary()
        .iter()
        .enumerate()
        .flat_map(|(ci, summary)| (0..summary.elements).map(move |ei| (ci, ei)))
        .collect();

    let outputs: Vec<Option<PerElem>> = work
        .par_iter()
        .map(|&(ci, ei)| {
            let ec = topo.elem_class_at(ci);
            let sc = ec.superclass;
            let n_nodes = ec.n_nodes;
            let row = &ec.conns[ei * n_nodes..(ei + 1) * n_nodes];
            let mat = ec.materials.get(ei).copied().unwrap_or(0);
            let mut corner_pos: Vec<[f32; 3]> = Vec::with_capacity(n_nodes);
            for &nid in row {
                let nid = nid as usize;
                if nid >= node_count {
                    return None;
                }
                corner_pos.push([coords[nid * 3], coords[nid * 3 + 1], coords[nid * 3 + 2]]);
            }
            clip_element(sc, row, &corner_pos, mat, plane, mode)
        })
        .collect();

    let mut verts: Vec<f32> = coords.to_vec();
    let mut scalar: Option<Vec<f32>> = base_scalar.map(<[f32]>::to_vec);
    let mut indices: Vec<u32> = Vec::new();
    let mut tri_material: Vec<u32> = Vec::new();
    let mut tri_flags: Vec<u32> = Vec::new();
    let mut tri_member_id: Vec<u32> = Vec::new();
    let mut edges: Vec<u32> = Vec::new();

    let resolve = |vr: &VertexRef, vert_base: u32| -> u32 {
        match vr {
            VertexRef::Existing(g) => *g,
            VertexRef::New(local) => vert_base + *local,
        }
    };

    for (out, &(ci, ei)) in outputs
        .into_iter()
        .zip(work.iter())
        .filter_map(|(o, w)| o.map(|x| (x, w)))
    {
        let kept_member = crate::geometry::pack_tri_member_id(ci as u32, ei as u32);
        let vert_base = (verts.len() / 3) as u32;
        for (i, v) in out.new_verts.iter().enumerate() {
            verts.extend_from_slice(v);
            // Resolve scalar at this new vertex (Decision 79).
            if let (Some(s_out), Some(s_in)) = (scalar.as_mut(), base_scalar) {
                let val = match out.new_vert_scalar[i] {
                    NewVertScalar::Edge { a, b, t } => {
                        let sa = s_in.get(a as usize).copied().unwrap_or(f32::NAN);
                        let sb = s_in.get(b as usize).copied().unwrap_or(f32::NAN);
                        sa + (sb - sa) * t
                    }
                    NewVertScalar::Centroid => {
                        // Mean of the previously-pushed polygon
                        // scalars from THIS element. They are the
                        // already-appended new_verts entries (Edge
                        // type) in [vert_base, vert_base + i).
                        let start = vert_base as usize;
                        let here = (vert_base as usize) + i;
                        let mut sum = 0.0_f32;
                        let mut cnt = 0u32;
                        for &s in &s_out[start..here] {
                            if s.is_finite() {
                                sum += s;
                                cnt += 1;
                            }
                        }
                        if cnt == 0 {
                            f32::NAN
                        } else {
                            sum / cnt as f32
                        }
                    }
                };
                s_out.push(val);
            }
        }
        for (tri, &mat) in out.kept_tris.iter().zip(&out.kept_mat) {
            indices.push(resolve(&tri[0], vert_base));
            indices.push(resolve(&tri[1], vert_base));
            indices.push(resolve(&tri[2], vert_base));
            tri_material.push(mat);
            tri_flags.push(0);
            tri_member_id.push(kept_member);
        }
        for tri in &out.cap_tris {
            indices.push(resolve(&tri[0], vert_base));
            indices.push(resolve(&tri[1], vert_base));
            indices.push(resolve(&tri[2], vert_base));
            tri_material.push(out.cap_mat);
            tri_flags.push(0);
            tri_member_id.push(crate::geometry::TRI_MEMBER_NONE);
        }
        for e in &out.edges {
            edges.push(resolve(&e[0], vert_base));
            edges.push(resolve(&e[1], vert_base));
        }
    }

    ClipBuffers {
        verts,
        indices,
        tri_material,
        tri_flags,
        edges,
        scalar,
        tri_member_id,
    }
}

/// Compose a cut blob with an additional slice clip (phase-4-m9.md
/// Decision 80). The two operations are independent — the slice
/// sees the full mesh, not just the kept side of the cut — so we
/// run them as two passes and concatenate the outputs. `Existing`
/// vertex refs in the slice's pass index into the same `coords`
/// base, so the merge just rebases the slice's new verts by the
/// cut blob's existing vertex count.
#[must_use]
pub fn append_clip(
    mut into: ClipBuffers,
    mut tail: ClipBuffers,
    base_n_verts: usize,
) -> ClipBuffers {
    // The first `base_n_verts*3` floats of `tail.verts` duplicate
    // `into.verts` (both seeded from the same `coords`). Strip the
    // duplicate; rebase tail's new-vertex indices by the offset
    // (current `into` verts count minus the duplicate base).
    let into_v_count = into.verts.len() / 3;
    let new_offset = (into_v_count - base_n_verts) as u32;
    let dup_floats = base_n_verts * 3;
    if tail.verts.len() >= dup_floats {
        into.verts.extend_from_slice(&tail.verts[dup_floats..]);
    }
    if let (Some(out), Some(t)) = (into.scalar.as_mut(), tail.scalar.as_mut()) {
        if t.len() >= base_n_verts {
            out.extend_from_slice(&t[base_n_verts..]);
        }
    }
    for &i in &tail.indices {
        let rebased = if (i as usize) < base_n_verts {
            i
        } else {
            i + new_offset
        };
        into.indices.push(rebased);
    }
    into.tri_material.extend_from_slice(&tail.tri_material);
    into.tri_flags.extend_from_slice(&tail.tri_flags);
    into.tri_member_id.extend_from_slice(&tail.tri_member_id);
    for &e in &tail.edges {
        let rebased = if (e as usize) < base_n_verts {
            e
        } else {
            e + new_offset
        };
        into.edges.push(rebased);
    }
    into
}

#[cfg(test)]
mod tests {
    use super::*;

    fn unit_hex_corners() -> ([u32; 8], [[f32; 3]; 8]) {
        let g = [0, 1, 2, 3, 4, 5, 6, 7];
        let p = [
            [0.0, 0.0, 0.0],
            [1.0, 0.0, 0.0],
            [1.0, 1.0, 0.0],
            [0.0, 1.0, 0.0],
            [0.0, 0.0, 1.0],
            [1.0, 0.0, 1.0],
            [1.0, 1.0, 1.0],
            [0.0, 1.0, 1.0],
        ];
        (g, p)
    }

    #[test]
    fn unit_hex_straddle_produces_cap_quad() {
        let (g, p) = unit_hex_corners();
        let plane = Plane {
            origin: [0.5, 0.5, 0.5],
            normal: [1.0, 0.0, 0.0],
        };
        let out =
            clip_element(Superclass::Hex, &g, &p, 7, &plane, ClipMode::Cut).expect("straddle");
        assert!(
            !out.cap_tris.is_empty(),
            "x=0.5 cut produces a square cap (>= 2 tris)"
        );
        assert_eq!(out.kept_mat.len(), out.kept_tris.len());
    }

    #[test]
    fn unit_hex_all_keep_emits_full_hull_no_cap() {
        let (g, p) = unit_hex_corners();
        let plane = Plane {
            origin: [-1.0, 0.0, 0.0],
            normal: [1.0, 0.0, 0.0],
        };
        let out =
            clip_element(Superclass::Hex, &g, &p, 7, &plane, ClipMode::Cut).expect("all keep");
        assert!(out.cap_tris.is_empty(), "no cap on all-keep");
        assert!(out.new_verts.is_empty(), "no new verts on all-keep");
        assert_eq!(out.kept_tris.len(), 12);
    }

    #[test]
    fn unit_hex_all_drop_returns_none() {
        let (g, p) = unit_hex_corners();
        let plane = Plane {
            origin: [2.0, 0.0, 0.0],
            normal: [1.0, 0.0, 0.0],
        };
        assert!(clip_element(Superclass::Hex, &g, &p, 7, &plane, ClipMode::Cut).is_none());
    }

    #[test]
    fn unit_hex_slice_drops_kept_side_keeps_cap() {
        let (g, p) = unit_hex_corners();
        let plane = Plane {
            origin: [0.5, 0.0, 0.0],
            normal: [1.0, 0.0, 0.0],
        };
        let out = clip_element(Superclass::Hex, &g, &p, 7, &plane, ClipMode::Slice)
            .expect("slice straddler");
        assert!(out.kept_tris.is_empty(), "slice drops kept hull");
        assert!(out.kept_mat.is_empty());
        assert!(!out.cap_tris.is_empty(), "slice keeps the cap");
        assert_eq!(out.cap_mat, SLICE_MATERIAL);
    }

    #[test]
    fn unit_hex_slice_all_keep_emits_nothing() {
        let (g, p) = unit_hex_corners();
        let plane = Plane {
            origin: [-1.0, 0.0, 0.0],
            normal: [1.0, 0.0, 0.0],
        };
        assert!(clip_element(Superclass::Hex, &g, &p, 7, &plane, ClipMode::Slice).is_none());
    }

    #[test]
    fn scalar_interpolation_is_linear_along_straddled_edge() {
        // Two-corner edge straddler: corner 0 at x=0 scalar=0,
        // corner 1 at x=1 scalar=10. Cut at x=0.5 → expected scalar
        // at the intersection point = 5.0.
        let g = [0u32, 1, 2, 3, 4, 5, 6, 7];
        let p = [
            [0.0, 0.0, 0.0],
            [1.0, 0.0, 0.0],
            [1.0, 1.0, 0.0],
            [0.0, 1.0, 0.0],
            [0.0, 0.0, 1.0],
            [1.0, 0.0, 1.0],
            [1.0, 1.0, 1.0],
            [0.0, 1.0, 1.0],
        ];
        let s = [0.0_f32, 10.0, 0.0, 0.0, 0.0, 10.0, 0.0, 0.0];
        let plane = Plane {
            origin: [0.5, 0.0, 0.0],
            normal: [1.0, 0.0, 0.0],
        };
        let out = clip_element(Superclass::Hex, &g, &p, 7, &plane, ClipMode::Slice).unwrap();
        // Find the edge new vert for (0,1): t at 0.5 along scalar
        // 0→10 = 5.
        let mut saw_5 = false;
        for nv in &out.new_vert_scalar {
            if let NewVertScalar::Edge { a, b, t } = nv {
                if (*a == 0 && *b == 1) || (*a == 1 && *b == 0) {
                    let sa = s[*a as usize];
                    let sb = s[*b as usize];
                    let v = sa + (sb - sa) * t;
                    if (v - 5.0).abs() < 1e-5 {
                        saw_5 = true;
                    }
                }
            }
        }
        assert!(saw_5, "linear blend of 0..10 at midpoint = 5");
    }
}
