# `mili-viz` status — live tracker (START HERE)

> **This is the single source of truth for Phase 4/5/6 (`mili-viz`).**
> The `mili-rs` core and the `milox` Python bindings (Phases 1–3) are
> **complete and frozen** — see [`../mili-rs/status.md`](../mili-rs/status.md)
> and [`../mili-py/README.md`](../mili-py/README.md). All remaining
> work in this repo is `mili-viz`.
>
> **Quick phase status:**
> - **Phase 4 (`mili-viz` server): ✅ M1–M6 LANDED** (no deferred
>   derived families); **🟡 M7–M9 PLANNED** — post-MVP volumetric
>   batch (`MVG3` blob → cut-plane → slice; see
>   [`phase-4-m7.md`](phase-4-m7.md) / [`phase-4-m8.md`](phase-4-m8.md)
>   / [`phase-4-m9.md`](phase-4-m9.md)).
> - **Phase 5 (`mili-viz` client): 🟢 M1–M9 + MVP polish all landed.**
>   The agent integration polish (M6) and the volumetric batch
>   (M7/M8/M9) are now complete on the client side; Phase 5 itself is
>   complete with no remaining work.
> - **Phase 6 (`pygriz` scripting client): 🟢 M1–M3 landed;
>   M4/M5/M6 not started.**
>
> **Client wireframe coverage** (placeholder/partial inventory at a
> finer grain than the milestones below) lives in its own tracker:
> [`wireframe-parity.md`](wireframe-parity.md).

## TL;DR — where we are

- **Phase 4 (`mili-viz` server): ✅ COMPLETE.** M1 (proto + in-process
  transport), M2 (load + state-nav + real geometry), M3 (primal result
  coloring), M4 (selection + enable/disable), M5 (derived: scalar
  stress invariants, +M5b eigenvalue families, +M5c `surfstrain*` +
  nodal-time, +M5d `*_alt` trig principal-strain — derived story now
  fully complete with no deferrals), M6 (gRPC + Arrow Flight over TCP)
  all landed. Per-milestone scope/decisions in
  [`phase-4-m1.md`](phase-4-m1.md) through [`phase-4-m6.md`](phase-4-m6.md);
  the frozen `mili_viz.proto` was untouched after M1.
- **Phase 5 (`mili-viz` client): 🟢 COMPLETE — M1 ✅, M2 ✅, M3 ✅,
  M3.5 ✅, M4 ✅, M5 ✅, M6 ✅, M7 ✅, M8 ✅, M9 ✅ landed (+ the
  MVP-polish rollup; see "Immediate next steps" item 23).** Phase 5
  is complete; only Phase 6 M4/M5/M6 remain.
  - **M1 — `wgpu` renderer skeleton.** New standalone
    `crates/mili-viz-client` (`wgpu` 29 / `winit` 0.30 / `glam`, no
    mili dep); orbit `Camera` field-aligned to the frozen
    `CameraState` + render-to-texture-first scaffold.
    [`phase-5-m1.md`](phase-5-m1.md) (Decisions 38–40).
  - **M2 — render server output.** Client gains `mili-viz-proto`+
    `-server` deps; `fetch_server_mesh` spawns an in-process server
    and renders the decoded `MVG1`/`MVG2` blob with an auto-framed
    orbit camera. [`phase-5-m2.md`](phase-5-m2.md) (41–43).
  - **M3 — `egui` shell.** `egui` 0.34.2 paints the L1 layout
    (toolbar / left dock / overlays / status bar) as an additive
    second pass over the byte-stable mesh pass; `MVG2` drives a
    cool→warm colormap autoscaled by `ResultState.{min,max}`.
    [`phase-5-m3.md`](phase-5-m3.md) (44–47).
  - **M3.5 — bottom tabs.** Layer-0 command line, scripting runner
    (initially a placeholder, lit up in the MVP-polish rollup),
    `egui_plot` time-history; default-collapsed body keeps the M3
    composite seam byte-stable. [`phase-5-m3.5.md`](phase-5-m3.5.md)
    (48–52).
  - **M4 — local view manipulation + pre-M4 hardening.**
    Predict-and-reconcile mouse orbit/pan/zoom (radians end-to-end),
    last-broadcast-wins against `DELTA_CAMERA`; absolute auto-frame
    via `SetCamera`; `Colormap`/`LegendLimits` honoured client-side
    via a viz-local named-ramp table + effective-range. Pre-M4: the
    HiDPI `Surface::configure` abort + the never-coded griz-subset
    CLI ([`phase-4-m1.md`](phase-4-m1.md) Decision 4). No proto
    change. [`phase-5-m4.md`](phase-5-m4.md) (62–66).
  - **M5 — remote mode (gRPC + Arrow Flight over TCP).**
    `Session::connect_tcp(endpoint, root)` and `Session::attach(id,
    root)` ride the same Phase 4 M6 wire `pygriz` already speaks: one
    tuned `tonic::Channel` cloned for `MiliVizClient` +
    `FlightServiceClient`, ticket resolution streams Flight `DoGet`
    chunks into the byte-identical M2/M3 blob, `fetch_catalog` fronts
    the same `CATALOG_TICKET` over the same Flight client. CLI gains
    `-r`/`--remote <host:port>` and `--attach [<id>]` (mutually
    exclusive, in-process stays the default). `--attach` reuses the
    same `~/.griz/sessions/<id>.json` resolver the server binary's
    `main` writes (newest-live pid liveness via `kill(pid, 0)` —
    Phase 6 M2 Decision 56/57). HPC-latency tuning: `tcp_nodelay`,
    TCP + HTTP/2 keep-alives, explicit 10 s `connect_timeout`. The
    in-process arm is byte-stable: `M1` `m1_renderer.rs` / `M2`
    `m2_render_server_output.rs` / `M3` `m3_egui_shell.rs` / `M3.5`
    `m3_5_bottom_tabs.rs` / `M4` `m4_view_manipulation.rs` and the
    MVP-polish suite unchanged and green. No `.proto` change; no
    Phase 4 crate touched. Scope/decisions:
    [`phase-5-m5.md`](phase-5-m5.md) (90–93). Gating test
    `crates/mili-viz-client/tests/m5_remote_mode.rs` (5 always-on:
    CLI dispatch / mutual exclusion / `attach`-missing-id /
    `attach`-empty-dir / `attach`-with-dead-pid; 3 skip-on-absent
    real-TCP end-to-end against `serial/basic1`: byte-identical
    `resolve_geometry`, Flight `fetch_catalog`, `attach()`
    round-trip).
