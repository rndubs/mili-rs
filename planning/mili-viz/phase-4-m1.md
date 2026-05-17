# `mili-viz` Phase 4 M1 — proto crate + in-process transport (consolidated, buildable scope)

> Scope doc for Phase 4 Milestone 1. The analogue of
> [`../mili-py/m1.md`](../mili-py/m1.md). It reconciles the three M1
> surfaces that grew across separate docs into **one buildable spec**:
> the base griz command vocabulary ([`README.md`](README.md)), the
> scripting subscription / `StateDelta` / handshake surface
> ([`scripting.md`](scripting.md)), and the server-hosted-agent surface
> — `AgentChat` / `DELTA_AGENT` / framebuffer capture / `Interrupt`
> ([`client.md`](client.md)). It resolves [`status.md`](status.md)
> open question #3 (the blocking item) and folds in the decisions on
> #4 (picking), #5 (time-history), #6 (CLI compat), #7 (agent on the
> critical path), #8 (derived-result validation).
>
> Read [`status.md`](status.md) first (the live tracker), then
> `README.md` / `scripting.md` / `client.md`. The reference behavior is
> read-only griz under `reference/griz/Src/`, cited by `file:line`.
> This is **design only** — no crate is scaffolded by this doc.
> Decision entries below use the `m4.md` decision-22..26 framing.

## Goal

Stand up `crates/mili-viz-proto` (the wire types) and a `tonic`
`mili-viz-server` stub reachable over an **in-process** transport, and
freeze the entire M1 wire contract — every RPC, message, and broadcast
kind that Phases 4 M2–M6 and Phase 5 build against — so no later
milestone has to renegotiate the protocol. M1 ships the *contract* and
the dispatch/broadcast plumbing; it does **not** ship mesh extraction,
result computation, the renderer, or a live LLM backend (those are
M2+, Phase 5, and M6). Exit criterion in [§ M1 acceptance
gate](#m1-acceptance-gate).

## The reconciliation (the blocking item, open Q3)

The M1 surface had been stated three times, never unified:

- `README.md` § `mili-viz-proto`: the griz command vocabulary
  (`load`/`state`/`select`/`show`/`rot`/`iso`/`contour`/`enable`/…),
  line-oriented, batchable from a `grizinit` file.
- `scripting.md` § "Protocol impact": adds a **subscription RPC**, a
  server→client streaming `StateDelta`, a **version/capability
  handshake**, and the on-disk session/connection file.
- `client.md` § "Proto impact": adds **`AgentChat`**, a
  **`DELTA_AGENT`** broadcast kind, a framebuffer **capture**
  capability (the frame behind `s.snapshot()`), and **`Interrupt`**,
  plus an `agent` handshake capability flag.

`proto/mili_viz.proto` (draft, commit `5bd8195`) already implements the
first two. M1 = **all three**, with the draft amended per Decision 1.

### Decision 1 — the M1 proto surface is the union of the three docs, with the agent surface added and the `Snapshot` name collision resolved (reconciles open Q3; supersedes the draft `proto/mili_viz.proto`)

The central question: is the agent surface (`client.md`) in M1, or a
later add that re-opens the wire contract? If it is deferred out of the
contract, M6 (or Phase 5) must add RPCs, a `DeltaKind`, and a
`StateDelta.payload` arm to a protocol that scripting clients and a
shipped `egui` client are already generated against — exactly the
versioned-mismatch pain `scripting.md` exists to avoid.

**Decision: the M1 wire contract is the full union — base vocabulary +
scripting multi-client surface + agent surface — frozen now. The agent
*implementation* is off the M1–M5 critical path (Decision 6); its
*contract* is in M1.** Locking the bytes early is nearly free (the
agent messages are additive and small) and is the only way the
`protocol_version` handshake can do its job: a server built without an
LLM backend advertises `agent` absent and the client hides the panel
(Decision 6) — it never sees an unknown message.

Concretely, M1 `mili-viz-proto` is the draft **plus** the following
deltas. Every delta from `proto/mili_viz.proto` @ `5bd8195`:

| # | Delta vs. draft | Source | Kind |
|---|---|---|---|
| Δ1 | `rpc AgentChat(AgentChatRequest) returns (AgentChatReply)` — client→server user turn; the server runs the loop and the agent's commands flow through the **existing** internal dispatch (so they broadcast as ordinary `StateDelta`s tagged with the agent's `origin_client_id`). | client.md | add RPC |
| Δ2 | `rpc Interrupt(InterruptRequest) returns (InterruptReply)` — barge-in; cancels the in-flight agent turn. | client.md | add RPC |
| Δ3 | `rpc CaptureFrame(FrameRequest) returns (FrameReply)` — offscreen `(w,h,format) → encoded bytes`. **Backs both** the agent vision tool and Python `s.snapshot()`; one capability, two callers. | client.md / scripting.md `s.snapshot()` | add RPC |
| Δ4 | `enum DeltaKind { … DELTA_AGENT = 10; }` — agent transcript/tool/status events ride the existing `Subscribe` stream so every client mirrors the conversation (consistent with camera being broadcast state). | client.md | add enum value |
| Δ5 | `message StateDelta { oneof payload { … AgentEvent agent = 13; } }` | client.md | add `oneof` arm |
| Δ6 | New messages: `AgentChatRequest/Reply`, `InterruptRequest/Reply`, `FrameRequest/Reply`, `AgentEvent`, `AgentStatus` (+ `AgentStatusKind`), `AgentToken`, `AgentToolBegin`, `AgentToolEnd`, `AgentUserTurn`, `AgentTranscript`, `AgentMessage`. | client.md | add messages |
| Δ7 | **`Snapshot` name collision resolved.** The draft already has `message Snapshot` = the *full session-state* delta sent once at stream open (`DELTA_SNAPSHOT`). client.md's "Snapshot" is a *framebuffer capture* — a different thing. The framebuffer RPC/messages are named **`CaptureFrame` / `FrameRequest` / `FrameReply`**; the state snapshot keeps the name `Snapshot`. (Scripting API keeps `s.snapshot()` — it lowers to `CaptureFrame`.) | this doc | rename to disambiguate |
| Δ8 | `message Snapshot { … AgentTranscript agent = 7; }` — a late-joining client gets the running transcript + agent status in the opening `DELTA_SNAPSHOT`, the same way it gets camera/selection. | client.md ("transcript in the opening `DELTA_SNAPSHOT`") | add field |
| Δ9 | `agent` documented as a reserved value in `HelloReply.capabilities` (and the `capabilities` request/echo): present iff the server has an LLM backend configured. The client keys the AI panel's existence off it. | client.md / scripting.md cap-negotiation | doc/contract, no new field |

