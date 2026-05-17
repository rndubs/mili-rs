//! Phase 5 M2 `mili-viz` client — render server output.
//!
//! M1 was the `wgpu` renderer skeleton (window, orbit camera, a
//! hard-coded triangle; `phase-5-m1.md` Decisions 38–40). M2 wires
//! the transport in: [`fetch_server_mesh`] spawns a `mili-viz-server`
//! over the in-process transport, drives `load`/`show`, resolves the
//! returned `GeometryRef`, and [`decode_mvg`]s the self-describing
//! blob into a [`Mesh`] the [`Renderer`] draws through the orbit
//! [`Camera`]. Scope + Decisions 41–43:
//! `planning/mili-viz/phase-5-m2.md`.
//!
//! The renderer stays render-to-texture-first
//! ([`render_mesh_to_image`]); the windowed [`run`] path is a thin
//! wrapper around the same [`Renderer`]. Remote mode (gRPC + Flight
//! TCP) is Phase 5 M5; the `egui` controls are M3.

#![allow(clippy::pedantic)]

mod app;
mod camera;
mod mesh;
mod renderer;
mod session;

pub use app::run;
pub use camera::Camera;
pub use mesh::{decode_mvg, DecodeError, Mesh};
pub use renderer::{
    headless_device, render_mesh_to_image, render_to_image, Renderer, CLEAR_COLOR, OFFSCREEN_FORMAT,
};
pub use session::fetch_server_mesh;
