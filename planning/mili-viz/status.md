# `mili-viz` status — live tracker (START HERE)

> **This is the single source of truth for Phase 4/5/6 (`mili-viz`).**
> The `mili-rs` core and the `milox` Python bindings (Phases 1–3) are
> **complete and frozen** — see [`../mili-rs/status.md`](../mili-rs/status.md)
> and [`../mili-py/README.md`](../mili-py/README.md). All remaining
> work in this repo is `mili-viz`.
>
> **Client wireframe coverage** (placeholder/partial inventory at a
> finer grain than the milestones below) lives in its own tracker:
> [`wireframe-parity.md`](wireframe-parity.md).

## TL;DR — where we are

- **Phase 4 (`mili-viz` server): ✅ COMPLETE — M1 ✅, M2 ✅, M3 ✅, M4 ✅, M5 ✅ (+ M5 follow-up ✅: eigenvalue families; + M5 third slice ✅: `surfstrain*` per-face Hex + nodal-time families; + M5d ✅: the `*_alt` trig principal-strain variants — core kernel + viz routing), M6 ✅ (remote transport) landed.** The derived family is now **fully complete**: the last deferral (`*_alt`, `phase-4-m5c.md` Decision 28) is discharged — the parity-gated `mili_rs::compute_principal_strain_alt` core kernel + its trivial viz seam landed (`phase-4-m5d.md`; `../mili-py/m4.md` Decision 27).
- **Phase 5 (`mili-viz` client): 🟢 IN PROGRESS — M1 ✅, M2 ✅,
  M3 ✅, M3.5 ✅, M4 ✅ landed.** M4 = local view manipulation +
  the pre-M4 hardening: mouse orbit/pan/zoom with client-side
  **prediction** (the local `Camera` moves immediately *and* emits
  the frozen `View` op) reconciled **last-broadcast-wins** against
  the server-authoritative `DELTA_CAMERA`/`Snapshot.camera`
  (`camera_from_state`/`Camera::from_orbit` is a field-for-field
  copy, radians end-to-end — the proto's "degrees" comment is
  non-normative since the server is a unit-agnostic add); the
  auto-frame is proposed to the server via an absolute `SetCamera`
  on a run's first geometry so the orbit persists across state
  steps/recolours and `reset`/`fit` mean *re-frame*;
  `Colormap`/`LegendLimits` honoured client-side (a viz-local
  named-ramp table — `cool`/`warm`/`grayscale`/`hot`, unknown →
  `cool` — + a `ShellState::effective_range` autoscale/override).
  Pre-M4: the HiDPI `Surface::configure` `panic_cannot_unwind`
  abort and the never-coded `phase-4-m1.md` Decision 4 griz CLI.
  Decisions 62–66 [`phase-5-m4.md`](phase-5-m4.md). No proto change
  (`mili_viz.proto` byte-untouched); no Phase 4 crate touched.
  Gating test `m4_view_manipulation.rs` (always-on reconcile /
  effective-range / named-colormap logic) + the always-on
  `cli::parse_args` unit tests; M1/M2/M3/M3.5 gates and the frozen
  Phase 4 server suite unchanged and green. M5–M6 ⏳ not started.
  M3.5 = the bottom tabs: the Layer-0
  command line (verbatim `Execute(Command{raw})` over the live
  in-process `Session`, client-side `griz>` transcript), the
  scripting runner (a structured **disabled placeholder** — the
  subprocess+`attach()` runner is blocked on the uncoded Phase 6
  `pygriz`), and the `egui_plot` time-history (fed by the
  already-implemented `Subscribe`/`ResultState` stream — the
  server's `Query` RPC is a shape-only stub, so the `Query`-fed
  per-element series is the documented forward path). The bottom
  panel is an always-present 22 px tab strip with a
  default-collapsed body, so the M3 render footprint and
  `m3_egui_shell.rs` stay byte-stable and the Decision-45
  additive-composition seam is untouched. `egui_plot` 0.35.0 pinned
  (sparse-index-verified vs the frozen `egui` 0.34.2). No proto
  change; no Phase 4 crate touched. Scope + Decisions 48–52:
  [`phase-5-m3.5.md`](phase-5-m3.5.md). Gating test
  `tests/m3_5_bottom_tabs.rs` (always-on tab/transcript/time-series
  logic; skip-on-absent collapsed-vs-open composite render); M1's
  `m1_renderer.rs` + M2's `m2_render_server_output.rs` + M3's
  `m3_egui_shell.rs` and every Phase 4 server gating test unchanged
  and green. M5–M6 ⏳ not started. M1 = `wgpu` renderer skeleton
  (`crates/mili-viz-client`, orbit camera + hard-coded triangle,
  render-to-texture-first). M2 = render server output: depends on
  `mili-viz-proto` + `mili-viz-server`, drives `load`/`show` over the
  in-process transport, resolves the frozen `GeometryRef` via
  `VizService::fetch_geometry`, decodes the `MVG1`/`MVG2` blob to a
  `Mesh` (CPU normals; `MVG2` scalar ignored — color is M3), depth-
  tested indexed pipeline + auto-framed orbit `Camera`. **M3 = the
  `egui` shell:** the `egui` 0.34.2 stack
  (`egui`/`egui-wgpu`/`egui-winit`, verified vs the frozen `wgpu` 29 /
  `winit` 0.30) paints the L1 layout — toolbar (transport / stride /
  animate / view / overlay chips / state counter), left dock (4
  `CollapsingHeader` sections, Results→`show <result>`), the five
  viewport overlays (title/state/legend/axes/bbox), status bar, and
  the not-attached / idle / animating session states — as an
  **additive non-clearing second pass** over the byte-for-byte
  unchanged mesh pass (the M1/M2 render-to-texture seam never moves),
  and the `MVG2` scalar now becomes vertex colour through a viz-local
  cool→warm colormap autoscaled by `ResultState.{min,max}`. No proto
  change. Scope + Decisions 44–47:
  [`phase-5-m3.md`](phase-5-m3.md). Gating test
  `tests/m3_egui_shell.rs` (always-on colormap + `MVG2` decode +
  pure `ShellState`/`build_shell_ui` logic; skip-on-absent composite
  mesh+`egui` render); M1's `m1_renderer.rs` + M2's
  `m2_render_server_output.rs` unchanged and green. (M3.5 ✅ landed
  — see the M3.5 entry above; M5–M6 ⏳ not started.)
