# `mili-viz` — client / server visualization stack

> **Status: ⏳ NOT STARTED — design phase, needs more planning
> iterations before implementation.** This README is the architecture;
> the live tracker, the open design questions, and the concrete next
> steps are in **[`status.md`](status.md) — start there.** Phases 1–3
> (`mili-rs` + `milox`) are complete; `mili-viz` is the remaining work
> and no `mili-viz-*` crate exists yet.

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

## Phase 4 (server) milestones

1. **M1 — proto crate + in-process transport.** Define commands;
   stand up a `tonic` server stub that accepts them over an in-memory
   channel. Also defines the multi-client surface (subscription RPC +
   server→client `StateDelta` stream + version handshake) that the
   scripting client and live GUI sync depend on — see `scripting.md`.
2. **M2 — load + state navigation.** `load`, `state`, `next`, `prev`.
   Server streams back vertex + index buffers for the loaded mesh
   at each state.
3. **M3 — primal result display.** `show <svar>`. Server colors
   vertices from a `mili-rs` query and streams the color array.
4. **M4 — selection and enable/disable.** Mesh filtering, material
   visibility. The natural griz command set is the spec.
5. **M5 — derived results.** Port the most-used derived computations
   from griz (stress invariants first, then strain). `rayon` for the
   per-element loops.
6. **M6 — remote transport.** Same proto over gRPC + Arrow Flight on
   a TCP socket. Validate over a real network mount.

## Phase 5 (client) milestones

1. **M1 — `wgpu` renderer skeleton.** Window, camera, a hard-coded
   triangle. No mili involvement.
2. **M2 — render server output.** Connect to the in-process server
   from Phase 4 M2; draw whatever mesh comes back.
3. **M3 — `egui` controls.** State scrubber, result picker, view
   controls, command-line entry.
4. **M4 — local view manipulation.** Rotate/zoom without
   round-tripping the server.
5. **M5 — remote mode.** Connect to a remote server; tune buffer
   sizes for typical HPC network latency.

## Open questions

- **Picking.** griz supports picking elements/nodes via mouse. Round
  trip or do it client-side from cached geometry? Probably
  client-side with a separate "describe picked id" RPC.
- **Time-history plots.** griz embeds 2D plots
  (`reference/griz/Src/time_hist.c`). On the client, `egui_plot` is
  good enough for v1.
- **Scripting.** Resolved — see `scripting.md`. A pip-installable
  pure-Python package is a second client of `mili-viz-proto` (no
  Visit-style bundled interpreter). Camera is server-authoritative;
  interactive `attach()` to a running GUI session is the priority
  connection path. `grizinit`-style batch files keep working via
  `session.run_script(path)`. This expands Phase 4 M1 with a
  subscription RPC + `StateDelta` stream + version handshake.
- **Backwards-compatible CLI.** Whether the new client binary
  accepts griz's command-line flags. Probably yes for the common
  ones (`-i`, `-b`), as a courtesy.
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