- **Phase 6 (`pygriz` scripting client): 🟢 IN PROGRESS — M1 ✅, M2 ✅,
  M3 ✅ landed. M4/M5/M6 ⏳ not started.**
  - **M1 — scaffold + stubs + connect/handshake.**
    `griz.connect(host, port, token=...)`, `Hello` version handshake
    (mismatch → warning, never exception), Layer-0
    `session.command()`/`run_script()` lowering to verbatim
    `Command{raw}` (server's `parse_raw` is the one parser).
    Gitignored stub generator off the canonical proto.
    [`phase-6-m1.md`](phase-6-m1.md) (35–37 & 53–55).
  - **M2 — connection model.** `attach()` (priority: newest live
    `~/.griz/sessions/<id>.json`), `attach(id=)`/`attach(host=,port=)`,
    `launch(gui=)`, `list_sessions()`. The session file is written by
    the **binary's `main`** — frozen library transport / proto / Hello
    echo byte-untouched. Discharges the Phase 5 M3.5 scripting-tab
    placeholder. [`phase-6-m2.md`](phase-6-m2.md) (56–58).
  - **M3 — Layer-1 object API.** `s.open/state/.../show/isosurface/
    contour/cutplane/colormap` + `s.selection`/`s.materials`/
    `s.legend` + `s.view.*` (server-authoritative); typed handles
    (`Result.range`, `Isosurface.remove()`). Every Layer-1 call lowers
    to a **typed `Command` oneof variant, never `raw`**; the Layer-0
    ≡ Layer-1 invariant pinned two ways (fake-stub lowering pin +
    skip-on-absent identical-`DELTA_SNAPSHOT` against two real
    `launch()`ed servers). `crates/` byte-for-byte untouched.
    [`phase-6-m3.md`](phase-6-m3.md) (59–61).

## What is already decided (read these first)

| Doc | What it pins | State |
|---|---|---|
| [`phase-4-m1.md`](phase-4-m1.md) | Consolidated Phase 4 M1 wire contract (base + scripting + agent vocab); Δ1–Δ9 from the proto draft; M1 acceptance gate; Decisions 1–7 resolving open Q3–Q8 | ✅ pinned + landed (2026-05-17) |
| [`phase-4-m2.md`](phase-4-m2.md) | Phase 4 M2 `mili-rs`-backed `load`/state-nav/geometry; in-process geometry store keyed by frozen `flight_ticket`, self-describing `MVG1` blob, per-state `nodpos`. Decisions 10–12. No proto change | ✅ pinned + landed (2026-05-17) |
| [`phase-4-m3.md`](phase-4-m3.md) | Phase 4 M3 primal result display; leaf-svar resolution, optional `MVG2` per-vertex `scalar_f32`, element→nodal-average/nodal-direct/vector→comp0, autoscale in `ResultState.{min,max}`. Decisions 13–15 | ✅ pinned + landed (2026-05-17) |
| [`phase-4-m4.md`](phase-4-m4.md) | Phase 4 M4 selection + enable/disable; `enable`/`disable` filters emitted triangles by per-triangle material, selection metadata via existing `DELTA_SELECTION`+`Snapshot`. Decisions 16–18 | ✅ pinned + landed (2026-05-17) |
| [`phase-4-m5.md`](phase-4-m5.md) | Phase 4 M5 first slice: scalar stress invariants reuse the parity-exact `mili_rs::compute_stress_invariant` (supersedes `phase-4-m1.md` Decision 5 — no formula port, no griz golden). Decisions 19–21 | ✅ pinned + landed (2026-05-17) |
| [`phase-4-m5b.md`](phase-4-m5b.md) | Phase 4 M5 follow-up: 14 eigenvalue families (principal/deviatoric stress+strain, max-shear, vol-strain) via the verbatim M5 seam. Decisions 22–24 | ✅ pinned + landed (2026-05-17) |
| [`phase-4-m5c.md`](phase-4-m5c.md) | Phase 4 M5 third slice: `surfstrain*` via a separate per-face Hex gather + nodal-time families via a factored node-direct mapping. Decisions 28–31 | ✅ pinned + landed (2026-05-17) |
| [`phase-4-m5d.md`](phase-4-m5d.md) | Phase 4 M5d: `*_alt` trig principal-strain variants; Part A `mili-rs` core kernel (gated to f32 tolerance, not bitwise — numpy `arccos`/`cos` non-reproducible), Part B trivial viz routing. Decisions 32–34; discharges `phase-4-m5c.md` Decision 28 | ✅ pinned + landed (2026-05-17) |
| [`phase-4-m6.md`](phase-4-m6.md) | Phase 4 M6 remote transport: gRPC + Arrow Flight over TCP via vendored `Flight.proto` on the existing protoc-free `protox` path; `DoGet` only, blob/ticket byte-stable. Decisions 25–27 | ✅ pinned + landed (2026-05-17) |
| [`phase-5-m1.md`](phase-5-m1.md) | Phase 5 M1: standalone `crates/mili-viz-client` (`wgpu`/`winit`/`glam`, no mili dep); orbit `Camera` aligned to frozen `CameraState`; render-to-texture-first scaffold. Decisions 38–40 | ✅ pinned + landed (2026-05-17) |
| [`phase-5-m2.md`](phase-5-m2.md) | Phase 5 M2: client wired to `mili-viz-proto`+`-server`; decodes `MVG1`/`MVG2` to a `Mesh` via the frozen in-process `fetch_geometry` seam; depth-tested indexed pipeline + auto-frame. Decisions 41–43 | ✅ pinned + landed (2026-05-17) |
| [`phase-5-m3.md`](phase-5-m3.md) | Phase 5 M3 `egui` shell (0.34.2, verified vs frozen wgpu 29/winit 0.30): toolbar/left-dock/overlays as an additive non-clearing second pass over the byte-stable mesh pass; `MVG2`→cool→warm colormap. Decisions 44–47 | ✅ pinned + landed (2026-05-17) |
| [`phase-5-m3.5.md`](phase-5-m3.5.md) | Phase 5 M3.5 bottom tabs: Layer-0 command line, scripting placeholder, `egui_plot` time-history; default-collapsed body keeps the M3 footprint byte-stable. `egui_plot` 0.35.0 pinned. Decisions 48–52 | ✅ pinned + landed (2026-05-17) |
| [`phase-6-m1.md`](phase-6-m1.md) | Phase 6 M1: `pygriz` scaffold, gitignored stub generator, `connect()`+Hello handshake, Layer-0 `command()`/`run_script()` lowering to verbatim `Command{raw}`. Decisions 35–37 & 53–55 | ✅ pinned + landed (2026-05-17) |
| [`phase-6-m2.md`](phase-6-m2.md) | Phase 6 M2 connection model: `attach()`/`launch()`/`list_sessions()`; the session file is written by the **binary's `main`** so the frozen library/proto/Hello echo are untouched. Discharges `phase-5-m3.5.md` Decision 49. Decisions 56–58 | ✅ pinned + landed (2026-05-18) |
| [`phase-6-m3.md`](phase-6-m3.md) | Phase 6 M3 Layer-1 object API; every call lowers to a typed `Command` oneof (never `raw`); Layer-0 ≡ Layer-1 pinned two ways (fake-stub + identical-`DELTA_SNAPSHOT`). `crates/` byte-for-byte untouched. Decisions 59–61 | ✅ pinned + landed (2026-05-18) |
| [`phase-5-m4.md`](phase-5-m4.md) | Phase 5 M4 local view manipulation + pre-M4 hardening: predict-and-reconcile mouse orbit, radians end-to-end, client-side colormap/legend, HiDPI fix, griz-subset CLI. Decisions 62–66 | ✅ pinned + landed (2026-05-18) |
| [`phase-5-m5.md`](phase-5-m5.md) | Phase 5 M5 remote mode: `Session::connect_tcp`/`Session::attach` ride the Phase 4 M6 wire (one tuned `tonic::Channel` cloned for `MiliVizClient`+`FlightServiceClient`; Flight `DoGet` blob is byte-identical to in-process `fetch_geometry`); CLI `-r`/`--remote <host:port>` + `--attach [<id>]`; HPC-latency tuning (`tcp_nodelay`, TCP+HTTP/2 keep-alives, 10 s `connect_timeout`). No proto change. Decisions 90–93 | ✅ pinned + landed (2026-05-23) |
| [`phase-5-m6.md`](phase-5-m6.md) | Phase 5 M6 agent integration polish: lights up the frozen `AgentChat`/`Interrupt`/`CaptureFrame`/`DELTA_AGENT`/`Snapshot.agent` surface with `AgentBackend` trait + always-on `MockAgent` (real LLM backend gated separately); commands flow through the existing `VizService::dispatch` seam (tagged with the agent's `origin_client_id` per `client.md` §"Design principle"); per-turn client-side snapshot for `↶ revert to here`; `CaptureFrame` returns a deterministic placeholder PNG (production wgpu offscreen swap deferred); peer count rides `AgentStatus.detail = "peers=N"` gated on the `agent` capability. Zero `.proto` change. Decisions 94–99 | ✅ pinned + landed (2026-05-23) |
| [`phase-4-m7.md`](phase-4-m7.md) | Phase 4 M7 volumetric geometry contract (`MVG3`): superset of `MVG2`, length-prefixed, free-form-tag carrier (zero `.proto` change), per-superclass element-edge buffer (fixes the hex-face-diagonal wireframe artifact / VB-005), opt-in interior-triangle emit via `MaterialVisibility` sentinel. **Supersedes `phase-4-m2.md` Decision 11 additively.** Decisions 72–74 | 🟡 planned (drafted 2026-05-23) |
| [`phase-4-m8.md`](phase-4-m8.md) | Phase 4 M8 cut-plane operator: wires the long-frozen `Cmd::Cutplane` arm (`crates/mili-viz-server/src/lib.rs:528` stub), closed clipped hull (kept-side ∪ cap), per-superclass marching tables, rayon parallel-per-element, session-state that composes with `show`/state-step/material toggles. Decisions 75–77 | 🟡 planned (drafted 2026-05-23) |
| [`phase-4-m9.md`](phase-4-m9.md) | Phase 4 M9 slice operator: additive `slice_only: bool` on `CutPlane` (**second** post-M1 proto change), cap-only emit, scalar interpolation linear along straddled edges, composes with cut. Decisions 78–80 | 🟡 planned (drafted 2026-05-23) |
| [`phase-5-m7.md`](phase-5-m7.md) | Phase 5 M7 render modes consuming `MVG3`: `Translucent`/`Xray`/`Interior` arms; `Edges`/`Wireframe` prefer `Mesh::element_edges` (VB-005 client side), fall back to extractor (byte-stable for older servers); interior is a server round-trip via the M7 sentinel. Decisions 81–83 | 🟡 planned (drafted 2026-05-23) |
| [`phase-5-m8.md`](phase-5-m8.md) | Phase 5 M8 cut-plane gizmo + Rendering→Cut UI: egui-overlay handle (no new `wgpu` pipeline), 30 Hz wall-clock-throttled preview with un-throttled drag-end commit, `Preferences → Interactive clip` suppress for low-bandwidth links (cross-session-persisted via `tweaks.json`). Decisions 84–86 | ✅ pinned + landed (2026-05-23) |
| [`phase-5-m9.md`](phase-5-m9.md) | Phase 5 M9 slice gizmo + Rendering→Slice UI: thin sibling to M8 (shared gizmo machinery), distinct status-bar readout, slice-cap colormap-painted when a result is mapped; slice always opaque by default. Decisions 87–89 | 🟡 planned (drafted 2026-05-23) |
| [`README.md`](README.md) | Server/client split, crate layout, transport + renderer stack, Phase 4/5 milestone outline | ✅ architecture settled (stale on status/Phase 6 — `status.md` authoritative) |
| [`scripting.md`](scripting.md) | Scripting = second pure-Python client of `mili-viz-proto`; camera server-authoritative; `attach()` to a running GUI; `grizinit` via `run_script()`. Implementation home: Phase 6 | ✅ resolved |
| [`client.md`](client.md) | Client wireframe (griz-shaped docks) + AI-first design; server-hosted agent peer with barge-in + provenance journal. Adds `AgentChat`/`DELTA_AGENT`/`Snapshot`/`Interrupt` to M1; introduces Phase 5 M3.5/M6 | ✅ resolved (2026-05-17) |
| [`agent-local-llm.md`](agent-local-llm.md), [`agent-local-llm-posttraining.md`](agent-local-llm-posttraining.md), [`posttraining-dataset.md`](posttraining-dataset.md), [`agent-local-llm-baseline.md`](agent-local-llm-baseline.md) | Local-LLM agent investigation (model + post-training) + the dataset-construction build plan + the v0 baseline plan (the next concrete milestone). Progress in § "Local LLM agent (exploratory)" below | 🔎 research notes — not yet binding; v0 baseline plan drafted |

The reference implementation we are porting from is read-only under
`reference/griz/Src/` (cited by file:path in the docs above).

## Open design questions

All blocking questions are now **resolved or explicitly deferred
with a reason** in [`phase-4-m1.md`](phase-4-m1.md). "✅" = decided;
"⏸️" = deliberately deferred, non-blocking.

| # | Question | State | Where |
|---|---|---|---|
| 1 | Scripting client model + camera authority | ✅ resolved | `scripting.md` |
| 2 | Client wireframe + AI assistant as a first-class panel | ✅ resolved | `client.md` |
| 3 | **Phase 4 M1 surface = union of base RPC + scripting + agent, as one consolidated buildable spec** | ✅ resolved — the blocking item is closed | `phase-4-m1.md` Decision 1 (Δ1–Δ9) |
| 4 | **Picking** — server round-trip vs. client-side | ✅ resolved: client-side from cached `GeometryRef`; readout reuses `Query`; no M1 proto | `phase-4-m1.md` Decision 2 |
| 5 | **Time-history plots** — client vs. server | ✅ resolved: client-side `egui_plot` (Ph5 M3.5) fed by existing `Query`; no M1 proto | `phase-4-m1.md` Decision 3 |
| 6 | **Backwards-compatible CLI** — griz flags | ✅ resolved: portable subset only (`-i`/`-b`/`-V`/`-w`); rest dropped; client-only, no proto | `phase-4-m1.md` Decision 4 |
| 7 | **Local-LLM agent**: model / post-training / host-runtime | ⏸️ deferred (non-blocking): agent *contract* in M1; *impl* + model choice **off the M1–M5 critical path** (Ph4/5 M6), capability-gated | `phase-4-m1.md` Decision 6; research in `agent-local-llm*.md` |
| 8 | Derived-result port + validation (no Python oracle) | ✅ resolved, then **superseded at M5**: the premise was false — `mili-rs::derived` is bit-exact vs the `mili` Python package (frozen parity suite), so M5-viz reuses that kernel (no formula port, no griz golden, no `parity` feature in `mili-viz-server`) | `phase-4-m1.md` Decision 5 → **`phase-4-m5.md` Decision 19** |

## Phase 4 — `mili-viz` server (✅ COMPLETE)

Milestones from [`README.md`](README.md) § "Phase 4 milestones",
expanded by `scripting.md` / `client.md`. None started.

- [x] **M1 — proto crate + in-process transport.** ✅ **Landed.**
      `crates/mili-viz-proto` + `crates/mili-viz-server` in the
      workspace; the frozen Δ1–Δ9 contract is codegen'd protoc-free
      and served over an in-process `tokio::io::duplex` channel with
      live `Hello`/`Subscribe`/`Execute`/`Query` and
      frozen-`UNIMPLEMENTED` `AgentChat`/`Interrupt`/`CaptureFrame`
      (Decision 7). Every `phase-4-m1.md` § "M1 acceptance gate"
      box is checked; gating tests in
      `crates/mili-viz-server/tests/acceptance.rs`:
      `handshake_match_and_mismatch`, `capability_agent_present_absent`,
      `layer0_equals_raw`, `subscription_fanout`,
      `frozen_stubs_unimplemented`, `conformance_all_command_arms`.
      Build-reality decisions logged: `phase-4-m1.md` Decisions 8–9.
- [x] **M2 — load + state navigation.** ✅ **Landed.** `mili-rs`
      wired into `mili-viz-server`: real `load` (`num_states`/
      `state_times`/element `class_names`), state cursor clamped to
      `[1, num_states]`, `show` delivers the per-state triangulated
      hull via the frozen `ResultState.geometry` `GeometryRef`
      (real ticket → in-process geometry store; vertices from the
      primal `nodpos` query). No proto change. Scope/decisions:
      [`phase-4-m2.md`](phase-4-m2.md) (Decisions 10–12). Gating
      test: `crates/mili-viz-server/tests/m2_geometry.rs`
      `load_state_nav_and_real_geometry`; M1's six acceptance tests
      unchanged and green.
- [x] **M3 — primal result display.** ✅ **Landed.** `show
      <result> [component]` resolves the leaf svar
      (`classes_of_state_variable`) and delivers a per-vertex scalar
      in the geometry blob (element → nodal-averaged, nodal →
      direct, vector → component 0); `ResultState.{min,max}` = the
      autoscale data range. Optional trailing `scalar_f32`
      (`MVG2:...`); no/unknown result stays the M2 `MVG1` bare hull
      (never errors). No proto change. Scope/decisions:
      [`phase-4-m3.md`](phase-4-m3.md) (13–15). Gating test:
      `crates/mili-viz-server/tests/m3_primal.rs`
      `primal_result_colors_the_mesh`; M1's six + the M2 test
      unchanged and green.
- [x] **M4 — selection + enable/disable.** ✅ **Landed.**
      `enable`/`disable` (`MaterialVisibility`) filters the emitted
      geometry — disabled-material triangles are dropped from the blob
      on the next `show`, a single pass over the M2 per-triangle
      material that composes identically with `MVG1`/`MVG2` (scalar +
      `ResultState.{min,max}` byte-stable; only `num_indices` shrinks;
      no material disabled → byte-identical blob). `select`/`clrsel`
      stay metadata-only via the existing `DELTA_SELECTION`
      `SelectionState` + `Snapshot` (griz non-destructive overlay;
      `clrsel` empty-class clears all). One delta per `Execute`; no
      proto/format change. Scope/decisions:
      [`phase-4-m4.md`](phase-4-m4.md) (16–18). Gating test:
      `crates/mili-viz-server/tests/m4_visibility.rs`
      `material_visibility_and_selection`; M1's six + the M2 + M3
      tests unchanged and green.
- [x] **M5 — derived results.** ✅ **Landed (first slice: scalar
      stress invariants).** `show pressure`/`eff_stress`/
      `triaxiality`/`norm_press` routes through the already-parity-
      exact `mili_rs::compute_stress_invariant` kernel (resolve →
      per-class `query_full` → kernel → M3's unchanged nodal scatter;
      same `MVG2` blob/range). No formula re-port, no griz golden, no
      `parity` feature in `mili-viz-server` — `phase-4-m1.md`
      Decision 5's "no oracle" premise was false, so it is superseded
      by `phase-4-m5.md` Decision 19. Eigensolver/`surfstrain`/time
      families are deferred sub-slices (Decision 20). No proto change.
      Scope/decisions: [`phase-4-m5.md`](phase-4-m5.md) (19–21).
      Gating test: `crates/mili-viz-server/tests/m5_derived.rs`
      `derived_stress_invariants` (linear-pressure identity); M1's six
      + M2 + M3 + M4 tests unchanged and green.
  - [x] **M5 follow-up — eigenvalue families.** ✅ **Landed.**
        `prin_stress[1-3]`/`prin_dev_stress[1-3]`/`max_shear_stress`/
        `prin_strain[1-3]`/`prin_dev_strain[1-3]`/`vol_strain` via the
        identical M5 seam (two branches, only `*_spec`/`*_primals`/
        `compute_*` swapped; M5 invariant + M3 primal paths
        byte-stable). No proto change, no `parity` feature.
        Scope/decisions: [`phase-4-m5b.md`](phase-4-m5b.md) (22–24).
        Gating test:
        `crates/mili-viz-server/tests/m5b_principal.rs`
        `derived_principal_families` (single-shared-gather invariants:
        ordering + relative tracelessness + max-shear); M1's six + M2
        + M3 + M4 + M5 tests unchanged and green.
  - [x] **M5 third slice — `surfstrain*` + nodal-time families.**
        ✅ **Landed.** `show disp_{x,y,z}`/`disp_mag`/
        `disp_rad_mag_xy`/`vel_{x,y,z}`/`acc_{x,y,z}` route through the
        parity-exact `mili_rs::compute_node_*` +
        `nodal_reference_from_coords` kernels into M3's node-direct
        mapping (factored into a shared helper — the M3 primal nodal
        path byte-stable); `show surfstrain{x,y,z,xy,yz,zx}` routes
        through the parity-exact `mili_rs::Database::
        surface_strain_query` via a **separate** per-face Hex
        connectivity gather (`scatter_hex_faces`, a viz-local
        `miliinternal.py:675-682` face table — a connectivity
        constant, not a formula re-port), kept distinct from the
        M5/M5b element-class scatter. The `*_alt` trig
        principal-strain variants are **re-deferred** (no parity-exact
        `mili-rs` kernel; would breach M5 Decision 19 — they belong in
        a future `mili-rs` core derived sub-slice). No proto change,
        no `parity` feature; the M5/M5b element scatter, the M3
        primal/nodal path, and the M6 transport are byte-stable.
        Scope/decisions: [`phase-4-m5c.md`](phase-4-m5c.md) (28–31).
        Gating test:
        `crates/mili-viz-server/tests/m5c_derived.rs`
        `derived_surfstrain_and_nodal_time` (single-shared-gather
        invariants: the exact displacement-magnitude norm identity +
        structural/state-tracking + the `vel_*`-at-state-1-zero kernel
        fact); M1's six + M2 + M3 + M4 + M5 + M5b + M6 tests unchanged
        and green.
  - [x] **M5d — the `*_alt` griz closed-form trig principal-strain
        variants.** ✅ **Landed.** Two parts. **Part A (`mili-rs`
        core):** `mili_rs::compute_principal_strain_alt` +
        `PrincipalStrainAlt`/`principal_strain_alt_spec`/`_primals`
        (re-exported from `lib.rs`), wired through
        `crates/mili-py/src/database.rs`, gated vs the `mili` oracle by
        `crates/mili-py/tests/test_alt_strain_parity.py` — **f32
        tolerance** (`np.allclose`, not bitwise: numpy's float32
        `arccos`/`cos` are its own SIMD polynomials, ≠ libm, not
        cross-language bit-reproducible; ≈1.7e-10 worst abs vs strain
        ~1e-2; `../mili-py/m4.md` Decision 27). Strict 0-xfail harness
        still green (950 passed). **Part B (viz routing):**
        `show prin_strain{1,2,3}_alt`/`prin_dev_strain{1,2,3}_alt`
        routes through that kernel via the **identical** M5b
        element-scatter branch (only `*_spec`/`*_primals`/`compute_*`
        swapped). No proto change, no `parity` feature; M5/M5b/M5c/M3/
        M6 paths byte-stable. **Discharges `phase-4-m5c.md`
        Decision 28.** Scope/decisions:
        [`phase-4-m5d.md`](phase-4-m5d.md) (32–34). Gating test:
        `crates/mili-viz-server/tests/m5d_alt_strain.rs`
        `derived_alt_principal_strain` (single-shared-gather: structural
        + `ResultState` bracketing + the per-vertex principal-ordering
        `1≥2≥3` identity + state-tracking + totality/Decision-28
        closure); M5c's `*_alt`→bare-hull assertion intentionally
        removed (superseded — Decision 34); all prior gating tests
        (M1×6 + m2 + m3 + m4 + m5 + m5b + m5c + m6) unchanged and green.
- [x] **M6 — remote transport.** ✅ **Landed.** A real **gRPC +
      Arrow Flight over TCP** transport joins the in-process one:
      `serve_tcp(svc, addr)` co-serves `MiliVizServer` + a real
      `arrow.flight.protocol.FlightService` (canonical vendored
      `Flight.proto` on the existing protoc-free `protox` path —
      **zero change to the frozen `mili_viz.proto`**; only `DoGet`
      implemented, other Flight RPCs `UNIMPLEMENTED`) on one TCP
      port. The frozen `GeometryRef.flight_ticket` resolves through
      a real Flight `DoGet` streaming the **byte-identical** M2/M3
      `MVG1`/`MVG2` blob (`phase-4-m2.md` Decision 10 redeemed; its
      tonic-version premise now superseded). `spawn_in_process` /
      `fetch_geometry` kept unchanged as the in-process seam. No
      proto/blob/ticket change. Scope/decisions:
      [`phase-4-m6.md`](phase-4-m6.md) (25–27). Gating test:
      `crates/mili-viz-server/tests/m6_transport.rs`
      `remote_transport_grpc_and_flight_over_tcp`; M1's six + M2 +
      M3 + M4 + M5 + M5b tests unchanged and green.
- [x] **M7 — volumetric geometry contract (`MVG3`).** ✅ **Landed.**
      `MVG3` is a strict superset of `MVG2` ridden through the
      free-form `GeometryRef.layout` tag (zero `.proto` change),
      adds a per-superclass element-edge buffer (fixes the
      hex-face-diagonal wireframe artifact, VB-005 — Decision 73's
      Hex/Tet/Quad/Tri/Wedge/Pyramid tables), a `tri_flags` column
      (bit 0 = interior), and an opt-in interior-triangle emit
      toggled via a reserved `MaterialVisibility{ material:
      u32::MAX }` sentinel (Decision 74). Encoder activates `MVG3`
      only when `materials[u32::MAX] == true`; otherwise the
      M2/M3/M4 path stays byte-identical (VB-001 — verified by the
      gating test's "revert restores byte-identical blob" leg).
      Client `Mesh` gains `element_edges`/`tri_flags`. Scope/
      decisions: [`phase-4-m7.md`](phase-4-m7.md) (72–74). Gating
      test
      `crates/mili-viz-server/tests/m7_mvg3.rs::volumetric_geometry_contract`
      (skip-on-absent end-to-end) + five in-module unit tests on
      the volumetric build helpers (always-on: 12-edge Hex,
      multi-superclass table counts, two-hex interior dedup).
- [x] **M8 — cut-plane operator.** ✅ **Landed.** Wires the
      previously stubbed `Cmd::Cutplane` arm: a new
      `crates/mili-viz-server/src/clip.rs` module runs a
      per-element Sutherland–Hodgman clip in a `rayon` parallel
      pass against the cached `MeshTopology`, emits a closed
      clipped hull (kept-side faces ∪ fan-triangulated cap),
      and packs the result through the existing `MVG3` carrier
      (`MeshTopology::pack_mvg3_buffers`). Cap triangles ride
      `tri_material == u32::MAX - 1` (Decision 75 sentinel).
      Session-level state (`Session.cut`) composes with
      `show`/state-step/material toggles (Decision 77); a cut
      with an all-zero normal clears it and restores the
      byte-identical M2/M3 path (verified by the gating test).
      No `.proto` change. Scope/decisions:
      [`phase-4-m8.md`](phase-4-m8.md) (75–77). Gating test
      `crates/mili-viz-server/tests/m8_cutplane.rs::cutplane_operator`
      (skip-on-absent against `basic1.pltA` — `bar71.pltA` is not
      in the fixture tree; `basic1` carries 238 hex bricks
      driving the same code paths) + three always-on unit tests
      on `clip_element` (straddle/all-keep/all-drop cases on a
      unit hex).
- [x] **M9 — slice operator.** ✅ **Landed.** Cap-only sister to
      M8. Adds the **second** post-M1 proto change (the additive
      `optional bool slice_only = 8;` on `CutPlane`, Decision 78);
      proto3 default `false` keeps an M8-only client byte-compatible.
      Server reuses the M8 per-element clip module with a `ClipMode`
      arm — `Slice` skips the kept-side polygons + element-edges,
      keeps the cap (tagged `tri_material == u32::MAX - 2`,
      Decision 80). Scalar interpolation is linear along the
      straddled element-edges (Decision 79); cap centroids are the
      mean of their polygon's resolved scalars. Cut + slice
      compose into one `MVG3` blob via a new `append_clip` helper
      (independent session-state fields). Scope/decisions:
      [`phase-4-m9.md`](phase-4-m9.md) (78–80). Gating test
      `crates/mili-viz-server/tests/m9_slice.rs::slice_operator`
      (skip-on-absent against `basic1.pltA`) + three additional
      always-on `clip` unit tests (Slice mode drops kept hull on
      straddle, returns None on all-keep, scalar lerps to 5 at
      0.5 along a 0→10 edge). **`mili_viz.proto` is again frozen
      from here forward unless a comparable bar is met.**

## Phase 5 — `mili-viz` client (IN PROGRESS — M1–M4 + MVP polish ✅ landed)

- [x] **M1 — `wgpu` renderer skeleton.** ✅ **Landed.** New
      standalone crate `crates/mili-viz-client` (`wgpu` 29 / `winit`
      0.30 / `glam`; **no** `mili-rs`/`mili-viz-proto`/`mili-viz-server`
      dependency — README "No mili involvement"). Orbit `Camera`
      (`azimuth`/`elevation`/`distance` + focus, field-aligned 1:1 to
      the frozen proto `CameraState`) + a hard-coded triangle through
      a minimal pipeline. Renderer is render-to-texture-first
      (`render_to_image`); the windowed `run()` path is a thin
      `winit::ApplicationHandler` wrapper around the same `Renderer`.
      No proto change; no Phase 4 crate touched (server's frozen
      tests unaffected). Scope/decisions:
      [`phase-5-m1.md`](phase-5-m1.md) (Decisions 38–40). Gating
      test: `crates/mili-viz-client/tests/m1_renderer.rs` — four
      always-on `camera_*` view-projection assertions +
      `headless_render_draws_triangle_over_clear` (real off-screen
      GPU render asserting clear-corner vs triangle-center;
      skip-on-absent when no `wgpu` adapter, per CLAUDE.md /
      Decision 39). Builds + runs under the existing
      `cargo test --workspace --exclude mili-py` and
      `clippy --workspace --all-targets`; no new CI job.
- [x] **M2 — render server output.** ✅ **Landed.**
      `crates/mili-viz-client` now depends on `mili-viz-proto` +
      `mili-viz-server`; `fetch_server_mesh` spawns an in-process
      server (`spawn_in_process`), `Subscribe`s, drives
      `load`/`show`, reads the broadcast `DELTA_RESULT`'s
      `GeometryRef`, resolves the `flight_ticket` through the frozen
      in-process `VizService::fetch_geometry` seam, and decodes the
      `MVG1`/`MVG2` blob (`phase-4-m2.md` Decision 11) into a `Mesh`
      with CPU per-vertex normals (the `MVG2` scalar is decoded past
      and ignored — color is M3). The M1 hard-coded triangle is
      deleted; the `Renderer` is generalized to a depth-tested
      indexed-mesh pipeline drawn through the auto-framed orbit
      `Camera` (`Camera::looking_at`). Flight/remote transport stays
      M5. No proto change; no Phase 4 crate touched. Scope/decisions:
      [`phase-5-m2.md`](phase-5-m2.md) (41–43). Gating test:
      `crates/mili-viz-client/tests/m2_render_server_output.rs`
      (always-on `MVG1` decode + skip-on-absent end-to-end render);
      M1's `m1_renderer.rs` (camera math + triangle smoke) and every
      Phase 4 server gating test unchanged and green.
- [x] **M3 — `egui` controls.** ✅ **Landed.** The `egui` 0.34.2
      stack paints the L1 shell over the M2 renderer: toolbar
      (transport / stride / animate / view / five overlay chips /
      state counter), left dock (`Runs/sessions`, `Results` →
      `show <result>`, `Materials`, `Surfaces`), the five viewport
      overlays (title / state / legend / axes / bbox), status bar,
      and the not-attached / attached-idle / animating session
      states. The `egui` paint is an **additive non-clearing second
      pass** over the byte-for-byte unchanged mesh pass (the M1/M2
      render-to-texture seam never moves); the `MVG2` per-vertex
      scalar now drives a viz-local cool→warm colormap autoscaled by
      `ResultState.{min,max}`. Camera stays server-authoritative
      (M3 emits `Step`/`View(reset)`; full reconcile is M4). No proto
      change; no Phase 4 crate touched. Scope/decisions:
      [`phase-5-m3.md`](phase-5-m3.md) (44–47). Gating test:
      `crates/mili-viz-client/tests/m3_egui_shell.rs` (always-on
      colormap + `MVG2` decode + pure `ShellState`/`build_shell_ui`
      logic; skip-on-absent composite render); M1's `m1_renderer.rs`
      + M2's `m2_render_server_output.rs` and every Phase 4 server
      gating test unchanged and green.
- [x] **M3.5 — bottom tabs.** ✅ **Landed.** The Layer-0 command
      line (verbatim `Execute(Command{raw})` over the live in-process
      `Session`; client-side `griz>` echo + dim/error transcript on
      `ShellState`), the scripting runner (a structured **disabled
      placeholder** — the subprocess+`attach()` runner is blocked on
      the uncoded Phase 6 `pygriz`; lights up as a fill-in when
      Phase 6 lands), and the `egui_plot` time-history (fed by the
      already-implemented `Subscribe`/`ResultState` stream — the
      server `Query` RPC is a shape-only stub, so the `Query`-fed
      per-element series is the documented forward path). The bottom
      panel is an always-present 22 px tab strip with a
      default-collapsed body, so the M3 render footprint /
      `m3_egui_shell.rs` / the Decision-45 additive seam stay
      byte-stable. `egui_plot` 0.35.0 pinned (sparse-index-verified
      vs the frozen `egui` 0.34.2; the name-matching 0.34.x line
      wrongly needs `egui` 0.33). No proto change; no Phase 4 crate
      touched. Scope/decisions:
      [`phase-5-m3.5.md`](phase-5-m3.5.md) (48–52). Gating test:
      `crates/mili-viz-client/tests/m3_5_bottom_tabs.rs` (always-on
      tab-toggle / verbatim-Layer-0 / time-series logic;
      skip-on-absent collapsed-vs-open composite render); M1's
      `m1_renderer.rs` + M2's `m2_render_server_output.rs` + M3's
      `m3_egui_shell.rs` and every Phase 4 server gating test
      unchanged and green. _(The AI Assistant panel — formerly
      mislabeled "M3.5" here — is Phase 5 M6 per `client.md`
      §"Phasing".)_
- [x] **M4 — local view manipulation** (rotate/zoom without server
      round-trip; reconcile against server-authoritative camera).
      Landed in the M4 PR (see Decisions 62–66 / item 21 below).
- [x] **M5 — remote mode.** ✅ **Landed.** `Session::connect_tcp`
      and `Session::attach` over the Phase 4 M6 wire — one tuned
      `tonic::Channel` cloned for `MiliVizClient` +
      `FlightServiceClient`, Flight `DoGet` streams the byte-identical
      M2/M3 blob into the in-process `decode_mvg` (the same `Mesh`
      either way; M6 guarantee). CLI gains `-r <host:port>`
      (`--remote`) and `--attach [<id>]` (mutually exclusive,
      in-process default unchanged). `--attach` reuses the same
      `~/.griz/sessions/<id>.json` resolver `pygriz` and the server
      `main` already share (Phase 6 M2 Decisions 56/57). HPC-latency
      tuning: `tcp_nodelay`, TCP + HTTP/2 keep-alives, explicit 10 s
      `connect_timeout`. `Session::resolve_geometry`/`fetch_catalog`
      become `async` (`rt.block_on` at the windowed call sites
      mirrors the existing `execute` pattern); in-process arm
      byte-stable. No proto change; no Phase 4 crate touched.
      Scope/decisions: [`phase-5-m5.md`](phase-5-m5.md) (90–93).
      Gating test
      `crates/mili-viz-client/tests/m5_remote_mode.rs::{cli_dispatches_remote_attach_and_default_distinctly, cli_rejects_mixed_or_double_transports, attach_explicit_id_missing_is_a_clear_error, attach_empty_dir_is_a_clear_error, attach_with_dead_pid_id_still_attempts_connect_and_fails_cleanly, remote_session_resolves_geometry_byte_identical_to_in_process, remote_session_fetches_catalog_via_flight, attach_round_trip_resolves_a_real_running_server}`
      (5 always-on + 3 skip-on-absent against `serial/basic1`).
- [x] **M6 — agent integration polish.** ✅ **Landed.** The agent
      surface frozen since Phase 4 M1 Decision 1 Δ4–Δ9 (`AgentChat`,
      `Interrupt`, `CaptureFrame`, `DELTA_AGENT`, `AgentEvent`,
      `AgentTranscript`, `Snapshot.agent`,
      `HelloReply.capabilities[agent]`) is now lit up end-to-end with
      **zero `.proto` change**. Server: a new
      `crates/mili-viz-server/src/agent.rs` carries an
      `AgentBackend` trait + the always-on `MockAgent` deterministic
      backend (Decision 94); `VizService::builder().agent_backend(...)`
      plugs one in and implicitly flips the `CAP_AGENT` advertisement.
      `agent_chat` spawns one turn task that broadcasts `UserTurn` /
      `Status(thinking)` / streamed `Token`s / `ToolBegin`/`ToolEnd`
      pair around a real command dispatched through the **same
      `VizService::dispatch`** seam typed `Execute` uses (Decision 95
      — `client.md` §"Design principle"), then `Status(idle)` (or
      `Status(interrupted)` on barge-in — Decision 98). `interrupt`
      flips an `Arc<AtomicBool>` the backend observes between emits.
      `capture_frame` returns a deterministic placeholder PNG/JPEG via
      the `image` crate (Decision 96 — a real server-side wgpu
      offscreen renderer is a separate follow-up). The opening
      `DELTA_SNAPSHOT` now carries a populated `AgentTranscript`
      (Decision 97 — late joiners see the running conversation).
      Each `subscribe` broadcasts the peer count via an idle-status
      `Status.detail = "peers=N"` piggyback (Decision 99 — the
      free-form detail field is the only protocol-stable carrier);
      gated on the `agent` capability so the M1 acceptance gate's
      vanilla `.agent(false)` server stays byte-stable. Client:
      `crates/mili-viz-client/src/ai_panel.rs` carries `AiPanelState`
      (capability gate, expand/collapse, transcript rows, composer
      buffer, attached-frame toggle, status pill, per-turn snapshots
      for client-side revert); `shell.rs` paints the wireframes-spec
      panel (header + transcript + composer + 📷 attach / Send /
      ⏹ Stop / `↶ revert to here`); `app.rs` ingests `DELTA_AGENT`
      events, lowers `UiAction::{AgentChat, AgentInterrupt,
      AgentRevert, SetAiExpanded, ToggleAttachFrame}` to the frozen
      RPCs, and stashes a `TurnSnapshot` synchronously off each
      `agent_chat` reply (revert lowers to typed `SetState` / `Show` /
      `View(SetCamera)` — no `raw`). The windowed in-process arm
      plugs in `MockAgent` by default so the panel lights up against
      a vanilla `cargo run`; a real LLM backend is gated separately
      (Decision 94 — its dep tree + config UX live behind a future
      Cargo feature). Scope/decisions
      [`phase-5-m6.md`](phase-5-m6.md) (94–99). Gating tests:
      `crates/mili-viz-server/tests/m6_agent.rs` (10 always-on:
      capability+backend matrix / full agent_chat broadcast in order
      / interrupt mid-turn + noop / CaptureFrame PNG+JPEG+zero / late
      subscriber transcript replay / peer-count broadcast) and
      `crates/mili-viz-client/tests/m6_agent_panel.rs` (10 always-on:
      panel hidden absent CAP_AGENT / cap+expanded paints cleanly /
      event-to-row folding / submit-clears intents / Stop-swap-on-
      in-flight / peers parsing / revert typed-command pin /
      turn-boundary race-safe ordering); the M1 acceptance gate's
      `frozen_stubs_unimplemented` test (`acceptance.rs`) was
      updated in place to assert both the no-backend and
      backend-wired arms. M1–M5/M7/M8/M9 + Phase 6 M1–M3 + MVP-polish
      gating tests unchanged and green. `cargo clippy --workspace
      --all-targets` clean.
- [x] **M7 — render modes consuming `MVG3`.** ✅ **Landed.**
      Sibling to Phase 4 M7 on the client side.
      `RenderMode::{Translucent, Xray}` arms (alpha-blended fill
      pipeline; depth-test on, depth-write off; the `Xray` pass
      additionally overlays element-edges); `Edges`/`Wireframe`/
      `Translucent`/`Xray` all **prefer** the server-supplied
      `Mesh::element_edges` when present and fall back to the
      legacy triangle-edge extractor when not (Decision 82 — the
      VB-005 fix activates only when the server speaks `MVG3`;
      `MVG1`/`MVG2` paths stay byte-stable, VB-001).
      `Interior` is a separate viz-state toggle on
      `ShellState::interior_on` (Decision 83 — composes with any
      `RenderMode`); the windowed app lowers
      `UiAction::SetInteriorMode` to the frozen `Cmd::Material`
      with the reserved `material: Some(u32::MAX)` sentinel, which
      the server reads through `MaterialsState.visible` to re-emit
      an `MVG3` blob carrying the interior triangles. Zero proto
      change. Scope/decisions: [`phase-5-m7.md`](phase-5-m7.md)
      (81–83). Gating test
      `crates/mili-viz-client/tests/m7_render_modes.rs::{render_mode_arms_have_distinct_labels, interior_toggle_is_pure_observable_and_emits_no_proto_directly, decode_mvg3_roundtrips_all_four_flag_bits, mvg2_decode_has_no_mvg3_columns, element_edges_supersede_triangle_extraction_on_mvg3, render_modes_differ_translucent_and_xray}`
      (five always-on + one skip-on-absent composite-render).
- [x] **M8 — cut-plane gizmo + Rendering→Cut UI.** ✅ **Landed.**
      Client UI for Phase 4 M8's server operator. `egui`-overlay
      gizmo (origin disc + normal arrow drawn through the live
      camera as additional egui shapes only — no new `wgpu`
      pipeline; the M3 additive-paint seam stays untouched per
      VB-001), 30 Hz wall-clock-throttled preview with un-
      throttled drag-end commit (Decision 85), Rendering → Cut
      menu rows (show-gizmo toggle, clear-cut emits a zero-
      normal `Cmd::Cutplane` that the server treats as a clear),
      `Preferences → Interactive clip` toggle (default on; off
      suppresses preview emits, drag-end commit still fires —
      cross-session-persisted via `tweaks.json` like Theme).
      Lowers to the typed `Cmd::Cutplane` via
      `crate::shell::cutplane_cmd`. Status-bar cut readout
      shows `cut: o=(...) n=(...)` when active. Zero proto
      change; zero `crates/mili-viz-server` change. Scope/
      decisions: [`phase-5-m8.md`](phase-5-m8.md) (84–86).
      Gating test
      `crates/mili-viz-client/tests/m8_cut_gizmo.rs` (15
      always-on: state transitions, throttle math at 60 Hz →
      30 Hz, lowering identity, `interactive_clip` persistence
      round-trip + `is_persisted_action`, paint with the gizmo
      on; 1 skip-on-absent composite render vs. `basic1.pltA`
      — `bar71.pltA` is not in the fixture tree).
- [ ] **M9 — slice gizmo + Rendering→Slice UI.** 🟡 **Planned.**
      Thin sibling of M8: shared gizmo machinery, distinct
      status-bar readout, slice-cap colormap-painted when a
      result is mapped (slice always opaque by default; users
      wanting translucency pick `RenderMode::Translucent`).
      Lowers to `Cmd::Cutplane{ slice_only: true }`.
      Scope/decisions: [`phase-5-m9.md`](phase-5-m9.md) (87–89).
      Gating test
      `crates/mili-viz-client/tests/m9_slice_gizmo.rs`.

## Phase 6 — `pygriz` scripting client (IN PROGRESS — M1–M3 ✅ landed)

The pip-installable pure-Python client from
[`scripting.md`](scripting.md) — a **third client** of the frozen
`mili-viz-proto`. Independent of the Phase 5 renderer; gated only on
Phase 4 M1 (long landed). Distribution `pygriz`, import namespace
`griz`, under the new top-level `python/` tree (the non-crate
parallel of `crates/`). Milestone breakdown + M1 detail:
[`phase-6-m1.md`](phase-6-m1.md).

- [x] **M1 — `pygriz` scaffold + stubs + connect/handshake.**
      ✅ **Landed.** `griz.connect(host, port, token=...)` + the
      `Hello` version/capability handshake (a bumped client major →
      `compatible == False` + non-empty `mismatch_detail` + a
      `ProtocolMismatchWarning`, never an exception), Layer-0
      `session.command()` / `run_script()` → a single verbatim
      `Command{raw}` (the server's `parse_raw` is the one parser; no
      Python griz parser). Stubs are gitignored build output
      regenerated by `scripts/gen-pygriz-stubs.sh` from the one
      canonical `mili_viz.proto`. Decisions 35–37 & 53–55. Gate:
      `python/pygriz/tests/test_m1_connect.py` (always-on stub-gen +
      import + the one-verbatim-raw invariant; skip-on-absent
      connect/handshake/Layer-0 vs a spawned `mili-viz-server` TCP
      binary). No proto change; no Phase 4 crate touched;
      `cargo test --workspace --exclude mili-py` unchanged and green.
      **Unblocks the Phase 5 M3.5 scripting placeholder**
      (`phase-5-m3.5.md` Decision 49 — `attach()` itself is M2).
- [x] **M2 — connection model.** ✅ **Landed.** `griz.attach()`
      (priority: newest **live** `~/.griz/sessions/<id>.json`),
      `attach(id=...)`, `attach(host=,port=)`, `launch(gui=...)`
      (spawns the `mili-viz-server` binary on a free port + attaches
      via the file it wrote; `gui=True` warns + headless — the
      renderer is the independent Phase 5 track), `list_sessions()`.
      The session/connection file is written by the **binary's
      `main`** (Decision 56) — the frozen library transport / proto /
      `Hello` echo are byte-untouched; the token is written for the
      Jupyter contract but unenforced so the frozen tokenless M1 gate
      stays green. Decisions 56–58. Gate:
      `python/pygriz/tests/test_m2_attach.py` (always-on
      `list_sessions`/`attach` selection + malformed-skip + empty-dir
      error vs fabricated JSON in a hermetic `GRIZ_SESSIONS_DIR`;
      skip-on-absent server-writes-the-file + `attach()`/`launch()`
      end-to-end vs the spawned binary). No proto change; no `lib.rs`
      edit; `cargo test --workspace --exclude mili-py` + the M1 gate
      unchanged and green. **Fully discharges `phase-5-m3.5.md`
      Decision 49** — `attach()` now exists, not just `connect()`.
- [ ] **M3 — Layer-1 object API** + the **Layer-0 ≡ Layer-1**
      equivalence test (`show()`→`Result`, `view.*`
      server-authoritative, typed handles).
- [ ] **M4 — live sync** (`Subscribe` → `@s.on(...)`; GUI/script
      stay in sync).
- [ ] **M5 — query payoff** (`query`/`to_dataframe`, same
      numpy/pandas types as milox; Arrow Flight for large results).
- [ ] **M6 — output + remote tuning** (`render`/`save_animation`/
      `snapshot` via `CaptureFrame`; HPC-latency buffers).

## Local LLM agent (exploratory)

> **Status: research / exploratory / off the Phase 4–6 critical
> path.** Capability-gated per `phase-4-m1.md` Decision 6 — the agent
> *contract* is in M1 (the frozen `AgentChat`/`Interrupt` stubs); the
> *impl* and any post-training work is **not** gating any Phase 4/5/6
> milestone. This tracker exists so the work has one central
> checkbox surface alongside the phase tables above; nothing here
> blocks anything above.
>
> Read in this order:
> [`agent-local-llm.md`](agent-local-llm.md) →
> [`agent-local-llm-posttraining.md`](agent-local-llm-posttraining.md) →
> [`posttraining-dataset.md`](posttraining-dataset.md) →
> [`agent-local-llm-baseline.md`](agent-local-llm-baseline.md).

### Surface + post-training decisions (pinned in the research docs)

| # | Decision | State | Where |
|---|---|---|---|
| L0 | **Surface choice — typed `Command` JSON tool calls**, not raw Layer-0 DSL and not free-form pygriz Python; ≈18 tools (15 typed + `query`/`snapshot` + `griz_raw` fallback); pygriz is *humans and the reasoning agent*, not the tiny model's emit target | ✅ pinned | [`agent-local-llm.md`](agent-local-llm.md) § "Surface choice" |
| L1 | Constrained decoding: per-tool JSON schema as primary; griz GBNF scoped to the `griz_raw` argument only | ✅ pinned | [`agent-local-llm.md`](agent-local-llm.md) Decision 3 |
| L2 | Verifier tiers refolded onto the typed-tool surface (L0 = tool-call shape; L1 = name + schema; L2 = dispatch ok; L3 = post-condition); **L1 thins, L2 carries more weight** | ✅ pinned | [`posttraining-dataset.md`](posttraining-dataset.md) Stage 4 |
| L3 | Canonical rollout record updated to FunctionGemma-style `tool_calls`/`tool` messages + `tools` schemas + `tool_calls_flat` (the new dedup key) | ✅ pinned | [`posttraining-dataset.md`](posttraining-dataset.md) §1, Stage 6 |
| L4 | V0 baseline plan drafted: produce one defensible L3 pass-rate number for stock FunctionGemma-270M-it on a 50-scenario bootstrap eval, no fine-tune, pygriz-driven | ✅ drafted | [`agent-local-llm-baseline.md`](agent-local-llm-baseline.md) |
| L5 | **Harness contract pinned.** Three consumers share one factored harness (eval driver, future teacher rollouts, future server-side `AgentChat`). `run_turn(provider, session, messages, tools, ...) → TurnResult` with closed `error_kind` enum; dispatches N tool calls per turn in order; parse errors fed back as `tool` responses (model self-corrects, option (b)); replay mode ships in v0 (`ReplayLlmProvider`) for verifier regression and dataset validation | ✅ pinned | [`agent-local-llm-baseline.md`](agent-local-llm-baseline.md) §W4a |
| L6 | **Harness invariants pinned.** Three fields **never** reach the LLM under any code path: `Snapshot.loaded.state_times` (unbounded `repeated double`; replaced by `state_time_range`+`current_time`), `GeometryRef.flight_ticket` (opaque bytes; `GeometryRef` dropped wholesale from projected responses), `Snapshot.agent` (self-echo trap). Enforced by per-field unit tests on a fabricated raw `Snapshot` | ✅ pinned | [`agent-local-llm-baseline.md`](agent-local-llm-baseline.md) §W1 "Harness invariants" |

### V0 baseline milestone (the next concrete thing to build)

The build order from [`agent-local-llm-baseline.md`](agent-local-llm-baseline.md).
Each row flips to ✅ when its gating test lands.

- [x] **W1 — Tool-schema artifact.** ✅ **Landed.** Auto-derive
      `data/posttraining/grammar/tools.json` (input + output schema
      per tool) from `crates/mili-viz-proto/proto/mili_viz.proto`'s
      `Command` oneof + the two read tools + `griz_raw`. Honest-diff
      test (`python/mili-llm-bench/tests/test_schemas.py`)
      regenerates and diffs vs the pinned file — drift fails CI.
      No interface dep; runs off the proto alone. Lightweight
      hand-parser of the .proto text (option (a) per
      [`agent-local-llm-baseline.md`](agent-local-llm-baseline.md))
      keeps the v0 dep surface small (no `grpcio-tools` runtime dep
      just for schema derivation). 18 tools (15 typed Command
      variants + `query`/`snapshot` + `griz_raw`; `render` and `raw`
      excluded — Decision L0). Output schemas mirror the W1
      projection table; the three harness invariants (no
      `state_times` / `flight_ticket` / `agent`) are pre-enforced
      at the schema layer by parametrized property-tree-walk tests.
      Always-on (no pygriz, no LLM, no GPU). Discharges
      [`agent-local-llm-baseline.md`](agent-local-llm-baseline.md) §W1.
- [x] **W2 — Bootstrap eval scenarios.** ✅ **Landed.** 50 hand-authored
      scenarios in `data/posttraining/eval/bootstrap.jsonl` covering
      the 10 v0 intents (`load`, `set-state`, `step`, `select`,
      `clrsel`, `show-primal`, `show-derived`, `material`,
      `view-reset`, `colormap`) across `d3samp6` (101 states, 5 mats)
      and `cylinder` (11 states, 2 mats) plus one compound multi-tool
      scenario. Fixture facts (class names, state counts, material
      ids) pulled from `reference/mili-python/tests/test_miliinternal.py`
      and a live probe of both fixtures — not invented. Closed-kind
      `postcondition` per scenario, validated at load time so drift
      between the file and the W3 verifier fails loudly (the W2 × W3
      contract). `mili_llm_bench.scenarios` carries the
      `Scenario`/`Postcondition` dataclasses + a JSONL loader/dumper
      that round-trips byte-identically. Always-on (no LLM, no GPU,
      no pygriz). Discharges §W2. Gating test
      `python/mili-llm-bench/tests/test_scenarios.py` (28 always-on:
      round-trip, unique ids, count == 50, closed-kind enforcement,
      10×2 intent×fixture coverage matrix, compound presence, loader
      rejects unknown kinds / missing keys, dataclass round-trip);
      W1's `test_schemas.py` (12) unchanged and green.
- [x] **W3 — Verifier (L0..L3 + failure-mode taxonomy).** ✅ **Landed.**
      `mili_llm_bench.verifier.verify(messages, postcondition)`
      returns a `VerifierResult(max_tier: int, reward: float,
      failure_mode: str | None)`. Two-column L0..L3 table (typed
      tool call vs `griz_raw` arm — the latter's L1 grammar check is
      a `# TODO(stage-1)` pointer at
      `posttraining-dataset.md` Stage 1; v0 degenerates it to
      "`arguments.line` is a string"). Closed 16-entry failure-mode
      taxonomy mirroring the W4a `error_kind` enum pre-declared in
      the design (`parse_error`/`unknown_tool`/`schema_mismatch` at
      L0/L1, `dispatch_error`/`nonexistent_{material,class,result}`/
      `state_out_of_range` at L2 from a structured
      `response.error_kind` tag, `wrong_{final_state,selection,result,
      range,materials}` at L3, driver-level
      `step_cap_hit`/`token_cap_hit`/`timeout` via a synthetic
      `stop:<reason>` system message). One handler per
      post-condition kind (the 7-arm closed dispatch table). Reuses
      the W1 `tools.json` artifact for input-schema validation; no
      live pygriz, no LLM, no GPU. Discharges §W3. Gating tests
      `python/mili-llm-bench/tests/test_verifier.py` (43 always-on:
      one per L0..L3 tier, one per failure-mode taxonomy entry, one
      happy + one failure per post-condition kind, taxonomy
      completeness + closed-set pin, griz_raw degeneration,
      `verify` rejects unknown kinds) + the W2 × W3 contract test
      `python/mili-llm-bench/tests/test_scenarios_meet_verifier.py`
      (1 always-on: fabricates the canonical perfect rollout for
      each of the 50 scenarios and asserts `max_tier == 3`); W1's
      `test_schemas.py` (12) + W2's `test_scenarios.py` (28)
      unchanged and green.
- [x] **W4a — Agent harness (the factored core).** ✅ **Landed.**
      `python/mili-llm-bench/src/mili_llm_bench/harness.py` ships
      `run_turn(provider, dispatcher, messages, tools, *,
      step_index, ...) → TurnResult` with a closed `error_kind`
      enum re-exported from `verifier.FAILURE_MODES` (the same
      tuple — enum-identity pinned by
      `test_error_kind_enum_identity_with_verifier_failure_modes`,
      so the two-source-of-truth anti-pattern is structurally
      impossible). The harness owns: tool registry
      (`Registry.load_from_artifact()` reads the pinned W1
      `tools.json`), `jsonschema` input validation, per-slot
      dispatch through a separated `Dispatcher` Protocol seam,
      the defensive forbidden-field projection belt (the three
      harness invariants from baseline.md §W1), structured error
      responses (parse/unknown/schema/dispatch + dispatcher-tagged
      argument-level L2 failures), and the per-turn wall-clock
      `timeout_s` budget. **N tool calls per turn dispatched in
      declared order**; **parse errors fed back as one synthetic
      `<parse_error>` `ExecutedCall` + matching `tool` message so
      the model self-corrects on the next turn (option (b))**.
      The pygriz adapter (`dispatchers/pygriz.py`) is the **only**
      file in the package that imports `pygriz` — kept behind the
      `[project.optional-dependencies].pygriz` extra so the
      always-on test path stays GPU-free / server-free /
      pygriz-free. `ReplayLlmProvider` lands here too (Mock +
      Replay are W5's pure-Python sliver; FunctionGemma +
      Anthropic deferred to PR-5 — Decision L5 surface unchanged).
      Discharges §W4a. Gating tests
      `python/mili-llm-bench/tests/test_harness.py` (17 always-on:
      three forbidden-field per-field invariants on a fabricated
      raw `Snapshot` + end-to-end strip through `run_turn`,
      N-tool-calls-per-turn dispatch ordering + canonical message
      shape, parse-error feedback both standalone and embedded in
      a mixed batch, error_kind enum identity vs verifier, replay
      round-trip on a fabricated `rollouts.jsonl` + exhaustion
      raise, per-turn timeout, schema mismatch + unknown tool +
      dispatcher-classified L2 + closed-taxonomy fallback,
      final_text arm, registry sanity, projection no-op, and the
      W2×W3×W4a contract test driving `bs-001` to L3 via the
      live `verifier.verify`); W1's `test_schemas.py` (12) + W2's
      `test_scenarios.py` (28) + W3's `test_verifier.py` (43) +
      W2×W3 contract (1) all unchanged and green (101 always-on
      total).
- [x] **W4b — Eval driver.** ✅ **Landed.**
      `python/mili-llm-bench/src/mili_llm_bench/driver.py` ships
      `EvalConfig` (frozen; baseline caps pinned — `step_cap=8`,
      `max_new_tokens=256`, `temperature=0`, `seed=0`, per-turn
      `timeout=60s`, plus the pinned `system_prompt` whose
      SHA-256 prefix is the falsifiability pin per baseline.md
      §"Acceptance gate" #1), `run_one_scenario(provider,
      dispatcher_factory, scenario, tools, config, registry)`,
      `extract_tool_calls_flat`, `write_rollout_record`,
      `build_summary` + `write_summary`, and a top-level
      `run_eval(scenarios, provider_factory, dispatcher_factory,
      config, out_dir, ...)` orchestrator that writes
      `rollouts.jsonl` (one record per scenario) +
      `summary.json` (pretty-printed). Driver-level stops ride
      on the synthetic `"stop:<reason>"` system message
      convention W3's `verifier._detect_driver_stop` reads —
      the harness stays neutral on driver bookkeeping (returns
      `kind="error"` / `error_kind="timeout"` and leaves
      `messages` alone; the driver appends `"stop:timeout"` and
      on `step_cap` exhaustion appends `"stop:step_cap_hit"`).
      `token_cap_hit` stays reserved in the closed enum but is
      **not emitted in v0** — the provider's per-call
      `max_new_tokens` is sufficient; a per-rollout token
      budget is a deliberate non-goal (baseline §W4b). The
      `dispatcher_factory(Scenario) -> Dispatcher` +
      `provider_factory(Scenario) -> LlmProvider` seams keep
      the always-on test path pygriz-free / LLM-free /
      GPU-free — PR-5's CLI plugs in `PygrizDispatcher`-per-
      scenario + a live provider behind the same signatures
      without touching the orchestrator. Rollout records match
      the canonical `posttraining-dataset.md` §1 shape with
      `instruction_source = "bootstrap-handauthored"` (Stage 5
      teacher rollouts will flip it to `"teacher-paraphrase"`);
      `tool_calls_flat` carries the **parsed-dict** form (the
      dedup key — the wire-shape JSON-stringified arguments
      live only in `messages`). `summary["by_failure_mode"]`
      is zero-initialized over the entire `verifier.FAILURE_MODES`
      tuple so "we don't know which failure mode dominates" is
      structurally impossible per baseline.md §"Acceptance
      gate" #6. Discharges §W4b. Gating tests
      `python/mili-llm-bench/tests/test_driver.py` (11
      always-on: happy-path L3 on `bs-001`, step-cap
      exhaustion → `"stop:step_cap_hit"`, per-turn timeout →
      `"stop:timeout"`, rollout-record canonical-shape pin
      (every §1 key + parsed-dict `tool_calls_flat`),
      `extract_tool_calls_flat` ordering across N-call turns +
      wire-string parsing, summary-writer completeness over
      4 scripted scenarios covering L3 / schema_mismatch /
      wrong_result / timeout, end-to-end `run_eval` smoke over
      `bootstrap.jsonl[0:3]`, system-prompt content-hash
      stability, `write_summary` returns the same dict it
      serializes, `EvalConfig` frozen with baseline defaults);
      W1's `test_schemas.py` (12) + W2's `test_scenarios.py`
      (28) + W3's `test_verifier.py` (43) + W2×W3 contract (1)
      + W4a's `test_harness.py` (17) all unchanged and green
      (112 always-on total).
- [x] **W5 — Inference provider seam.** ✅ **Landed.** `LlmProvider`
      Protocol + `ProviderOutput` dataclass in `providers/base.py`;
      `MockLlmProvider` + `ReplayLlmProvider` shipped in PR-3 with W4a
      (pure-Python, no LLM / GPU / network); **PR-5 lands the two
      heavy providers behind their own extras**:
      `FunctionGemmaProvider` (HF transformers via the model card's
      documented `processor.apply_chat_template(messages,
      tools=tools, ...)` path, parses the
      `<start_function_call>...<end_function_call>` block to canonical
      `[{"name", "arguments": dict}, ...]`; lazy imports
      `transformers` / `torch` so the always-on test path stays clean;
      pinnable `--functiongemma-revision` so the v0 number is
      reproducible) and `AnthropicProvider` (official SDK, our
      messages/tools converted to Anthropic's `tool_use`/`tool_result`
      blocks + the developer-role message promoted to top-level
      `system`; reads `ANTHROPIC_API_KEY` at call time, not import
      time; **CRITICAL** `cache_control: {"type": "ephemeral"}` on
      both the system block and the final tool entry — without that
      pin the recurring frontier-baseline run is ~10x more expensive
      than it needs to be, so a unit test pins the markers on the
      request body to guard against silent regression; `tokens_used`
      includes `usage.input_tokens + output_tokens +
      cache_read_input_tokens + cache_creation_input_tokens`).
      Discharges §W5. Heavy-dep gating tests
      `python/mili-llm-bench/tests/test_providers_functiongemma.py`
      (5 always-on parse-helper tests + 1 skip-on-absent model smoke
      gated on `MILI_LLM_BENCH_RUN_FUNCTIONGEMMA_SMOKE=1`) and
      `tests/test_providers_anthropic.py` (4 always-on conversion-
      helper / prompt-caching pin tests via a fake SDK client + 1
      skip-on-absent live-API smoke gated on `ANTHROPIC_API_KEY`).
- [x] **W6 — Bootstrap run + report.** ✅ **Landed.** Three CLI
      subcommands ship in `python/mili-llm-bench/src/mili_llm_bench/cli.py`:
      `derive-schemas [--check]` (the operator-facing surface for the
      W1 honest-diff gate), `run --provider {mock|replay|functiongemma|
      anthropic}` (writes `config.yaml`+`rollouts.jsonl`+`summary.json`+
      `report.md` under `--out`), and `replay --rollouts ...` (re-grades
      a stored rollout file under the current verifier; self-contained
      — each record reconstructs its scenario, no separate bootstrap
      file needed). The pygriz dispatcher factory (`dispatchers/pygriz.py`
      `pygriz_dispatcher_factory`) opens one `griz.Session` per scenario
      and the W4b driver tears it down in a `finally` block via the new
      `PygrizDispatcher.close()` so a 50-scenario run doesn't leak
      processes. `report.md` carries the six required sections (headline
      with provider/model/sysprompt hash, by_max_tier, by_failure_mode,
      timing, per-intent breakdown, raw-fallback rate, artifact
      pointers); `config.yaml` carries every falsifiability pin
      (system_prompt_sha256, tools_sha256, scenarios_sha256, model id,
      seed, step_cap, per_turn_timeout_s, bench_version, run_timestamp)
      per baseline.md §"Acceptance gate" #5. The test seam is option (c)
      — public `build_factories` / `build_replay_factories` /
      `write_config_yaml` that tests call directly with
      `provider_factory_override` + `dispatcher_factory_override` (no
      hidden `--dispatcher` flag, no env-var hook). Heavy deps
      (`transformers` / `torch` / `anthropic` / `pygriz`) lazy-import
      inside the provider branch; a `sys.modules` test pins this.
      Discharges §W6 **and closes the v0 acceptance gate.** Gating
      tests `python/mili-llm-bench/tests/test_cli.py` (11 always-on:
      `derive-schemas --check` byte-equality, `--out` round-trip,
      `run --provider mock` end-to-end smoke via the public factory
      seam, replay round-trip identity on `max_tier` per scenario,
      replay drift detection on a postcondition change, `config.yaml`
      pinned-field completeness, `--help` lists all four providers,
      lazy-import gate via fresh `sys.modules` reload, unknown
      provider rejected, `replay` not allowed through
      `build_factories`, `derive-schemas --check` fail-on-drift) and
      `tests/test_report.py` (6 always-on: per-intent rate math,
      raw-fallback counting, failure-mode count-desc/name-asc sort,
      tier-row zero-bucket completeness, headline falsifiability
      pins, write round-trip). Acceptance smoke
      `mili-llm-bench run --provider mock --scenarios
      data/posttraining/eval/bootstrap.jsonl --out /tmp/v0-mock/`
      verified locally to write all four artifacts on a no-GPU,
      no-pygriz laptop. **The v0 acceptance gate is closed; the L4
      decision tree (baseline.md §"After v0") becomes the next
      trackable surface.**

### Post-v0 branches (decided after the W6 number is in hand)

The decision tree in
[`agent-local-llm-baseline.md`](agent-local-llm-baseline.md) §"After
v0". Not yet trackable as checkboxes — which branch we take depends
on the failure-mode breakdown.

- 🔎 **If L3 already adequate** → declare the
  [`posttraining-dataset.md`](posttraining-dataset.md) Stage 8
  pre-experiment passed; post-training is moot for v1; bench
  becomes a regression test. *Stop.*
- 🔎 **If L0/L1 mostly green but L3 inadequate** → proceed to
  [`posttraining-dataset.md`](posttraining-dataset.md) Stages 1
  (grammar artifact, for `griz_raw`), 5 (teacher rollouts), 6
  (assemble), 7 (eval); SFT then DPO/GRPO only on measured need.
- 🔎 **If L0/L1 mostly red** → re-baseline against
  Qwen2.5-Coder 1.5B/3B per
  [`agent-local-llm.md`](agent-local-llm.md) Decision 5, behind the
  same `LlmProvider` seam, before spending teacher tokens.
- 🔎 **If compound multi-turn fails specifically** → enrich
  per-tool response shapes (W1) or add the first analysis macro
  (`query_extreme`), per the open Q in
  [`agent-local-llm.md`](agent-local-llm.md).

Each branch reuses the v0 plumbing — bench, verifier, schemas,
scenarios — so none of W1–W6 is throwaway.

## Immediate next steps (pick up here)

The planning gate is **cleared**. Items 1–4 below are **done**
([`phase-4-m1.md`](phase-4-m1.md), Decisions 1–7; proto updated;
open Q3–Q8 resolved/deferred). Remaining work is coding:

1. ✅ **`phase-4-m1.md` written** — consolidated buildable M1 scope;
   the three surfaces reconciled into one frozen contract; every
   proto delta enumerated (Decision 1 Δ1–Δ9); M1 acceptance gate
   defined. (Open Q3 closed.)
2. ✅ **Open Q4–Q6 resolved** (picking / time-history / CLI compat)
   — `phase-4-m1.md` Decisions 2–4, recorded in `README.md` § Open
   questions.
3. ✅ **Derived-result validation (Q8) decided** —
   `phase-4-m1.md` Decision 5: formulas-as-spec + committed golden +
   tolerance, no live griz in CI; detail deferred to M5.
4. ✅ **Local-LLM agent (Q7) decided as a scope call** —
   `phase-4-m1.md` Decision 6: contract in M1, impl + model choice
   off the M1–M5 critical path (capability-gated). `agent-local-llm*.md`
   stays research, explicitly non-gating.
5. ✅ **DONE (coding M1):** `crates/mili-viz-proto` (protoc-free
   `protox`+`tonic` codegen) and `crates/mili-viz-server`
   (in-process transport, handshake, Layer-0≡raw, subscription
   fan-out, frozen-stub `UNIMPLEMENTED`, conformance) landed; the
   `phase-4-m1.md` § "M1 acceptance gate" checklist is fully
   satisfied (six tests in `tests/acceptance.rs`).
6. ✅ **DONE (coding M2):** `mili-rs` wired into `mili-viz-server`;
   real `load`/state-nav; per-state triangulated hull delivered via
   the frozen `GeometryRef` (in-process geometry store, real ticket;
   `nodpos`-driven vertices). Scope/decisions in
   [`phase-4-m2.md`](phase-4-m2.md) (10–12). Gating test
   `m2_geometry.rs::load_state_nav_and_real_geometry`.
7. ✅ **DONE (coding M3):** `show <result> [component]` colors the
   mesh — leaf svar resolved via `classes_of_state_variable`,
   per-vertex scalar in the blob (`MVG2`), autoscale range in
   `ResultState`. Scope/decisions in
   [`phase-4-m3.md`](phase-4-m3.md) (13–15). Gating test
   `m3_primal.rs::primal_result_colors_the_mesh`.
8. ✅ **DONE (coding M4):** selection + enable/disable —
   `enable`/`disable` filters the emitted triangle list by
   per-triangle material (composes with `MVG1`/`MVG2`, scalar/range
   byte-stable); `select`/`clrsel` stay metadata-only via the existing
   `DELTA_SELECTION` + `Snapshot` (`clrsel` empty-class clears all).
   One delta per `Execute`; no proto/format change. Scope/decisions in
   [`phase-4-m4.md`](phase-4-m4.md) (16–18). Gating test
   `m4_visibility.rs::material_visibility_and_selection`.
9. ✅ **DONE (coding M5, first slice):** derived results — scalar
   stress invariants (`pressure`/`eff_stress`/`triaxiality`/
   `norm_press`) route through the already-parity-exact
   `mili_rs::compute_stress_invariant` into M3's nodal scatter; no
   formula re-port / no griz golden (`phase-4-m1.md` Decision 5
   superseded by `phase-4-m5.md` Decision 19). Eigensolver/`surfstrain`/
   time families deferred (Decision 20). Gating test
   `m5_derived.rs::derived_stress_invariants`.
10. ✅ **DONE (coding M5 follow-up):** eigenvalue families —
    `prin_stress[1-3]`/`prin_dev_stress[1-3]`/`max_shear_stress`/
    `prin_strain[1-3]`/`prin_dev_strain[1-3]`/`vol_strain` route
    through `mili_rs::compute_principal_{stress,strain}` via the
    identical M5 seam (only `*_spec`/`*_primals`/`compute_*` swapped).
    Scope/decisions in [`phase-4-m5b.md`](phase-4-m5b.md) (22–24).
    Gating test `m5b_principal.rs::derived_principal_families`.
    `surfstrain*`/`*_alt`/nodal-time families remain deferred
    (Decision 22).
11. ✅ **DONE (coding M6):** remote transport — a real **gRPC +
    Arrow Flight over TCP** transport (`serve_tcp`) co-serves
    `MiliVizServer` + a real `arrow.flight.protocol.FlightService`
    (canonical vendored `Flight.proto` on the existing protoc-free
    `protox` path — zero change to the frozen `mili_viz.proto`; only
    `DoGet` implemented). The frozen `GeometryRef.flight_ticket`
    resolves through a real Flight `DoGet` streaming the
    byte-identical M2/M3 blob (`phase-4-m2.md` Decision 10 redeemed
    and its tonic-version premise superseded); `spawn_in_process` /
    `fetch_geometry` kept as the in-process seam. Scope/decisions in
    [`phase-4-m6.md`](phase-4-m6.md) (25–27). Gating test
    `m6_transport.rs::remote_transport_grpc_and_flight_over_tcp`.
    **Phase 4 (`mili-viz` server) is complete.**
12. ✅ **DONE (coding M5 third slice):** `surfstrain*` + nodal-time
    families — `disp_{x,y,z}`/`disp_mag`/`disp_rad_mag_xy`/
    `vel_{x,y,z}`/`acc_{x,y,z}` via the parity-exact
    `mili_rs::compute_node_*` into M3's factored node-direct mapping;
    `surfstrain{x,y,z,xy,yz,zx}` via the parity-exact
    `mili_rs::Database::surface_strain_query` through a separate
    per-face Hex `scatter_hex_faces` gather (kept distinct from the
    M5/M5b element seam). `*_alt` re-deferred (no parity-exact
    `mili-rs` kernel — a future `mili-rs` core derived sub-slice).
    Scope/decisions in [`phase-4-m5c.md`](phase-4-m5c.md) (28–31).
    Gating test `m5c_derived.rs::derived_surfstrain_and_nodal_time`.
13. ✅ **DONE (coding M5d — the `*_alt` core sub-slice + viz routing):**
    `compute_principal_strain_alt` +
    `PrincipalStrainAlt`/`principal_strain_alt_spec`/`_primals` in the
    `mili-rs` core (re-exported from `lib.rs`), wired through
    `crates/mili-py/src/database.rs`, gated vs the `mili` oracle by
    `crates/mili-py/tests/test_alt_strain_parity.py` to a tight **f32
    tolerance** (not bitwise — numpy's float32 `arccos`/`cos` are
    numpy's own SIMD polynomials, not cross-language bit-reproducible;
    `../mili-py/m4.md` Decision 27); then the trivial viz seam (the
    verbatim M5b element-scatter branch). **`phase-4-m5c.md`
    Decision 28 discharged; the Phase 4 derived story is now fully
    complete.** Scope/decisions in
    [`phase-4-m5d.md`](phase-4-m5d.md) (32–34). Gating test
    `m5d_alt_strain.rs::derived_alt_principal_strain`. **Phase 4
    (`mili-viz` server) is complete with no deferred derived
    families.**
14. ✅ **DONE (coding Phase 5 M1 — `wgpu` renderer skeleton):** new
    standalone crate `crates/mili-viz-client` (`wgpu` 29 / `winit`
    0.30 / `glam`; no mili/proto/server dep — README "No mili
    involvement"). Orbit `Camera` (field-aligned to the frozen proto
    `CameraState`) + hard-coded triangle through a minimal pipeline;
    render-to-texture-first `Renderer` with a thin windowed `run()`
    wrapper. Scope/decisions [`phase-5-m1.md`](phase-5-m1.md)
    (38–40). Gating test
    `mili-viz-client/tests/m1_renderer.rs` (four always-on
    `camera_*` + skip-on-absent `headless_render_*`); no Phase 4
    crate touched, server's frozen tests unaffected.
15. ✅ **DONE (coding Phase 5 M2 — render server output):**
    `crates/mili-viz-client` wired to the in-process transport —
    `fetch_server_mesh` spawns a `mili-viz-server`, `Subscribe`s,
    drives `load`/`show`, resolves the frozen `GeometryRef` via
    `VizService::fetch_geometry`, decodes the `MVG1`/`MVG2` blob to a
    `Mesh` (CPU normals; `MVG2` scalar ignored — M3), and draws it
    through a depth-tested indexed pipeline (triangle deleted) and the
    auto-framed orbit `Camera`. Remote/Flight stays M5; no proto
    change; no Phase 4 crate touched. Scope/decisions in
    [`phase-5-m2.md`](phase-5-m2.md) (41–43). Gating test
    `m2_render_server_output.rs` (always-on decode + skip-on-absent
    end-to-end render).
16. ✅ **DONE (coding Phase 5 M3 — the `egui` shell):** the
    `egui` 0.34.2 stack paints the L1 layout over the M2 renderer —
    toolbar, left dock (Results→`show <result>`), the five viewport
    overlays, status bar, and the not-attached / idle / animating
    session states — as an **additive non-clearing second pass** over
    the byte-for-byte unchanged mesh pass (M1/M2 seam preserved); the
    `MVG2` scalar now drives a viz-local cool→warm colormap autoscaled
    by `ResultState.{min,max}`. Camera stays server-authoritative
    (M3 emits, M4 reconciles). No proto change; no Phase 4 crate
    touched. Scope/decisions in [`phase-5-m3.md`](phase-5-m3.md)
    (44–47). Gating test
    `m3_egui_shell.rs` (always-on colormap + `MVG2` decode + pure
    shell logic; skip-on-absent composite render).
17. ✅ **DONE (coding Phase 5 M3.5 — the bottom tabs):** the
    Layer-0 command line (verbatim `Execute(Command{raw})` over the
    live in-process `Session`; client-side `griz>` transcript), the
    scripting runner (a structured **disabled placeholder** — the
    subprocess+`attach()` runner is blocked on the uncoded Phase 6
    `pygriz`), and the `egui_plot` time-history (fed by the
    already-implemented `Subscribe`/`ResultState` stream — the server
    `Query` RPC is a shape-only stub, so the `Query`-fed per-element
    series is the documented forward path). Always-present 22 px tab
    strip + default-collapsed body keeps the M3 footprint /
    `m3_egui_shell.rs` / the Decision-45 seam byte-stable; `egui_plot`
    0.35.0 pinned (sparse-index-verified vs the frozen `egui`
    0.34.2). No proto change; no Phase 4 crate touched.
    Scope/decisions in [`phase-5-m3.5.md`](phase-5-m3.5.md) (48–52).
    Gating test `m3_5_bottom_tabs.rs` (always-on tab/transcript/
    time-series logic; skip-on-absent collapsed-vs-open composite
    render).
18. ✅ **DONE (coding Phase 6 M1 — `pygriz` scaffold + stubs +
    connect/handshake):** `scripts/gen-pygriz-stubs.sh` regenerates
    the gitignored `griz._proto` build output from the one canonical
    `mili_viz.proto` (`grpc_tools.protoc` + a package-relative import
    rewrite — the Python analogue of the Rust `protox` `build.rs`);
    `griz.connect(host, port, token=...)` completes the `Hello`
    handshake (matching major → `compatible`; a bumped major →
    `compatible == False` + non-empty `mismatch_detail` + a
    `ProtocolMismatchWarning`, never an exception — the Visit
    guarantee); `session.command()` / `run_script()` lower to a
    single verbatim `Command{raw}` (the server's `parse_raw` is the
    one parser; no Python griz parser — Decisions 37 & 54). No proto
    change; no Phase 4 crate touched. Scope/decisions
    [`phase-6-m1.md`](phase-6-m1.md) (35–37 & 53–55). Gating test
    `python/pygriz/tests/test_m1_connect.py` (always-on stub-gen +
    import + one-verbatim-raw; skip-on-absent connect/Layer-0 vs a
    spawned `mili-viz-server` TCP binary);
    `cargo test --workspace --exclude mili-py` unchanged and green.
    **Discharges the cross-milestone dependency:** the Phase 5 M3.5
    scripting-tab placeholder (`phase-5-m3.5.md` Decision 49) is now
    unblocked at the `connect()`/Layer-0 level (its `attach()`
    session-file path is Phase 6 M2).
19. ✅ **DONE (coding Phase 6 M2 — `pygriz` connection model +
    server-side session file):** `griz.attach()` (priority: newest
    **live** `~/.griz/sessions/<id>.json`), `attach(id=...)`,
    `attach(host=,port=)`, `launch(gui=...)` (spawns the
    `mili-viz-server` binary on a free port + attaches via the file it
    wrote; `gui=True` → `GuiUnavailableWarning` + headless, the
    renderer is the independent Phase 5 track), `list_sessions()`. The
    session/connection file is written by the **binary's `main`**
    (`crates/mili-viz-server/src/main.rs`) — the frozen library
    transport, `mili_viz.proto`, and the `Hello`/`HelloReply.session`
    echo are **byte-untouched** (Decision 56); the token is written
    for the Jupyter-file contract but **not** enforced (`main` does
    not opt into `.expected_token`) so the frozen tokenless M1 gate
    keeps passing; staleness handled read-side (dead-pid skip);
    `$GRIZ_SESSIONS_DIR` redirect makes the gate hermetic. `attach()`
    precedence explicit-endpoint > `id` > newest-live, every branch
    lowering to the one M1 `connect()` (no parallel client).
    Decisions 56–58 [`phase-6-m2.md`](phase-6-m2.md). Gating test
    `python/pygriz/tests/test_m2_attach.py` (always-on selection /
    malformed-skip / empty-dir error vs fabricated JSON; skip-on-absent
    server-writes-the-file + `attach()`/`launch()` end-to-end vs the
    spawned binary); `cargo test --workspace --exclude mili-py`, the
    frozen Phase 4/5 suites, and the M1 gate unchanged and green.
    **Fully discharges the cross-milestone dependency:** the Phase 5
    M3.5 scripting-tab placeholder (`phase-5-m3.5.md` Decision 49) is
    now unblocked at the **`attach()`** level — M1 only half-closed it
    at `connect()`.
20. ✅ **DONE (coding Phase 6 M3 — `pygriz` Layer-1 object API + the
    Layer-0 ≡ Layer-1 test):** the object API on the landed `Session`
    — `s.open/state/next/prev/first/last/select/show/isosurface/
    contour/cutplane/colormap` + `s.selection`/`s.materials`/
    `s.legend` helpers + `s.view.*` (server-authoritative), the typed
    handles (`Result.range`, `Isosurface.remove()`, minimal
    `Database`/`Contour`). Every Layer-1 call lowers to a **typed
    `Command` oneof variant, never `raw`** (Decision 59 — no second
    griz parser *or* emitter in Python; the M1 single-parser invariant
    generalized); server-authoritative read-backs use a one-shot
    `Subscribe` snapshot, never client prediction (Decision 61 — that
    is Phase 5 M4). The **Layer-0 ≡ Layer-1** invariant is pinned two
    ways (Decision 60): an always-on fake-stub lowering pin + a
    skip-on-absent identical-`DELTA_SNAPSHOT` leg (typed call vs the
    hand-written equivalent raw line, two freshly-`launch()`ed real
    servers, against `basic1.pltA`). Scope/decisions in
    [`phase-6-m3.md`](phase-6-m3.md) (59–61). Gating test
    `python/pygriz/tests/test_m3_layer1.py` (always-on lowering +
    snapshot-read; skip-on-absent Layer-0 ≡ Layer-1 vs spawned
    servers); no proto change; **`crates/` byte-for-byte untouched**
    (`cargo test --workspace --exclude mili-py` green by
    construction); the frozen Phase 4/5 suites + the M1 & M2 gates
    unchanged and green.
21. ✅ **DONE (coding Phase 5 M4 — local view manipulation + the
    pre-M4 hardening):** pre-M4 — two untracked client bugs fixed:
    the HiDPI `Surface::configure` `panic_cannot_unwind` abort
    (Decision 62 — raise only `max_texture_dimension_2d` off
    `downlevel_defaults()` + clamp the surface/offscreen target in
    `app.rs`+`renderer.rs`) and the never-coded griz-subset CLI
    (Decision 63 — pure `cli::parse_args`, `phase-4-m1.md`
    Decision 4). M4 proper — mouse orbit/pan/zoom predict + emit the
    frozen `View` op, last-broadcast-wins reconcile against
    `DELTA_CAMERA`/`Snapshot.camera` (`Camera::from_orbit`
    field-for-field, radians end-to-end), the auto-frame proposed via
    an absolute `SetCamera` on a run's first geometry (orbit persists
    across states/recolours; `reset`/`fit` = re-frame),
    `Colormap`/`LegendLimits` honoured client-side (viz-local
    named-ramp table + `effective_range`). File→Open/`rfd` deferred
    to its own milestone by maintainer decision. Decisions 62–66
    [`phase-5-m4.md`](phase-5-m4.md). No proto change; no Phase 4
    crate touched. Gating test `m4_view_manipulation.rs` (always-on
    reconcile / effective-range / named-colormap) + the always-on
    `cli::parse_args` unit tests; `cargo test --workspace --exclude
    mili-py` green (51 suites); M1/M2/M3/M3.5 + the frozen Phase 4
    suite + Phase 6 M1/M2/M3 unchanged and green.
22. ✅ **DONE (coding Phase 5 M5 — remote mode):** `Session::
    connect_tcp(endpoint, root)` + `Session::attach(id, root)` ride
    the Phase 4 M6 wire — one tuned `tonic::Channel` cloned for
    `MiliVizClient` + `FlightServiceClient`, Flight `DoGet` streams
    the byte-identical M2/M3 blob into the in-process `decode_mvg`
    (the same `Mesh` either way; M6 guarantee). CLI: `-r`/`--remote
    <host:port>` and `--attach [<id>]` (mutually exclusive, in-process
    default unchanged). `--attach` reuses the same
    `~/.griz/sessions/<id>.json` resolver pygriz uses (newest-live
    pid liveness via `kill(pid, 0)`; Phase 6 M2 Decisions 56/57).
    HPC-latency tuning: `tcp_nodelay`, TCP + HTTP/2 keep-alives,
    explicit 10 s `connect_timeout`. `Session::resolve_geometry`/
    `fetch_catalog` become `async` (`rt.block_on` at the windowed
    call sites mirrors the existing `execute` pattern); in-process
    arm byte-stable. No proto change; no Phase 4 crate touched.
    Scope/decisions: [`phase-5-m5.md`](phase-5-m5.md) (90–93).
    Gating test `m5_remote_mode.rs` (5 always-on CLI/resolver +
    3 skip-on-absent real-TCP end-to-end). The independent
    **Phase 6 M4** (`pygriz` live sync — `Subscribe` stream →
    `@s.on` callbacks) and Phase 5 M6 (agent integration polish)
    remain. No `mili-rs`/`mili-py` derived work remains.
23. ✅ **DONE (Phase 5 M4 follow-up — MVP polish, contract-preserving).**
    A rollup milestone of GUI-feedback fixes landed on top of M4 from
    exercising the windowed shell on real corpora. Categories that
    landed: HiDPI/CLI pre-M4 fixes; viewport hardening (sub-rect mesh
    framing tracking the egui-occluded scene rect; stepping↔show
    round-trip so animation re-deforms); mesh outlines/wireframe
    rendering modes (Shaded/Edges/Wireframe) with the VB-004 edge
    pipeline fix; client-side picking + viewport highlight glyph;
    materials toggle (whole-class enable/disable); scripting runner
    (lights up the Decision-49 placeholder via a `pygriz` subprocess);
    `Control` menu; Preferences (Theme, Left-dock-collapsed) +
    cross-session tweaks persistence via XDG `tweaks.json`; dock-rail
    R/M/S/P glyphs; L3 focus mode (`Ctrl+\`); primal+derived result
    catalogs delivered over a Flight side-channel (`MVCAT1` blob via
    the conventional `catalog:current` ticket, no proto change — the
    `mili-rs` core gained a parity-gated `DERIVED_REGISTRY` +
    enumeration accessors, mirrored into `crates/mili-py`); status-bar
    `proto` / peer cell de-hard-coded against
    `PROTOCOL_VERSION`. Each cut has a gating test under
    `crates/mili-viz-client/tests/` (19 test files at landing — e.g.
    `scripting_runner.rs`, `control_menu.rs`, `preferences_tweaks.rs`,
    `picking_highlight.rs`, `dock_rail.rs`, `l3_focus_mode.rs`,
    `tweaks_persistence.rs`, `result_catalog.rs`,
    `status_bar_proto_peer.rs`, `vb004_edge_pipeline_validation.rs`).
    Region-by-region wireframe coverage: see
    [`wireframe-parity.md`](wireframe-parity.md). Defect log
    (VB-001..VB-004, plus VB-005 newly logged as a known-gap
    against the Phase 4/5 M7 fix): see
    [`bug-tracker.md`](bug-tracker.md). Per-PR
    detail: `git log`. Only proto-adjacent surface touched in the
    rollup is the maintainer-approved Decision-67/70 catalog
    side-channel (no `.proto`/blob/ticket/RPC change; one
    `fetch_catalog` seam + one Flight `DoGet` ticket branch).
24. 🟡 **PLANNED (post-MVP volumetric batch — clip / slice /
    translucent / faithful internal edges).** Six new milestone
    docs drafted 2026-05-23 in response to maintainer direction
    "keep all of the rendering server side so that we can scale
    with compute". The batch fits on top of the frozen Phase 4 M1
    proto with **one** additive field (the `CutPlane.slice_only`
    bool at M9 — the second post-M1 proto change after the
    catalog side-channel); the geometry blob extension (`MVG3`)
    uses the free-form `GeometryRef.layout` tag and is zero
    `.proto` change. Order: server-first, client follows
    one-to-one.
    - **Phase 4 M7 — `MVG3` volumetric geometry contract**
      ([`phase-4-m7.md`](phase-4-m7.md), Decisions 72–74). Strict
      superset of `MVG2`: per-superclass element-edge buffer
      (fixes [`bug-tracker.md`](bug-tracker.md) **VB-005** hex
      face-diagonals in `Edges`/`Wireframe`), `tri_flags` column,
      opt-in interior triangles via reserved `MaterialVisibility`
      sentinel. Supersedes [`phase-4-m2.md`](phase-4-m2.md)
      Decision 11 additively; `MVG1`/`MVG2` decoders stay live.
      Gating test
      `crates/mili-viz-server/tests/m7_mvg3.rs::volumetric_geometry_contract`.
    - **Phase 4 M8 — cut-plane operator**
      ([`phase-4-m8.md`](phase-4-m8.md), Decisions 75–77). Wires
      the frozen `Cmd::Cutplane` stub at
      `crates/mili-viz-server/src/lib.rs:528`. Closed clipped
      hull (kept-side ∪ cap), per-superclass marching tables,
      `rayon` parallel-per-element, session-state composing with
      `show`/state-step. Gating test
      `crates/mili-viz-server/tests/m8_cutplane.rs::cutplane_operator`.
    - **Phase 4 M9 — slice operator**
      ([`phase-4-m9.md`](phase-4-m9.md), Decisions 78–80). The
      one additive `.proto` field (`slice_only: bool`). Reuses
      M8 marching-tables; cap-only emit; scalar interpolation
      linear along straddled edges; composes with cut. Gating
      test `crates/mili-viz-server/tests/m9_slice.rs::slice_operator`.
    - **Phase 5 M7 — render modes consuming `MVG3`**
      ([`phase-5-m7.md`](phase-5-m7.md), Decisions 81–83).
      `Translucent`/`Xray`/`Interior` `RenderMode` arms; the
      `Edges`/`Wireframe` fallback path stays byte-stable for
      older servers (VB-005 fix activates only when `MVG3` is
      present). Gating test
      `crates/mili-viz-client/tests/m7_render_modes.rs`.
    - ✅ **Phase 5 M8 — cut-plane gizmo + Rendering→Cut UI**
      ([`phase-5-m8.md`](phase-5-m8.md), Decisions 84–86).
      **Landed.** `egui`-overlay gizmo (origin disc + normal
      arrow as additional egui shapes — no new `wgpu`
      pipeline; M3 additive seam untouched per VB-001), 30 Hz
      wall-clock-throttled preview + un-throttled drag-end
      commit, Rendering → Cut menu (show-gizmo toggle, clear-
      cut zero-normal `Cmd::Cutplane`), `Preferences →
      Interactive clip` suppress for low-bandwidth links
      (cross-session-persisted via `tweaks.json`). Zero proto
      change; zero `crates/mili-viz-server` change. Gating test
      `crates/mili-viz-client/tests/m8_cut_gizmo.rs::{defaults_are_the_byte_stable_m7_polish_values, set_cut_plane_mutates_and_returns_commit_action, preview_cut_plane_mutates_and_returns_preview_action, clear_cut_drops_the_plane_and_emits_clear, gizmo_visibility_and_interactive_clip_are_pure_toggles, seed_from_aabb_centres_origin_and_uses_view_normal, throttle_first_call_passes_and_subsequent_within_window_blocks, throttle_blocks_60hz_into_30hz, throttle_reset_re_arms_for_drag_end_commit, lowering_copies_origin_normal_and_keeps_proto3_defaults, clear_lowering_is_a_zero_normal_default_cutplane, interactive_clip_persists_through_tweaks_round_trip, absent_tweaks_file_keeps_interactive_clip_default_on, is_persisted_action_classifies_interactive_clip_toggle, shell_paints_input_free_with_gizmo_on_and_cut_active, composite_render_with_gizmo}`
      (15 always-on + 1 skip-on-absent).
    - ✅ **Phase 5 M9 — slice gizmo + Rendering→Slice UI**
      ([`phase-5-m9.md`](phase-5-m9.md), Decisions 87–89).
      **Landed.** Thin sibling of M8: independent
      `ShellState.slice_plane` slot composing with `cut_plane`
      server-side per Decision 80, shared `draw_plane_gizmo`
      machinery with a contrasting slice-gizmo colour
      ([`SLICE_GIZMO_COLOR`](../../crates/mili-viz-client/src/shell.rs)
      cyan vs M8 orange), shared `CutThrottle` across both verbs
      (one drag at a time), new `slice_cmd` lowering that flips
      `slice_only = Some(true)` while `cutplane_cmd` keeps
      `slice_only = None` (M8 byte-stability), Rendering → Slice
      menu rows (show-gizmo toggle + clear-slice → zero-normal
      `Cmd::Cutplane { slice_only: Some(true) }`), distinct
      `slice: o=(…) n=(…)` status-bar readout that coexists with
      the M8 cut readout. Slice-cap colormap-painted falls out of
      the existing M3/M7 per-vertex-scalar path (the server already
      interpolates scalars onto cap vertices, so no client-side
      material-id branch is needed). Slice is always opaque by
      default (Decision 89 — translucency is the M7 `RenderMode`
      lever). Zero proto change (M9 server already shipped the
      additive `optional bool slice_only = 8`); zero
      `crates/mili-viz-server` change. Gating test
      `crates/mili-viz-client/tests/m9_slice_gizmo.rs::{defaults_are_the_byte_stable_m8_polish_values, set_slice_plane_mutates_and_returns_commit_action, preview_slice_plane_mutates_and_returns_preview_action, clear_slice_drops_the_plane_and_emits_clear, slice_gizmo_visibility_is_a_pure_toggle, slice_and_cut_compose_in_shell_state, slice_lowering_sets_slice_only_some_true, cut_and_slice_lowerings_differ_only_in_slice_only_flag, clear_slice_lowering_pin, shared_throttle_blocks_a_cut_then_slice_burst, slice_actions_are_not_persisted, slice_and_cut_gizmo_colours_are_distinct, rendering_menu_emits_slice_actions_through_the_shell, shell_paints_input_free_with_both_gizmos_on_and_planes_active, composite_render_with_slice_gizmo}`
      (14 always-on + 1 skip-on-absent).

    Ordering rationale: each Phase 5 Mn consumes Phase 4 Mn's
    server contract, so the natural land order is M7 → M7,
    M8 → M8, M9 → M9 (server first per pair; client follows
    once the gating test ships). Independent of the still-open
    Phase 5 M5 (remote mode) and Phase 6 M4–M6 — the batch can
    interleave with either track.

25. ✅ **DONE (coding Phase 5 M6 — agent integration polish).** The
    agent surface frozen since Phase 4 M1 Decision 1 Δ4–Δ9
    (`AgentChat` / `Interrupt` / `CaptureFrame` / `DELTA_AGENT` /
    `AgentEvent` / `AgentTranscript` / `Snapshot.agent` /
    `HelloReply.capabilities[agent]`) is now lit up end-to-end with
    **zero `.proto` change** — the contract was pinned nine
    milestones ago precisely so M6 could land without re-opening it.
    Server-side `crates/mili-viz-server/src/agent.rs` carries the
    `AgentBackend` trait + always-on `MockAgent`
    (`VizService::builder().agent_backend(...)` plugs one in and
    flips `CAP_AGENT`); the `MiliViz::agent_chat` / `interrupt` /
    `capture_frame` impls run a turn-task that broadcasts the
    `UserTurn` echo / streamed `Token`s / `ToolBegin`/`ToolEnd` pair
    around an ordinary `VizService::dispatch` call (so the agent's
    commands tag with `AGENT_MOCK_ORIGIN` and broadcast as any other
    `StateDelta` — `client.md` §"Design principle"), then the
    closing `Status(idle|interrupted)`. `Interrupt` flips an
    `Arc<AtomicBool>` the backend observes between emits;
    `CaptureFrame` returns a deterministic placeholder PNG/JPEG via
    the `image` crate (Decision 96 — production server-side wgpu
    offscreen is a separate follow-up). Peer count rides
    `AgentStatus.detail = "peers=N"` (Decision 99), gated on the
    `agent` capability so the M1 acceptance gate's vanilla
    `.agent(false)` server stays byte-stable (every prior gating
    test green by construction). Client-side
    `crates/mili-viz-client/src/ai_panel.rs` carries `AiPanelState`
    + the `↶ revert to here` typed-command lowering (`SetState` /
    `Show` / `View(SetCamera)` — no `raw`); `shell.rs` paints the
    wireframes-spec panel chrome (collapsed rail / expanded panel
    with header, transcript, composer, 📷 attach / Send / ⏹ Stop);
    `app.rs` ingests `DELTA_AGENT` events, lowers the new
    `UiAction::{AgentChat, AgentInterrupt, AgentRevert,
    SetAiExpanded, ToggleAttachFrame}` to the frozen RPCs, and
    stashes a `TurnSnapshot` synchronously off each `agent_chat`
    reply. Windowed in-process arm plugs in `MockAgent` by default
    so a vanilla `cargo run` lights up the panel; a real
    Anthropic / local-model backend stays a future Cargo-feature
    follow-up (Decision 94). Scope/decisions
    [`phase-5-m6.md`](phase-5-m6.md) (94–99). Gating tests:
    `m6_agent.rs::{capability_advertised_without_backend_returns_clear_error,
    backend_present_lights_up_agent_chat,
    agent_chat_broadcasts_user_status_token_tool_pair_and_dispatched_delta,
    interrupt_causes_an_interrupted_status_to_broadcast,
    interrupt_with_no_active_turn_is_a_clean_noop_success,
    capture_frame_returns_a_png_of_requested_extent,
    capture_frame_returns_non_empty_jpeg,
    capture_frame_rejects_zero_extent_cleanly,
    late_subscriber_sees_populated_transcript_in_opening_snapshot,
    second_subscriber_triggers_peer_count_broadcast_to_first}`
    (server-side; 10 always-on) and
    `m6_agent_panel.rs::{cap_agent_false_hides_the_panel,
    cap_agent_true_expanded_panel_paints_without_input_actions,
    ingest_event_folds_full_turn_into_ordered_rows,
    submit_returns_intent_and_clears_buffers,
    in_flight_status_is_the_stop_swap_signal,
    peer_count_parses_assorted_details,
    peer_count_drives_panel_state,
    revert_lowers_to_typed_setstate_show_setcamera_sequence,
    revert_without_camera_emits_setstate_and_show_only,
    turn_boundary_inserts_between_user_and_assistant}` (client-side;
    10 always-on). The M1 acceptance gate's
    `frozen_stubs_unimplemented` test was updated in place to assert
    both the no-backend and backend-wired arms; `cargo test
    --workspace --exclude mili-py` green; `cargo clippy --workspace
    --all-targets` clean. **Phase 5 (`mili-viz` client) is now
    complete** — only Phase 6 M4 (`pygriz` live sync), M5 (query
    payoff), and M6 (output + remote tuning) remain across the
    whole `mili-viz` track.

## Update protocol

Mirror the `mili-py`/`mili-rs` discipline: each milestone lands as its
own PR; flip its `[ ]` → `[x]` here with the gating test named; record
any real architecture/scope decision in the relevant `mili-viz/*.md`
(decision-numbered, like `m4.md`'s 22–26); keep this tracker's TL;DR
and the open-questions table honest so a cold reader can resume from
this file alone.

Defects found exercising the client/server on real corpora (GUI-visual
bugs the fixture/parity suites can't catch) go in
[`bug-tracker.md`](bug-tracker.md) — symptom → root cause → fix →
commit — not as milestone items here. This file stays the *milestone*
log; that file is the *defect* log.