- **Phase 6 (`pygriz` scripting client): 🟢 IN PROGRESS — M1 ✅,
  M2 ✅, M3 ✅ landed.** A third pure-Python client of the frozen contract,
  gated only on Phase 4 M1, independent of Phase 5. M1 = the
  gitignored stub generator (`scripts/gen-pygriz-stubs.sh` — the
  Python analogue of the Rust `protox` `build.rs`, off the one
  canonical `mili_viz.proto`), `griz.connect(host, port, token=...)`
  + the `Hello` version/capability handshake (a bumped client major →
  `compatible == False` + non-empty `mismatch_detail` + a
  `ProtocolMismatchWarning`, **never an exception** — the Visit
  guarantee), and the Layer-0 escape hatch `session.command(...)` /
  `session.run_script(path)` → a single verbatim `Command{raw}` (no
  Python-side griz parser — the server's `parse_raw` is the one
  parser; Decisions 37 & 54). Scope + Decisions 35–37 & 53–55:
  [`phase-6-m1.md`](phase-6-m1.md). **M2 = the connection model:**
  `griz.attach()` (priority: newest **live**
  `~/.griz/sessions/<id>.json`), `attach(id=)`, `attach(host=,port=)`,
  `launch(gui=)` (spawns the `mili-viz-server` binary on a free port +
  attaches via the file it wrote; `gui=True` warns + headless — the
  renderer is the independent Phase 5 track), `list_sessions()`. The
  session/connection file is written by the **binary's `main`**
  (Decision 56) — the frozen library transport, `mili_viz.proto`, and
  the `Hello`/`HelloReply.session` echo are **byte-untouched**; the
  token is written for the Jupyter-file contract but **not** enforced
  so the frozen tokenless M1 gate stays green; staleness handled
  read-side (dead-pid skip); `$GRIZ_SESSIONS_DIR` redirect makes the
  gate hermetic. `attach()` precedence explicit-endpoint > `id` >
  newest-live, all lowering to the one M1 `connect()` (no parallel
  client; Decisions 57–58). Scope + Decisions 56–58:
  [`phase-6-m2.md`](phase-6-m2.md). **M2 fully discharges the Phase 5
  M3.5 scripting-tab placeholder** (`phase-5-m3.5.md` Decision 49 —
  M1 half-closed it at `connect()`; the subprocess+`attach()` runner
  now has its `attach()`). No proto change; no `lib.rs` edit; the only
  server-side change is the binary's `main` emitting on-disk JSON.
  Gating tests `python/pygriz/tests/test_m1_connect.py` +
  `test_m2_attach.py` (always-on pure logic; skip-on-absent vs a
  spawned `mili-viz-server` TCP binary, per CLAUDE.md);
  `cargo test --workspace --exclude mili-py` + the frozen Phase 4/5
  suites unchanged and green. **M3 = the Layer-1 object API:**
  `s.open/state/next/prev/first/last/select/show/isosurface/contour/
  cutplane/colormap` + `s.selection`/`s.materials`/`s.legend` helpers +
  `s.view.*` (server-authoritative), the typed handles
  (`Result.range`, `Isosurface.remove()`, minimal `Database`/
  `Contour`). Every Layer-1 call lowers to a **typed `Command` oneof
  variant, never `raw`** (Decision 59 — no second griz parser *or*
  emitter in Python; the M1 single-parser invariant generalizes);
  server-authoritative read-backs (`Result.range`, `s.state`,
  `legend.limits`) use a one-shot `Subscribe` snapshot, never client
  prediction (Decision 61 — that is Phase 5 M4). The **Layer-0 ≡
  Layer-1** invariant is pinned two ways (Decision 60): an always-on
  fake-stub lowering pin + a skip-on-absent identical-`DELTA_SNAPSHOT`
  leg (typed call vs the hand-written equivalent raw line, two
  freshly-`launch()`ed real servers, against `basic1.pltA`). Scope +
  Decisions 59–61: [`phase-6-m3.md`](phase-6-m3.md). No proto change;
  **`crates/` byte-for-byte untouched** (`cargo test --workspace
  --exclude mili-py` green by construction); the frozen Phase 4/5
  suites + the M1 & M2 gates unchanged and green. Gating test
  `python/pygriz/tests/test_m3_layer1.py`. M5–M6 ⏳ not started.
- **✅ Phase 4 M1 is implemented.** `crates/mili-viz-proto`
  (protoc-free `protox`+`tonic` codegen of the frozen Δ1–Δ9
  contract) and `crates/mili-viz-server` (in-process `tokio::io::
  duplex` transport, dispatch+broadcast plumbing) are in the
  workspace; `cargo test --workspace` is green. Every
  `phase-4-m1.md` § "M1 acceptance gate" box is checked, satisfied
  by the six gating tests in
  `crates/mili-viz-server/tests/acceptance.rs`:
  `handshake_match_and_mismatch`, `capability_agent_present_absent`,
  `layer0_equals_raw`, `subscription_fanout`,
  `frozen_stubs_unimplemented`, `conformance_all_command_arms`.
  Two build-reality decisions were logged (`phase-4-m1.md`
  Decisions 8–9: `Command`→`DeltaKind` is many-to-one by design;
  proto built protoc-free via `protox`).
- **✅ Phase 4 M2 is implemented.** `mili-viz-server` now links
  `mili-rs`: `load` opens a real `Database` (real `num_states` /
  `state_times` / element `class_names`), `state`/`next`/`prev`/
  `first`/`last` clamp to `[1, num_states]`, and `show` delivers the
  per-state triangulated hull through the frozen
  `ResultState.geometry` `GeometryRef` — a real ticket resolving
  through an in-process geometry store
  (`VizService::fetch_geometry`), vertices from the parity-exact
  primal `nodpos` query. **No proto change** (the M1 contract is
  frozen; M2 is server-side only). Scope + 3 decisions pinned in
  [`phase-4-m2.md`](phase-4-m2.md) (Decisions 10–12, continuing the
  M1 log). Gating test:
  `crates/mili-viz-server/tests/m2_geometry.rs`
  `load_state_nav_and_real_geometry` (skip-on-absent per CLAUDE.md);
  all six M1 acceptance tests still pass unchanged.
- **✅ Phase 4 M3 is implemented.** `show <result> [component]`
  resolves the leaf svar (via `Database::classes_of_state_variable`)
  and delivers a per-vertex scalar in the geometry blob: element
  results nodal-averaged, nodal results mapped directly, vectors
  colored by component 0; `ResultState.{min,max}` carry the
  autoscale data range. The blob gains an optional trailing
  `scalar_f32` array (layout `MVG2:...`); no scalar / unknown result
  stays the M2 `MVG1` bare hull (graceful, never errors). **No proto
  change.** Scope + Decisions 13–15 in
  [`phase-4-m3.md`](phase-4-m3.md). Gating test:
  `crates/mili-viz-server/tests/m3_primal.rs`
  `primal_result_colors_the_mesh`; M1's six + the M2 test still pass
  unchanged.
- **✅ Phase 4 M4 is implemented.** `enable`/`disable`
  (`MaterialVisibility`) now filters the emitted geometry — triangles
  of a disabled material are excluded from the blob on the next `show`,
  a single pass over the M2 per-triangle material that composes
  identically with the `MVG1` bare hull and the `MVG2` scalar hull (the
  per-vertex scalar array and `ResultState.{min,max}` are byte-stable;
  only `num_indices` shrinks). No material disabled → byte-identical
  blob, so the frozen M2/M3 tests are untouched. `select`/`clrsel` stay
  metadata-only — broadcast via the existing `DELTA_SELECTION`
  `SelectionState` + the late-joiner `Snapshot` (griz's non-destructive
  overlay; mirrors M1 Decision 2); `clrsel` with an empty class now
  clears the whole selection (griz `clrsel`/`poof`). One delta per
  `Execute` preserved; no proto/blob-format change. Scope + Decisions
  16–18 in [`phase-4-m4.md`](phase-4-m4.md). Gating test:
  `crates/mili-viz-server/tests/m4_visibility.rs`
  `material_visibility_and_selection`; M1's six + the M2 + M3 tests
  still pass unchanged.
