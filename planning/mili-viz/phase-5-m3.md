# `mili-viz` Phase 5 M3 — `egui` shell (buildable scope)

> Scope doc for Phase 5 Milestone 3, the analogue of
> [`phase-5-m2.md`](phase-5-m2.md) on the renderer side. M1 stood up
> the `wgpu`/`winit` skeleton (`phase-5-m1.md` Decisions 38–40); M2
> wired the in-process transport and drew the decoded server hull
> (`phase-5-m2.md` Decisions 41–43, `MVG2` scalar deliberately
> decoded-past-and-ignored). M3 grows an `egui` shell onto that
> renderer: the toolbar, the left dock, and the viewport overlay
> layer, and turns the `MVG2` per-vertex scalar into vertex colour
> driven by a left-dock result pick. No proto change — the Phase 4
> M1 contract is frozen.
>
> Read [`status.md`](status.md) first, then
> [`griz_wgpu_wireframes/README.md`](griz_wgpu_wireframes/README.md)
> (the authoritative layout spec), `client.md`, and `phase-5-m2.md`.
> Decision entries continue the global log (Phase 4 ended at 34;
> Phase 6 M1 took 35–37; Phase 5 M1 38–40, M2 41–43; Phase 5 M3
> starts at 44).

## Goal

README Phase 5 M3: **"`egui` controls. State scrubber, result
picker, view controls, command-line entry."** Refined by the
wireframes' §"Implementation order" item 1 to the concrete M3 slice:
**toolbar + left dock + the five-element viewport overlay set**, in
the **L1** default layout, handling the three non-agent session
states (not attached / attached idle / animating). The command-line
Layer-0 tab and the scripting runner are the wireframes' explicitly
distinct **M3.5**; the AI panel is **M6**. Both are out of M3 —
the AI rail is a 28 px non-interactive placeholder and the bottom
tabs are a collapsed stub.

Concretely, `crates/mili-viz-client` grows an `egui` layer over the
existing depth-tested mesh `Renderer`:

- **Toolbar** — seven groups (Transport / Stride / Animate / View /
  Overlays / spacer / State counter) emitting the frozen
  `SetState`/`Step`/`View(reset)` commands and toggling overlay
  visibility.
- **Left dock** — `SidePanel::left` (230 px, resizable) with four
  `CollapsingHeader` sections (Runs/sessions, Results, Materials,
  Surfaces); selecting a Results item emits `Execute(show <result>)`,
  which is where the `MVG2` scalar becomes a colormap.
- **Viewport overlays** — five `egui` overlays drawn over the `wgpu`
  surface (title / state / legend / axes / bbox), gated by the
  toolbar Overlays chips.
- **Status bar** + the not-attached "attach to session" card +
  the animating state.

