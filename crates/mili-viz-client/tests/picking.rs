//! Client-side picking gating test (wireframe-parity "Picking" /
//! MVP-cut 4). All **always-on** pure logic — picking is GPU-free
//! ray-cast against the cached hull; the windowed click path is not
//! headlessly verifiable in CI (no display), so the ray math
//! ([`Camera::ray_from_screen`]), the hull intersection
//! ([`Mesh::pick`]) and the [`ShellState`] readout are tested
//! directly, mirroring the m4 pattern.

use mili_viz_client::{build_shell_ui, Camera, Mesh, ShellState, UiAction};

/// A 2×2 quad in the z=0 plane, facing -Z (so a camera on +Z sees it).
fn quad() -> Mesh {
    Mesh {
        positions: vec![
            [-1.0, -1.0, 0.0],
            [1.0, -1.0, 0.0],
            [1.0, 1.0, 0.0],
            [-1.0, 1.0, 0.0],
        ],
        normals: vec![[0.0, 0.0, 1.0]; 4],
        indices: vec![0, 1, 2, 0, 2, 3],
        scalars: Some(vec![10.0, 20.0, 30.0, 40.0]),
    }
}

#[test]
fn ray_through_viewport_centre_hits_the_framed_hull() {
    let mesh = quad();
    let (center, radius) = mesh.bounds();
    let camera = Camera::looking_at(center, radius);
    let (w, h) = (200u32, 200u32);

    // The centre pixel of a framed model must hit it.
    let (o, d) = camera.ray_from_screen(100.0, 100.0, w, h);
    let hit = mesh.pick(o, d).expect("centre ray hits the framed quad");
    assert!(hit.distance > 0.0, "a forward hit");
    // The hit point is on the z=0 plane the quad lives in.
    assert!(hit.point[2].abs() < 1e-3, "hit on the quad plane: {hit:?}");
    assert!(hit.scalar.is_some(), "MVG2 scalar carried through");

    // A ray out at a far corner misses the hull entirely.
    let (o2, d2) = camera.ray_from_screen(1.0, 1.0, w, h);
    assert!(
        mesh.pick(o2, d2).is_none(),
        "a corner ray misses the centred quad"
    );
}

#[test]
fn pick_reports_nearest_node_and_is_two_sided() {
    let mesh = quad();
    // Straight down -Z onto the +X/+Y corner: nearest node is index 2
    // (1,1,0); the ray hits regardless of triangle winding.
    let hit = mesh
        .pick(glam::vec3(0.9, 0.9, 5.0), glam::vec3(0.0, 0.0, -1.0))
        .expect("hits near the (1,1) corner");
    assert_eq!(hit.node, 2, "nearest node is the (1,1,0) corner");
    assert_eq!(hit.scalar, Some(30.0), "scalar at node 2");

    // From behind (-Z side) the same hull still picks (two-sided, like
    // the no-cull renderer).
    let back = mesh.pick(glam::vec3(0.0, 0.0, -5.0), glam::vec3(0.0, 0.0, 1.0));
    assert!(back.is_some(), "two-sided pick from behind");
}

#[test]
fn shell_picking_toggle_and_readout_are_pure() {
    let mut s = ShellState::default();
    assert!(!s.picking, "off by default — status bar stays —");
    assert_eq!(s.pick, "—");

    let a = s.toggle_picking();
    assert!(s.picking);
    assert_eq!(a, UiAction::TogglePicking, "observability-only");

    // (0.6,0.9): y>x ⇒ the upper-left triangle (tri 1, verts 0,2,3);
    // nearest corner is node 2 (1,1,0) → scalar 30.
    let hit = quad()
        .pick(glam::vec3(0.6, 0.9, 5.0), glam::vec3(0.0, 0.0, -1.0))
        .unwrap();
    s.apply_pick(Some(&hit));
    assert_eq!(s.pick, "node 2 · tri 1 · v=3.000e1");
    s.apply_pick(None);
    assert_eq!(s.pick, "(no hit)");

    // Turning picking off resets the readout.
    s.toggle_picking();
    assert!(!s.picking);
    assert_eq!(s.pick, "—");

    // The pure layout still runs head­lessly with picking on and emits
    // no actions without pointer input (the m3/m4 pattern).
    s.toggle_picking();
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
