//! Phase 5 M4 gating test (`phase-5-m4.md` Decisions 64–66).
//!
//! All **always-on** pure logic — the windowed predict/reconcile loop
//! is not exercised by CI (no display), so the reconcile core
//! ([`Camera::from_orbit`]), the colour-mapping effective range
//! ([`ShellState::effective_range`]) and the named colormaps are
//! tested directly, mirroring the M1-Decision-40 pattern.

use mili_viz_client::{
    colormap_sample, colormap_sample_named, Camera, ResultInfo, ShellState, COLORMAP_NAMES,
};

fn close(a: [f32; 3], b: [f32; 3]) -> bool {
    (0..3).all(|i| (a[i] - b[i]).abs() < 1e-6)
}

#[test]
fn reconcile_overwrites_a_predicted_camera_field_for_field() {
    // A locally-predicted (mid-drag) camera...
    let predicted = Camera::from_orbit(2.5, 0.7, 13.0, glam::vec3(9.0, 9.0, 9.0), 4.0);
    // ...is unconditionally replaced by the server broadcast
    // (last-broadcast-wins, Decision 64): azimuth/elevation/distance
    // and focus copy field-for-field, in radians (Decision 65).
    let radius = 4.0;
    let reconciled = Camera::from_orbit(0.30, -0.20, 7.5, glam::vec3(1.0, 2.0, 3.0), radius);

    assert_ne!(predicted, reconciled, "the broadcast must move the view");
    assert!((reconciled.azimuth - 0.30).abs() < 1e-6);
    assert!((reconciled.elevation - -0.20).abs() < 1e-6);
    assert!((reconciled.distance - 7.5).abs() < 1e-6);
    assert_eq!(reconciled.focus, glam::vec3(1.0, 2.0, 3.0));
    // Client-only projection planes are re-bracketed around the
    // reconciled distance + cached radius (not carried on the wire).
    assert!(reconciled.z_near > 0.0 && reconciled.z_near < reconciled.distance);
    assert!(reconciled.z_far > reconciled.distance);
    // Idempotent: reconciling the same state again is a no-op.
    let again = Camera::from_orbit(0.30, -0.20, 7.5, glam::vec3(1.0, 2.0, 3.0), radius);
    assert_eq!(reconciled, again);
}

#[test]
fn camera_basis_is_orthonormal() {
    let c = Camera::from_orbit(0.9, 0.4, 5.0, glam::Vec3::ZERO, 2.0);
    let (r, u, f) = c.basis();
    for v in [r, u, f] {
        assert!((v.length() - 1.0).abs() < 1e-5, "unit length");
    }
    assert!(r.dot(u).abs() < 1e-5 && r.dot(f).abs() < 1e-5 && u.dot(f).abs() < 1e-5);
}

#[allow(clippy::field_reassign_with_default)]
fn with_result(name: &str, min: f64, max: f64) -> ShellState {
    let mut s = ShellState::default();
    s.result = Some(ResultInfo {
        name: name.to_string(),
        component: String::new(),
        min,
        max,
        num_vertices: 0,
        num_indices: 0,
    });
    s
}

#[test]
fn effective_range_autoscales_then_legend_limits_override() {
    // No result → no scalar range (bare hull keeps the M2 base
    // colour, unchanged from M3).
    assert_eq!(ShellState::default().effective_range(), None);
    // Empty result name (bare `show ""`) is also no-range.
    assert_eq!(with_result("", 1.0, 9.0).effective_range(), None);

    // Autoscale from the broadcast ResultState when unset.
    let mut s = with_result("eff_stress", 2.0, 6.0);
    assert_eq!(s.effective_range(), Some((2.0, 6.0)));

    // A LegendLimits override replaces only the bounds that are set.
    s.legend_min = Some(3.5);
    assert_eq!(s.effective_range(), Some((3.5, 6.0)));
    s.legend_max = Some(5.0);
    assert_eq!(s.effective_range(), Some((3.5, 5.0)));
    // Clearing reverts to autoscale (no stale clamp).
    s.legend_min = None;
    s.legend_max = None;
    assert_eq!(s.effective_range(), Some((2.0, 6.0)));
}

#[test]
fn named_colormaps_are_distinct_and_cool_is_the_default_and_fallback() {
    // The M3 entry point is exactly the `cool` ramp (m3 gate stable).
    assert!(close(
        colormap_sample(0.0),
        colormap_sample_named("cool", 0.0)
    ));
    assert!(close(
        colormap_sample(1.0),
        colormap_sample_named("cool", 1.0)
    ));

    // Every advertised name resolves; the table is non-trivial.
    for &n in COLORMAP_NAMES {
        let lo = colormap_sample_named(n, 0.0);
        let hi = colormap_sample_named(n, 1.0);
        assert!(!close(lo, hi), "{n} ramp endpoints differ");
        // Clamped + bounded channels.
        assert!(close(colormap_sample_named(n, -1.0), lo), "{n} clamps t<0");
        assert!(close(colormap_sample_named(n, 2.0), hi), "{n} clamps t>1");
    }
    // grayscale is a true gray (R==G==B) and monotone.
    let mid = colormap_sample_named("grayscale", 0.5);
    assert!((mid[0] - mid[1]).abs() < 1e-6 && (mid[1] - mid[2]).abs() < 1e-6);

    // An unknown name falls back to `cool`, never panics/errors.
    assert!(close(
        colormap_sample_named("does-not-exist", 0.42),
        colormap_sample_named("cool", 0.42)
    ));
}
