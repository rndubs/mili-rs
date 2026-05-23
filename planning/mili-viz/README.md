# `mili-viz` — client / server visualization stack

> **Status: Phase 4 (server) ✅ COMPLETE; Phase 5 (client) 🟢 IN
> PROGRESS; Phase 6 (`pygriz` scripting client) 🟢 IN PROGRESS.**
> This README is the original architecture/rationale; the live
> tracker, the open design questions, and the concrete next steps
> are in **[`status.md`](status.md) — start there.** Crates landed:
> `crates/mili-viz-proto`, `crates/mili-viz-server`,
> `crates/mili-viz-client`, and `python/pygriz/`.

A from-scratch replacement for griz, split into a server that does the
heavy I/O and a thin renderer client. The server links `mili-rs`
directly; the client speaks a small RPC protocol to the server and
draws with `wgpu` + `egui`.

## Why split

griz today is a 115k-LOC single-process monolith
(`reference/griz/Src/`): immediate-mode GL renderer, Motif UI, direct
`libmili` calls in the same address space. That worked when datasets
fit on a workstation. The split exists because:

- HPC datasets live on the cluster filesystem. Round-tripping state
  files to a laptop is unacceptable.
- We want to keep the renderer light and the data-prep heavy, with
  the heavy side colocated with the data.
- The same server can drive a Rust client, a notebook, or a future
  web client without each one re-implementing the I/O layer.

## Components

### `mili-viz-proto`

The shared RPC types. A small command vocabulary lifted from griz's
command interpreter (`reference/griz/Src/interpret.c`):

- `load <root>`, `close`
- `state <N>`, `next`, `prev`, `first`, `last`
- `select <class> <range>`, `clrsel <class>`
- `show <result_name> [opts]`
- `rot <axis> <angle>`, `tx <d>`, `ty <d>`, `tz <d>`, `scale <f>`
- `iso on/off <result> <min> <max>`
- `contour <result>`
- `enable/disable <class> [<material>]`
- `zf <f>`, `zb <f>`

griz already supports these as a line-oriented command stream that
can be batched from a `grizinit` file. That stream is our natural
protocol surface. The server accepts the same strings; the client
emits them.

The wire format is `tonic`/gRPC with Arrow Flight for bulk geometry
payloads. Commands themselves are tiny; the responses (vertex
buffers, index buffers, color arrays) are where Flight pays off.

### `mili-viz-server`

Owns:

- A `mili-rs` `Database` per loaded run.
- A mesh prep cache: extracted faces, per-class vertex/index buffers,
  material lookup tables. Replaces griz's
  `MO_class_data.data_buffer` (`reference/griz/Src/mesh.h:208`).
- A result computation pipeline. Primal results come straight out of
  `mili-rs`; derived results (stress invariants, strain measures,
  isosurfaces, contours) run here with `rayon`. The current griz
  implementations under `Src/stress.c`, `Src/strain.c`,
  `Src/iso_surface.c`, `Src/contour.c` are the spec.
- A command dispatcher that turns RPC commands into mutations of
  the session state and streams back the geometry deltas.

The server has no rendering code. It does not link `wgpu`. It only
produces vertex/index/color arrays in well-defined layouts.

### `mili-viz-client`

Owns:

- A `wgpu` renderer for the geometry the server sends.
- An `egui` UI (panel for state navigation, result picker, view
  controls, command line for power users — mirroring griz's text
  command input).
- A local `mili-viz-proto` client. Two run modes:
  1. **In-process** — the client spawns a server in the same
     binary, communicating over channels. The default for local
     workstation use.
  2. **Remote** — connect to a server on an HPC login node over
     gRPC + Flight.

Caches the last frame's geometry locally so the client can *predict*
view manipulation (rotation, zoom) responsively, but the camera is
**server-authoritative**: the client sends the view command and
reconciles against the server's broadcast, so a script and an open
GUI window stay in sync. Result selection and state stepping
round-trip. See `scripting.md` for the full rationale.

## Why `wgpu` + `egui`

- **Cross-platform.** Vulkan on Linux, Metal on macOS, DX12 on
  Windows, GL fallback for older HPC viz nodes. Same source.
- **Mature.** `rerun.io` ships a scientific viewer on this exact
  stack at production quality. The mili viz use case is narrower
  than rerun's.
