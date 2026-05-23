//! Gating test — [`RenderMode::FeatureEdges`] (dihedral-angle
//! "geometry-only" wireframe).
//!
//! Two halves, the `vb003_render_modes.rs` shape:
//!  * always-on pure logic — [`Mesh::compute_feature_edges`] over
//!    synthetic geometry (cube / quad / pyramid / 24-side cylinder
//!    wall / sphere-icosa). Pinned to the dihedral-angle definition in
//!    `planning/mili-viz/feature-edges.md` Decision 100.
//!  * `feature_edges_mode_differs_from_edges_mode` — headless composite
//!    render: same mesh in `Shaded`, `Edges`, `FeatureEdges` must give
//!    three byte-distinct images and `Shaded` is unchanged from
//!    `render_mesh_to_image` (VB-001 byte-stability gate). Skip-on-
//!    absent when no `wgpu` adapter.

use mili_viz_client::{
    render_mesh_to_image, render_mesh_to_image_with_mode, Camera, Mesh, RenderMode,
};

/// 30° in radians — the renderer's `FEATURE_EDGE_ANGLE_DEG` default.
/// Mirrored here so the unit tests pin behaviour at the same threshold
/// the renderer ships with; a future Preferences slider will widen the
/// matrix but keep 30° as the canonical anchor.
const DEFAULT_THRESHOLD: f32 = std::f32::consts::FRAC_PI_6;

/// Decode an undirected edge buffer (`u32` pairs canonicalised low→high
/// during compute) into a sorted, deduped `(u32, u32)` set so the
/// assertions don't depend on the sort key inside
/// `compute_feature_edges`.
fn edge_set(edges: &[u32]) -> std::collections::BTreeSet<(u32, u32)> {
    edges
        .chunks_exact(2)
        .map(|p| (p[0].min(p[1]), p[0].max(p[1])))
        .collect()
}

#[test]
fn coplanar_quad_keeps_boundary_drops_diagonal() {
    // A flat unit square triangulated 0-1-2, 0-2-3. The 0–2 diagonal
    // has dihedral 0° (both tris coplanar) and must drop; the four
    // outer edges have only one adjacent tri and stay as boundary.
    let quad = Mesh {
        positions: vec![
            [0.0, 0.0, 0.0],
            [1.0, 0.0, 0.0],
            [1.0, 1.0, 0.0],
            [0.0, 1.0, 0.0],
        ],
        normals: vec![[0.0, 0.0, 1.0]; 4],
        indices: vec![0, 1, 2, 0, 2, 3],
        scalars: None,
        element_edges: None,
        tri_flags: None,
    };
    let feats = edge_set(&quad.compute_feature_edges(DEFAULT_THRESHOLD));
    let want: std::collections::BTreeSet<(u32, u32)> =
        [(0, 1), (1, 2), (2, 3), (0, 3)].into_iter().collect();
    assert_eq!(
        feats, want,
        "coplanar diagonal must drop, boundary must stay"
    );
}

/// Build a unit-cube surface mesh with each face triangulated into 2
/// triangles (12 tris, 24 directed face-edges, 18 unique undirected
/// edges = 12 outer cube + 6 face-diagonals).
fn cube_mesh() -> Mesh {
    // Vertices of a unit cube centred at the origin.
    let p = [
        [-0.5, -0.5, -0.5],
        [0.5, -0.5, -0.5],
        [0.5, 0.5, -0.5],
        [-0.5, 0.5, -0.5],
        [-0.5, -0.5, 0.5],
        [0.5, -0.5, 0.5],
        [0.5, 0.5, 0.5],
        [-0.5, 0.5, 0.5],
    ];
    // Each face as two CCW triangles seen from outside.
    let f = [
        // -Z (looking from +Z it's CW; from outside (-Z dir) it's CCW)
        [0u32, 3, 2],
        [0, 2, 1],
        // +Z
        [4, 5, 6],
        [4, 6, 7],
        // -Y
        [0, 1, 5],
        [0, 5, 4],
        // +Y
        [3, 7, 6],
        [3, 6, 2],
        // -X
        [0, 4, 7],
        [0, 7, 3],
        // +X
        [1, 2, 6],
        [1, 6, 5],
    ];
    let indices: Vec<u32> = f.into_iter().flatten().collect();
    let positions = p.to_vec();
    let normals = vec![[0.0, 0.0, 1.0]; positions.len()];
    Mesh {
        positions,
        normals,
        indices,
        scalars: None,
        element_edges: None,
        tri_flags: None,
    }
}

