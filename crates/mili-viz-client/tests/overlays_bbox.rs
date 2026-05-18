//! Bbox / camera-tracking-gizmo overlay gating test (wireframe-parity
//! Viewport overlays / MVP-cut 5). All **always-on** pure logic — the
//! projection math is GPU-free; the windowed overlay paint is not
//! headlessly verifiable in CI (no display), so the projection and
//! the pure `build_shell_ui` path are tested directly (m3/m4 shape).

use mili_viz_client::{build_shell_ui, Camera, Mesh, SessionPhase, ShellState};

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
        scalars: None,
    }
}

#[test]
fn camera_project_centres_the_focus_and_culls_behind() {
    let (c, r) = quad().bounds();
    let cam = Camera::looking_at(c, r);

    // The focus point projects to the viewport centre (0.5, 0.5).
    let f = cam.project(c, 200, 200).expect("focus is in front");
    assert!(
        (f.x - 0.5).abs() < 1e-3 && (f.y - 0.5).abs() < 1e-3,
        "{f:?}"
    );

    // A point well behind the eye is culled (no garbage edge).
    let behind = cam.eye() + (cam.eye() - cam.focus).normalize() * 10.0;
    assert!(cam.project(behind, 200, 200).is_none(), "behind-eye culled");
}

#[test]
fn framed_aabb_corners_all_project_in_front() {
    let mesh = quad();
    let (c, r) = mesh.bounds();
    let cam = Camera::looking_at(c, r);
    let (lo, hi) = mesh.aabb();

    for i in 0..8 {
        let p = glam::vec3(
            if i & 1 == 0 { lo[0] } else { hi[0] },
            if i & 2 == 0 { lo[1] } else { hi[1] },
            if i & 4 == 0 { lo[2] } else { hi[2] },
        );
        let s = cam.project(p, 300, 200).expect("framed corner in front");
        // A framed model sits comfortably inside the viewport.
        assert!(
            (-0.2..1.2).contains(&s.x) && (-0.2..1.2).contains(&s.y),
            "corner {i} off-screen: {s:?}"
        );
    }
}

#[test]
fn shell_overlays_run_headlessly_with_and_without_a_live_camera() {
    let mesh = quad();
    let (c, r) = mesh.bounds();

    // With a live camera + AABB: the real projected bbox / tracking
    // gizmo path. Pure ⇒ no actions, but it paints.
    let mut s = ShellState {
        phase: SessionPhase::AttachedIdle,
        camera: Some(Camera::looking_at(c, r)),
        model_aabb: Some(mesh.aabb()),
        ..ShellState::default()
    };
    let ctx = egui::Context::default();
    let raw = egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(
            egui::Pos2::ZERO,
            egui::vec2(900.0, 600.0),
        )),
        ..Default::default()
    };
    let mut actions = Vec::new();
    let out = ctx.run_ui(raw.clone(), |ui| actions = build_shell_ui(ui, &mut s));
    assert!(actions.is_empty(), "no input ⇒ no actions: {actions:?}");
    assert!(!out.shapes.is_empty(), "real bbox/gizmo must paint");

    // Without a camera (the headless composite default): the M3
    // placeholder path still runs, byte-stable for that gate.
    let mut s2 = ShellState {
        phase: SessionPhase::AttachedIdle,
        ..ShellState::default()
    };
    assert!(s2.camera.is_none() && s2.model_aabb.is_none());
    let _ = ctx.run_ui(raw, |ui| {
        let _ = build_shell_ui(ui, &mut s2);
    });
}
