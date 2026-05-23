//! Decode the self-describing server geometry blob into a renderer
//! `Mesh` (`phase-5-m2.md` Decision 42).
//!
//! The byte layout is `phase-4-m2.md` Decision 11
//! (`MVG1:verts_f32x3+idx_u32+trimat_u32`); the M3 superset
//! (`MVG2:...+scalar_f32`) is now **kept** — the trailing per-vertex
//! scalar becomes vertex colour through a colormap (`phase-5-m3.md`
//! Decision 47). Per-vertex normals are computed on the CPU so the
//! hull reads as a 3-D surface, not a flat silhouette.

use glam::Vec3;

/// A decoded indexed triangle mesh ready for GPU upload.
pub struct Mesh {
    /// `x,y,z` per vertex, in node-array order (current state).
    pub positions: Vec<[f32; 3]>,
    /// Area-weighted, normalized per-vertex normals (`positions.len()`
    /// entries).
    pub normals: Vec<[f32; 3]>,
    /// Triangle-list indices into `positions` (multiple of 3).
    pub indices: Vec<u32>,
    /// Optional per-vertex `MVG2` scalar (`positions.len()` entries),
    /// `None` for a bare `MVG1` hull. Drives the colormap +
    /// legend (`phase-5-m3.md` Decision 47).
    pub scalars: Option<Vec<f32>>,
    /// Server-supplied per-element edge buffer (`MVG3`, line-list
    /// pairs into [`Mesh::positions`]; `phase-4-m7.md` Decision 73).
    /// `None` for `MVG1`/`MVG2` — the renderer falls back to
    /// [`Mesh::edge_indices`] (extraction from triangles, which over-
    /// emits hex face diagonals; VB-005). When present, the wireframe
    /// pass should prefer this buffer (Phase 5 M7 Decision 82).
    pub element_edges: Option<Vec<u32>>,
    /// Optional per-triangle `MVG3` flag column (`tri_flags`); bit 0 =
    /// interior face. Parallel to `indices.len() / 3`. `None` for
    /// `MVG1`/`MVG2`. Lets a translucent renderer distinguish
    /// boundary from cell-cell shared faces (`phase-4-m7.md`
    /// Decision 74).
    pub tri_flags: Option<Vec<u32>>,
}

/// A client-side pick hit against the cached hull. The frozen proto's
/// `GeometryRef` carries no node/element label catalog (only
/// verts/idx/trimat), so picking reports what the cached geometry
/// actually has: the hit triangle, the nearest node (vertex index),
/// the world-space hit point and, for an `MVG2` result, the scalar at
/// that node.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Pick {
    /// Triangle index (`indices[3*tri..]`).
    pub tri: usize,
    /// Nearest vertex of the hit triangle (node-array index).
    pub node: usize,
    /// Ray parameter (world-unit distance along the ray).
    pub distance: f32,
    /// World-space hit point.
    pub point: [f32; 3],
    /// Result scalar at `node` for an `MVG2` hull, else `None`.
    pub scalar: Option<f32>,
}

/// Error decoding a geometry blob.
#[derive(Debug)]
pub struct DecodeError(pub String);

impl std::fmt::Display for DecodeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "geometry decode error: {}", self.0)
    }
}

impl std::error::Error for DecodeError {}

fn le_u32(b: &[u8], off: usize) -> u32 {
    u32::from_le_bytes(b[off..off + 4].try_into().unwrap())
}

fn le_u64(b: &[u8], off: usize) -> u64 {
    u64::from_le_bytes(b[off..off + 8].try_into().unwrap())
}

fn le_f32(b: &[u8], off: usize) -> f32 {
    f32::from_le_bytes(b[off..off + 4].try_into().unwrap())
}

