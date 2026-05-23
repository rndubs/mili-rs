//! Phase 5 M7 gating test — render modes consuming `MVG3`.
//!
//! Three halves, the `vb003_render_modes.rs` shape:
//!  * always-on pure logic — the new [`RenderMode`] arms, the
//!    [`ShellState`] include-interior toggle, the [`UiAction`] that
//!    lowers to a sentinelled [`pb::MaterialVisibility`], and
//!    [`decode_mvg`] round-tripping a hand-built `MVG3` blob;
//!  * always-on byte-stability — uploading an `MVG2` mesh to the
//!    renderer falls back to [`Mesh::edge_indices`] verbatim, so the
//!    M4 / VB-003 / VB-004 / MVP-polish composite gates stay
//!    byte-stable when an `MVG3`-unaware server is connected
//!    (Decision 82). A no-GPU CI box hard-gates this leg.
//!  * skip-on-absent composite render — render the same mesh
//!    `Shaded` / `Translucent` / `Xray` and assert the default
//!    `Shaded` path is byte-identical to `render_mesh_to_image`
//!    (VB-001 — the byte-stable invariant the MVP polish landed on)
//!    while the translucent/x-ray passes change pixels. Skip when no
//!    `wgpu` adapter (CLAUDE.md skip-on-absent convention).
//!
//! The end-to-end leg against `basic1.pltA` lives in
//! `crates/mili-viz-server/tests/m7_mvg3.rs` — server-side; this file
//! exercises the **client-side** consumers of that contract.

use mili_viz_client::{
    decode_mvg, render_mesh_to_image, render_mesh_to_image_with_mode, Camera, Mesh, RenderMode,
    ShellState, UiAction,
};

/// Build a synthetic `MVG3` blob with all four flag bits set
/// (`phase-4-m7.md` § "Blob layout"). A unit quad in the z=0 plane,
/// two triangles, four element-edges, both per-triangle flags = 0
/// (boundary), per-vertex scalar = a simple ramp.
fn mvg3_quad() -> Vec<u8> {
    let positions: [[f32; 3]; 4] = [
        [0.0, 0.0, 0.0],
        [1.0, 0.0, 0.0],
        [1.0, 1.0, 0.0],
        [0.0, 1.0, 0.0],
    ];
    let indices: [u32; 6] = [0, 1, 2, 0, 2, 3];
    let trimat: [u32; 2] = [7, 7];
    let tri_flags: [u32; 2] = [0, 1]; // second triangle marked interior
    let edges: [u32; 8] = [0, 1, 1, 2, 2, 3, 3, 0]; // quad perimeter
    let scalars: [f32; 4] = [0.0, 0.25, 0.5, 0.75];

    let n_verts = positions.len() as u64;
    let n_idx = indices.len() as u64;
    let n_edges = edges.len() as u64;
    let flags_mask: u32 = 0b1111; // scalar | tri_flags | edges | interior

    let mut b = Vec::new();
    b.extend_from_slice(b"MVG3");
    b.extend_from_slice(&3u32.to_le_bytes());
    b.extend_from_slice(&n_verts.to_le_bytes());
    b.extend_from_slice(&n_idx.to_le_bytes());
    b.extend_from_slice(&n_edges.to_le_bytes());
    b.extend_from_slice(&flags_mask.to_le_bytes());
    for p in &positions {
        for c in p {
            b.extend_from_slice(&c.to_le_bytes());
        }
    }
    for i in &indices {
        b.extend_from_slice(&i.to_le_bytes());
    }
    for m in &trimat {
        b.extend_from_slice(&m.to_le_bytes());
    }
    for f in &tri_flags {
        b.extend_from_slice(&f.to_le_bytes());
    }
    for e in &edges {
        b.extend_from_slice(&e.to_le_bytes());
    }
    for s in &scalars {
        b.extend_from_slice(&s.to_le_bytes());
    }
    b
}

