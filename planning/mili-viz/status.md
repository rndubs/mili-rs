# `mili-viz` status — live tracker (START HERE)

> **This is the single source of truth for Phase 4/5 (`mili-viz`).**
> The `mili-rs` core and the `milox` Python bindings (Phases 1–3) are
> **complete and frozen** — see [`../mili-rs/status.md`](../mili-rs/status.md)
> and [`../mili-py/README.md`](../mili-py/README.md). All remaining
> work in this repo is `mili-viz`.

## TL;DR — where we are

- **Phase 4 (`mili-viz` server): ✅ COMPLETE — M1 ✅, M2 ✅, M3 ✅, M4 ✅, M5 ✅ (+ M5 follow-up ✅: eigenvalue families; + M5 third slice ✅: `surfstrain*` per-face Hex + nodal-time families), M6 ✅ (remote transport) landed.** (Only the `*_alt` trig principal-strain variants remain deferred — they have **no** parity-exact `mili-rs` kernel, so they belong in a future `mili-rs` **core** derived sub-slice, not a viz producer slice; `phase-4-m5c.md` Decision 28.)
- **Phase 5 (`mili-viz` client): ⏳ NOT STARTED** (was gated on
  Phase 4 M1; now unblocked).
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
| [`phase-4-m6.md`](phase-4-m6.md) | **The buildable Phase 4 M6 scope (remote transport).** gRPC + Arrow Flight over TCP behind the frozen contract; Decisions 25–27 (real Flight + gRPC TCP transport redeems `phase-4-m2.md` Decision 10 — its tonic-version premise is now factually false but the deferral was still correct; ticket/blob/layout byte-stable; in-process seam kept. Flight via the canonical vendored `Flight.proto` on the existing protoc-free `protox` path — zero change to the frozen `mili_viz.proto`; only `DoGet` implemented, other Flight RPCs `UNIMPLEMENTED`; verbatim opaque blob in `FlightData.data_body`; heavy `arrow-flight` crate rejected for dependency surface. `serve_tcp(addr)` pre-binds a `TcpListener`, co-serves both services on one port; gating test binds a real ephemeral `127.0.0.1:0`). No `mili_viz.proto`/blob/ticket change | ✅ pinned + landed (2026-05-17) |
| [`README.md`](README.md) | Server/client split, crate layout (`mili-viz-proto` / `-server` / `-client`), `tonic`+Arrow-Flight transport, `wgpu`+`egui` renderer, Phase 4/5 milestone outline | ✅ architecture settled |
| [`scripting.md`](scripting.md) | Scripting is a second pure-Python client of `mili-viz-proto`; **camera is server-authoritative**; interactive `attach()` to a running GUI; `grizinit` batch via `session.run_script()`. Expands Phase 4 M1 with a subscription RPC + `StateDelta` stream + version handshake | ✅ resolved |
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

## Phase 5 — `mili-viz` client (NOT STARTED, gated on Phase 4 M1)

- [ ] **M1 — `wgpu` renderer skeleton** (window, camera, hard-coded
      triangle).
- [ ] **M2 — render server output** (draw the M2 server mesh).
- [ ] **M3 — `egui` controls** (state scrubber, result picker, view
      controls, command line).
- [ ] **M3.5 — AI Assistant panel** (`client.md`).
- [ ] **M4 — local view manipulation** (rotate/zoom without server
      round-trip; reconcile against server-authoritative camera).
- [ ] **M5 — remote mode** (connect to a remote server; tune buffers
      for HPC latency).
- [ ] **M6 — agent integration polish** (`client.md`).

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
    **Phase 4 derived story is complete (modulo the `*_alt` core
    follow-up).**
13. ⏭️ **NEXT:** Phase 5 (`mili-viz` client — `wgpu`/`egui`
    renderer; M1 was its only gate and is long landed), and/or a
    future `mili-rs` **core** derived sub-slice porting +
    parity-validating `__compute_principal_strain_alt` (the only
    remaining deferred derived family; once it has a parity-exact
    kernel, routing it into viz is a trivial follow-up through the
    M5/M5b seam).

## Update protocol

Mirror the `mili-py`/`mili-rs` discipline: each milestone lands as its
own PR; flip its `[ ]` → `[x]` here with the gating test named; record
any real architecture/scope decision in the relevant `mili-viz/*.md`
(decision-numbered, like `m4.md`'s 22–26); keep this tracker's TL;DR
and the open-questions table honest so a cold reader can resume from
this file alone.
