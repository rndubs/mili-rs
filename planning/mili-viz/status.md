# `mili-viz` status — live tracker (START HERE)

> **This is the single source of truth for Phase 4/5 (`mili-viz`).**
> The `mili-rs` core and the `milox` Python bindings (Phases 1–3) are
> **complete and frozen** — see [`../mili-rs/status.md`](../mili-rs/status.md)
> and [`../mili-py/README.md`](../mili-py/README.md). All remaining
> work in this repo is `mili-viz`.

## TL;DR — where we are

- **Phase 4 (`mili-viz` server): ⏳ NOT STARTED.**
- **Phase 5 (`mili-viz` client): ⏳ NOT STARTED** (gated on Phase 4 M1).
- **No `mili-viz` code exists yet.** There are no `mili-viz-*` crates
  in the workspace; this phase is still entirely in design.
- **✅ The Phase 4 M1 surface is now pinned.**
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
| [`README.md`](README.md) | Server/client split, crate layout (`mili-viz-proto` / `-server` / `-client`), `tonic`+Arrow-Flight transport, `wgpu`+`egui` renderer, Phase 4/5 milestone outline | ✅ architecture settled |
| [`scripting.md`](scripting.md) | Scripting is a second pure-Python client of `mili-viz-proto`; **camera is server-authoritative**; interactive `attach()` to a running GUI; `grizinit` batch via `session.run_script()`. Expands Phase 4 M1 with a subscription RPC + `StateDelta` stream + version handshake | ✅ resolved |
| [`client.md`](client.md) | Client wireframe (griz-shaped docks) + AI-first design: a **server-hosted** agent peer of the command vocabulary, autonomous with barge-in + provenance journal, data-first debugging. Expands Phase 4 M1 with `AgentChat`, a `DELTA_AGENT` broadcast kind, `Snapshot`, `Interrupt`; adds Phase 5 M3.5/M6 | ✅ resolved (2026-05-17) |
| [`agent-local-llm.md`](agent-local-llm.md), [`agent-local-llm-posttraining.md`](agent-local-llm-posttraining.md) | Local-LLM agent investigation (model choice / post-training) | 🔎 research notes — not yet a binding decision |

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
| 8 | Derived-result port + validation (no Python oracle) | ✅ resolved: formulas-as-spec + committed golden + tolerance, **no live griz in CI**; detail at M5 | `phase-4-m1.md` Decision 5 |

## Phase 4 — `mili-viz` server (NOT STARTED)

Milestones from [`README.md`](README.md) § "Phase 4 milestones",
expanded by `scripting.md` / `client.md`. None started.

- [ ] **M1 — proto crate + in-process transport.** ✅ **Scope
      pinned** in [`phase-4-m1.md`](phase-4-m1.md) (frozen wire
      contract = base vocab + scripting subscription/`StateDelta`/
      handshake + agent `AgentChat`/`DELTA_AGENT`/`CaptureFrame`/
      `Interrupt`; proto updated to match). **Coding-ready.** Gating
      tests = the `phase-4-m1.md` § "M1 acceptance gate" checklist
      (handshake/capability, Layer-0≡raw equivalence, subscription
      fan-out, frozen-stub `UNIMPLEMENTED`, conformance). Agent RPCs
      are frozen-but-`UNIMPLEMENTED` until Ph4/5 M6 (Decision 6/7).
- [ ] **M2 — load + state navigation.** `load`/`state`/`next`/`prev`;
      stream vertex+index buffers per state.
- [ ] **M3 — primal result display.** `show <svar>`; color array from
      a `mili-rs` query.
- [ ] **M4 — selection + enable/disable.** Mesh filtering, material
      visibility (griz command set is the spec).
- [ ] **M5 — derived results.** Port stress invariants, then strain,
      from griz `Src/*.c`; `rayon` per-element loops. (See open Q8 —
      needs a validation strategy.)
- [ ] **M6 — remote transport.** Same proto over gRPC + Arrow Flight
      on TCP; validate over a real network mount.

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
5. ⏭️ **NEXT (coding):** scaffold `crates/mili-viz-proto` with the
   `tonic` `build.rs` against the updated `proto/mili_viz.proto`, and
   satisfy the `phase-4-m1.md` § "M1 acceptance gate" checklist
   (in-process transport, handshake, Layer-0≡raw, subscription
   fan-out, frozen-stub `UNIMPLEMENTED`).

## Update protocol

Mirror the `mili-py`/`mili-rs` discipline: each milestone lands as its
own PR; flip its `[ ]` → `[x]` here with the gating test named; record
any real architecture/scope decision in the relevant `mili-viz/*.md`
(decision-numbered, like `m4.md`'s 22–26); keep this tracker's TL;DR
and the open-questions table honest so a cold reader can resume from
this file alone.
