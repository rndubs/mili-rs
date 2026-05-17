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
- **⚠️ More planning iterations are needed before implementation.**
  The architecture is sketched ([`README.md`](README.md)) and two big
  sub-designs are resolved ([`scripting.md`](scripting.md),
  [`client.md`](client.md)), but the Phase 4 M1 surface has grown
  across those docs and several design questions are still open (see
  [§ Open design questions](#open-design-questions)). **Do not start
  coding Phase 4 M1 until the M1 surface is pinned in a dedicated
  scope doc** (the analogue of `mili-py/m1.md` — see
  [§ Immediate next steps](#immediate-next-steps)).

## What is already decided (read these first)

| Doc | What it pins | State |
|---|---|---|
| [`README.md`](README.md) | Server/client split, crate layout (`mili-viz-proto` / `-server` / `-client`), `tonic`+Arrow-Flight transport, `wgpu`+`egui` renderer, Phase 4/5 milestone outline | ✅ architecture settled |
| [`scripting.md`](scripting.md) | Scripting is a second pure-Python client of `mili-viz-proto`; **camera is server-authoritative**; interactive `attach()` to a running GUI; `grizinit` batch via `session.run_script()`. Expands Phase 4 M1 with a subscription RPC + `StateDelta` stream + version handshake | ✅ resolved |
| [`client.md`](client.md) | Client wireframe (griz-shaped docks) + AI-first design: a **server-hosted** agent peer of the command vocabulary, autonomous with barge-in + provenance journal, data-first debugging. Expands Phase 4 M1 with `AgentChat`, a `DELTA_AGENT` broadcast kind, `Snapshot`, `Interrupt`; adds Phase 5 M3.5/M6 | ✅ resolved (2026-05-17) |
| [`agent-local-llm.md`](agent-local-llm.md), [`agent-local-llm-posttraining.md`](agent-local-llm-posttraining.md), [`posttraining-dataset.md`](posttraining-dataset.md) | Local-LLM agent investigation (model choice / post-training) + the ordered dataset-construction build plan | 🔎 research notes + build plan — not yet a binding decision |

The reference implementation we are porting from is read-only under
`reference/griz/Src/` (cited by file:path in the docs above).

## Open design questions

These **must be resolved (or explicitly deferred with a reason)**
before the Phase 4 M1 scope doc is final. "✅" = decided in a doc
above; "❓" = still open.

| # | Question | State | Where |
|---|---|---|---|
| 1 | Scripting client model + camera authority | ✅ resolved | `scripting.md` |
| 2 | Client wireframe + AI assistant as a first-class panel | ✅ resolved | `client.md` |
| 3 | **Phase 4 M1 surface is now the union of base RPC + scripting (subscription/`StateDelta`/handshake) + agent (`AgentChat`/`DELTA_AGENT`/`Snapshot`/`Interrupt`). It has never been written down as one consolidated, buildable M1 spec.** | ❓ open — **the blocking item** | needs a new `phase-4-m1.md` |
| 4 | **Picking** — element/node pick: server round-trip vs. client-side from cached geometry + a "describe picked id" RPC | ❓ open (leaning client-side) | `README.md` § Open questions |
| 5 | **Time-history plots** — `egui_plot` client-side for v1 vs. server-computed | ❓ soft-leaning `egui_plot` v1; not pinned | `README.md` § Open questions |
| 6 | **Backwards-compatible CLI** — does the client accept griz's `-i`/`-b` flags | ❓ open (leaning "yes, common ones") | `README.md` § Open questions |
| 7 | **Local-LLM agent**: model + whether/how it is post-trained, and the host/runtime story for the server-hosted agent | ❓ open — research only | `agent-local-llm*.md` |
| 8 | Derived-result port order + parity strategy (no Python oracle here — griz `Src/{stress,strain,iso_surface,contour}.c` is the spec; how do we validate?) | ❓ open | Phase 4 M5 |

## Phase 4 — `mili-viz` server (NOT STARTED)

Milestones from [`README.md`](README.md) § "Phase 4 milestones",
expanded by `scripting.md` / `client.md`. None started.

- [ ] **M1 — proto crate + in-process transport.** `mili-viz-proto`
      command vocab + `tonic` server stub over an in-memory channel.
      **Scope is larger than the README bullet:** also the
      multi-client subscription RPC + server→client `StateDelta`
      stream + version handshake (`scripting.md`) **and** `AgentChat`
      + `DELTA_AGENT` broadcast kind + `Snapshot` + `Interrupt`
      (`client.md`). ⚠️ Pin this in `phase-4-m1.md` before coding.
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

The work is **planning, not coding** — in priority order:

1. **Write `planning/mili-viz/phase-4-m1.md`** — a consolidated,
   buildable M1 scope doc (the analogue of `mili-py/m1.md`). It must
   reconcile the three M1 surfaces into one: the base command vocab
   (`README.md`), the scripting subscription/`StateDelta`/handshake
   (`scripting.md`), and the agent `AgentChat`/`DELTA_AGENT`/
   `Snapshot`/`Interrupt` (`client.md`). Output: the exact
   `mili-viz-proto` message set and the M1 acceptance gate. **This is
   the one blocking item (open Q3).**
2. **Resolve open questions 4–6** (picking, time-history, CLI compat)
   — each is a short decision; record it in `README.md` § Open
   questions and reference it from this tracker's table.
3. **Decide the derived-result validation strategy (Q8)** — there is
   no Python oracle for Phase 4 M5; settle now whether griz itself is
   run as a golden, or numeric tolerances vs. `Src/*.c`, before M5 is
   scheduled.
4. **Take the local-LLM agent investigation to a decision (Q7)** —
   promote `agent-local-llm*.md` from research notes to a pinned
   decision (model, post-training, server host/runtime), or explicitly
   defer the agent panel out of the Phase 4/5 critical path.
5. Only once (1) lands: scaffold the `mili-viz-proto` crate in the
   workspace and begin Phase 4 M1.

## Update protocol

Mirror the `mili-py`/`mili-rs` discipline: each milestone lands as its
own PR; flip its `[ ]` → `[x]` here with the gating test named; record
any real architecture/scope decision in the relevant `mili-viz/*.md`
(decision-numbered, like `m4.md`'s 22–26); keep this tracker's TL;DR
and the open-questions table honest so a cold reader can resume from
this file alone.
