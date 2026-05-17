//! Phase 5 M1 gating test (`phase-5-m1.md` § "Acceptance gate").
//!
//! Two halves, per Decision 39:
//!  * `camera_*` — pure view-projection math, **always runs** (this
//!    is what a no-GPU CI box hard-gates).
//!  * `headless_render_*` — a real GPU render to an off-screen
//!    texture, **skip-on-absent** when no `wgpu` adapter exists
//!    (CLAUDE.md skip-on-absent convention; not a failure).

use glam::{Vec3, Vec4Swizzles};
use mili_viz_client::Camera;

/// Project a world point to NDC (`xy` in `[-1,1]`, `z` in `[0,1]`).
fn project(cam: &Camera, w: u32, h: u32, p: Vec3) -> glam::Vec3 {
    let clip = cam.view_projection(w, h) * p.extend(1.0);
    assert!(clip.w > 0.0, "point must be in front of the camera");
    clip.xyz() / clip.w
}

#[test]
fn camera_focus_maps_to_clip_origin() {
    let cam = Camera::default();
    let ndc = project(&cam, 800, 600, cam.focus);
    assert!(ndc.x.abs() < 1e-4, "focus x off-center: {}", ndc.x);
    assert!(ndc.y.abs() < 1e-4, "focus y off-center: {}", ndc.y);
}

#[test]
fn camera_depth_is_wgpu_range() {
    let cam = Camera::default();
    // A point between the eye (z=+3) and the focus (origin) is in
    // front; wgpu clip depth is [0, 1].
    let ndc = project(&cam, 800, 600, Vec3::new(0.0, 0.0, 1.0));
    assert!(
        (0.0..=1.0).contains(&ndc.z),
        "depth {} outside wgpu [0,1]",
        ndc.z
    );
}

#[test]
fn camera_aspect_scales_x() {
    let cam = Camera::default();
    let p = Vec3::new(0.2, 0.0, 0.0);
    let wide = project(&cam, 200, 100, p).x; // aspect 2.0
    let tall = project(&cam, 100, 200, p).x; // aspect 0.5
    assert!(
        wide < tall,
        "wider viewport should compress x: wide={wide} tall={tall}"
    );
}

#[test]
fn camera_eye_orbits_focus_at_distance() {
    let cam = Camera {
        azimuth: 1.0,
        elevation: 0.3,
        distance: 7.0,
        focus: Vec3::new(1.0, 2.0, -3.0),
        ..Camera::default()
    };
    let d = (cam.eye() - cam.focus).length();
    assert!((d - 7.0).abs() < 1e-3, "eye not at orbit distance: {d}");
}

#[test]
fn headless_render_draws_triangle_over_clear() {
    let (w, h) = (64u32, 64u32);
    let Some(px) = mili_viz_client::render_to_image(w, h, &Camera::default()) else {
        eprintln!(
            "skip: no wgpu adapter (no GPU / software rasterizer) — \
             skip-on-absent per phase-5-m1.md Decision 39"
        );
        return;
    };
    assert_eq!(px.len() as u32, w * h * 4);

    let at = |x: u32, y: u32| {
        let i = ((y * w + x) * 4) as usize;
        [px[i], px[i + 1], px[i + 2]]
    };

    // A corner is background clear color (dark: ~5,5,20 in u8).
    let corner = at(1, 1);
    assert!(
        corner.iter().all(|&c| c < 40),
        "corner should be the clear color, got {corner:?}"
    );

    // The center projects to a point inside the triangle, so it is
    // the interpolated (bright) vertex color, not the clear color.
    let center = at(w / 2, h / 2);
    assert!(
        center.iter().copied().max().unwrap() > 80,
        "center should be the triangle, got {center:?}"
    );
}