- **Matches griz's UI shape.** griz's UI is minimal — text command
  input, a few menus, a render window. `egui`'s immediate-mode model
  is a direct fit; we are not trying to ship a ParaView competitor.
- **Pure Rust.** Avoids dragging Qt or Motif into the build.

Alternatives considered and rejected: `slint` (less GL/wgpu
integration story), `bevy` (overkill, ECS we do not need), Qt
(Rust bindings still rough, and we wanted no C++ in the build).

## Phase 4 (server) milestones — ✅ all landed

1. **M1** — proto crate (`crates/mili-viz-proto`, protoc-free
   `protox` codegen) + in-process `tokio::io::duplex` transport.
2. **M2** — real `mili-rs`-backed `load`/state-nav + per-state
   triangulated hull via the frozen `GeometryRef`.
3. **M3** — primal result display (`show <result> [component]`),
   per-vertex scalar in the `MVG2` blob.
4. **M4** — selection (metadata-only) + `enable`/`disable` material
   visibility (filters emitted triangles).
5. **M5** — derived results: scalar invariants (M5), eigenvalue
   families (M5b), surfstrain + nodal-time (M5c), `*_alt` trig
   principal strains (M5d).
6. **M6** — gRPC + Arrow Flight over TCP (`serve_tcp`).

See [`status.md`](status.md) and the per-milestone
`phase-4-mN.md` files for landed-summary detail.

## Phase 5 (client) milestones

1. **M1** ✅ — `wgpu` renderer skeleton (`crates/mili-viz-client`).
2. **M2** ✅ — render server output (in-process transport,
   `MVG1`/`MVG2` decode, depth-tested indexed pipeline).
3. **M3** ✅ — `egui` shell (toolbar / left dock / five overlays /
   status bar; additive non-clearing second pass).
4. **M3.5** ✅ — bottom tabs (Layer-0 command line + scripting
   placeholder + `egui_plot` time-history).
5. **M4** ✅ — local view manipulation (predict + reconcile against
   server-authoritative camera) + extensive MVP polish.
6. **M5** ⏳ — remote mode (wire `connect`/`attach` over the
   landed gRPC + Flight TCP transport; HPC-latency buffer tuning).
7. **M6** ⏳ — agent integration polish (`client.md`).

## Phase 6 (`pygriz` scripting client) milestones

1. **M1** ✅ — scaffold + `connect`/`Hello` handshake +
   Layer-0 `command()`/`run_script()`.
2. **M2** ✅ — `attach()`/`launch()`/`list_sessions()` +
   server-side session file.
3. **M3** ✅ — Layer-1 object API + Layer-0 ≡ Layer-1 test.
4. **M4** ⏳ — live sync (`Subscribe` → `@s.on(...)`).
5. **M5** ⏳ — `query`/`to_dataframe` (Arrow Flight for large
   results).
6. **M6** ⏳ — `render`/`save_animation`/`snapshot`.

## Open questions

- **Picking.** ✅ Resolved — `phase-4-m1.md` Decision 2. Computed
  **client-side** from the cached `GeometryRef` buffers; the
  status-bar "describe picked id" readout is one ordinary `Query`
  for `(class, label, current state)` — **no separate RPC, no M1
  proto change**. A pick that changes selection emits the existing
  `Select` command (broadcasts like any mutation).
- **Time-history plots.** ✅ Resolved — `phase-4-m1.md` Decision 3.
  Client-side `egui_plot` (Phase 5 M3.5 bottom tab), fed by the
  **existing** `Query` over a state range (`time_hist.c`
  (`reference/griz/Src/time_hist.c`) is just a rendering of that
  data). No server plot RPC; **no M1 proto change**.