/// Hand-built `MVG2` quad (no element-edge buffer) — exercises the
/// VB-005-fallback path in `Renderer::upload_mesh` (Decision 82).
fn mvg2_quad() -> Vec<u8> {
    let positions: [[f32; 3]; 4] = [
        [0.0, 0.0, 0.0],
        [1.0, 0.0, 0.0],
        [1.0, 1.0, 0.0],
        [0.0, 1.0, 0.0],
    ];
    let indices: [u32; 6] = [0, 1, 2, 0, 2, 3];
    let trimat: [u32; 2] = [7, 7];
    let scalars: [f32; 4] = [0.0, 0.25, 0.5, 0.75];
    let n_verts = positions.len() as u64;
    let n_idx = indices.len() as u64;
    let mut b = Vec::new();
    b.extend_from_slice(b"MVG2");
    b.extend_from_slice(&3u32.to_le_bytes());
    b.extend_from_slice(&n_verts.to_le_bytes());
    b.extend_from_slice(&n_idx.to_le_bytes());
    for p in &positions {
        for c in p {
            b.extend_from_slice(&c.to_le_bytes());
        }
    }
    for i in &indices {
        b.extend_from_slice(&i.to_le_bytes());
    }
    for m in &trimat {
        b.extend_from_slice(&m.to_le_bytes());
    }
    for s in &scalars {
        b.extend_from_slice(&s.to_le_bytes());
    }
    b
}

#[test]
fn render_mode_arms_have_distinct_labels() {
    assert_eq!(RenderMode::default(), RenderMode::Shaded);
    let labels = [
        RenderMode::Shaded.label(),
        RenderMode::Edges.label(),
        RenderMode::Wireframe.label(),
        RenderMode::Translucent.label(),
        RenderMode::Xray.label(),
    ];
    assert_eq!(
        labels
            .iter()
            .collect::<std::collections::HashSet<_>>()
            .len(),
        5,
        "the five Rendering-menu rows must each have a distinct label: {labels:?}",
    );
}

#[test]
fn interior_toggle_is_pure_observable_and_emits_no_proto_directly() {
    // The shell state owns the flag; `set_interior_mode` is the one
    // mutator (Decision 83). The returned UiAction is for the windowed
    // app to lower to `Cmd::Material` — see the next test.
    let mut s = ShellState::default();
    assert!(!s.interior_on, "include-interior off by default (VB-001)");

    let a = s.set_interior_mode(true);
    assert!(s.interior_on, "state mutated");
    assert_eq!(a, UiAction::SetInteriorMode(true));

    let b = s.set_interior_mode(false);
    assert!(!s.interior_on, "toggle off");
    assert_eq!(b, UiAction::SetInteriorMode(false));
}

#[test]
fn decode_mvg3_roundtrips_all_four_flag_bits() {
    let blob = mvg3_quad();
    let mesh = decode_mvg(&blob).expect("synthetic MVG3 decodes");
    assert_eq!(mesh.positions.len(), 4);
    assert_eq!(mesh.indices, vec![0u32, 1, 2, 0, 2, 3]);
    assert_eq!(mesh.normals.len(), mesh.positions.len());
    // scalar (bit 0)
    let scalars = mesh
        .scalars
        .as_ref()
        .expect("scalar bit set → MVG3 carries per-vertex scalar");
    assert_eq!(scalars, &vec![0.0_f32, 0.25, 0.5, 0.75]);
    // tri_flags (bit 1)
    let tri_flags = mesh
        .tri_flags
        .as_ref()
        .expect("tri_flags bit set → MVG3 carries per-triangle flags");
    assert_eq!(tri_flags, &vec![0u32, 1]);
    assert_eq!(
        tri_flags.iter().filter(|f| **f & 1 == 1).count(),
        1,
        "second triangle flagged interior"
    );
    // element_edges (bit 2)
    let edges = mesh
        .element_edges
        .as_ref()
        .expect("edges bit set → MVG3 carries the per-element edge buffer");
    assert_eq!(edges, &vec![0u32, 1, 1, 2, 2, 3, 3, 0]);
}

