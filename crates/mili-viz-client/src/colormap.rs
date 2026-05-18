//! Viz-local named scalar colormaps (`phase-5-m3.md` Decision 47,
//! `phase-5-m4.md` Decision 66).
//!
//! M3 shipped a single fixed cool→warm ramp. M4 honours the frozen
//! `Colormap` proto command client-side: a small fixed table of named
//! ramps (`cool`, `warm`, `grayscale`, `hot`); an unknown name falls
//! back to `cool` (logged by the caller, never an error). The
//! `LegendLimits` clamp is applied by the caller's effective range,
//! not here.

/// The default cool→warm ramp (blue→cyan→green→yellow→red). Kept
/// byte-identical to the M3 constant so `m3_egui_shell.rs` is
/// unaffected.
const COOL: [[f32; 3]; 5] = [
    [0.23, 0.30, 0.75], // cool blue
    [0.20, 0.70, 0.85], // cyan
    [0.35, 0.78, 0.35], // green
    [0.95, 0.85, 0.20], // yellow
    [0.83, 0.20, 0.18], // warm red
];

/// `cool` reversed — warm low, cool high.
const WARM: [[f32; 3]; 5] = [
    [0.83, 0.20, 0.18],
    [0.95, 0.85, 0.20],
    [0.35, 0.78, 0.35],
    [0.20, 0.70, 0.85],
    [0.23, 0.30, 0.75],
];

/// Perceptual-ish black→white.
const GRAYSCALE: [[f32; 3]; 5] = [
    [0.0, 0.0, 0.0],
    [0.25, 0.25, 0.25],
    [0.5, 0.5, 0.5],
    [0.75, 0.75, 0.75],
    [1.0, 1.0, 1.0],
];

/// Black-body-ish black→red→yellow→white.
const HOT: [[f32; 3]; 5] = [
    [0.0, 0.0, 0.0],
    [0.55, 0.0, 0.0],
    [0.95, 0.45, 0.0],
    [1.0, 0.9, 0.2],
    [1.0, 1.0, 1.0],
];

/// The selectable colormap names, in UI order. `cool` is the default
/// (index 0) and the unknown-name fallback.
pub const NAMES: &[&str] = &["cool", "warm", "grayscale", "hot"];

fn stops(name: &str) -> &'static [[f32; 3]; 5] {
    match name {
        "warm" => &WARM,
        "grayscale" | "gray" | "greyscale" => &GRAYSCALE,
        "hot" => &HOT,
        _ => &COOL,
    }
}

/// Map `t` (clamped to `[0, 1]`) through the `cool` ramp — the M3
/// entry point, kept for the always-on `m3_egui_shell.rs` gate.
#[must_use]
pub fn sample(t: f32) -> [f32; 3] {
    sample_named("cool", t)
}

/// Map `t` (clamped to `[0, 1]`) through the named ramp via piecewise
/// linear interpolation; an unknown name uses `cool`.
#[must_use]
pub fn sample_named(name: &str, t: f32) -> [f32; 3] {
    let table = stops(name);
    let t = t.clamp(0.0, 1.0);
    let span = (table.len() - 1) as f32;
    let scaled = t * span;
    let i = (scaled.floor() as usize).min(table.len() - 2);
    let f = scaled - i as f32;
    let a = table[i];
    let b = table[i + 1];
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
