# Phase 5 M1 — landed (`wgpu` renderer skeleton)

> **Status: ✅ COMPLETE.** This milestone is landed and frozen.
> Full live status in [`status.md`](status.md); this file is retained
> for the decision-number cross-references in the rest of the tree.

## What landed

- New `crates/mili-viz-client` crate, dependency-free of mili/proto/
  server (only `wgpu`, `winit`, `bytemuck`, `glam`, `pollster`),
  opening a `winit` window and presenting a `wgpu` surface with a
  hard-coded triangle through a minimal pipeline.
- Render-to-texture-first `Renderer` exposing `render_to_image(w,h)
  -> Vec<u8>` (RGBA8); the windowed `app.rs` is a thin
  `winit::ApplicationHandler` pointing the same `Renderer` at the
  surface texture. This is the seam reused by every later
  milestone's headless/`Snapshot` path.
- Orbit `Camera` (azimuth/elevation/distance/focus) producing a
  `view_projection` matrix, with field shape pre-aligned to the
  frozen proto `CameraState` so M4's reconcile is a field copy.

## Gating test

`crates/mili-viz-client/tests/m1_renderer.rs` — always-on `Camera`
view-projection assertions plus a skip-on-absent headless GPU render
that asserts clear-color and triangle-color pixels.

## Decisions

- Decisions 38–40; index lives in [`status.md`](status.md).