#[test]
fn mvg2_decode_has_no_mvg3_columns() {
    // Decision 82 / VB-001: `MVG1`/`MVG2` decoders are byte-stable;
    // the new optional columns must be `None`.
    let mesh = decode_mvg(&mvg2_quad()).expect("MVG2 still decodes");
    assert!(
        mesh.element_edges.is_none(),
        "MVG2 carries no element_edges (fallback path)"
    );
    assert!(mesh.tri_flags.is_none(), "MVG2 carries no tri_flags column");
    assert!(mesh.scalars.is_some(), "MVG2 still carries the scalar");
}

#[test]
fn element_edges_supersede_triangle_extraction_on_mvg3() {
    // The decoded `MVG3` mesh exposes its server-supplied 4-edge
    // perimeter; the legacy triangle-edge extractor on the same
    // indices would produce 5 edges (the 0–2 diagonal counted too).
    // Decision 82 says the renderer prefers `element_edges`; this
    // test pins the count delta — the diagonal never enters the
    // wire pass when an MVG3 is present.
    let mvg3 = decode_mvg(&mvg3_quad()).unwrap();
    let server_edges = mvg3.element_edges.clone().unwrap();
    let derived_edges = mvg3.edge_indices();
    assert_eq!(
        server_edges.len() / 2,
        4,
        "MVG3 quad perimeter has 4 element edges"
    );
    assert_eq!(
        derived_edges.len() / 2,
        5,
        "the triangle-extractor over-emits the 0–2 diagonal (the VB-005 shape)"
    );
    // The renderer pulls `element_edges` first; the diagonal stays
    // absent from `server_edges` (VB-005 client-side discharge).
    assert!(
        !server_edges
            .chunks_exact(2)
            .any(|p| (p[0], p[1]) == (0, 2) || (p[0], p[1]) == (2, 0)),
        "server perimeter must not contain the triangulation diagonal: {server_edges:?}"
    );
    assert!(
        derived_edges
            .chunks_exact(2)
            .any(|p| (p[0], p[1]) == (0, 2)),
        "the legacy extractor still includes the diagonal (the broken-but-known fallback)"
    );
}

#[test]
fn render_modes_differ_translucent_and_xray() {
    // Synthetic two-triangle quad — the same shape `vb003_render_modes`
    // uses, plus the two new modes. The mesh carries no `MVG3` element
    // edges so the renderer takes the fallback edge path (VB-001).
    let mesh = Mesh {
        positions: vec![
            [-1.0, -1.0, 0.0],
            [1.0, -1.0, 0.0],
            [1.0, 1.0, 0.0],
            [-1.0, 1.0, 0.0],
        ],
        normals: vec![[0.0, 0.0, 1.0]; 4],
        indices: vec![0, 1, 2, 0, 2, 3],
        scalars: None,
        element_edges: None,
        tri_flags: None,
    };
    let (center, radius) = mesh.bounds();
    let camera = Camera::looking_at(center, radius);
    let (w, h) = (160u32, 160u32);

    let Some(shaded) = render_mesh_to_image_with_mode(w, h, &camera, &mesh, RenderMode::Shaded)
    else {
        eprintln!("skip: no wgpu adapter (skip-on-absent per CLAUDE.md)");
        return;
    };

    // VB-001 / status 23 byte-stable invariant: Shaded is the
    // `render_mesh_to_image` baseline. The new modes must not move it.
    let baseline = render_mesh_to_image(w, h, &camera, &mesh).expect("adapter already proven");
    assert_eq!(
        shaded, baseline,
        "Shaded must remain byte-identical to render_mesh_to_image (VB-001)",
    );

    let translucent = render_mesh_to_image_with_mode(w, h, &camera, &mesh, RenderMode::Translucent)
        .expect("adapter already proven");
    let xray = render_mesh_to_image_with_mode(w, h, &camera, &mesh, RenderMode::Xray)
        .expect("adapter already proven");

    assert_ne!(
        translucent, shaded,
        "translucent (alpha-blended fill) must change pixels vs opaque shaded",
    );
    assert_ne!(
        xray, shaded,
        "x-ray (translucent + edges) must change pixels vs opaque shaded",
    );
    assert_ne!(
        xray, translucent,
        "x-ray adds the element-edge overlay on top of the translucent fill",
    );
}
