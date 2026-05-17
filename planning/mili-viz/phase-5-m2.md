# `mili-viz` Phase 5 M2 — render server output (buildable scope)

> Scope doc for Phase 5 Milestone 2, the analogue of
> [`phase-5-m1.md`](phase-5-m1.md) on the renderer side. M1 stood up
> the `wgpu`/`winit` skeleton with a hard-coded triangle and **no**
> mili/proto/server dependency (`phase-5-m1.md` Decisions 38–40). M2
> is the first client milestone that wires the transport in: connect a
> `mili-viz-proto` client to a `mili-viz-server` over the in-process
> transport, send `load`/`show`, resolve the returned `GeometryRef`,
> decode the self-describing `MVG1` blob, and draw that mesh through
> the M1 `Renderer`/`Camera`. No proto change — the M1 contract is
> frozen; Phase 4 is complete and frozen.
>
> Read [`status.md`](status.md) first, then `phase-5-m1.md` and
> [`phase-4-m2.md`](phase-4-m2.md) (the server side that produces this
> geometry). Decision entries continue the global log (Phase 4 ended
> at 34; Phase 6 M1 took 35–37; Phase 5 M1 took 38–40; Phase 5 M2
> starts at 41).

## Goal

README Phase 5 M2: **"render server output. Connect to the in-process
server from Phase 4 M2; draw whatever mesh comes back."**

Concretely, `crates/mili-viz-client`:

- Depends on `mili-viz-proto` + `mili-viz-server` and drives the
  frozen contract: `Subscribe`, `Execute(load)`, `Execute(show)`.
- Reads the broadcast `DELTA_RESULT` `ResultState.geometry`
  `GeometryRef`, resolves its `flight_ticket` through the in-process
  `VizService::fetch_geometry` seam, and decodes the
  Decision-11 `MVG1`/`MVG2` blob into a renderer `Mesh`.
- Draws that indexed mesh through a generalized `Renderer` (depth
  buffer + per-vertex normal shading), viewed through the M1 orbit
  `Camera` auto-framed to the mesh bounds. The hard-coded triangle
  is deleted (M1 Decision 40: it was scaffolding).

Out of scope (later milestones, unchanged): the `egui` dock/controls
and an interactive command line (M3), the AI panel (M3.5), local view
manipulation reconciled against the server-authoritative camera (M4),
**remote** mode over gRPC + Flight TCP (M5). M2 uses the **in-process**
transport only — the README's run mode 1, and the seam
`phase-4-m2.md` Decision 10 froze for exactly this.

## Decisions (continuing the global log)

### Decision 41 — M2 attaches the in-process transport; the `GeometryRef` resolves through the frozen `VizService::fetch_geometry` seam (remote/Flight is M5)

The README sequences the client M1 as "no mili involvement" so the
graphics bring-up is isolated; M2 is "connect to the **in-process**
server from Phase 4 M2". The client therefore now depends on
`mili-viz-proto` (the wire types + client stub) and `mili-viz-server`
(the in-process server it spawns in the same binary — README run mode
1). It connects with `spawn_in_process`, the M1 acceptance-gate
transport, and drives the frozen contract exactly as the server's own
`m2_geometry.rs` does: `Subscribe`, then `Execute(load)`, then
`Execute(show)`, then read the broadcast `DELTA_RESULT`'s
`ResultState.geometry`.

The bulk geometry is resolved through `VizService::fetch_geometry`
(`phase-4-m2.md` Decision 10) — the in-process geometry-store seam,
not an Arrow-Flight `DoGet`. This is deliberate and symmetric with the
server side: Flight-over-TCP is a named **M5** ("remote mode")
deliverable, and `fetch_geometry` is the documented in-process
resolution path the in-process client (sharing the server's address
space) is meant to use. M5 swaps this call for a Flight `DoGet`
client; the ticket, the layout string, and the decoded blob are
frozen, so M5 is a transport swap, not a format or contract change.

**Trade-off recorded.** Standing up a Flight `DoGet` client now would
make M2's fetch path bit-identical to M5's. Rejected: it pulls the
remote-transport client surface forward into the milestone the README
scopes as in-process-only, for zero rendering benefit (the in-process
client does not need the wire); the cost of the M5 swap is a single
localized call site because the blob/ticket are frozen here.

### Decision 42 — the `MVG1`/`MVG2` blob decodes into a renderer `Mesh` with CPU per-vertex normals; the triangle is deleted and the `Renderer` generalized to a depth-tested indexed-mesh pipeline; M2 ignores the `MVG2` scalar

