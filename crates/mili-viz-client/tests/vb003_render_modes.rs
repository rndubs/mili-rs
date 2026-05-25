//! VB-003 gating test — mesh / element-edge render modes.
//!
//! Two halves, the `m4_view_manipulation.rs` shape:
//!  * always-on pure logic — [`RenderMode`], the [`ShellState`]
//!    render-mode switch, and [`Mesh::edge_indices`] unique-edge
//!    extraction. A no-GPU CI box hard-gates these.
//!  * `render_modes_differ` — headless: render the same mesh `Shaded`,
//!    `Edges`, `Wireframe` and assert the default `Shaded` path is
//!    byte-identical to `render_mesh_to_image` (the M3 byte-stable
//!    invariant, VB-001) while the edge/wireframe passes change pixels.
//!    **Skip-on-absent** when no `wgpu` adapter (CLAUDE.md convention).

use mili_viz_client::{
    build_shell_ui, render_mesh_to_image, render_mesh_to_image_with_mode, Camera, Mesh, RenderMode,
    ShellState, UiAction,
};

#[test]
fn render_mode_default_is_shaded_and_labels_are_distinct() {
    assert_eq!(RenderMode::default(), RenderMode::Shaded);
    let labels = [
        RenderMode::Shaded.label(),
        RenderMode::Edges.label(),
        RenderMode::Wireframe.label(),
    ];
    // All three labels are distinct (the menu rows must be readable).
    assert_eq!(
        labels
            .iter()
            .collect::<std::collections::HashSet<_>>()
            .len(),
        3
    );
}

#[test]
fn shell_state_render_mode_switch_is_pure_and_observable() {
    let mut s = ShellState::default();
    assert_eq!(s.render_mode, RenderMode::Shaded, "default is the M3 pass");

    let a = s.set_render_mode(RenderMode::Wireframe);
    assert_eq!(s.render_mode, RenderMode::Wireframe, "state mutated");
    assert_eq!(
        a,
        UiAction::SetRenderMode(RenderMode::Wireframe),
        "returned for observability; no proto command"
    );

    // The pure layout still runs head­lessly with the new mode and,
    // with no pointer input, emits no actions (the m3/m4 pattern).
    let ctx = egui::Context::default();
    let raw = egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(
            egui::Pos2::ZERO,
            egui::vec2(900.0, 600.0),
        )),
        ..Default::default()
    };
    let mut actions = Vec::new();
    let out = ctx.run_ui(raw, |ui| actions = build_shell_ui(ui, &mut s));
    assert!(actions.is_empty(), "no input ⇒ no actions: {actions:?}");
    assert!(!out.shapes.is_empty(), "the L1 shell must still paint");
}

#[test]
fn edge_indices_are_unique_undirected() {
    // A single triangle has exactly 3 edges.
    let tri = Mesh::unit_triangle();
    assert_eq!(tri.edge_indices().len(), 3 * 2, "3 edges as index pairs");

    // Two triangles sharing the 0–2 diagonal: 5 unique edges, not 6.
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
        tri_member_id: None,
    };
    let e = quad.edge_indices();
    assert_eq!(e.len(), 5 * 2, "shared diagonal is deduped: {e:?}");
    // Every pair is stored low→high (undirected canonical form).
    for p in e.chunks_exact(2) {
        assert!(p[0] < p[1], "edge {p:?} not canonical low→high");
    }
}

#[test]
fn render_modes_differ() {
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
        tri_member_id: None,
    };
    let (center, radius) = mesh.bounds();
    let camera = Camera::looking_at(center, radius);
    let (w, h) = (160u32, 160u32);

    let Some(shaded) = render_mesh_to_image_with_mode(w, h, &camera, &mesh, RenderMode::Shaded)
    else {
        eprintln!("skip: no wgpu adapter (skip-on-absent per CLAUDE.md)");
        return;
    };

    // Byte-stable invariant: the default Shaded path is identical to
    // the unchanged `render_mesh_to_image` (VB-001 / status 23).
    let baseline = render_mesh_to_image(w, h, &camera, &mesh).expect("adapter already proven");
    assert_eq!(shaded, baseline, "Shaded must not change the M2/M3 pass");

    let edges = render_mesh_to_image_with_mode(w, h, &camera, &mesh, RenderMode::Edges)
        .expect("adapter already proven");
    let wire = render_mesh_to_image_with_mode(w, h, &camera, &mesh, RenderMode::Wireframe)
        .expect("adapter already proven");

    assert_ne!(edges, shaded, "the edge overlay must change pixels");
    assert_ne!(wire, shaded, "the wireframe must change pixels");
    assert_ne!(wire, edges, "wireframe (no fill) ≠ filled+edges");
}
