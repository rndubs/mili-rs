//! Phase 5 M1 `mili-viz` client — a `wgpu` renderer skeleton.
//!
//! Scope is exactly README § "Phase 5 (client) milestones" M1:
//! window, orbit camera, a hard-coded triangle. **No `mili-rs`,
//! `mili-viz-proto`, or `mili-viz-server` dependency** — the
//! transport attaches at M2 ("render server output"). Scope +
//! Decisions 38–40: `planning/mili-viz/phase-5-m1.md`.
//!
//! The renderer is render-to-texture-first ([`render_to_image`]);
//! the windowed [`run`] path is a thin wrapper around the same
//! [`Renderer`]. The orbit [`Camera`] is the reusable core later
//! milestones build on; the triangle is throwaway scaffolding.

#![allow(clippy::pedantic)]

mod app;
mod camera;
mod renderer;

pub use app::run;
pub use camera::Camera;
pub use renderer::{headless_device, render_to_image, Renderer, CLEAR_COLOR, OFFSCREEN_FORMAT};