Non-deltas, recorded so they are not re-litigated: the command
*vocabulary* is unchanged from the draft (still the griz set —
`reference/griz/Src/interpret.c`); bulk geometry still rides Arrow
Flight via `GeometryRef`, never protobuf; the session/connection file
stays JSON-on-disk, not proto. Picking and time-history add **no**
proto (Decisions 2, 3).

**Trade-off recorded.** Freezing the agent bytes in M1 means M1's
`mili-viz-proto` carries messages with **no server implementation
until M6** (`AgentChat` returns `UNIMPLEMENTED`; see Decision 7). The
alternative — ship M1 without them, add them at M6 — was rejected: it
forces a `protocol_version` bump mid-Phase-4 and breaks the
"pip-upgraded client warns, never segfaults" guarantee for every
scripting client generated against the M1 stubs. The cost of carrying
unimplemented-but-frozen messages is a few `UNIMPLEMENTED` stubs and
documentation; the cost of not freezing them is a protocol break on a
shipped contract.

### Decision 2 — picking is client-side against cached `GeometryRef`; the "describe picked id" readout reuses `Query`, adding no M1 proto (resolves open Q4)

Central question (Q4, README § "Open questions"): element/node pick —
server round-trip per click, or client-side from the geometry the
client already cached, plus a "describe picked id" RPC?

griz's pick is `setpick <btn> <class>` + a mouse pick that highlights
an element/node and writes a status-bar readout
(`reference/griz/Src/interpret.c:3066` `setpick`,
`:1081` `select`/`unselect`; `reference/griz/Src/faces.c` pick
geometry). The selection verbs (`select`/`clrsel`) are **already**
typed `Command`s in the draft.