/// Decode an `MVG1`/`MVG2`/`MVG3` blob. The optional `MVG2` trailing
/// per-vertex scalar is kept in [`Mesh::scalars`] (`phase-5-m3.md`
/// Decision 47). The `MVG3` (`phase-4-m7.md` Decisions 72–74) adds an
/// optional per-element edge buffer ([`Mesh::element_edges`]) and a
/// per-triangle flag column ([`Mesh::tri_flags`]).
///
/// # Errors
/// Returns [`DecodeError`] if the magic is unknown or the buffer is
/// truncated relative to its self-described counts.
pub fn decode_mvg(blob: &[u8]) -> Result<Mesh, DecodeError> {
    if blob.len() < 4 {
        return Err(DecodeError("blob shorter than the magic".into()));
    }
    let magic = &blob[0..4];
    if matches!(magic, b"MVG1" | b"MVG2") {
        return decode_mvg_legacy(blob);
    }
    if magic == b"MVG3" {
        return decode_mvg3(blob);
    }
    Err(DecodeError(format!(
        "unknown magic {:?} (expected MVG1/MVG2/MVG3)",
        String::from_utf8_lossy(magic)
    )))
}

fn decode_mvg_legacy(blob: &[u8]) -> Result<Mesh, DecodeError> {
    if blob.len() < 24 {
        return Err(DecodeError("blob shorter than the 24-byte header".into()));
    }
    let magic = &blob[0..4];
    let with_scalar = matches!(magic, b"MVG2");
    let dims = le_u32(blob, 4);
    if dims != 3 {
        return Err(DecodeError(format!("expected dims=3, got {dims}")));
    }
    let n_verts = le_u64(blob, 8) as usize;
    let n_idx = le_u64(blob, 16) as usize;
    if !n_idx.is_multiple_of(3) {
        return Err(DecodeError(format!(
            "index count {n_idx} is not a triangle list"
        )));
    }

    let verts_bytes = n_verts * 3 * 4;
    let idx_bytes = n_idx * 4;
    let trimat_bytes = (n_idx / 3) * 4;
    let scalar_bytes = if with_scalar { n_verts * 4 } else { 0 };
    let need = 24 + verts_bytes + idx_bytes + trimat_bytes + scalar_bytes;
    if blob.len() < need {
        return Err(DecodeError(format!(
            "blob is {} bytes, layout needs {need}",
            blob.len()
        )));
    }

    let mut positions = Vec::with_capacity(n_verts);
    for v in 0..n_verts {
        let off = 24 + v * 12;
        positions.push([
            le_f32(blob, off),
            le_f32(blob, off + 4),
            le_f32(blob, off + 8),
        ]);
    }
    let idx_off = 24 + verts_bytes;
    let mut indices = Vec::with_capacity(n_idx);
    for i in 0..n_idx {
        indices.push(le_u32(blob, idx_off + i * 4));
    }
    let scalars = if with_scalar {
        let scalar_off = idx_off + idx_bytes + trimat_bytes;
        let mut s = Vec::with_capacity(n_verts);
        for v in 0..n_verts {
            s.push(le_f32(blob, scalar_off + v * 4));
        }
        Some(s)
    } else {
        None
    };

    let normals = compute_normals(&positions, &indices);
    Ok(Mesh {
        positions,
        normals,
        indices,
        scalars,
        element_edges: None,
        tri_flags: None,
    })
}

