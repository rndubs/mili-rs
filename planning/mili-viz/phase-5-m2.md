# Phase 5 M2 — landed (render server output)

> **Status: ✅ COMPLETE.** This milestone is landed and frozen.
> Full live status in [`status.md`](status.md); this file is retained
> for the decision-number cross-references in the rest of the tree.

## What landed

- `mili-viz-client` now depends on `mili-viz-proto` + `mili-viz-server`
  and drives the frozen contract over the in-process transport:
  `Subscribe`, `Execute(load)`, `Execute(show)`, then reads the
  broadcast `DELTA_RESULT`'s `ResultState.geometry`.
- `mesh.rs` decodes the `MVG1`/`MVG2` blob (Phase 4 M2 Decision 11)
  into a `Mesh` of positions + indices, computing CPU per-vertex
  normals (area-weighted face-normal accumulation); the trailing
  `MVG2` scalar is decoded-past-and-ignored (scalar→color is M3).
- `Renderer` generalized from the M1 triangle to a depth-tested
  (`Depth32Float`, `Less`) indexed-mesh pipeline; `Camera::looking_at
  (center, radius)` auto-frames the mesh bounds.
- `GeometryRef` resolved through the in-process
  `VizService::fetch_geometry` seam (Flight-over-TCP is M5).

## Gating test

`crates/mili-viz-client/tests/m2_render_server_output.rs` — always-on
synthetic `MVG1` decode unit + skip-on-absent end-to-end load/show/
resolve/decode/headless render against `serial/basic1`.

## Decisions

- Decisions 41–43; index lives in [`status.md`](status.md).