#[test]
fn cube_keeps_12_outer_edges_drops_6_face_diagonals() {
    let cube = cube_mesh();
    // Full element-edge extractor sees all 18 undirected edges (12
    // cube + 6 face diagonals). Compare against the feature extractor
    // — feature must be exactly the 12-edge cube skeleton.
    assert_eq!(
        cube.edge_indices().len(),
        18 * 2,
        "cube has 18 unique tri-edges"
    );

    let feats = cube.compute_feature_edges(DEFAULT_THRESHOLD);
    assert_eq!(
        feats.len(),
        12 * 2,
        "cube must reduce to its 12 outer edges (silhouette + creases)"
    );
    let set = edge_set(&feats);
    // Each kept edge connects two cube corners that share exactly one
    // axis (Hamming-1 in their {±} coordinates). The 6 dropped
    // diagonals connect corners differing in two axes (the face
    // diagonals of a cube).
    let coord = |i: u32| {
        let p = cube.positions[i as usize];
        [
            p[0].is_sign_positive(),
            p[1].is_sign_positive(),
            p[2].is_sign_positive(),
        ]
    };
    for (a, b) in set {
        let (ca, cb) = (coord(a), coord(b));
        let diffs = (0..3).filter(|&k| ca[k] != cb[k]).count();
        assert_eq!(
            diffs, 1,
            "feature edge {a}-{b} should differ in exactly 1 axis (cube edge), got {diffs}"
        );
    }
}

#[test]
#[allow(clippy::many_single_char_names)]
fn cylinder_wall_at_30deg_keeps_zero_lateral_edges() {
    // A 24-sided cylinder side wall (no caps): 24 quads triangulated
    // into 48 triangles. Each quad subdivision edge between two
    // adjacent rectangles folds by 360°/24 = 15°, well below the 30°
    // threshold — so all 24 lateral subdivision edges must drop. The
    // top and bottom rims are open boundary edges (only one adjacent
    // tri) — they stay as feature.
    let n: u32 = 24;
    let mut positions = Vec::with_capacity(2 * n as usize);
    // `k as f32` and `n as f32` are exact for the small ring counts the
    // test uses; the clippy::cast_precision_loss warning is a
    // false-positive here.
    #[allow(clippy::cast_precision_loss)]
    for k in 0..n {
        let theta = std::f32::consts::TAU * k as f32 / n as f32;
        positions.push([theta.cos(), 0.0, theta.sin()]); // bottom ring
    }
    #[allow(clippy::cast_precision_loss)]
    for k in 0..n {
        let theta = std::f32::consts::TAU * k as f32 / n as f32;
        positions.push([theta.cos(), 1.0, theta.sin()]); // top ring
    }
    let mut indices = Vec::with_capacity(6 * n as usize);
    for k in 0..n {
        let a = k;
        let b = (k + 1) % n;
        let c = a + n; // top of a
        let d = b + n; // top of b
                       // CCW outward (normal points away from cylinder axis).
        indices.extend_from_slice(&[a, b, d]);
        indices.extend_from_slice(&[a, d, c]);
    }
    let mesh = Mesh {
        positions,
        normals: vec![[0.0, 0.0, 1.0]; 2 * n as usize],
        indices,
        scalars: None,
        element_edges: None,
        tri_flags: None,
    };
    let feats = mesh.compute_feature_edges(DEFAULT_THRESHOLD);
    // Boundary edges only: 2N (top rim + bottom rim).
    assert_eq!(
        feats.len(),
        2 * 2 * n as usize,
        "cylinder N={n} side wall: only the 2N rim edges must stay; got {} edges",
        feats.len() / 2
    );
    // Sanity: every kept edge is within a single ring (bottom: both
    // indices < n; top: both ≥ n). A surviving lateral edge would
    // straddle the n boundary.
    for p in feats.chunks_exact(2) {
        let same_ring = (p[0] < n && p[1] < n) || (p[0] >= n && p[1] >= n);
        assert!(
            same_ring,
            "feature edge {}-{} straddles rings (lateral edge survived dihedral filter)",
            p[0], p[1]
        );
    }
}