/// `MVG3` header layout (`phase-4-m7.md` § "Blob layout"):
///
/// ```text
/// magic(4) dims(4) n_verts(8) n_idx(8) n_edges(8) flags_mask(4)
/// verts indices tri_material [tri_flags] [edges] [scalar]
/// ```
fn decode_mvg3(blob: &[u8]) -> Result<Mesh, DecodeError> {
    const HEADER: usize = 36;
    if blob.len() < HEADER {
        return Err(DecodeError(format!(
            "MVG3 blob shorter than the {HEADER}-byte header"
        )));
    }
    let dims = le_u32(blob, 4);
    if dims != 3 {
        return Err(DecodeError(format!("MVG3 expected dims=3, got {dims}")));
    }
    let n_verts = le_u64(blob, 8) as usize;
    let n_idx = le_u64(blob, 16) as usize;
    let n_edges = le_u64(blob, 24) as usize;
    let flags_mask = le_u32(blob, 32);
    if !n_idx.is_multiple_of(3) {
        return Err(DecodeError(format!(
            "MVG3 index count {n_idx} is not a triangle list"
        )));
    }
    if !n_edges.is_multiple_of(2) {
        return Err(DecodeError(format!(
            "MVG3 edge count {n_edges} is not a line list"
        )));
    }
    let has_scalar = flags_mask & 1 != 0;
    let has_tri_flags = flags_mask & 2 != 0;
    let has_edges = flags_mask & 4 != 0;

    let n_tri = n_idx / 3;
    let verts_bytes = n_verts * 3 * 4;
    let idx_bytes = n_idx * 4;
    let trimat_bytes = n_tri * 4;
    let triflag_bytes = if has_tri_flags { n_tri * 4 } else { 0 };
    let edges_bytes = if has_edges { n_edges * 4 } else { 0 };
    let scalar_bytes = if has_scalar { n_verts * 4 } else { 0 };
    let need = HEADER
        + verts_bytes
        + idx_bytes
        + trimat_bytes
        + triflag_bytes
        + edges_bytes
        + scalar_bytes;
    if blob.len() < need {
        return Err(DecodeError(format!(
            "MVG3 blob is {} bytes, layout needs {need}",
            blob.len()
        )));
    }

    let mut positions = Vec::with_capacity(n_verts);
    for v in 0..n_verts {
        let off = HEADER + v * 12;
        positions.push([
            le_f32(blob, off),
            le_f32(blob, off + 4),
            le_f32(blob, off + 8),
        ]);
    }
    let idx_off = HEADER + verts_bytes;
    let mut indices = Vec::with_capacity(n_idx);
    for i in 0..n_idx {
        indices.push(le_u32(blob, idx_off + i * 4));
    }
    // tri_material is filtered server-side already (M4); skipped here.
    let mut off = idx_off + idx_bytes + trimat_bytes;
    let tri_flags = if has_tri_flags {
        let mut tf = Vec::with_capacity(n_tri);
        for t in 0..n_tri {
            tf.push(le_u32(blob, off + t * 4));
        }
        off += triflag_bytes;
        Some(tf)
    } else {
        None
    };
    let element_edges = if has_edges {
        let mut e = Vec::with_capacity(n_edges);
        for i in 0..n_edges {
            e.push(le_u32(blob, off + i * 4));
        }
        off += edges_bytes;
        Some(e)
    } else {
        None
    };
    let scalars = if has_scalar {
        let mut s = Vec::with_capacity(n_verts);
        for v in 0..n_verts {
            s.push(le_f32(blob, off + v * 4));
        }
        Some(s)
    } else {
        None
    };

    let normals = compute_normals(&positions, &indices);
    Ok(Mesh {
        positions,
        normals,
        indices,
        scalars,
        element_edges,
        tri_flags,
    })
}

/// Area-weighted per-vertex normals (the cross product is already
/// proportional to triangle area, so plain accumulation weights by
/// area). Degenerate / unreferenced vertices fall back to +Z so the
/// shader's ambient term still lights them.
fn compute_normals(positions: &[[f32; 3]], indices: &[u32]) -> Vec<[f32; 3]> {
    let mut acc = vec![Vec3::ZERO; positions.len()];
    for tri in indices.chunks_exact(3) {
        let (a, b, c) = (tri[0] as usize, tri[1] as usize, tri[2] as usize);
        let pa = Vec3::from(positions[a]);
        let pb = Vec3::from(positions[b]);
        let pc = Vec3::from(positions[c]);
        let n = (pb - pa).cross(pc - pa);
        acc[a] += n;
        acc[b] += n;
        acc[c] += n;
    }
    acc.into_iter()
        .map(|v| v.try_normalize().unwrap_or(Vec3::Z).to_array())
        .collect()
}

impl Mesh {
    /// A camera-facing unit triangle in the `z = 0` plane — the M1
    /// pipeline smoke (`render_to_image`); the M1 triangle constant
    /// (`phase-5-m1.md` Decision 40) reborn as a `Mesh`.
    #[must_use]
    pub fn unit_triangle() -> Self {
        Self {
            positions: vec![[0.0, 0.6, 0.0], [-0.6, -0.5, 0.0], [0.6, -0.5, 0.0]],
            normals: vec![[0.0, 0.0, 1.0]; 3],
            indices: vec![0, 1, 2],
            scalars: None,
            element_edges: None,
            tri_flags: None,
        }
    }