- **Derived-result validation (no oracle).** ✅ Resolved, then
  **superseded** at M5 — `phase-4-m1.md` Decision 5 →
  `phase-4-m5.md` Decision 19. Decision 5's premise ("no `mili`-style
  oracle for viz") proved false: Phases 1–3 already ported every
  derived expression into `crates/mili-rs/src/derived.rs`, **bit-exact
  validated against the `mili` Python package** by the frozen
  parity suite. M5-viz therefore reuses that kernel
  (`mili_rs::compute_stress_invariant` &c.) — no formula re-port, no
  committed griz golden, no `parity` feature in `mili-viz-server`; the
  gating test validates the *viz routing* via the linear-pressure
  identity (`phase-4-m5.md` Decisions 19–21). The boundary held
  through M5b/M5c/M5d — including the final `*_alt` family, whose core
  kernel is gated vs the oracle to a tight **f32 tolerance** (not
  bitwise: numpy's float32 `arccos`/`cos` are its own SIMD
  polynomials, not cross-language bit-reproducible; `../mili-py/m4.md`
  Decision 27) while viz still merely *reuses* it (no formula re-port,
  no `parity` feature; `phase-4-m5d.md`). **No `mili_viz.proto`
  change** at any M5 sub-slice.
- **Server-hosted agent on the critical path.** ✅ Resolved —
  `phase-4-m1.md` Decision 6. The agent **wire contract** is frozen
  in M1 (so the `protocol_version` handshake never breaks); the
  agent **implementation** + the local-LLM model choice
  (`agent-local-llm*.md`) are **off the M1–M5 critical path**
  (Phase 4/5 M6), capability-gated behind `agent`. The panel is not
  dropped — it is sequenced last and isolated behind a flag.
- **Scripting.** Resolved — see `scripting.md`. A pip-installable
  pure-Python package is a second client of `mili-viz-proto` (no
  Visit-style bundled interpreter). Camera is server-authoritative;
  interactive `attach()` to a running GUI session is the priority
  connection path. `grizinit`-style batch files keep working via
  `session.run_script(path)`. This expands Phase 4 M1 with a
  subscription RPC + `StateDelta` stream + version handshake.
- **Backwards-compatible CLI.** ✅ Resolved — `phase-4-m1.md`
  Decision 4. The client accepts only the portable subset —
  `-i <base>` → initial `load`, `-b`/`-batch <file>` →
  `run_script`, `-V` → version, `-w <w> <h>` → window size. The rest
  of griz's flags (`reference/griz/Src/viewer.c:2900`) are
  Motif/X11/launcher-specific and dropped. Client-only; no proto.
- **Edge rendering — thicker / anti-aliased lines.** Open follow-up.
  Today the VB-003 element-edge pass uses `wgpu::PrimitiveTopology::LineList`,
  which is fixed at 1 device pixel (core WebGPU has no `lineWidth`).
  We landed **4× MSAA on the windowed path** (`renderer.rs`
  `Renderer::new_with_samples`, called with `4` from `app.rs`) plus a
  black edge colour (`edges.wgsl`) which fixes ~80% of the broken-edge
  look — but at HiDPI the lines still read thin. If we ever want
  variable-width, crisply anti-aliased edges, the proper fix is a
  **screen-space line-quad pass**: expand each edge to two triangles
  in the vertex shader using the endpoints + a screen-space normal,
  then do an AA falloff in the fragment shader. ~60 lines of new
  WGSL, a new pipeline, and a `tweaks.rs` thickness knob; the
  `Mesh::edge_indices` / `element_edges` wiring upstream of it stays
  put. An alternative is a **barycentric wireframe** rendered inside
  the mesh fragment shader (cheapest GPU cost) but it conflicts with
  the current indexed vertex sharing — needs un-indexing or vertex
  duplication. MSAA was deliberately kept off the headless paths
  (`render_mesh_to_image*`, `render_shell_to_image`) so the VB-001 /
  status 23 byte-stable composite gate stays pixel-exact.
- **Client wireframe + AI-first design.** Resolved — see
  `client.md`. The window mirrors griz's shape (left dock for
  Results/Materials/Surfaces, center viewport, bottom tabs for
  command-line/scripting/time-history) with an AI Assistant as a
  first-class panel. Key decisions (2026-05-17): the agent is a
  **server-hosted** service (colocated with the data, one API key,
  conversation is broadcast shared state); it is a peer of the
  command vocabulary, not a second mechanism; fully autonomous with
  barge-in + a provenance journal; debugging is data-first
  (`Query`) with `Snapshot` corroboration. Expands Phase 4 M1 with
  `AgentChat`, a `DELTA_AGENT` broadcast kind, `Snapshot`, and
  `Interrupt`; adds Phase 5 M3.5/M6.