#[test]
fn icosahedron_below_threshold_drops_all_edges() {
    // Regular icosahedron: dihedral angle ≈ 138.19° between adjacent
    // faces → 180° − 138.19° = 41.8° fold. That's > 30°, so the
    // icosahedron *does* still have feature edges at the default
    // threshold (it looks faceted, correctly). Verify it: every edge
    // is a feature.
    // Then with a 50° threshold it should disappear (≥ 41.8° fold, but
    // 50° > 41.8° → not a feature). This pair of assertions pins the
    // monotone-threshold semantics.
    let phi = (1.0_f32 + 5.0_f32.sqrt()) * 0.5;
    let v = [
        [-1.0, phi, 0.0],
        [1.0, phi, 0.0],
        [-1.0, -phi, 0.0],
        [1.0, -phi, 0.0],
        [0.0, -1.0, phi],
        [0.0, 1.0, phi],
        [0.0, -1.0, -phi],
        [0.0, 1.0, -phi],
        [phi, 0.0, -1.0],
        [phi, 0.0, 1.0],
        [-phi, 0.0, -1.0],
        [-phi, 0.0, 1.0],
    ];
    let tris: [[u32; 3]; 20] = [
        [0, 11, 5],
        [0, 5, 1],
        [0, 1, 7],
        [0, 7, 10],
        [0, 10, 11],
        [1, 5, 9],
        [5, 11, 4],
        [11, 10, 2],
        [10, 7, 6],
        [7, 1, 8],
        [3, 9, 4],
        [3, 4, 2],
        [3, 2, 6],
        [3, 6, 8],
        [3, 8, 9],
        [4, 9, 5],
        [2, 4, 11],
        [6, 2, 10],
        [8, 6, 7],
        [9, 8, 1],
    ];
    let mesh = Mesh {
        positions: v.to_vec(),
        normals: vec![[0.0, 0.0, 1.0]; 12],
        indices: tris.into_iter().flatten().collect(),
        scalars: None,
        element_edges: None,
        tri_flags: None,
    };
    let n_unique = mesh.edge_indices().len() / 2;
    assert_eq!(n_unique, 30, "icosahedron has 30 edges");

    // 30° threshold: icosa dihedral fold ≈ 41.8° > 30° ⇒ all 30 edges kept.
    let kept_30 = mesh.compute_feature_edges(DEFAULT_THRESHOLD).len() / 2;
    assert_eq!(kept_30, 30, "@30°: icosa is faceted, all edges feature");

    // 50° threshold: 41.8° < 50° ⇒ no edges kept (icosa "smooths out").
    let kept_50 = mesh.compute_feature_edges(50.0_f32.to_radians()).len() / 2;
    assert_eq!(kept_50, 0, "@50°: icosa fold below threshold, all drop");
}

#[test]
fn empty_mesh_compute_returns_empty() {
    let m = Mesh {
        positions: vec![],
        normals: vec![],
        indices: vec![],
        scalars: None,
        element_edges: None,
        tri_flags: None,
    };
    assert!(m.compute_feature_edges(DEFAULT_THRESHOLD).is_empty());
}

#[test]
fn feature_edges_mode_differs_from_other_modes() {
    // Use the cube — its feature set (12 edges) is a strict subset of
    // its element-edge set (18 edges), so the FeatureEdges raster must
    // differ from Edges. Skip-on-absent when no `wgpu` adapter.
    let mesh = cube_mesh();
    let (center, radius) = mesh.bounds();
    let camera = Camera::looking_at(center, radius);
    let (w, h) = (160u32, 160u32);

    let Some(shaded) = render_mesh_to_image_with_mode(w, h, &camera, &mesh, RenderMode::Shaded)
    else {
        eprintln!("skip: no wgpu adapter (skip-on-absent per CLAUDE.md)");
        return;
    };

    // Byte-stable invariant (VB-001): Shaded path is untouched by the
    // new mode / new MeshBuffers field.
    let baseline = render_mesh_to_image(w, h, &camera, &mesh).expect("adapter already proven");
    assert_eq!(shaded, baseline, "Shaded must not change the M2/M3 pass");

    let edges = render_mesh_to_image_with_mode(w, h, &camera, &mesh, RenderMode::Edges)
        .expect("adapter already proven");
    let feat = render_mesh_to_image_with_mode(w, h, &camera, &mesh, RenderMode::FeatureEdges)
        .expect("adapter already proven");

    assert_ne!(feat, shaded, "FeatureEdges overlay must change pixels");
    assert_ne!(
        feat, edges,
        "FeatureEdges (12 edges) must differ from Edges (18 edges, includes face diagonals)"
    );
}