**Decision: picking is computed client-side from the cached
`GeometryRef` buffers (the client already holds them to render);
the status-bar "describe" readout (class · label · current-result
value at the picked id) is one `Query` for that `(class_name,
labels=[id], states=[current])`. No new RPC, no new message — Q4 adds
nothing to the M1 proto.** Mouse→ray→nearest-primitive is a pure
client computation over data it must already have; turning a hover into
a structured fact is exactly what `Query` already does. A pick that
should change *selection* (griz `select`) emits the existing `Select`
`Command`, which broadcasts a `DELTA_SELECTION` like any other mutation
— so a script and the GUI stay in sync on selection for free.

**Trade-off recorded.** Server-side picking would be exact for
degenerate/curved elements where the client's cached triangulation is
an approximation. Rejected for M1: it puts a synchronous round-trip on
the mouse-move path (the latency the split exists to avoid for
interaction), and the readout precision that matters for debugging is
the *result value*, which `Query` already gives exactly. Revisit only
if a corpus shows client-side id resolution drifting from griz's.

### Decision 3 — time-history is a Phase-5 client `egui_plot` view fed by the existing `Query` state-range; no server plot RPC in M1 (resolves open Q5)

Central question (Q5): time-history plots
(`reference/griz/Src/time_hist.c`) — `egui_plot` client-side, or a
server-computed plot surface?

**Decision: time-history is client-side `egui_plot` (Phase 5 M3.5
bottom tab, per client.md), and its data is the **existing**
`QueryRequest` with `states` spanning the range (`QueryReply` already
carries `[states × labels × components]`). No server-side plotting,
no plot RPC, nothing added to the M1 proto.** `time_hist.c`'s model is
X-Y series of a result over states for a set of objects — that is
literally a `Query` over a state range; the server already owes that
payload for the scripting layer. The plot is a *rendering* of data the
contract already returns, and rendering is the client's job (README §
"Why split").

**Trade-off recorded.** Server-side plot computation (e.g. server
returns plot-ready aggregates) would offload the client and centralize
griz's glyph/averaging logic. Rejected for M1: it would invent a second
data path parallel to `Query` for no contract benefit, and Q5 was only
ever "soft-leaning `egui_plot`" — pinning it client-side keeps the M1
surface minimal. The port of `time_hist.c`'s series/glyph specifics is
Phase 5 M3.5 work, not M1, and rides `Query` unchanged.

### Decision 4 — the client accepts only a small portable subset of griz's CLI flags (`-i`, `-b`/`-batch`, `-V`, `-w`); the rest are dropped as launcher/X11-specific; this is a client concern, not proto (resolves open Q6)

Central question (Q6): does the new client accept griz's `-i`/`-b`
flags? griz's full flag set
(`reference/griz/Src/viewer.c:500` `scan_args`, `:2900` `usage`):
`-i base` (required input base), `-b`/`-batch file` (batch grizinit),
`-s` (single-buffer GL), `-f` (foreground), `-v`/`-w` (image sizing),
`-V` (version), `-u` (Motif util panel), `-gid`, `-win32`, `-tv`
(TotalView), `-beta`/`-alpha`/`-version` (launcher version-switching),
`-man`, `-q`, `-nodialog`, `-bname`, `-checkresults`/`-cr`.

**Decision: the `mili-viz-client` binary accepts exactly the portable
subset — `-i <base>` → an initial `load`, `-b`/`-batch <file>` →
`session.run_script(<file>)` on startup, `-V` → print version & exit,
`-w <w> <h>` → initial window size. Everything else is dropped: it is
either Motif/X11-specific (`-s`, `-u`, `-win32`, `-nodialog`), a
legacy launcher concern (`-beta`/`-alpha`/`-version`, `-tv`, `-gid`,
`-bname`, `-man`), or already a first-class command rather than a flag
(`-checkresults` → the data-first NaN/Inf scan is `Query`, see
Decision 5). This is `mili-viz-client` argv parsing; it touches no
proto and is recorded here only to close Q6.** `-i`/`-b` are the only
flags in muscle memory for batch/grizinit users and they map cleanly
onto the existing `Load` command and `run_script` (which streams lines
to the same dispatcher). The dropped flags have no referent in a
`wgpu`+`egui`, server-split world.

