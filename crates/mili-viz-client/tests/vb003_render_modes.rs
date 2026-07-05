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

/// Rendering-quality regression: the wireframe must be *visible* over
/// the dark clear colour. The original pass drew hard-coded opaque
/// black lines (`edges.wgsl`), which over the near-black background
/// rendered an invisible model; the fill-less modes now use a light
/// edge colour.
#[test]
fn wireframe_is_visible_on_dark_background() {
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
    let Some(px) = render_mesh_to_image_with_mode(w, h, &camera, &mesh, RenderMode::Wireframe)
    else {
        eprintln!("skip: no wgpu adapter (skip-on-absent per CLAUDE.md)");
        return;
    };
    // The clear colour maxes out at ~20/255; light edge pixels clear
    // 100 comfortably. Black edges (the regression) leave zero.
    let bright = px
        .chunks_exact(4)
        .filter(|c| c[0].max(c[1]).max(c[2]) > 100)
        .count();
    assert!(
        bright > 50,
        "wireframe edges must be light over the dark background, got \
         {bright} bright pixels — black-on-near-black regression?"
    );
}

/// Rendering-quality regression: on a mesh whose elements are only a
/// few pixels on screen, the `Edges` overlay must dissolve into the
/// fill (projected-length density fade + sub-1 strength) instead of
/// blacking the model out. The original pass drew every edge at full
/// opaque black — a dense hull rendered as a black mass.
#[test]
fn dense_edge_overlay_keeps_the_fill_visible() {
    // A 64×64-cell unit grid: at 160×160 the framed cells are ~2 px —
    // far below the fade-in threshold.
    let n = 64usize;
    let mut positions = Vec::new();
    for j in 0..=n {
        for i in 0..=n {
            positions.push([
                i as f32 / n as f32 * 2.0 - 1.0,
                j as f32 / n as f32 * 2.0 - 1.0,
                0.0,
            ]);
        }
    }
    let mut indices = Vec::new();
    for j in 0..n {
        for i in 0..n {
            let a = (j * (n + 1) + i) as u32;
            let b = a + 1;
            let c = a + (n + 1) as u32;
            let d = c + 1;
            indices.extend_from_slice(&[a, b, d, a, d, c]);
        }
    }
    let count = positions.len();
    let mesh = Mesh {
        positions,
        normals: vec![[0.0, 0.0, 1.0]; count],
        indices,
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
    let edges = render_mesh_to_image_with_mode(w, h, &camera, &mesh, RenderMode::Edges)
        .expect("adapter already proven");

    let mean_luma = |px: &[u8]| {
        let sum: u64 = px
            .chunks_exact(4)
            .map(|c| u64::from(c[0].max(c[1]).max(c[2])))
            .sum();
        sum as f64 / (px.len() / 4) as f64
    };
    let ls = mean_luma(&shaded);
    let le = mean_luma(&edges);
    assert!(
        le > 0.5 * ls,
        "dense edges must not black out the fill: Edges mean luminance \
         {le:.1} vs Shaded {ls:.1}"
    );
}