- **✅ Phase 4 M5 is implemented (first slice: scalar stress
  invariants).** `show pressure`/`eff_stress`/`triaxiality`/
  `norm_press` resolves via `mili_rs::stress_invariant_spec`, queries
  the component stress primals per element class with
  `Database::query_full` at the current state, computes the invariant
  with the **already-parity-exact** `mili_rs::compute_stress_invariant`
  kernel (the same one `crates/mili-py` drives), and feeds the
  per-element values into M3's **unchanged** nodal-average scatter
  (same `MVG2` blob, same `ResultState.{min,max}` autoscale). Unknown
  derived → M3 bare hull (never errors). **No formula re-port, no griz
  golden, no `parity` feature in `mili-viz-server`** — `phase-4-m1.md`
  Decision 5's "no oracle" premise was false (the kernel is bit-exact
  vs the `mili` Python package via the frozen Phase-1–3 parity suite),
  so Decision 5 is **superseded** by `phase-4-m5.md` Decision 19. The
  eigensolver / `surfstrain` / time-derived families are explicitly
  deferred sub-slices (Decision 20). No proto change. Scope +
  Decisions 19–21 in [`phase-4-m5.md`](phase-4-m5.md). Gating test:
  `crates/mili-viz-server/tests/m5_derived.rs`
  `derived_stress_invariants` (validates the viz routing via the
  linear-pressure identity); M1's six + M2 + M3 + M4 tests still pass
  unchanged.
- **✅ Phase 4 M5 follow-up is implemented (eigenvalue families).**
  `show prin_stress[1-3]`/`prin_dev_stress[1-3]`/`max_shear_stress`/
  `prin_strain[1-3]`/`prin_dev_strain[1-3]`/`vol_strain` routes
  through `mili_rs::{principal_stress,principal_strain}_spec` →
  per-class `query_full` → the already-parity-exact
  `mili_rs::compute_principal_{stress,strain}` kernel → M3's
  **unchanged** nodal scatter — the identical M5 seam, only the
  `*_spec`/`*_primals`/`compute_*` calls swapped (two branches added
  before the primal `classes_of_state_variable` lookup; the M5
  stress-invariant branch and the M3 primal path are byte-stable). No
  proto change, no `parity` feature. The gating test uses only
  single-shared-gather algebraic invariants (eigenvalue descending
  order, relative deviatoric tracelessness, the max-shear relation) —
  cross-cardinality "trace" phrasings were rejected because the
  IP-inconsistent `serial/basic1` corpus makes a 3- vs 6-primal
  `query_full` select different IP samples (a real ~1.5e-3 skew, not a
  routing defect; pinned in Decision 24). Scope + Decisions 22–24 in
  [`phase-4-m5b.md`](phase-4-m5b.md). Gating test:
  `crates/mili-viz-server/tests/m5b_principal.rs`
  `derived_principal_families`; M1's six + M2 + M3 + M4 + M5 tests
  still pass unchanged.
- **✅ Phase 4 M6 is implemented (remote transport).** The
  in-process `tokio::io::duplex` transport is joined by a real
  **gRPC + Arrow Flight over TCP** transport: `serve_tcp(svc, addr)`
  binds a `TcpListener` (ephemeral `:0` supported) and co-serves
  `MiliVizServer` **and** a real `arrow.flight.protocol.
  FlightService` on the one port via tonic's router. The frozen
  `GeometryRef.flight_ticket` resolves through a real Flight `DoGet`
  that streams the **byte-identical** M2/M3 `MVG1`/`MVG2` blob — the
  ticket bytes, the `layout` string, and the encoded blob are
  unchanged (`phase-4-m2.md` Decision 10 redeemed; that decision's
  tonic-version premise is now factually false — `arrow-flight` 57+
  = tonic 0.14 — and is explicitly superseded). The Flight surface
  is the **canonical vendored `Flight.proto`** compiled through the
  existing protoc-free `protox` path (zero change to the frozen
  `mili_viz.proto`); only `DoGet` is implemented, every other Flight
  RPC returns `UNIMPLEMENTED` (the frozen-stub discipline of
  `phase-4-m1.md` Decision 7); the heavy `arrow-flight` crate was
  rejected for dependency surface (we never build a `RecordBatch` —
  the blob is opaque per Decision 11). `spawn_in_process` and
  `VizService::fetch_geometry` are **kept unchanged** as the
  in-process test/embedding seam (README run mode 1). No
  `mili_viz.proto` change, no blob/ticket change. Scope + Decisions
  25–27 in [`phase-4-m6.md`](phase-4-m6.md). Gating test:
  `crates/mili-viz-server/tests/m6_transport.rs`
  `remote_transport_grpc_and_flight_over_tcp` (real ephemeral TCP;
  Flight `DoGet` blob byte-identical to `fetch_geometry`;
  skip-on-absent per CLAUDE.md); M1's six + M2 + M3 + M4 + M5 + M5b
  tests still pass unchanged. **Phase 4 is now complete; Phase 5
  (`mili-viz` client) is the remaining work.**
