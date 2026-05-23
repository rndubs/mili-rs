//! Shared helpers for `mili-viz-client` integration tests.
//!
//! Cargo treats each file directly under `tests/` as its own test
//! binary; a sub-directory like `tests/common/` is the documented way
//! to share code without it being picked up as a third test target.
//! Each consumer adds `#[path = "common/mod.rs"] mod common;` (or
//! `mod common;` once they live in a `common/` subdir) and calls into
//! these helpers.
//!
//! # Why a helper at all
//!
//! Many composite / mesh-only render tests historically asserted
//! `at(w/2, h/2)` was a lit mesh pixel. Two latent problems hid behind
//! `skip-on-absent` (no GPU adapter in CI) until the SessionStart hook
//! installed Mesa lavapipe:
//!
//!  * Real corpus meshes like `serial/basic1` are non-convex / curved
//!    (the bar71 "quarter-pipe" family), so the bounding-sphere centre
//!    can fall in a hollow region with no triangle on the line of
//!    sight — the centre pixel is the clear colour and the assertion
//!    fails, even though the mesh is otherwise rendered correctly.
//!  * 240×240 composite frames are smaller than the default left dock
//!    (230 px) + AI rail (28 px), so the geometric centre falls
//!    *inside* the dock chrome. The assertion picks up the panel grey
//!    rather than the mesh underneath.
//!
//! Both cases mean "the centre pixel = the mesh" is the wrong signal.
//! The right signal is **"a meaningful fraction of mesh-coloured
//! pixels exist in the frame"** — exercises the pipeline end-to-end
//! without depending on a particular corpus shape or chrome layout.

#![allow(dead_code)] // not every test consumer needs every helper

/// `true` when an RGB triple is plausibly a lit pixel of the M2 base
/// colour `(0.62, 0.68, 0.80)` × shade. The base is bluish-white, so
/// lit mesh pixels satisfy `b > r` with a wide margin; clear colour
/// `(0.02, 0.02, 0.08)` ≈ `[5, 5, 20]` has `b - r = 15` but max
/// channel 20; egui chrome (dark `[27,27,27]` or light `[240,240,240]`)
/// is grey, `b ≈ r`. The combined test isolates the mesh from both
/// chrome and background across themes.
#[inline]
pub fn is_mesh_pixel(c: &[u8]) -> bool {
    let r = i32::from(c[0]);
    let g = i32::from(c[1]);
    let b = i32::from(c[2]);
    let max = r.max(g).max(b);
    // `b - r` excludes grey chrome; `b > g` is also true for the base
    // colour and rules out a lit yellow/red colormap result that would
    // accidentally satisfy `b > r`. `max > 60` rules out the clear
    // colour (max 20).
    max > 60 && (b - r) > 5 && (b - g) > 2
}

/// Count mesh-coloured pixels in an `RGBA8`, top-left-origin frame.
#[inline]
pub fn count_mesh_pixels(px: &[u8]) -> usize {
    px.chunks_exact(4).filter(|c| is_mesh_pixel(c)).count()
}

/// Assert that `px` contains at least `min_pixels` mesh-coloured
/// pixels (`is_mesh_pixel`). Replaces the historical
/// `at(w/2, h/2)[max] > 60` assertion, which was brittle against
/// non-convex meshes (centre on a hollow line of sight) and small
/// composite frames where the dock covers the geometric centre.
///
/// `label` is folded into the panic message for failure provenance —
/// pass the same string the old centre-pixel assertion used.
pub fn assert_mesh_visible(px: &[u8], min_pixels: usize, label: &str) {
    let count = count_mesh_pixels(px);
    assert!(
        count >= min_pixels,
        "{label}: expected at least {min_pixels} mesh-coloured pixels \
         (lit M2 base, `b > r + 5` heuristic), got {count}. The headless \
         compose pipeline ran but the mesh isn't visibly rendered — \
         likely a chrome/dock layout regression or a hull-extraction \
         regression."
    );
}
