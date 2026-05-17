# `mili-viz` Phase 5 M1 — `wgpu` renderer skeleton (buildable scope)

> Scope doc for Phase 5 Milestone 1 — the first client-side milestone,
> the analogue of [`phase-4-m1.md`](phase-4-m1.md) on the renderer
> side. Phase 4 (server) is complete and frozen ([`status.md`](status.md));
> Phase 5 builds the `wgpu` + `egui` viewer described in
> [`README.md`](README.md) and [`client.md`](client.md).
>
> Read [`status.md`](status.md) first, then `README.md` § "Why
> `wgpu` + `egui`" and § "Phase 5 (client) milestones". Decision
> entries continue the global log (Phase 4 ended at Decision 34;
> Phase 6 M1 took 35–37; Phase 5 M1 starts at 38).

## Goal

README Phase 5 M1: **"`wgpu` renderer skeleton. Window, camera, a
hard-coded triangle. No mili involvement."**

Concretely, a new `crates/mili-viz-client` crate that:

- Opens a `winit` window and presents a `wgpu` surface.
- Draws a single hard-coded triangle through a minimal render
  pipeline, viewed through an orbit `Camera` (the camera the later
  milestones reuse for the real mesh).
- Can render the same scene **headless** to an off-screen texture and
  read the pixels back — the gating-test seam, and the future
  `Snapshot` / CI-screenshot seam (`client.md`).

Out of scope (later milestones): rendering server-streamed geometry
(M2), the `egui` dock/controls (M3), the AI panel (M3.5), local view
manipulation reconciled against the server-authoritative camera (M4),
remote mode (M5). M1 has **no** `mili-rs`, `mili-viz-proto`, or
`mili-viz-server` dependency — the README is explicit ("No mili
involvement"), and keeping the skeleton dependency-free keeps the
renderer bring-up isolated from the (already-frozen) transport.

## Decisions (continuing the global log)

### Decision 38 — `mili-viz-client` is a standalone crate with no mili/proto/server dependency at M1

The README sequences Phase 5 M1 as "No mili involvement" precisely so
the `wgpu`/`winit` bring-up — driver quirks, surface configuration,
the device/adapter dance — is debugged in isolation before any
transport is attached. The crate therefore depends only on the
graphics stack (`wgpu`, `winit`, `bytemuck`, `glam`, `pollster`). The
`mili-viz-proto` client and the in-process transport land at M2, where
"render server output" actually needs them. This mirrors the server
side, where M1 (`phase-4-m1.md` Decision 7) stood up the transport
with `mili-rs` deliberately *unwired* until M2.

### Decision 39 — the renderer is structured render-to-texture-first; the window path is a thin wrapper, so the gating test is a real GPU render with skip-on-absent

A windowed renderer is not testable in CI. Rather than ship M1 with
only compile-time coverage, the renderer's core (`Renderer`) targets
an arbitrary `wgpu::TextureView` and exposes
`render_to_image(width, height) -> Vec<u8>` (RGBA8). The windowed
`app.rs` is a thin `winit::ApplicationHandler` that points the same
`Renderer` at the surface texture. The gating test renders the
triangle off-screen and asserts a clear-color corner pixel and a
triangle-color center pixel.

CI runners have no GPU. Following the **canonical skip-on-absent
convention** this repo already uses for the parity/fixture suites
(`CLAUDE.md` § "Parity / fixture tests skip-on-absent"), the headless
render test requests a `wgpu::Adapter` and, if none is available
(no hardware adapter and no software rasterizer such as `llvmpipe`),
**prints and early-returns instead of failing**. The pure-math half
of the test (the `Camera` view-projection assertions) always runs and
is unconditional — that is the part a no-GPU CI box still hard-gates.
No new CI job is added; `mili-viz-client` builds and the always-on
camera assertions run under the existing `test` job (`cargo test
--workspace --exclude mili-py`), and `lint` covers it via
`clippy --workspace --all-targets`.

### Decision 40 — orbit `Camera` is the reusable, fully-unit-tested core; the triangle is throwaway

The hard-coded triangle is scaffolding deleted at M2. The `Camera`
is not — every later milestone (mesh display, M4 local manipulation
reconciled against the server-authoritative `CameraState`) builds on
it. So the `Camera` carries the milestone's real, unconditional test
weight: an orbit camera (`azimuth`, `elevation`, `distance`, focus
point) producing a `view_projection` matrix, with assertions that the
focus point maps to clip-space origin, that points in front of the
camera land in front (`0 < ndc.z < 1`), and that aspect ratio scales
x as expected. The `azimuth`/`elevation`/`distance`/focus field shape
is chosen to line up 1:1 with the frozen proto `CameraState`
(`phase-4-m1.md`; `mili-viz-server` `default_camera()`), so M4's
"reconcile against the server-authoritative camera" is a field copy,
not a conversion.

## Crate layout

```
crates/mili-viz-client/
├── Cargo.toml
├── src/
│   ├── lib.rs        # re-exports + `run()` window entrypoint
│   ├── main.rs       # binary → `mili_viz_client::run()`
│   ├── camera.rs     # orbit camera, pure math (glam), no GPU
│   ├── renderer.rs   # wgpu device/pipeline/triangle; render-to-texture
│   └── app.rs        # winit ApplicationHandler; surface + redraw
└── tests/
    └── m1_renderer.rs  # camera math (always) + headless render (skip-on-absent)
```

## Acceptance gate

- `cargo test --workspace --exclude mili-py` builds `mili-viz-client`
  and the always-on `Camera` assertions in `tests/m1_renderer.rs`
  pass.
- `cargo clippy --workspace --all-targets -- -D warnings` and
  `cargo fmt --all --check` are clean.
- The headless-render assertion passes when a `wgpu` adapter is
  available and is skipped (printed, not failed) when it is not —
  the documented skip-on-absent convention.
- No Phase 4 crate is touched; the server's frozen tests are
  unaffected (no shared code).

## Decision log (this doc)

- Decision 38 — standalone crate, no mili/proto/server dep at M1.
- Decision 39 — render-to-texture-first; gating test = always-on
  camera math + skip-on-absent headless GPU render.
- Decision 40 — orbit `Camera` is the reusable tested core, field
  shape aligned to the frozen proto `CameraState`; triangle is
  throwaway scaffolding.
