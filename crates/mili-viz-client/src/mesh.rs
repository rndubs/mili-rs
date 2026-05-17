//! Decode the self-describing server geometry blob into a renderer
//! `Mesh` (`phase-5-m2.md` Decision 42).
//!
//! The byte layout is `phase-4-m2.md` Decision 11
//! (`MVG1:verts_f32x3+idx_u32+trimat_u32`); the M3 superset
//! (`MVG2:...+scalar_f32`) is tolerated by **ignoring** the trailing
//! per-vertex scalar — scalar→color is Phase 5 M3, M2 draws the bare
//! hull. Per-vertex normals are computed on the CPU so the hull reads
//! as a 3-D surface, not a flat silhouette.

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

/// Decode an `MVG1`/`MVG2` blob (`phase-4-m2.md` Decision 11). The
/// optional `MVG2` trailing scalar is parsed past but discarded
/// (M2 draws the bare hull; scalar→color is M3).
///
/// # Errors
/// Returns [`DecodeError`] if the magic is unknown or the buffer is
/// truncated relative to its self-described counts.
pub fn decode_mvg(blob: &[u8]) -> Result<Mesh, DecodeError> {
    if blob.len() < 24 {
        return Err(DecodeError("blob shorter than the 24-byte header".into()));
    }
    let magic = &blob[0..4];
    let with_scalar = match magic {
        b"MVG1" => false,
        b"MVG2" => true,
        _ => {
            return Err(DecodeError(format!(
                "unknown magic {:?} (expected MVG1/MVG2)",
                String::from_utf8_lossy(magic)
            )))
        }
    };
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
    // trimat + (MVG2) scalar are intentionally not read — M2 hull only.

    let normals = compute_normals(&positions, &indices);
    Ok(Mesh {
        positions,
        normals,
        indices,
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
        }
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
