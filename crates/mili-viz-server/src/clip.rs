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

/// Reserved per-triangle material id for cap triangles
/// (phase-4-m8.md "What lands" — colormap treats `u32::MAX - 1` as
/// neutral grey because the cap is a synthetic surface, not a material
/// face). Distinct from [`crate::geometry::INTERIOR_SENTINEL`]
/// (`u32::MAX`).
pub const CAP_MATERIAL: u32 = u32::MAX - 1;

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
pub struct ClipBuffers {
    pub verts: Vec<f32>,
    pub indices: Vec<u32>,
    pub tri_material: Vec<u32>,
    pub tri_flags: Vec<u32>,
    pub edges: Vec<u32>,
}

/// Per-element output, accumulated by the parallel pass and merged
/// sequentially so vertex ids stay deterministic.
struct PerElem {
    new_verts: Vec<[f32; 3]>,
    // Kept-side triangles, vertex refs (Local(corner) or New(idx into
    // new_verts)) — resolved to global indices at merge time.
    kept_tris: Vec<[VertexRef; 3]>,
    kept_mat: Vec<u32>,
    cap_tris: Vec<[VertexRef; 3]>,
    // Edge refs (corner-corner or corner-new or new-new) for the
    // wireframe pass: kept portions of original element edges + cap
    // boundary edges.
    edges: Vec<[VertexRef; 2]>,
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

/// Linear-edge intersection: returns the point where signed distance
/// transitions through zero between two corners with signed distances
/// `da` (>= 0, keep side) and `db` (< 0, drop side). Pre-condition:
/// `da * db < 0`.
#[inline]
fn intersect(p_a: &[f32; 3], p_b: &[f32; 3], da: f64, db: f64) -> [f32; 3] {
    let t = da / (da - db);
    lerp(p_a, p_b, t)
}

/// Clip a single element against the plane (phase-4-m8.md Decisions
/// 75–76). Per-element only — no global state.
fn clip_element(
    sc: Superclass,
    corner_global: &[u32],
    corner_pos: &[[f32; 3]],
    material: u32,
    plane: &Plane,
) -> Option<PerElem> {
    let n = corner_global.len();
    let dists: Vec<f64> = corner_pos.iter().map(|p| plane.signed_distance(p)).collect();
    let eps = 1e-9_f64;
    let all_keep = dists.iter().all(|&d| d >= -eps);
    let all_drop = dists.iter().all(|&d| d <= eps);

    let faces = faces_table(sc);
    let elem_edges = element_edges_table(sc);

    if all_drop {
        return None;
    }
    if all_keep {
        // All-keep: emit element faces verbatim (no straddle, no cap).
        let mut kept_tris: Vec<[VertexRef; 3]> = Vec::new();
        for face in faces {
            if face.iter().any(|&li| li >= n) {
                continue;
            }
            // Fan-triangulate the face. Cap polygons (the new cut
            // face) are not present for all-keep elements.
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
            kept_tris,
            kept_mat,
            cap_tris: Vec::new(),
            edges: edges_out,
        });
    }

    // Straddle: build per-edge intersections (deduped within the
    // element by sorted-corner pair).
    let mut new_verts: Vec<[f32; 3]> = Vec::new();
    let mut isect: std::collections::HashMap<(usize, usize), u32> =
        std::collections::HashMap::new();
    let mut intersect_for = |a: usize, b: usize| -> u32 {
        let key = (a.min(b), a.max(b));
        if let Some(&idx) = isect.get(&key) {
            return idx;
        }
        let (da, db, pa, pb) = if dists[a] >= 0.0 {
            (dists[a], dists[b], &corner_pos[a], &corner_pos[b])
        } else {
            (dists[b], dists[a], &corner_pos[b], &corner_pos[a])
        };
        let p = intersect(pa, pb, da, db);
        let idx = new_verts.len() as u32;
        new_verts.push(p);
        isect.insert(key, idx);
        idx
    };

    let mut kept_tris: Vec<[VertexRef; 3]> = Vec::new();
    let mut edges_out: Vec<[VertexRef; 2]> = Vec::new();

    // Kept-side polygons via Sutherland–Hodgman per face.
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
        if poly.len() >= 3 {
            for i in 1..poly.len() - 1 {
                kept_tris.push([poly[0], poly[i], poly[i + 1]]);
            }
        }
    }
    let kept_mat = vec![material; kept_tris.len()];

    // Kept-side portions of the original element edges (so the
    // wireframe stays clean).
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
            let centroid = [
                (cx / k) as f32,
                (cy / k) as f32,
                (cz / k) as f32,
            ];
            let centroid_idx = new_verts.len() as u32;
            new_verts.push(centroid);
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
        kept_tris,
        kept_mat,
        cap_tris,
        edges: edges_out,
    })
}

/// Build the clipped volumetric output for the entire mesh
/// (phase-4-m8.md Decision 76 — `rayon` parallel-per-element). The
/// `verts` buffer starts as the state's coords and grows by the
/// concatenated per-element intersection points; per-element vertex
/// refs are resolved sequentially after the parallel pass so the
/// global vertex ids are deterministic across runs.
#[must_use]
pub fn clip_topology(topo: &MeshTopology, coords: &[f32], plane: &Plane) -> ClipBuffers {
    let node_count = coords.len() / 3;
    // (class_index, element_index) → per-element output. Flatten the
    // class/element nesting into one parallel iterator so rayon can
    // saturate; collect retains source order for the deterministic
    // sequential merge.
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
                corner_pos.push([
                    coords[nid * 3],
                    coords[nid * 3 + 1],
                    coords[nid * 3 + 2],
                ]);
            }
            clip_element(sc, row, &corner_pos, mat, plane)
        })
        .collect();

    let mut verts: Vec<f32> = coords.to_vec();
    let mut indices: Vec<u32> = Vec::new();
    let mut tri_material: Vec<u32> = Vec::new();
    let mut tri_flags: Vec<u32> = Vec::new();
    let mut edges: Vec<u32> = Vec::new();

    let resolve = |vr: &VertexRef, vert_base: u32| -> u32 {
        match vr {
            VertexRef::Existing(g) => *g,
            VertexRef::New(local) => vert_base + *local,
        }
    };

    for out in outputs.into_iter().flatten() {
        let vert_base = (verts.len() / 3) as u32;
        for v in &out.new_verts {
            verts.extend_from_slice(v);
        }
        for (tri, &mat) in out.kept_tris.iter().zip(&out.kept_mat) {
            indices.push(resolve(&tri[0], vert_base));
            indices.push(resolve(&tri[1], vert_base));
            indices.push(resolve(&tri[2], vert_base));
            tri_material.push(mat);
            tri_flags.push(0);
        }
        for tri in &out.cap_tris {
            indices.push(resolve(&tri[0], vert_base));
            indices.push(resolve(&tri[1], vert_base));
            indices.push(resolve(&tri[2], vert_base));
            tri_material.push(CAP_MATERIAL);
            tri_flags.push(0);
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
    }
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
        let out = clip_element(Superclass::Hex, &g, &p, 7, &plane).expect("straddle");
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
        let out = clip_element(Superclass::Hex, &g, &p, 7, &plane).expect("all keep");
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
        assert!(clip_element(Superclass::Hex, &g, &p, 7, &plane).is_none());
    }
}