Out of scope (unchanged from the wireframes' milestone split): the
bottom-tabs command line / scripting runner (M3.5), the AI panel
(M6), local view manipulation reconciled against the
server-authoritative camera beyond emitting the command (M4),
**remote** mode over gRPC + Flight TCP (M5). M3 uses the
**in-process** transport only.

## Decisions (continuing the global log)

### Decision 44 — pin the `egui` 0.34.2 stack (`egui` + `egui-wgpu` + `egui-winit`), verified compatible with the frozen `wgpu` 29 / `winit` 0.30

The renderer is frozen on `wgpu` 29 / `winit` 0.30 (M1 Decision 38;
`Cargo.lock` resolves `wgpu` 29.0.3, `winit` 0.30.13). The egui
integration crates must agree on both native deps or the workspace
will not resolve. The crates.io JSON API is blocked in this
environment, so the requirement was read straight off the **sparse
index** (`https://index.crates.io/eg/ui/egui-wgpu`,
`.../egui-winit`): `egui-wgpu` 0.34.2 declares `wgpu ^29.0.1`,
`winit ^0.30.13`, `egui ^0.34.2`; `egui-winit` 0.34.2 declares
`winit ^0.30.13`, `egui ^0.34.2`. Both ranges are satisfied by the
already-resolved `wgpu` 29.0.3 / `winit` 0.30.13, so the `egui`
0.34.2 / `egui-wgpu` 0.34.2 / `egui-winit` 0.34.2 trio drops in with
**no `wgpu`/`winit` version bump and no churn to the frozen Phase 4
crates** — they share the workspace's existing `wgpu`/`winit`.
Rejected: an older egui pinned to an older `wgpu` (would force a
renderer-wide `wgpu` downgrade, breaking M1/M2's pipeline and the
frozen server's nothing — the server has no `wgpu`, but the client's
M1/M2 gating tests would have to be re-validated for no benefit).

### Decision 45 — the `egui` paint is a second, non-clearing pass over the same target *after* the unchanged mesh pass; `Renderer::render` is byte-for-byte preserved so the M1/M2 render-to-texture seam never moves

The architecturally load-bearing question is how the immediate-mode
`egui` paint composes with the existing depth-tested mesh pass
without breaking the M1 headless render-to-texture seam (M1
Decision 39, reused verbatim by M2 Decision 43 and every later
milestone's screenshot/`Snapshot` path). The answer is **strict
addition, never modification**:

- The mesh pass (`Renderer::render`) is left **byte-for-byte
  unchanged** — it still clears to `CLEAR_COLOR`, depth-tests, and
  draws the uploaded `Mesh` over the *entire* target. M1's
  `render_to_image` and M2's `render_mesh_to_image` are untouched,
  so `m1_renderer.rs` and `m2_render_server_output.rs` keep passing
  with zero edits.
- `egui` is painted by a **separate** `egui_wgpu::Renderer` in a
  **second render pass** on the *same* `TextureView`, with
  `LoadOp::Load` (it composites onto the mesh, never clears it) and
  **no depth attachment** (UI is 2-D screen-space chrome). The
  chrome panels (`TopBottomPanel` toolbar/menu/status, `SidePanel`
  left dock + AI rail) are opaque and occlude the mesh where they
  sit; the `CentralPanel` is given a fully transparent frame so the
  full-surface mesh shows through exactly the viewport region; the
  five overlays are `egui` painter primitives inside that central
  rect.

This makes the composite a pure pipeline of *(unchanged mesh pass) →
(additive egui pass)* on one view. The headless seam is preserved by
adding a parallel headless entry point
(`render_shell_to_image`) that runs both passes into the same
off-screen texture and reads it back — the M1/M2 readback machinery
(`read_back`) is reused, not forked. Rejected: rendering the mesh
into its own offscreen texture and feeding it to `egui` as an image
widget — it doubles the GPU memory + a copy for zero correctness
gain at M3 (the mesh genuinely fills the central rect; chrome is
opaque) and would fork the readback seam M1 froze.

### Decision 46 — the shell UI is a pure `fn(&Context, &mut ShellState) -> Vec<UiAction>`; windowed input via `egui-winit`, the headless gate feeds synthetic `RawInput`; the camera stays server-authoritative (M3 emits the command, full reconcile is M4)

The entire layout — toolbar, dock tree, overlays, status bar,
session-state cards — is built by one GPU-free function of an
explicit `ShellState` (session phase, mirrored `LoadedState` /
`ResultState` / current state, overlay toggles, stride, status
fields, dock selection). It returns a `Vec<UiAction>` (a small
client-side enum: `First/Prev/Next/Last`, `SetStride`,
`ToggleAnimate/StopAnimate`, `ViewReset`, `Fit`,
`ToggleOverlay(kind)`, `Show(name)`). This is the milestone's real
unconditional test weight, the M1-Decision-40 pattern (the `Camera`
was the always-on core; here the shell-state→action map is): it runs
with **no GPU** by feeding `egui::Context::run` a synthetic
`RawInput` with a programmatic click and asserting the emitted
`UiAction`s and the `ShellState` mutations.

The windowed `app.rs` is the thin integration: `egui-winit`
translates `winit` events to `egui` `RawInput`; each `UiAction` is
lowered to the **frozen** proto `Command` and sent over the
in-process `Execute`; the `Subscribe` stream is drained every frame
into `ShellState` (a background task forwards `StateDelta`s through
a channel; `DELTA_RESULT` re-resolves the `GeometryRef` through the
frozen `VizService::fetch_geometry` seam and re-uploads the `Mesh`).
The camera remains **server-authoritative** per `scripting.md` /
`client.md` Decisions 1–2: M3's `⟲ view reset` / `⊞ fit` emit
`View(reset)` (and locally re-frame the cached bounds for
responsiveness) — the *full* reconcile against the broadcast
`DELTA_CAMERA` is M4; M3 does not fork a local interactive camera
owner. `fetch_server_mesh` (the M2 one-shot) is kept unchanged for
`m2_render_server_output.rs`; the live `Session` is new and
additive.

### Decision 47 — the `MVG2` per-vertex scalar becomes vertex colour through a viz-local cool→warm colormap autoscaled by `ResultState.{min,max}`; no scalar / no result → the M2 uniform base colour; the `Colormap` command is M4+

M2 Decision 42 deliberately decoded the `MVG2` scalar past and
ignored it ("scalar→color is Phase 5 M3"). M3 redeems that: `mesh.rs`
now keeps the trailing `scalar_f32` as `Mesh.scalars:
Option<Vec<f32>>`; the renderer `Vertex` gains a colour attribute;
when a result is active the per-vertex scalar is mapped through a
**viz-local** cool→warm colormap, autoscaled by the broadcast
`ResultState.{min,max}` (the server already computes that range —
`phase-4-m3.md` Decision 15; the client only *consumes* it, exactly
the M5 "reuse, don't re-port" boundary). No scalar (the bare `MVG1`
hull, or no result selected) keeps the M2 uniform lit base colour,
so the M2 render assertions are unaffected. The colormap is a fixed
client constant; honouring the frozen `Colormap` proto command
(named maps) and the `LegendLimits` clamp is **deferred to M4+** —
M3's legend overlay shows the autoscale range only. Rejected:
shipping the named-colormap command surface now — it is not in the
wireframes' M3 slice and adds command-plumbing for no M3-visible
gain; the single cool→warm map is enough to make the result pick
legible and to drive the legend.

## Crate layout (delta from M2)

```
crates/mili-viz-client/
├── Cargo.toml          # + egui, egui-wgpu, egui-winit (0.34.2)
├── src/
│   ├── lib.rs          # + ShellState, UiAction, render_shell_to_image
│   ├── main.rs         # optional argv[1] root → live in-process Session
│   ├── camera.rs       # unchanged
│   ├── mesh.rs         # + Mesh.scalars (MVG2 scalar now KEPT)
│   ├── colormap.rs     # NEW: viz-local cool→warm scalar→rgb
│   ├── shell.rs        # NEW: ShellState + build_shell_ui (pure, GPU-free)
│   ├── session.rs      # + live Session (subscribe task + Execute);
│   │                   #   fetch_server_mesh kept unchanged for M2 test
│   ├── renderer.rs     # Vertex gains colour; render() unchanged;
│   │                   #   + egui_pass() additive; + render_shell_to_image
│   ├── mesh.wgsl       # + per-vertex colour modulates the lit shade
│   └── app.rs          # egui-winit + egui-wgpu integration; live Session
└── tests/
    ├── m1_renderer.rs              # unchanged
    ├── m2_render_server_output.rs  # unchanged
    └── m3_egui_shell.rs            # NEW (always-on shell logic +
                                    #      skip-on-absent composite render)
```

## Acceptance gate

- `cargo test --workspace --exclude mili-py` builds `mili-viz-client`;
  the always-on `m3_egui_shell` shell-logic assertions (and the
  always-on `colormap`/scalar-decode unit) pass with no GPU.
- `cargo fmt --all --check` and
  `cargo clippy --workspace --all-targets -- -D warnings` are clean.
- The skip-on-absent composite render (`serial/basic1` + a `wgpu`
  adapter present) draws the mesh **and** the egui chrome into one
  off-screen texture: a left-dock column pixel is opaque chrome (not
  the clear colour and not the mesh-only tint) while the viewport
  centre is still the rendered mesh — proving Decision 45's additive
  composition. Printed-and-skipped (not failed) when the corpus or a
  `wgpu` adapter is absent (CLAUDE.md convention).
- `m1_renderer.rs` and `m2_render_server_output.rs` are **unchanged
  and green** (Decision 45 keeps `Renderer::render` byte-stable). No
  Phase 4 crate is touched; every Phase 4 server gating test is
  unchanged. No `mili_viz.proto`/blob/ticket change.

## Decision log (this doc)

| # | Title | Resolves |
|---|---|---|
| 44 | Pin the `egui` 0.34.2 stack; verified vs the frozen `wgpu` 29 / `winit` 0.30 (sparse-index check, crates.io API blocked) | M3 deps |
| 45 | `egui` is an additive non-clearing second pass on the same view; `Renderer::render` byte-for-byte preserved → M1/M2 render-to-texture seam never moves | M3 composition |
| 46 | Shell UI = pure `fn(&Context,&mut ShellState)->Vec<UiAction>` (always-on gate); windowed via `egui-winit`+live `Session`; camera stays server-authoritative (M3 emits, M4 reconciles) | M3 UI / transport |
| 47 | `MVG2` scalar → vertex colour via a viz-local cool→warm map autoscaled by `ResultState.{min,max}`; no scalar → M2 base colour; `Colormap`/`LegendLimits` deferred to M4+ | M3 colour |
</content>
</invoke>