**Trade-off recorded.** Bug-for-bug flag compatibility would smooth
migration of existing wrapper scripts that pass e.g. `-s`/`-u`.
Rejected: those flags toggle Motif/GLX behavior that does not exist
here; silently accepting-and-ignoring them is worse than a clear
"unknown flag" (it implies a behavior we don't honor). Power users who
need an exact legacy invocation use `run_script` with their existing
`grizinit`, which *is* bit-for-bit preserved (`Command.raw`, the
Layer-0≡raw integration test).

### Decision 5 — derived-result parity has no live oracle: griz `Src/*.c` formulas are the spec, validated against a committed golden fixture + numeric tolerance, with **zero** live-griz dependency in CI; full strategy pinned now, detailed at M5 (resolves open Q8 with a pinned approach)

Central question (Q8): Phase 4 M5 ports stress invariants / strain /
isosurface / contour from `reference/griz/Src/{stress,strain,
iso_surface,contour}.c`. Unlike `mili-rs`/`milox` (which have the
`mili` Python package as a bit-exact oracle), **there is no upstream
oracle for viz derived results**. How are they validated?

Options: (a) run griz itself as a golden in CI; (b) numeric tolerance
against values hand-extracted once from griz and committed as a
fixture; (c) re-derive the `Src/*.c` formulas as a written spec and
test against analytic/hand-computed expectations.

**Decision: (b)+(c). The `Src/*.c` formulas are transcribed into the
M5 scope doc as the written spec (the `m4.md` "byte layout in the doc,
corpus wins on conflict" discipline, applied to formulas); correctness
is gated by a committed golden fixture — expected derived values for a
fixed set of elements/states on the existing parity corpus, generated
**once** out of an instrumented griz run and checked into the repo as
JSON — diffed at a documented numeric tolerance (`f32` accumulation
order differs from griz's C, so this is tolerance, not bit-exact). The
*input* to the computation is `mili-rs`'s already-parity-exact primal
`query`. CI has **no** dependency on a griz binary** (griz is Motif/GLX,
not in the parity test path, and `setup-parity.sh` deliberately only
provisions `mili`/`mili-python`). The agent's NaN/Inf scan
(griz `-checkresults`, `reference/griz/Src/viewer.c`) is the same
`Query`-level corroboration, not a separate path.**

The detailed element/state list, the per-result tolerance, and the
fixture-generation procedure are M5 scope-doc work (analogous to how
`m4.md` pins write-parity detail at the write milestone) — but the
*approach* is pinned now so M5 can be scheduled: **golden fixture +
tolerance, formulas-as-spec, no live griz.**

**Trade-off recorded.** Option (a) (griz-as-golden in CI) would be
authoritative and self-updating. Rejected: griz needs X11/Motif/GLX and
a build toolchain absent from the parity environment; wiring it into CI
contradicts `setup-parity.sh` being the single provisioning source of
truth and adds a heavy, flaky dependency for a handful of golden
numbers. The committed-fixture cost is that regenerating the golden
(if a formula bug is found in our port *or* a deliberate deviation is
chosen) is a manual, documented step — acceptable, and the same
trade-off `mili-py` accepts for its checked-in expectations.

### Decision 6 — the agent *contract* is in M1; the agent *implementation* and the local-LLM model choice are off the M1–M5 critical path (Phase 4/5 M6, capability-gated) (resolves open Q7 as a scope call)

Central question (Q7): is the AI assistant panel on the Phase 4/5
critical path, and is the local-LLM investigation
(`agent-local-llm*.md`) gating?

**Decision: split contract from implementation.**
- **In M1 (critical path):** the agent *wire contract* — Δ1–Δ9 of
  Decision 1. Frozen now because the `protocol_version` handshake
  requires it (Decision 1 trade-off).
- **Off the M1–M5 critical path:** the agent *implementation* — the
  server reasoning loop, the `LlmProvider` trait, `CaptureFrame`'s
  offscreen renderer, the provenance journal. This is **Phase 4 M6 +
  Phase 5 M6** (client.md phasing), gated behind the `agent`
  capability flag (Decision 1 Δ9): a server with no LLM backend
  advertises `agent` absent and a conformant client hides the panel.
  Nothing in M1–M5 depends on it; M2–M5 (load/state/result/selection/
  derived) proceed with the agent server-side unimplemented.
- **Deferred, explicitly non-gating:** the local-LLM model choice and
  post-training story (`agent-local-llm.md`,
  `agent-local-llm-posttraining.md`) stay **research notes, not a
  pinned decision**. `client.md` decision 4 already pins "Claude API
  first behind a thin `LlmProvider` trait"; the offline-model bar is an
  M6-time question behind that trait and gates nothing earlier. The
  agent panel is **not** dropped from Phase 4/5 — it is the headline
  feature — but it is sequenced last and isolated behind a capability
  flag so the renderer/data path can land without it.

**Trade-off recorded.** Promoting the agent onto the M2–M5 path would
force the LLM backend and offscreen renderer to be solved before
basic mesh display works — inverting the README's dependency order and
blocking the entire viz stack on an LLM integration. Rejected.
Deferring the *contract* too (the symmetric alternative) was already
rejected in Decision 1 (protocol break). This split is the only option
that keeps the wire stable and the critical path LLM-free.

### Decision 7 — M1 ships frozen-but-unimplemented stubs for the deferred surface; `UNIMPLEMENTED` is a contract state, not a gap (applies Decisions 1 & 6 to the M1 deliverable boundary)

Because Decision 1 freezes more contract than M1 implements, M1 needs a
crisp "defined vs. live" line so the [acceptance gate](#m1-acceptance-gate)
is unambiguous.

**Decision: in M1, the RPCs whose behavior belongs to later milestones
return `tonic::Status::unimplemented` with a message naming the gating
milestone, and the conformance test asserts exactly that.** A frozen
message that returns a typed `UNIMPLEMENTED` is a *contract guarantee*
(the client can generate against it and degrade gracefully), not an
untested gap. M1 "implements" only: the handshake, the in-process
transport, the command-dispatch+broadcast plumbing, the
typed↔raw command equivalence, and subscription fan-out. Specifically
for M1:

| Surface | M1 state |
|---|---|
| `Hello` | **live** — version/capability negotiation, token check, `SessionInfo` echo. |
| `Subscribe` | **live** — opening `DELTA_SNAPSHOT` + fan-out of every broadcast to all subscribers. |
| `Execute` (all `Command` variants incl. `raw`) | **live as dispatch+broadcast**: parsed, validated, equivalence-checked, and broadcast as the correct `StateDelta` kind. The *effects that need a loaded mesh / result engine* (real geometry in `GeometryRef`, real colors, derived values) are M2–M5; M1 broadcasts the state transition with `GeometryRef` empty/placeholder. |
| `Query` | **live for shape/plumbing**, real values M3+ (needs `mili-rs` wired in at M2). |
| `AgentChat`, `Interrupt` | `UNIMPLEMENTED` → "Phase 4/5 M6" (Decision 6). |
| `CaptureFrame` | `UNIMPLEMENTED` → "needs offscreen renderer (Phase 4 M6 / Phase 5)". |

**Trade-off recorded.** A narrower M1 (only Hello+Subscribe+Execute
defined, the rest absent from the `.proto`) would have less dead
surface. Rejected — that is exactly the protocol-break Decision 1
forbids. The cost here (a handful of one-line `unimplemented` arms and a
test that pins them) is the price of a stable contract.

## M1 proto surface (the buildable enumeration)

After Decision 1, `crates/mili-viz-proto/proto/mili_viz.proto` is the
draft with Δ1–Δ9 applied. The authoritative file is
[`proto/mili_viz.proto`](proto/mili_viz.proto) (updated in this PR to
match). Surface summary:

- **Service `MiliViz`:** `Hello`, `Execute`, `Subscribe`, `Query`
  (draft) **+ `AgentChat`, `Interrupt`, `CaptureFrame`** (Δ1–Δ3).
- **Handshake:** `HelloRequest`/`HelloReply`/`SessionInfo`
  (unchanged); `agent` is a documented `capabilities` value (Δ9).
- **Commands:** the griz vocabulary `oneof` (unchanged) + the
  `Command.raw` Layer-0 escape hatch; Layer-1≡raw is a gate.
- **Subscription:** `SubscribeRequest`, `DeltaKind` (+ `DELTA_AGENT`,
  Δ4), `StateDelta` (+ `AgentEvent` arm, Δ5), `Snapshot`
  (+ `AgentTranscript`, Δ8) and the existing per-aspect state
  messages; bulk data via `GeometryRef`/Flight (unchanged).
- **Query:** `QueryRequest`/`QueryReply`/`InlineTable` (unchanged) —
  also the picking-readout (Decision 2) and time-history (Decision 3)
  data path.
- **Agent (frozen, M6-implemented):** `AgentChatRequest/Reply`,
  `InterruptRequest/Reply`, `FrameRequest/Reply`, `AgentEvent` +
  `AgentStatus`/`AgentStatusKind`/`AgentToken`/`AgentToolBegin`/
  `AgentToolEnd`/`AgentUserTurn`, `AgentTranscript`/`AgentMessage`
  (Δ6).

## M1 acceptance gate

"M1 done" = the contract is frozen and the in-process plumbing is
proven, with **no upstream oracle** (there is no `mili` analogue for
viz; the gate is conformance + internal-equivalence, per Decision 5's
reasoning generalized to the protocol). M1 is a planning deliverable;
this gate is what the *coding* M1 PR must satisfy and is recorded here
so it is not reinvented:

- [ ] `crates/mili-viz-proto` builds; `tonic` `build.rs` generates
      Rust from the Δ1–Δ9 `.proto`; the message set matches
      [§ M1 proto surface](#m1-proto-surface) exactly.
- [ ] `mili-viz-server` stub serves `MiliViz` over an **in-process**
      transport (in-memory channel, no TCP).
- [ ] **Handshake test:** matching `protocol_version` → `compatible`;
      a deliberately bumped version → `compatible == false` with
      `mismatch_detail` set and **no panic** (the Visit-without-the-
      lock-in guarantee, scripting.md).
- [ ] **Capability test:** a server built without an LLM backend omits
      `agent` from `HelloReply.capabilities`; with it, present.
- [ ] **Command-equivalence test (scripting.md "Layer 0 ≡ raw"):** for
      every typed `Command` variant, the typed form and the equivalent
      `Command.raw` string produce an identical `StateDelta` sequence.
      This is the M1 form of a parity test.
- [ ] **Subscription fan-out test:** two in-process clients subscribe;
      a mutation from one is observed by both, ordered, with
      `seq == CommandReply.delta_seq` and `origin_client_id` set;
      a late subscriber's stream opens with a `DELTA_SNAPSHOT`
      reflecting prior mutations (incl. an `AgentTranscript` field,
      empty in M1).
- [ ] **Frozen-stub test (Decision 7):** `AgentChat`, `Interrupt`,
      `CaptureFrame` return `UNIMPLEMENTED` naming the gating
      milestone; the messages compile and round-trip.
- [ ] **Conformance test:** every `Command` `oneof` arm dispatches and
      broadcasts the correct `DeltaKind` (geometry effects stubbed:
      `GeometryRef` empty until M2).
- [ ] `status.md` milestone checklist + open-questions table updated;
      `README.md` § "Open questions" records Decisions 2–6.

No `cargo`/render/LLM behavior beyond the above is in M1. M2 attaches
`mili-rs` and real geometry; the gate above is purely the contract +
transport + broadcast semantics.

## Out of scope for M1 (which milestone owns it)

- Real mesh extraction / `GeometryRef` payloads, Arrow Flight bytes —
  M2 (`README.md` Phase 4 M2).
- Real primal colors / `Query` values — M3.
- Selection/material *effects* on geometry — M4.
- Derived results + the Decision 5 golden-fixture validation — M5.
- TCP/remote transport, Flight over the wire — M6.
- Agent loop, `LlmProvider`, `CaptureFrame` renderer, provenance
  journal — Phase 4 M6 / Phase 5 M6 (Decision 6).
- `egui` client, picking math, `egui_plot` time-history — Phase 5
  (Decisions 2, 3 fix the contract these consume; they add no M1
  proto).

## Decision log (this doc)

| # | Title | Resolves |
|---|---|---|
| 1 | M1 proto surface = union of three docs; agent contract frozen; `Snapshot` collision resolved | open Q3 |
| 2 | Picking client-side; readout reuses `Query`; no M1 proto | open Q4 |
| 3 | Time-history client-side `egui_plot` fed by `Query`; no M1 proto | open Q5 |
| 4 | CLI: portable `-i`/`-b`/`-V`/`-w` subset only | open Q6 |
| 5 | Derived-result validation: formulas-as-spec + committed golden + tolerance, no live griz | open Q8 |
| 6 | Agent contract in M1; impl + local-LLM off the M1–M5 critical path | open Q7 |
| 7 | Frozen-but-`UNIMPLEMENTED` stubs are a contract state | applies 1 & 6 |
