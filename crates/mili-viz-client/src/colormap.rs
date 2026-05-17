//! Viz-local cool→warm scalar colormap (`phase-5-m3.md` Decision 47).
//!
//! A fixed five-stop blue→cyan→green→yellow→red ramp. This is a
//! client display constant, not a port of a griz colormap — the
//! frozen `Colormap` proto command (named maps) and the
//! `LegendLimits` clamp are deferred to M4+; M3 only needs *a* legible
//! map and a legend driven by the broadcast `ResultState.{min,max}`.

/// Five colormap control points, `t = 0.0 ..= 1.0`.
const STOPS: [[f32; 3]; 5] = [
    [0.23, 0.30, 0.75], // cool blue
    [0.20, 0.70, 0.85], // cyan
    [0.35, 0.78, 0.35], // green
    [0.95, 0.85, 0.20], // yellow
    [0.83, 0.20, 0.18], // warm red
];

/// Map `t` (clamped to `[0, 1]`) to a linear RGB triple via piecewise
/// linear interpolation over [`STOPS`].
#[must_use]
pub fn sample(t: f32) -> [f32; 3] {
    let t = t.clamp(0.0, 1.0);
    let span = (STOPS.len() - 1) as f32;
    let scaled = t * span;
    let i = (scaled.floor() as usize).min(STOPS.len() - 2);
    let f = scaled - i as f32;
    let a = STOPS[i];
    let b = STOPS[i + 1];
    [
        a[0] + (b[0] - a[0]) * f,
        a[1] + (b[1] - a[1]) * f,
        a[2] + (b[2] - a[2]) * f,
    ]
}

/// Normalize `value` into `[0, 1]` against an autoscale range
/// `[min, max]` (the broadcast `ResultState`). A degenerate range
/// (`max <= min`) maps everything to the ramp midpoint.
#[must_use]
pub fn normalize(value: f32, min: f32, max: f32) -> f32 {
    if max <= min || !(max - min).is_finite() {
        0.5
    } else {
        ((value - min) / (max - min)).clamp(0.0, 1.0)
    }
}