    /// Unique undirected triangle edges as a `LineList` index buffer
    /// (pairs into [`Mesh::positions`]). Each shared edge appears once
    /// regardless of how many triangles fan around it, so the
    /// element-edge / wireframe pass draws clean mesh lines rather than
    /// every triangle leg three times over (VB-003).
    #[must_use]
    pub fn edge_indices(&self) -> Vec<u32> {
        let mut seen = std::collections::HashSet::new();
        let mut edges = Vec::new();
        for tri in self.indices.chunks_exact(3) {
            for &(a, b) in &[(tri[0], tri[1]), (tri[1], tri[2]), (tri[2], tri[0])] {
                let key = (a.min(b), a.max(b));
                if seen.insert(key) {
                    edges.push(key.0);
                    edges.push(key.1);
                }
            }
        }
        edges
    }

    /// Nearest ray/hull intersection (Möller–Trumbore, two-sided to
    /// match the renderer's no-cull, two-sided shading). `dir` need not
    /// be unit; only positive-`t` hits count. Returns the closest
    /// triangle, its nearest vertex to the hit point, and the `MVG2`
    /// scalar there if any. `None` if the ray misses the hull.
    #[must_use]
    pub fn pick(&self, origin: Vec3, dir: Vec3) -> Option<Pick> {
        let eps = 1e-7_f32;
        let mut best: Option<Pick> = None;
        for (t_i, tri) in self.indices.chunks_exact(3).enumerate() {
            let (i0, i1, i2) = (tri[0] as usize, tri[1] as usize, tri[2] as usize);
            let v0 = Vec3::from(self.positions[i0]);
            let v1 = Vec3::from(self.positions[i1]);
            let v2 = Vec3::from(self.positions[i2]);
            let e1 = v1 - v0;
            let e2 = v2 - v0;
            let p = dir.cross(e2);
            let det = e1.dot(p);
            if det.abs() < eps {
                continue;
            }
            let inv = 1.0 / det;
            let tvec = origin - v0;
            let u = tvec.dot(p) * inv;
            if !(0.0..=1.0).contains(&u) {
                continue;
            }
            let q = tvec.cross(e1);
            let v = dir.dot(q) * inv;
            if v < 0.0 || u + v > 1.0 {
                continue;
            }
            let t = e2.dot(q) * inv;
            if t <= eps {
                continue;
            }
            if best.is_none_or(|b| t < b.distance) {
                let hit = origin + dir * t;
                // Nearest of the three triangle corners to the hit.
                let node = [(i0, v0), (i1, v1), (i2, v2)]
                    .into_iter()
                    .min_by(|a, b| {
                        (a.1 - hit)
                            .length_squared()
                            .total_cmp(&(b.1 - hit).length_squared())
                    })
                    .map(|(idx, _)| idx)
                    .unwrap_or(i0);
                best = Some(Pick {
                    tri: t_i,
                    node,
                    distance: t,
                    point: hit.to_array(),
                    scalar: self.scalars.as_ref().map(|s| s[node]),
                });
            }
        }
        best
    }

    /// Axis-aligned bounding box `(min, max)` of the vertex cloud at
    /// the current state — the input to the projected-bbox overlay
    /// (it deforms per state because the hull does). Empty hull → a
    /// unit box at the origin.
    #[must_use]
    pub fn aabb(&self) -> ([f32; 3], [f32; 3]) {
        if self.positions.is_empty() {
            return ([-0.5; 3], [0.5; 3]);
        }
        let mut lo = Vec3::splat(f32::INFINITY);
        let mut hi = Vec3::splat(f32::NEG_INFINITY);
        for p in &self.positions {
            let p = Vec3::from(*p);
            lo = lo.min(p);
            hi = hi.max(p);
        }
        (lo.to_array(), hi.to_array())
    }

    /// Bounding-sphere `(center, radius)` of the vertex cloud — the
    /// input to [`crate::Camera::looking_at`] so the gating render
    /// frames a real corpus regardless of its coordinate scale.
    #[must_use]
    pub fn bounds(&self) -> (Vec3, f32) {
        if self.positions.is_empty() {
            return (Vec3::ZERO, 1.0);
        }
        let mut lo = Vec3::splat(f32::INFINITY);
        let mut hi = Vec3::splat(f32::NEG_INFINITY);
        for p in &self.positions {
            let p = Vec3::from(*p);
            lo = lo.min(p);
            hi = hi.max(p);
        }
        let center = (lo + hi) * 0.5;
        let radius = (hi - center).length().max(1e-3);
        (center, radius)
    }
}