- **✅ The Phase 4 M1 surface is pinned.**
  [`phase-4-m1.md`](phase-4-m1.md) is the consolidated, buildable M1
  scope doc (the analogue of `mili-py/m1.md`): it reconciles the
  three M1 surfaces into one frozen wire contract, enumerates every
  delta from the proto draft (Decision 1 Δ1–Δ9), defines the M1
  acceptance gate, and resolves open questions #3–#8. The proto
  ([`proto/mili_viz.proto`](proto/mili_viz.proto)) is updated to
  match. **Phase 4 M1 is now coding-ready** (scaffold
  `mili-viz-proto` per the acceptance gate); no open question blocks
  it. The local-LLM agent investigation (#7) is explicitly deferred
  off the critical path, not blocking.

## What is already decided (read these first)

| Doc | What it pins | State |
|---|---|---|
| [`phase-4-m1.md`](phase-4-m1.md) | **The consolidated buildable Phase 4 M1 scope.** Frozen M1 wire contract = union of base vocab + scripting + agent; every delta from the proto draft enumerated (Decision 1 Δ1–Δ9); M1 acceptance gate (no oracle → conformance + Layer-0≡raw + fan-out); Decisions 1–7 resolving open Q3–Q8 | ✅ pinned (2026-05-17) |
| [`phase-4-m2.md`](phase-4-m2.md) | **The buildable Phase 4 M2 scope.** `mili-rs`-backed `load`/state-nav/geometry behind the frozen contract; Decisions 10–12 (in-process geometry store keyed by the frozen `flight_ticket`, real Flight wire deferred to M6; self-describing `MVG1` blob + per-superclass corner triangulation; per-state `nodpos`, state clamping, one-delta invariant). No proto change | ✅ pinned + landed (2026-05-17) |
| [`phase-4-m3.md`](phase-4-m3.md) | **The buildable Phase 4 M3 scope.** Primal result display behind the frozen contract; Decisions 13–15 (leaf-svar resolution via `classes_of_state_variable`, unresolvable → bare hull; optional per-vertex `scalar_f32`, `MVG2` layout, element→nodal-averaged / nodal→direct / vector→comp 0; `ResultState.{min,max}` = autoscale data range, `legend` stays a client clamp). No proto change | ✅ pinned + landed (2026-05-17) |
| [`phase-4-m4.md`](phase-4-m4.md) | **The buildable Phase 4 M4 scope.** Selection + enable/disable behind the frozen contract; Decisions 16–18 (`enable`/`disable` filters the emitted triangle list by per-triangle material, default-visible, scalar/range byte-stable, composes with `MVG1`/`MVG2`; selection stays metadata-only via the existing `DELTA_SELECTION` + `Snapshot`, `clrsel` empty-class clears all; effects on next `show`, one delta per `Execute`). No proto/format change | ✅ pinned + landed (2026-05-17) |
| [`phase-4-m5.md`](phase-4-m5.md) | **The buildable Phase 4 M5 scope (first slice).** Scalar stress invariants behind the frozen contract; Decisions 19–21 (the derived oracle exists — `mili-rs::derived`, bit-exact vs `mili` Python — so reuse `compute_stress_invariant`, no formula port, no griz golden, **supersedes `phase-4-m1.md` Decision 5**; resolve → per-class `query_full` → kernel → M3 nodal scatter, eigensolver/per-face/time families deferred; gating test uses the linear-pressure identity, no `parity` feature in `mili-viz-server`). No proto change | ✅ pinned + landed (2026-05-17) |
| [`phase-4-m5b.md`](phase-4-m5b.md) | **The buildable Phase 4 M5 follow-up scope (eigenvalue families).** Principal stress/strain, deviatoric, max-shear, volumetric strain behind the frozen contract; Decisions 22–24 (family set = the 14 eigensolver-on-already-prepped-element-class names, `surfstrain*`/`*_alt`/time deferred; routing reuses the M5 seam verbatim — two branches before the primal lookup, only `*_spec`/`*_primals`/`compute_*` swapped, M5/M3 paths byte-stable; gating test uses only single-shared-gather invariants — ordering + relative tracelessness + max-shear — cross-cardinality "trace" checks rejected for an IP-sampling skew). No proto change | ✅ pinned + landed (2026-05-17) |
| [`phase-4-m5c.md`](phase-4-m5c.md) | **The buildable Phase 4 M5 third-slice scope (`surfstrain*` + nodal-time families).** Surface strain + nodal displacement/velocity/acceleration behind the frozen contract; Decisions 28–31 (family set = nodal-time + `surfstrain{x,y,z,xy,yz,zx}`, `*_alt` re-deferred — no parity-exact `mili-rs` kernel, belongs in a core sub-slice, also corrects Decision 22's "nodal-time already M3-reachable" imprecision; nodal-time via a node-direct branch group + a factored M3 node→vertex helper, element scatter untouched; `surfstrain*` via a separate `scatter_hex_faces` per-face Hex gather over the parity-exact `surface_strain_query` + a viz-local canonical face table, M5/M5b/M3 byte-stable; gating test = the exact displacement-magnitude norm identity + structural/state-tracking, no cross-cardinality checks). No proto change | ✅ pinned + landed (2026-05-17) |
| [`phase-4-m5d.md`](phase-4-m5d.md) | **The buildable Phase 4 M5d scope (the `*_alt` griz closed-form trig principal-strain variants) — discharges `phase-4-m5c.md` Decision 28.** Two-part: Part A is a `mili-rs` **core** derived sub-slice (`compute_principal_strain_alt` + `PrincipalStrainAlt`/`*_alt_spec`/`_primals`, wired through `crates/mili-py`, gated vs the `mili` oracle to a tight **f32 tolerance** — not bitwise — because numpy's float32 `arccos`/`cos` are numpy's own SIMD polynomials, ≠ libm, not cross-language bit-reproducible; recorded in `../mili-py/m4.md` Decision 27); Part B is the trivial viz routing (Decisions 32–34: own `PrincipalStrainAlt` enum mirroring upstream's separate `compute_function`s, the verbatim M5b element-scatter branch with only `*_spec`/`*_primals`/`compute_*` swapped; gating test = single-shared-gather invariants only — structural + the per-vertex principal-ordering `1≥2≥3` identity + state-tracking + totality/Decision-28 closure; M5c's `*_alt`→bare-hull assertion intentionally removed, superseded). No proto change, no `parity` feature in `mili-viz-server`; M5/M5b/M5c/M3/M6 byte-stable | ✅ pinned + landed (2026-05-17) |
| [`phase-4-m6.md`](phase-4-m6.md) | **The buildable Phase 4 M6 scope (remote transport).** gRPC + Arrow Flight over TCP behind the frozen contract; Decisions 25–27 (real Flight + gRPC TCP transport redeems `phase-4-m2.md` Decision 10 — its tonic-version premise is now factually false but the deferral was still correct; ticket/blob/layout byte-stable; in-process seam kept. Flight via the canonical vendored `Flight.proto` on the existing protoc-free `protox` path — zero change to the frozen `mili_viz.proto`; only `DoGet` implemented, other Flight RPCs `UNIMPLEMENTED`; verbatim opaque blob in `FlightData.data_body`; heavy `arrow-flight` crate rejected for dependency surface. `serve_tcp(addr)` pre-binds a `TcpListener`, co-serves both services on one port; gating test binds a real ephemeral `127.0.0.1:0`). No `mili_viz.proto`/blob/ticket change | ✅ pinned + landed (2026-05-17) |
| [`phase-5-m1.md`](phase-5-m1.md) | **The buildable Phase 5 M1 scope (`wgpu` renderer skeleton).** First client-side milestone: a standalone `crates/mili-viz-client` (`wgpu`/`winit`/`glam`, **no** mili/proto/server dep — README "No mili involvement"). Decisions 38–40 (standalone crate, transport attaches at M2; render-to-texture-first so the gating test is a real headless GPU render with skip-on-absent, always-on camera-math half; orbit `Camera` is the reusable tested core with field shape aligned 1:1 to the frozen proto `CameraState`, triangle is throwaway scaffolding). No proto change | ✅ pinned + landed (2026-05-17) |
| [`phase-5-m2.md`](phase-5-m2.md) | **The buildable Phase 5 M2 scope (render server output).** First client milestone to wire the transport: `mili-viz-client` depends on `mili-viz-proto` + `mili-viz-server`, drives `Subscribe`/`load`/`show` over `spawn_in_process`, resolves the frozen `GeometryRef` via the in-process `VizService::fetch_geometry` seam (Flight/remote is M5), decodes the `MVG1`/`MVG2` blob to a `Mesh`. Decisions 41–43 (in-process transport + frozen `fetch_geometry` seam; `MVG1`/`MVG2`→`Mesh` with CPU normals, triangle deleted, depth-tested indexed pipeline, `MVG2` scalar ignored until M3, auto-framing camera; gating test = always-on decode unit + skip-on-absent end-to-end render, M1 camera gate + triangle smoke kept). No proto change | ✅ pinned + landed (2026-05-17) |
| [`phase-5-m3.md`](phase-5-m3.md) | **The buildable Phase 5 M3 scope (`egui` shell).** Toolbar + left dock + the five viewport overlays in the L1 layout behind the frozen contract; Decisions 44–47 (pin the `egui` 0.34.2 stack, verified vs the frozen `wgpu` 29 / `winit` 0.30 via the sparse index; `egui` is an **additive non-clearing second pass** on the same view — `Renderer::render` byte-for-byte preserved so the M1/M2 render-to-texture seam never moves; shell UI = pure `fn(&mut Ui,&mut ShellState)->Vec<UiAction>` always-on gate, windowed via `egui-winit` + a live in-process `Session`, camera stays server-authoritative — M3 emits the command, M4 reconciles; the `MVG2` scalar → vertex colour via a viz-local cool→warm map autoscaled by `ResultState.{min,max}`, `Colormap`/`LegendLimits` deferred to M4+). No proto change | ✅ pinned + landed (2026-05-17) |
| [`phase-5-m3.5.md`](phase-5-m3.5.md) | **The buildable Phase 5 M3.5 scope (bottom tabs).** The Layer-0 command line + scripting runner + `egui_plot` time-history behind the frozen contract; Decisions 48–52 (command line = verbatim `Execute(Command{raw})` over the live `Session`, client-side `griz>` transcript; scripting tab = structured **disabled placeholder**, the subprocess+`attach()` runner blocked on the uncoded Phase 6 `pygriz`; time-history fed by the already-implemented `Subscribe`/`ResultState` stream — the server `Query` RPC is a shape-only stub, so the `Query`-fed per-element series is the documented forward path; bottom panel = always-present 22 px strip + default-collapsed body so the M3 footprint / `m3_egui_shell.rs` / Decision-45 seam stay byte-stable; `egui_plot` 0.35.0 pinned, sparse-index-verified vs the frozen `egui` 0.34.2). No proto change; no Phase 4 crate touched | ✅ pinned + landed (2026-05-17) |
| [`phase-6-m1.md`](phase-6-m1.md) | **The buildable Phase 6 M1 scope (`pygriz` scaffold + stubs + connect/handshake) — landed.** A third pure-Python client of the frozen contract, gated only on Phase 4 M1 (independent of the Phase 5 renderer). Decisions 35–37 (top-level `python/` tree, dist `pygriz` / import `griz`, pure-Python no-pyo3; stubs are gitignored build output from the one canonical proto; M1 is Layer-0-only — reuse the server's `parse_raw`, Layer-1 + the Layer-0≡Layer-1 test is M3) + 53–55 (the `grpc_tools.protoc` generator + package-relative import rewrite, stale gitignore citation fixed; `run_script` = one verbatim `Command{raw}`, no Python line-split; the gate spawns the real `mili-viz-server` TCP binary, corpus-independent `load`). Unblocks `phase-5-m3.5.md` Decision 49. No proto change | ✅ pinned + landed (2026-05-17) |
| [`phase-6-m2.md`](phase-6-m2.md) | **The buildable Phase 6 M2 scope (`pygriz` connection model + server-side session file) — landed.** `griz.attach()` (priority: newest live `~/.griz/sessions/<id>.json`), `attach(id=)`, `attach(host=,port=)`, `launch(gui=)`, `list_sessions()`. Decisions 56–58 (the server writes the Jupyter-style session/connection file from the **binary's `main`**, never the frozen library transport / frozen proto — `lib.rs`/`mili_viz.proto` byte-untouched, `Hello` echo unchanged; token written for the Jupyter contract but **not** enforced so the frozen tokenless M1 gate stays green; staleness handled read-side via dead-pid skip; `$GRIZ_SESSIONS_DIR` redirect for a hermetic gate. `attach()` precedence explicit-endpoint > `id` > newest-live, all lowering to the one M1 `connect()` transport — no parallel client. `launch()` spawns the binary + attaches via the file it wrote; `gui=True` warns + proceeds headless — the renderer is the independent Phase 5 track). **Fully discharges `phase-5-m3.5.md` Decision 49** (M1 half-closed it at `connect()`; `attach()` now exists). No proto change; frozen Phase 4/5 suites + the M1 gate unchanged and green | ✅ pinned + landed (2026-05-18) |
| [`phase-6-m3.md`](phase-6-m3.md) | **The buildable Phase 6 M3 scope (`pygriz` Layer-1 object API + the Layer-0 ≡ Layer-1 test) — landed.** `s.open/state/next/prev/first/last/select/show/isosurface/contour/cutplane/colormap` + `s.selection`/`s.materials`/`s.legend` + `s.view.*` (server-authoritative), typed handles (`Result.range`, `Isosurface.remove()`, minimal `Database`/`Contour`). Decisions 59–61 (every Layer-1 call lowers to a **typed `Command` oneof variant, never `raw`** — no second griz parser *or* emitter in Python, the M1 single-parser invariant generalized; the **Layer-0 ≡ Layer-1** invariant pinned two ways — an always-on fake-stub lowering pin + a skip-on-absent identical-`DELTA_SNAPSHOT` leg against two freshly-`launch()`ed real servers, hand-written griz line in the test only; server-authoritative read-backs via a one-shot `Subscribe` snapshot, never client prediction — that is Phase 5 M4). `query`/`render`/`@s.on` deferred (M5/M6/M4). No proto change; **`crates/` byte-for-byte untouched**; frozen Phase 4/5 suites + the M1 & M2 gates unchanged and green | ✅ pinned + landed (2026-05-18) |
| [`phase-5-m4.md`](phase-5-m4.md) | **The buildable Phase 5 M4 scope (local view manipulation) + the pre-M4 hardening.** Decisions 62–63 (**pre-M4 bug fixes, landed**: 62 — keep `downlevel_defaults()` as the CI floor but raise only `max_texture_dimension_2d` to the adapter's real max + clamp the surface config / offscreen depth target, fixing the HiDPI `Surface::configure` `panic_cannot_unwind` abort, in both `app.rs` and `renderer.rs`; 63 — a pure hand-rolled `cli::parse_args` for exactly the `phase-4-m1.md` Decision 4 portable griz subset, `-i`/bare positional → load root, `-V` → version, `-b`/`-w` parsed-but-logged-no-op, unknown flag a clear error not a silent filename, no `clap`) + Decisions 64–66 (**M4 proper, pending maintainer approval**: 64 — predict-and-reconcile, the local `Camera` updates optimistically on mouse input *and* emits the frozen `View` op, every `DELTA_CAMERA` broadcast is authoritative/last-wins; 65 — radians end-to-end, the proto "degrees" comment non-normative since the server is a unit-agnostic add; 66 — `Colormap`/`LegendLimits` honoured client-side via a viz-local named-ramp table + an effective-range `ShellState` method). File→Open/`rfd` deferred to its own milestone by maintainer decision (recorded). No proto change | ✅ pinned + landed (2026-05-18) |
| [`README.md`](README.md) | Server/client split, crate layout (`mili-viz-proto` / `-server` / `-client`), `tonic`+Arrow-Flight transport, `wgpu`+`egui` renderer, Phase 4/5 milestone outline | ✅ architecture settled (stale on status/Phase 6 — `status.md` is authoritative) |
| [`scripting.md`](scripting.md) | Scripting is a second pure-Python client of `mili-viz-proto`; **camera is server-authoritative**; interactive `attach()` to a running GUI; `grizinit` batch via `session.run_script()`. Expands Phase 4 M1 with a subscription RPC + `StateDelta` stream + version handshake. **Implementation home: Phase 6** ([`phase-6-m1.md`](phase-6-m1.md)) | ✅ resolved |
| [`client.md`](client.md) | Client wireframe (griz-shaped docks) + AI-first design: a **server-hosted** agent peer of the command vocabulary, autonomous with barge-in + provenance journal, data-first debugging. Expands Phase 4 M1 with `AgentChat`, a `DELTA_AGENT` broadcast kind, `Snapshot`, `Interrupt`; adds Phase 5 M3.5/M6 | ✅ resolved (2026-05-17) |
| [`agent-local-llm.md`](agent-local-llm.md), [`agent-local-llm-posttraining.md`](agent-local-llm-posttraining.md), [`posttraining-dataset.md`](posttraining-dataset.md) | Local-LLM agent investigation (model choice / post-training) + the ordered dataset-construction build plan | 🔎 research notes + build plan — not yet a binding decision |

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

## Phase 4 — `mili-viz` server (NOT STARTED)

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

## Phase 5 — `mili-viz` client (IN PROGRESS — M1 ✅ landed)

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
- [ ] **M4 — local view manipulation** (rotate/zoom without server
      round-trip; reconcile against server-authoritative camera).
- [ ] **M5 — remote mode** (connect to a remote server; tune buffers
      for HPC latency).
- [ ] **M6 — agent integration polish** (`client.md`).

## Phase 6 — `pygriz` scripting client (IN PROGRESS — M1 ✅ landed)

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
22. ⏭️ **AFTER Phase 5 M4:** **Phase 5 M5** (remote mode — wire
    `connect`/`attach` over the landed gRPC+Flight TCP transport;
    HPC-latency buffer tuning) then the independent **Phase 6 M4**
    (`pygriz` live sync — `Subscribe` stream → `@s.on` callbacks).
    No `mili-rs`/`mili-py` derived work remains.
23. ✅ **DONE (Phase 5 M4 follow-up — GUI-feedback fixes,
    contract-preserving):** three untracked client bugs from running
    the windowed shell on a real corpus (`bar71.pltA`):
    - **Mesh framed/orbited off-centre.** The `wgpu` mesh pass drew
      to the *full* surface while the `egui` left dock / bottom tabs /
      AI rail occlude it asymmetrically, so the focus (orbit centre)
      projected to the full-window centre — left of the visible
      scene. Fix: `build_shell_ui` publishes the leftover central rect
      as resolution-independent screen fractions
      (`ShellState::scene_frac`); the app maps it onto the physical
      surface and the new `Renderer::render_in` restricts the pass to
      that sub-rect with the projection aspect taken from it. Orbit/
      pan/zoom sensitivity (`App::viewport`) now tracks the visible
      scene too. No proto change.
    - **Stepping/animation froze the mesh.** The frozen contract makes
      `state`/`next`/`prev`/`first`/`last` a bare `DELTA_STATE` (no
      geometry), so the counter + time-history advanced while the
      deformed hull / field colours stayed at the load state. Fix is
      **client-side and contract-preserving** (server `DELTA_STATE`
      stays `DELTA_STATE`, Layer-0 ≡ raw and the m6/fan-out gates
      untouched): the app round-trips the active `show` once per delta
      drain when the cursor moved, coalescing a strided burst to the
      final state. This also makes the time-history series accumulate
      while scrubbing/animating (was the "time/state mismatch").
    - **Mesh/element outlines: now implemented (VB-003 — `fixed`).**
      `Mesh::edge_indices` extracts unique undirected edges; a second
      `LineList` pipeline (sharing the camera bind group + vertex
      buffer) draws them. `Renderer::set_mode` selects `Shaded`
      (default — byte-for-byte the original single filled pass, so the
      M3 composite gate / VB-001 are untouched), `Edges` (depth-tested
      hidden-line overlay on the filled hull) or `Wireframe` (edges
      only over the cleared background). The previously-empty
      menu-bar `Rendering` menu now hosts the three-way toggle,
      emitting the pure-client `UiAction::SetRenderMode` (no proto
      change). See `bug-tracker.md` VB-003.
    - **Edge pipeline startup abort fixed (VB-004 — `fixed`).** The
      VB-003 `LineList` edge pipeline carried a non-zero
      `DepthBiasState`; wgpu 29 rejects depth bias on a non-triangle
      topology, so `Renderer::new` aborted at startup on a real device
      (macOS/Metal) — invisible to the `Shaded`-only composite gate.
      Fix is client-side / no proto change: zero the edge pipeline's
      depth bias and rely on the existing `LessEqual` compare (the
      edges share the triangle vertices, so coincident edges still
      draw over the fill). New always-on/skip-on-absent regression
      `tests/vb004_edge_pipeline_validation.rs` builds a real
      `Renderer` under a wgpu validation scope. See `bug-tracker.md`
      VB-004.
    - **Client-side picking + live status-bar readout (MVP-cut 4).**
      `Camera::ray_from_screen` unprojects the cursor; `Mesh::pick`
      (two-sided Möller–Trumbore over the cached hull) returns the hit
      triangle, nearest node and `MVG2` scalar. The previously-empty
      `Picking` menu toggles it (`UiAction::TogglePicking`); a
      left-click in picking mode ray-casts instead of orbiting and the
      status bar's permanently-`—` `pick:` field goes live. The frozen
      proto carries **no label catalog**, so the readout is the
      node/tri/scalar the cached `GeometryRef` actually has, not the
      wireframe's aspirational `class N` — a viewport highlight glyph
      and a label mapping remain (wireframe-parity Picking row).
    - **Materials enable/disable affordance (MVP-cut 3).** The
      server side was already done (item 8); this wires the GUI. Each
      left-dock Materials row is now a toggle (● shown / ○ hidden,
      weak label when off) emitting `UiAction::SetMaterialVisible`,
      lowered to the frozen `Command::Material`
      (`MaterialVisibility{ enable, class_name }`, whole class).
      Visibility is tracked client-authoritatively by class name
      (`ShellState::hidden_materials`, default empty → composite gate
      unchanged) since the broadcast `MaterialsState` is keyed by
      material id with no client-side class catalog.
    - **Real bbox overlay + camera-tracking axes gizmo (MVP-cut 5).**
      `Camera::project` unprojects world→viewport-fraction;
      `Mesh::aabb` gives the per-state box. The app publishes the live
      camera + AABB into `ShellState` (the `scene_frac` pattern), so
      the bbox overlay draws the 12 projected edges (tracking
      orbit/pan/zoom and per-state deform) and the axes gizmo projects
      world X/Y/Z through the camera basis. `camera`/`model_aabb`
      default `None` (headless composite / not-attached) → the M3
      placeholder inset + static triad, so that gate stays byte-stable.
    - **Scripting runner wired (MVP-cut 9).** The Decision-49 disabled
      placeholder is now a working runner: an enabled monospace editor
      (seeded with a `griz.launch()` template), a Run button, a
      streamed stdout/stderr pane and the `venv:…·attach:…` line.
      `ShellState::run_script` gates blank/in-flight and emits the
      pure-client `UiAction::RunScript`; the windowed app spawns a
      `pygriz` subprocess (`$GRIZ_PYTHON` else `python3`, the landed
      Phase 6 M2 `griz`/`launch()` made importable via a
      `python/pygriz/src`-prepended `PYTHONPATH`) on a worker thread and
      streams its output back through a channel drained each frame
      (`poll_script`, mirroring `ingest_deltas`). `attach()`-into-*this*
      GUI is **not** wired: the in-process client writes no
      `~/.griz/sessions` file, so a script that drives *this* viewport
      needs the deferred Phase 5 M5 remote transport — the runner uses
      `launch()` (a headless server it owns) in the interim, mirroring
      the Decision-49/50 "ship what's landed, document the forward
      path" precedent. A `pip install`ed managed venv (decision 3's
      production shape) is also forward work. `script*` fields default
      inert (`bottom_tab: None`, no subprocess) so the M3 composite
      gate stays byte-stable. Gating test
      `crates/mili-viz-client/tests/scripting_runner.rs` (always-on
      run-gate/stream/finish + headless paint; skip-on-absent composite
      render proving the open body composites and the collapsed seam is
      byte-stable). The `pygriz` **subprocess** itself is windowed-only
      and **not headlessly verifiable in CI**.
    - **`Control` menu wired (MVP-cut 1).** The last open-but-empty
      `let _ = ui.menu_button("Control", |_| {})` placeholder is now a
      real menu. The legacy griz `Control` Motif menu
      (`reference/griz/Src/gui.c`) is session/app control
      (Copyright, Material Mgr, Session save/load, Quit) — all needing
      a proto or windowed-lifecycle contract this slice does not touch
      — so, following the griz idiom of menus duplicating the toolbar/
      `Time` menu, `Control` hosts the session-control verbs that
      already have a `UiAction` *and* an `app.rs` lowering: transport
      (`First`/`Prev`/`Next`/`Last`), `ToggleAnimate`/`StopAnimate`,
      `ViewReset`/`Fit`. The rows are pure data (`control_menu_items`)
      so the wiring unit-tests without driving egui pointer input; the
      menu body is greyed when not attached (matching the toolbar). No
      frozen-proto change, no new `UiAction`, no Phase 4 crate touched;
      `ShellState` defaults are untouched so the M3 composite gate
      stays byte-stable. Gating test
      `crates/mili-viz-client/tests/control_menu.rs` (always-on:
      `control_menu_items` is exactly the expected already-lowered
      actions + the wired shell paints input-free in all three phases;
      skip-on-absent composite render proving the closed-by-default
      menu leaves the M3 seam byte-stable). The menu-open click path is
      windowed pointer input, **not headlessly verifiable in CI**.
    - **View / Preferences tweaks surface (MVP-cut 7, partial).** A new
      `Preferences` menu hosts the wireframe Tweaks set. The legacy
      griz menu bar has no settings menu, so the wireframe maps the
      Tweaks to a "View / Preferences" menu; MVP scope is the two
      pure-client tweaks needing no proto/contract change: **Theme**
      (`Theme::Dark`/`Light` → `UiAction::SetTheme`; `build_shell_ui`
      applies the egui `Visuals` each frame — default `Dark` *is*
      egui's `Visuals::dark()`, so the default composite path is
      pixel-unchanged and VB-001 is unaffected) and **Left dock
      collapsed** (`UiAction::SetDockCollapsed` → the L1 230 px dock
      becomes a 28 px click-to-expand rail; default `false` so
      `scene_frac` / the composite gate are unchanged). "Show bottom
      tabs" is already reachable via the tab strip's `▾ hide`;
      "AI panel position" is M6 (the panel is a placeholder). Both
      actions are returned for the (still-unbuilt) cross-session
      persistence hook (`app.rs` `let _ = Overlay::Title;`). No proto
      change, no new contract, no Phase 4 crate touched. Gating test
      `crates/mili-viz-client/tests/preferences_tweaks.rs` (always-on:
      pure/observable switches, byte-stable defaults, input-free paint
      in every theme×collapse combo, and a no-GPU `scene_frac`-widens
      check that the collapse re-lays-out; skip-on-absent composite
      render proving the default seam is unperturbed and the
      Light+collapsed render still composites over the unchanged mesh
      pass while visibly relighting the chrome). The menu-open click
      path is windowed pointer input, **not headlessly verifiable in
      CI**. (Full L3 focus mode and the cross-session persistence
      wiring both landed in follow-up bullets below.)
    - **Picking viewport highlight glyph (MVP-cut 4 remainder).** The
      ray-cast + status-bar readout already landed; this adds the
      missing viewport marker. `Pick` already carries the world-space
      hit `point`; `apply_pick` now caches it in
      `ShellState::pick_point` (a miss / `toggle_picking`-off clears
      it, so no stale marker), default `None`. `overlays` projects it
      through the live camera (the `project_bbox`/gizmo pattern) and
      strokes a ring + crosshair in the accent amber — *not* chip-gated
      (it is a picking-mode artifact, not one of the five HUD
      overlays), drawn only when picking is on, a hit is cached **and**
      a live camera is attached, so the headless composite path
      (camera `None`, picking off, `pick_point None`) is byte-stable
      (`bug-tracker.md` VB-001). Pure-client: no proto change, no new
      `UiAction`, no `app.rs` change (the existing `apply_pick` call
      feeds it), no Phase 4 crate touched. Gating test
      `crates/mili-viz-client/tests/picking_highlight.rs` (always-on:
      `apply_pick` cache/clear, byte-stable default, and a
      deterministic no-GPU shape-count delta proving the glyph only
      draws with picking+camera+hit; skip-on-absent composite render
      proving the accent glyph composites over the unchanged mesh pass
      while the default render shows none). The frozen proto still
      carries no label catalog, so the `class N` mapping stays
      deferred (design-first). The windowed click→ray-cast path is
      **not headlessly verifiable in CI**.
    - **Left-dock collapsed R/M/S/P icon rail (wireframe-parity Left
      dock row; wireframes §"L3 — Focus mode").** The Preferences-slice
      collapsed rail showed only a bare `▸`; it is now the wireframe's
      `R/M/S/P` glyph column (Results/Materials/Surfaces/Picking). The
      glyph set is pure data (`dock_rail_glyphs(picking)`) so the
      wiring unit-tests without egui pointer input — the `P` hint
      reflects the live picking state (the rail doubles as an
      at-a-glance status), the other three are state-independent. Every
      glyph's only action is to expand the dock
      (`UiAction::SetDockCollapsed(false)`, already pinned by
      `preferences_tweaks.rs`) — no proto change, no new `UiAction`, no
      `app.rs` change, no Phase 4 crate touched. Default stays expanded
      (`dock_collapsed` false) so `scene_frac` / the M3 composite gate
      are byte-stable (`bug-tracker.md` VB-001). All four wireframe
      left-dock sections (Runs/sessions, Results, Materials, Surfaces)
      already carry a `· N` count badge; the Surfaces/primal counts
      remain placeholders pending a real catalog path (design-first).
      Gating test `crates/mili-viz-client/tests/dock_rail.rs`
      (always-on: `dock_rail_glyphs` is exactly R/M/S/P with a
      picking-tracking `P` hint + the collapsed shell paints input-free
      in every phase×picking combo and the expanded default is
      unchanged; skip-on-absent composite render proving the rail
      composites over the unchanged mesh pass and the expanded seam is
      byte-stable). The glyph-click→expand path is windowed pointer
      input, **not headlessly verifiable in CI**.
    - **L3 focus mode completed (wireframe-parity L3 row; wireframes
      §"L3 — Focus mode").** `Ctrl+\` toggles a new `focus_mode` flag
      (`set_focus_mode` also collapses the dock so the R/M/S/P rail
      shows); in focus mode `build_shell_ui` hides the AI rail + bottom
      tabs, stripping the chrome to the viewport. A rail glyph or a
      second `Ctrl+\` restores full L1. The key is read from egui input
      in the pure shell (a real key event, so the "no input ⇒ no
      actions" invariant is preserved); `app.rs` lowers
      `UiAction::SetFocusMode` as a pure-client no-op (state already
      applied). Default `focus_mode`/`dock_collapsed` false → the full
      L1 chrome and `scene_frac` are unchanged, so the M3 composite
      gate is byte-stable (`bug-tracker.md` VB-001). No proto change,
      no Phase 4 crate touched. Gating test
      `crates/mili-viz-client/tests/l3_focus_mode.rs` (always-on:
      `set_focus_mode` pure/observable + dock round-trip, a synthetic
      `Ctrl+\` event toggles it while no key stays inert, and a
      deterministic no-GPU `scene_frac`-enlarges check that AI/tabs are
      hidden; skip-on-absent composite render proving the default seam
      is unperturbed and the focus render drops the AI-rail chrome
      while still compositing the mesh). The windowed key path is
      exercised by the synthetic-event leg; cross-session persistence
      of the tweak flags landed in the next bullet.
    - **Cross-session tweak persistence (MVP-cut 7 remainder — the
      last `wireframe-parity.md` MVP-cut item 7 piece).** The `app.rs`
      `let _ = Overlay::Title;` placeholder hook is now a real
      `serde`-backed config. A new `tweaks.rs` defines `PersistedTweaks`
      — the **wireframe-justified** set from
      `griz_wgpu_wireframes/README.md` §"Tweaks": the five overlay-chip
      states ("should persist between sessions") + the two Tweaks-table
      preferences (**Theme**, **Left dock collapsed**). `stride` /
      `focus_mode` are deliberately *not* persisted (runtime modes, not
      preferences). The windowed `run` loads it into `ShellState` at
      startup; `redraw` re-writes it whenever a frame's actions include
      a persisted one (`is_persisted_action` — exactly
      `ToggleOverlay`/`SetTheme`/`SetDockCollapsed`). Path is the XDG
      base-dir spec (`$XDG_CONFIG_HOME` absolute, else
      `$HOME/.config`) + `mili-viz/tweaks.json`, with a
      `MILI_VIZ_CONFIG` override; an unresolvable/unwritable config is
      a silent no-op (losing persistence never breaks the GUI).
      `PersistedTweaks::default` is *by construction*
      `from_state(&ShellState::default())`, so a **missing** config
      restores the byte-identical default shell — the headless
      `render_shell_to_image` path never touches disk and stays
      byte-stable (`bug-tracker.md` VB-001). No frozen-proto change,
      no new `UiAction`, no Phase 4 crate touched (serde/serde_json
      added to the client crate only). Gating test
      `crates/mili-viz-client/tests/tweaks_persistence.rs` (always-on:
      default == default-shell snapshot, absent-file load == default +
      `apply_to` leaves the byte-stable defaults, loss-free JSON +
      on-disk round-trip via the explicit-path API, `apply_to` purity
      — only the persisted fields move, and `is_persisted_action`
      classifies exactly the three tweak actions; skip-on-absent
      composite render proving a restored-from-absent state is
      **pixel-identical** to the untouched default and a JSON
      round-tripped Light+collapsed config still composites + relights
      the chrome). The windowed disk read/write itself (no event loop /
      display in CI) is **not headlessly verifiable**; the pure
      (de)serialization + default-equivalence + explicit-path API are
      the pinned contract.
    - **Primal result catalog via a Flight side-channel (MVP-cut 8;
      `phase-5-m4.md` Decision 67).** Opened **design-first** (the
      frozen proto carries no svar catalog); the maintainer chose the
      Flight side-channel. The Phase 4 `mili-viz-server` enumerates the
      loaded run's primal svars via mili-rs
      `Database::queriable_svars(false,false)` — a *reshape*, no
      formula/golden re-port — into a small **self-describing blob**
      (`MVCAT1\n` + `P\t<name>` lines; opaque, never an Arrow
      `RecordBatch`, so it rides `FlightData.data_body` exactly like
      the `MVG1`/`MVG2` geometry blob). It is fetched by the
      *conventional* `mili_viz_server::CATALOG_TICKET`
      (`catalog:current`) over **both** the in-process
      `VizService::fetch_catalog` seam (the path the current client
      uses, mirroring `fetch_geometry`) and a one-line Flight `DoGet`
      ticket-prefix branch (for the deferred Phase 5 M5 remote mode).
      **No `mili_viz.proto`/blob/ticket/format change, no new RPC or
      message.** Client: `catalog.rs::decode_catalog` (pure, mirroring
      `decode_mvg`) → `ShellState::catalog: Option<ResultCatalog>`;
      `app.rs` fetches it once per run in `apply_loaded` (windowed-only)
      and the left-dock `primal` sub-tree lists the names (selectable →
      the same `UiAction::Show` the command line emits) with a
      `primal · N` badge. `None` (stub `LoadedState` / no real DB /
      undecodable) keeps the static `(catalog: M4+)` placeholder and
      the *exact* pre-Decision-67 collapsed `primal` label + `Results ·
      DERIVED_RESULTS.len()` badge, so the default `ShellState`
      (`catalog: None`) leaves `render_shell_to_image` byte-stable
      (`bug-tracker.md` VB-001). `time-indep` stays an honest labelled
      placeholder — mili-rs has no TI accessor (the blob format already
      reserves a `T` tag for that follow-up). Gating tests
      `crates/mili-viz-server/tests/catalog.rs` (always-on: no-DB ⇒
      `fetch_catalog` `None`; skip-on-absent: well-formed blob + a
      **real Flight `DoGet`** byte-identical to the in-process seam,
      the M6 transport-swap-parity shape) and
      `crates/mili-viz-client/tests/result_catalog.rs` (always-on:
      `decode_catalog` round-trip / non-catalog rejection /
      unknown-tag tolerance, default `catalog None`, wired shell paints
      inert with and without a catalog; skip-on-absent composite:
      `Session::fetch_catalog` over the in-process side-channel yields
      a non-empty primal catalog and the populated left dock still
      composites the mesh). The windowed `apply_loaded` fetch site is
      not headlessly verifiable; the `Session::fetch_catalog` API it
      calls is.
    - **`time-indep` result catalog — deferred (Decision-67
      continuation, scope-guarded; `phase-5-m4.md` Decision 69).** The
      `time-indep` left-dock sub-tree **stays** the honest labelled
      placeholder. A faithful TI-results enumeration is a substantive
      **re-port**, not the trivial reshape `queriable_svars` was for
      Decision 67: `TI_PARAM` is a junk-drawer (mili-rs `ParamTable`
      collapses it with `MILI_PARAM`/`APPLICATION_PARAM` and it also
      stores labels/materials/element-sets/coords — a raw name dump
      would render internal bookkeeping as fake "results", the
      live-looking stub Decision 67 forbade); mili-python exposes **no**
      TI-results accessor (only raw `parameters()`), so there is no
      `mili` parity oracle; and a faithful filter needs the
      `mc_ti_get_metadata_from_name` TI-name grammar + a
      TI-type-aware `ParamTable` mili-rs does not have. No data-lib
      accessor, no server `T` line, no client field were added —
      **zero code change**, so the byte-stable headless composite path
      (default `ShellState`, `catalog: None`) is trivially unperturbed
      (`bug-tracker.md` VB-001). Forward seam already clean: the blob's
      reserved `T` tag + `decode_catalog`'s unknown-tag tolerance mean
      a future `mili-rs` core TI-results accessor (the named blocker)
      lights this up with no wire/proto/ticket change.
    - **Status-bar `proto` / peer count de-hard-coded (MVP polish;
      `wireframe-parity.md` "Status bar"; `phase-5-m4.md` Decision
      68).** `shell.rs::status_bar` rendered a literal `proto v1` and
      no peer count. The proto cell is now the **major** of the
      single-source `mili_viz_proto::v1::PROTOCOL_VERSION` (compile-
      time — the in-process `Session` never runs `Hello`, so the
      constant *is* the truth; negotiated-`Hello` is deferred to M5
      remote mode), byte-identical to the old literal so the default
      `ShellState` composite seam is unmoved (VB-001). An honest
      **local** `(1 peer)` is shown attached-state only — the real
      `n peer(s)` fan-out + peer banner is M6; not-attached carries no
      peer cell, so the byte-stable path is unperturbed. No `.proto`
      change, no Phase 4 crate touched. Gating
      `tests/status_bar_proto_peer.rs` (always-on: the proto cell
      tracks the constant major and is byte-stable, not-attached has
      no peer cell, attached gains `(1 peer)`; skip-on-absent
      composite: the default not-attached frame still shows the mesh).
    Gating: existing `mili-viz-client` / `mili-viz-server` suites stay
    green (the M3 composite `render_shell_to_image` path is byte-stable
    — it still renders full-surface; only the windowed `render_in` path
    sub-rects). The only proto-adjacent change is the
    maintainer-approved Decision-67 catalog side-channel: **no
    `mili_viz.proto` change**; the Phase 4 `mili-viz-server` is touched
    for that one approved mini-milestone only (a `fetch_catalog` seam +
    a one-line Flight `DoGet` branch, no new RPC/message).

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