The decoder reads the `phase-4-m2.md` Decision 11 layout
(`MVG1:verts_f32x3+idx_u32+trimat_u32`) and tolerates the M3
`MVG2:...+scalar_f32` superset by **ignoring** the trailing per-vertex
scalar — scalar→color is Phase 5 M3 (`egui` result picker + colormap);
M2 draws the bare hull. It produces a `Mesh` of positions + indices,
and computes **CPU per-vertex normals** (area-weighted face-normal
accumulation) so the hull reads as a 3-D surface under a single fixed
directional light + ambient term rather than a flat silhouette. The M1
hard-coded triangle constant is removed (M1 Decision 40 declared it
throwaway, deleted at M2); the `Renderer` is generalized from a
fixed triangle to an indexed `Mesh` pipeline with a **depth buffer**
(`Depth32Float`, depth-test `Less`) so an overlapping closed hull
renders correctly without relying on consistent face winding for
back-face culling. The M1 render-to-texture-first structure
(Decision 39) and the orbit `Camera` (Decision 40) are unchanged and
reused; `Camera::looking_at(center, radius)` auto-frames the mesh
bounds so the gating render is robust to real corpus coordinate
scales (basic1 is not centered on the origin at unit scale).

### Decision 43 — gating test = an always-on pure decode unit + a skip-on-absent end-to-end render; the M1 camera assertions stay, M1's triangle-pixel assertion is superseded

Mirroring this repo's canonical skip-on-absent convention
(`CLAUDE.md`; M1 Decision 39) and the two-halves shape of
`m1_renderer.rs`:

- **Always-on:** a synthetic in-memory `MVG1` blob decodes to the
  expected positions/indices and a normals array of matching length —
  no GPU, no corpus, hard-gated on every CI box.
- **Skip-on-absent:** the end-to-end path — `spawn_in_process`,
  `load` the `serial/basic1` corpus, `show`, resolve the
  `GeometryRef`, decode, then render headless and assert the mesh
  draws over the clear color (a corner is the clear color, the
  framed-mesh center is not). It prints + early-returns when the
  corpus fixture is absent **or** when no `wgpu` adapter is available
  (no GPU / software rasterizer), exactly as M1's headless half does.

The M1 `camera_*` view-projection assertions remain the always-on
hard gate for the reusable `Camera`. M1's
`headless_render_draws_triangle_over_clear` keeps passing unchanged —
`render_to_image` still renders a built-in unit-triangle `Mesh`
through the new pipeline as a pipeline smoke; the *server-mesh* render
is the new M2 test. No Phase 4 crate is modified; the frozen server
tests are untouched.

## Crate layout (delta from M1)

```
crates/mili-viz-client/
├── Cargo.toml          # + mili-viz-proto, mili-viz-server, tonic, tokio
├── src/
│   ├── lib.rs          # + Mesh, fetch_server_mesh, render_mesh_to_image
│   ├── main.rs         # optional `argv[1]` root → in-process fetch + draw
│   ├── camera.rs       # + Camera::looking_at(center, radius)
│   ├── mesh.rs         # NEW: MVG1/MVG2 decode → Mesh (+ normals, bounds)
│   ├── session.rs      # NEW: in-process load/show → decoded Mesh
│   ├── renderer.rs     # triangle deleted; indexed mesh + depth buffer
│   ├── mesh.wgsl       # NEW: normal-lit mesh shader (triangle.wgsl gone)
│   └── app.rs          # winit wrapper now carries an optional Mesh
└── tests/
    ├── m1_renderer.rs  # unchanged (camera math + triangle smoke)
    └── m2_render_server_output.rs  # NEW (always-on decode + skip-on-absent)
```

## Acceptance gate

- `cargo test --workspace --exclude mili-py` builds `mili-viz-client`;
  the always-on decode unit and the M1 `camera_*` assertions pass.
- `cargo clippy --workspace --all-targets -- -D warnings` and
  `cargo fmt --all --check` are clean.
- The end-to-end render passes when the `serial/basic1` corpus **and**
  a `wgpu` adapter are present, and is skipped (printed, not failed)
  when either is absent — the documented skip-on-absent convention.
- No Phase 4 crate is touched; every Phase 4 server gating test
  (M1×6 + m2 + m3 + m4 + m5 + m5b + m5c + m5d + m6) is unchanged and
  green. No `mili_viz.proto`/blob/ticket change.

## Decision log (this doc)

| # | Title | Resolves |
|---|---|---|
| 41 | In-process transport attached; `GeometryRef` via the frozen `fetch_geometry` seam; remote/Flight is M5 | M2 transport |
| 42 | `MVG1`/`MVG2` → `Mesh` (+ CPU normals); triangle deleted; depth-tested indexed pipeline; `MVG2` scalar ignored (M3); auto-framing camera | M2 render |
| 43 | Gating test = always-on decode unit + skip-on-absent end-to-end render; M1 camera gate kept, triangle smoke kept | M2 test |
</content>
</invoke>
