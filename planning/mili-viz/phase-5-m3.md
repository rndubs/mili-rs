# Phase 5 M3 — landed (`egui` shell: toolbar / dock / overlays / status)

> **Status: ✅ COMPLETE.** This milestone is landed and frozen.
> Full live status in [`status.md`](status.md); this file is retained
> for the decision-number cross-references in the rest of the tree.

## What landed

- `egui` 0.34.2 / `egui-wgpu` 0.34.2 / `egui-winit` 0.34.2 stack
  pinned and verified compatible with the frozen `wgpu` 29 /
  `winit` 0.30 (sparse-index check).
- Toolbar (seven groups: Transport / Stride / Animate / View /
  Overlays / spacer / State counter), `SidePanel::left` 230 px dock
  with four `CollapsingHeader` sections (Runs/sessions, Results,
  Materials, Surfaces), the five viewport overlays (title / state /
  legend / axes / bbox), and the status bar — built by a pure
  `fn(&Context, &mut ShellState) -> Vec<UiAction>` (GPU-free).
- `egui` paints as a second non-clearing pass (`LoadOp::Load`, no
  depth) on the same target *after* the unchanged mesh pass;
  `Renderer::render` is byte-for-byte preserved so the M1/M2
  render-to-texture seam never moves. New `render_shell_to_image`
  reuses the M1 readback machinery.
- `MVG2` per-vertex scalar now kept on `Mesh.scalars` and mapped
  through a viz-local cool→warm colormap autoscaled by
  `ResultState.{min,max}`; live `Session` (subscribe + Execute)
  added alongside the M2 one-shot `fetch_server_mesh`.

## Gating test

`crates/mili-viz-client/tests/m3_egui_shell.rs` — always-on shell-
logic + colormap unit + skip-on-absent composite render asserting
opaque chrome at a dock column pixel while the viewport centre is
still the rendered mesh.

## Decisions

- Decisions 44–47; index lives in [`status.md`](status.md).
