## Phase 5 M6 — agent integration polish (server-hosted agent loop + client AI panel)

> Status: 🟢 in progress (drafted 2026-05-23). Decisions 94–99 below;
> the live tracker is [`status.md`](status.md). The wire contract
> ([`phase-4-m1.md`](phase-4-m1.md) Decision 1 Δ4–Δ9) was pinned **nine
> milestones ago** specifically to let M6 light up without a `.proto`
> change — and it does.

## What lands

The agent surface that has been `Status::unimplemented` since Phase 4 M1
(per `phase-4-m1.md` Decision 7 — "freeze the shape, defer the impl")
goes live as a complete sub-step batch M6a–M6f per
[`client.md`](client.md) §"Phasing":

- **M6a — AI panel chrome.** `crates/mili-viz-client/src/ai_panel.rs`
  carries the `AiPanelState` (transcript rows, composer buffer,
  attached frame, status pill, capability gate). The right-dock `ai`
  panel expands from the 28 px placeholder rail (`shell.rs:1248-1258`)
  to a 340 px transcript+composer when the user clicks the rail and
  the server advertised `CAP_AGENT`. Otherwise it stays the rail (a
  server without an agent backend → no `CAP_AGENT` → no panel, per
  the [`scripting.md`](scripting.md) capability-negotiation pattern).
- **M6b — agent loop.** `crates/mili-viz-server/src/agent.rs` introduces
  the `AgentBackend` trait + the always-on `MockAgent` deterministic
  implementation. `VizService::builder().agent_backend(...)` plugs one
  in; `agent(true)` continues to advertise the capability when a
  backend is present. `MiliViz::agent_chat` spawns a turn task that:
  1. broadcasts `AgentEvent::UserTurn` (the human's message echoes to
     every attached client per `client.md` §"Design principle"),
  2. snapshots session state for the provenance journal (Decision 97),
  3. drives `backend.run_turn(ctx)` — the backend streams
     `AgentStatus` / `AgentToken` / `AgentToolBegin` / `AgentToolEnd`
     events through the broadcast bus,
  4. dispatches any tool-call command through the **existing
     `VizService::dispatch`** so it broadcasts as an ordinary
     `StateDelta` tagged with the agent's `origin_client_id`
     (`client.md` §"Design principle"),
  5. emits the closing `Status(idle)` on normal completion or
     `Status(interrupted)` on barge-in.
- **M6c — `CaptureFrame` + 📷 attach-frame.** `MiliViz::capture_frame`
  returns a deterministic placeholder PNG via the `image` crate (size
  fills the requested rect with a midtone fill; encoded format matches
  `format`). The client's composer 📷 button toggles
  `AiPanelState.attach_frame`; sending sets
  `AgentChatRequest.attach_frame = true` and (optionally) pre-encodes
  the bytes via `capture_frame`. The real server-side wgpu offscreen
  is a separate follow-up — this milestone is "the RPC is no longer
  `UNIMPLEMENTED`".
- **M6d — provenance.** Each user-turn entry captures
  `Session::snapshot()` and stashes it in a `Vec<TurnSnapshot>` under
  `Inner`. The next opening `DELTA_SNAPSHOT` carries the running
  `AgentTranscript` (now populated, where M1 left it empty per the
  `phase-4-m1.md` Decision 1 Δ8 plumbing). The transcript renderer
  paints an inline turn-boundary row with `↶ revert to here`; revert
  lowers to the already-typed `SetState` / `Show` / `SetCamera`
  commands reconstructed from the captured snapshot. No new typed
  variant — every revert primitive is in the frozen set.
- **M6e — `Interrupt` + ⏹ Stop.** `MiliViz::interrupt(turn_id)` flips
  the active turn's `Arc<AtomicBool>` cancel flag; the mock backend
  observes it between ticks and exits early. Server broadcasts
  `Status(interrupted)`. Client Stop button replaces Send when the
  status pill is `thinking` / `running` and calls `Interrupt`.
- **M6f — peer banner + status-bar peer count.** Every `subscribe`
  broadcasts an `AgentEvent::Status { kind: AGENT_IDLE, detail:
  "peers=N" }` carrying the new `tx.receiver_count()` so all clients
  learn that a peer joined. The client parses `peers=N` out of the
  detail string and renders the wireframes §"Session states" *peer
  attached* banner + the status bar `n peer(s)` cell. Peer-leave is
  detected on the next subscribe or agent turn (the broadcast bus
  prunes dead receivers on next send).

**No `mili_viz.proto` change.** Every wire surface above
(`AgentChat`, `Interrupt`, `CaptureFrame`, `DELTA_AGENT`,
`AgentEvent`/`AgentStatus`/`AgentToken`/`AgentToolBegin`/`AgentToolEnd`,
`AgentTranscript`, `Snapshot.agent`, `HelloReply.capabilities[agent]`)
has existed in the frozen proto since Phase 4 M1 Decision 1 Δ4–Δ9
specifically to land *exactly* this milestone without a contract
change.

## Decisions

### Decision 94 — Backend trait, always-on `MockAgent`, real LLM gated separately

The agent loop lives behind a tiny `AgentBackend` trait in the server
crate. The always-on `MockAgent` is the gating-test driver and the
build-default-when-`agent(true)`-but-no-backend; a real
LLM-backed implementation (Anthropic Claude API per
[`client.md`](client.md) Decision 4) is **out of M6 scope** and stays
gated behind a future Cargo feature or env-var. The trait surface:

```rust
pub trait AgentBackend: Send + Sync + 'static {
    fn run_turn<'a>(
        &'a self,
        ctx: AgentTurnCtx,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send + 'a>>;
}
```

`AgentTurnCtx` owns the broadcast handle, the dispatch closure (so the
backend's tool calls flow through `VizService::dispatch` exactly like
any human's `Execute` — `client.md` §"Design principle"), the cancel
token, and the user's request (text + attached frame). Object-safe by
construction (no async fn in the trait — manual `Pin<Box<dyn Future>>`
return — so no `async-trait` macro dep).

Rationale for not adding a real LLM backend in M6:
[`client.md`](client.md) Decision 4 calls Claude API "the pragmatic
default" but the same paragraph and `phase-4-m1.md` Decision 6 mark
the model choice **off the M1–M5 critical path**. M6 lands the
**wiring** that makes a real backend a 50-line plug-in; shipping a
default API-key-bearing backend touches dep tree, network egress
policy, and config UX that does not belong in this milestone.

### Decision 95 — Agent commands flow through `VizService::dispatch`

The backend's `AgentTurnCtx::dispatch(cmd, origin)` is a thin proxy
for the existing `VizService::dispatch` (`lib.rs:391`). That
single seam — already shared by typed `Execute`, the `raw` escape
hatch, and now the agent — is what makes the [`client.md`](client.md)
§"Design principle" promise hold: "every agent action is an ordinary
`Command` producing an ordinary `StateDelta`". The agent's stable
`origin_client_id` (`agent:mock` for the mock; future real backends
pick their own) tags every broadcast `StateDelta` so the provenance
journal can correlate.

The mock's deterministic turn does:
`SetState` (single state step) → broadcast `DELTA_STATE` → the
windowed client re-renders. The point is **the dispatch path**, not
the chosen command — the gating test only asserts (a) the broadcast
happened, (b) the tag is the agent's id, and (c) the `AgentToolEnd`
carries the matching `delta_seq`.

### Decision 96 — `CaptureFrame` returns a deterministic placeholder PNG

`MiliViz::capture_frame` is no longer `UNIMPLEMENTED`: it encodes a
midtone-grey image of the requested `(width, height)` in the
requested `format` ("png" / "jpeg"). This satisfies the
[`client.md`](client.md) §"Vision is deliberate but agent-initiated"
contract surface — the RPC returns bytes — without standing up a
server-side wgpu adapter (that is a server-side renderer milestone
of its own; the client renderer's offscreen path stays the
authoritative production answer).

The 📷 button on the composer toggles
`AiPanelState.attach_frame_pending`; on Send, the windowed app
issues `capture_frame` and pins the bytes onto the outgoing
`AgentChatRequest` (`attach_frame = true`,
`attached_frame = bytes`, `attached_frame_format = format`). The
backend has the user's pinned frame on the first message of the
turn; the agent's own mid-turn vision calls (`Snapshot` tool
internally) reuse the same `capture_frame` RPC.

`image` crate dep is added to `mili-viz-server` for this; it is
small (≈1 MB), already part of the workspace's transitive tree (via
`wgpu`), and replacing the placeholder with a real wgpu-server
encoder is a one-function swap.

### Decision 97 — Per-turn snapshot is client-side; transcript is the only server-side carrier

The pre-turn snapshot used by `↶ revert to here` lives **client-side**
in the windowed app: every peer that receives the broadcast
`AgentEvent::UserTurn` already has the corresponding `ShellState` it
observed and stashes a `TurnSnapshot` (state, result, camera) in its
own provenance journal. The server-side `TurnRecord` is just the
late-joiner replay carrier — the assistant text + dense tool-line
summaries — so the opening `DELTA_SNAPSHOT` for any new subscriber
carries a populated `AgentTranscript` (`Snapshot.agent`, the existing
Δ8 carrier where M1 left it empty).

A late joiner can revert against turns that landed **after** it joined
(it observed their `UserTurn` deltas and snapshotted its own state);
reverting to a turn from before it joined is out of scope for v1 —
the alternative would either serialize the full session-state
`Snapshot` into `AgentMessage.tool_lines` (an ugly carrier overload)
or add a per-turn field to the proto (forbidden — the contract is
frozen). The wireframes' P3 timeline strip would extend this, but is
itself deferred.

The client transcript renders each `AgentMessage` plus an inline
turn-boundary row showing the snapshot summary
(`state=N · result=name`) and the `↶ revert to here` link. Revert
lowers to a small chain of typed commands reconstructed from the
captured snapshot — `SetState(state)`, `Show(result.result,
result.component)`, and a `View(SetCamera(...))` if the camera
moved. Every primitive is in the frozen `Command` set; no new
typed variant.

### Decision 98 — `Interrupt` cancels via an `Arc<AtomicBool>`

`Inner.active_turn: Mutex<Option<ActiveTurn>>` holds the current
turn's id + an `Arc<AtomicBool>` cancel flag. `MiliViz::interrupt(
turn_id)` looks up the active turn (or any if `turn_id` is empty —
the proto's "empty = current turn" convention), flips the flag, and
returns `ok = true`. The mock backend observes
`ctx.cancelled()` between every emit + before every dispatch; on
cancel it returns early, the server broadcasts
`Status(interrupted)`, and the client's Stop button reverts to Send.

The `ActiveTurn` slot is cleared at turn end (idle or interrupted),
so the next `agent_chat` is a fresh turn. Multiple in-flight turns
are out of scope (the frozen contract has a single agent transcript
per session; barge-in is the only concurrency model — `client.md`
§"Open questions" *Multi-user conflict*).

### Decision 99 — Peer count rides `AgentStatus.detail = "peers=N"`

The wireframes §"Session states" peer banner / status-bar peer cell
need a dynamic count of attached clients. The frozen proto has no
peer-count field, no `Hello`-redo, no dedicated `DELTA_PEERS` kind —
all three would require a contract change.

The minimal-impact carrier is the existing `DELTA_AGENT`
broadcast channel: each `subscribe` finishes by sending an
`AgentEvent { ev: Status { kind: AGENT_IDLE, detail: "peers=N" } }`
where `N = tx.receiver_count()`. The client parses the detail string
out of every status event it receives. Late joiners get the count
from the opening `DELTA_SNAPSHOT`'s `Snapshot.agent.status.detail`.
Peer-leave is auto-detected on the next subscribe or agent turn
because the broadcast bus prunes dropped receivers before sending.

**Gated on the `agent` capability.** A vanilla `.agent(false)` server
(the M1 acceptance-gate default) emits **no** peer-status broadcasts —
so the M1 `subscription_fanout` / `conformance_all_command_arms`
tests stay byte-stable. Only `.agent(true)` / `.agent_backend(...)`
servers (the deployments that have a panel to surface the count to)
emit the broadcasts.

This uses `AgentStatus.detail`'s free-form text field for a signal
that is adjacent-to-but-not-strictly-agent (a peer joining the
session). The justification: it is the only protocol-stable surface
for the signal, the detail field is already free-form, and the
alternative is a proto change. Recorded explicitly so a future reader
sees the trade-off.

## M6 acceptance gate

`crates/mili-viz-server/tests/m6_agent.rs` (always-on; server-side
acceptance — no fixture corpus needed because the `MockAgent` is
self-contained):

1. **Capability gate ties to backend presence.** A server built with
   `.agent(true)` but no backend advertises `CAP_AGENT` and returns
   `agent_chat`/`interrupt` as not-configured (clear `error`, not
   `UNIMPLEMENTED`); a server with a backend plugged in succeeds.
2. **`agent_chat` end-to-end.** Subscribe, send one chat, observe in
   order: `UserTurn` → `Status(thinking)` → ≥1 `Token` → `ToolBegin`
   → an ordinary `StateDelta` (e.g. `DELTA_STATE`) tagged with the
   agent's `origin_client_id` and an integer `delta_seq` → `ToolEnd`
   carrying that same `delta_seq` → `Status(idle, "peers=...")`.
3. **`interrupt` causes early `Status(interrupted)`.** Issue
   `agent_chat`, immediately `interrupt(turn_id)`, observe
   `Status(interrupted)` and no further `Token` deltas for that turn.
4. **`capture_frame` returns a non-empty `(width, height)` image of
   the requested format.** A 16×16 PNG decodes to a 16×16 image; a
   32×24 JPEG returns non-empty bytes.
5. **`Snapshot.agent` is populated after a turn lands.** A late
   subscriber's opening `DELTA_SNAPSHOT` carries an `AgentTranscript`
   with the prior user message + the closing `Status` — discharging
   the empty-in-M1 carrier (`phase-4-m1.md` Decision 1 Δ8).
6. **Peer count broadcast on subscribe.** A new subscribe causes a
   `DELTA_AGENT` with `Status.detail == "peers=N"` to land on every
   prior subscriber (Decision 99).

`crates/mili-viz-client/tests/m6_agent_panel.rs` (always-on; pure
`ShellState`/`AiPanelState` logic):

1. **Panel hidden absent `CAP_AGENT`.** With `cap_agent == false` the
   AI rail stays the 28 px placeholder; expand-rail does nothing.
2. **Capability sets gate to expanded.** With `cap_agent == true` and
   `ai_expanded == true` the panel renders header + transcript +
   composer.
3. **Agent-event → transcript line mapping.** Folding a sequence of
   `AgentEvent`s into `AiPanelState` produces the expected ordered
   rows: user message, assistant tokens concatenated into one
   row per turn, dense-one-liner tool-call rows
   (`client.md` §"AI Assistant panel" — `▸ ran      …`,
   `▸ queried   …`), turn-boundary rows with the captured snapshot.
4. **Composer Send → typed action.** With non-empty input and the
   attach-frame toggle on, `submit_agent_chat()` returns a
   `UiAction::AgentChat { text, attach_frame: true }` and clears the
   buffers; with empty input it returns `None`.
5. **Stop swaps for Send when status is thinking/running.** A status
   transition into `Thinking` or `Running` flips the composer
   primary button; Stop emits `UiAction::AgentInterrupt(turn_id)`.
6. **Peer count parse + count cell.** `peers=3` in
   `AgentStatus.detail` updates `ShellState.peer_count`; the status
   bar formats `n peer(s)` accordingly.
7. **Revert lowering pin.** A `revert_to(turn_idx)` against a
   captured snapshot lowers to the expected `SetState` / `Show` /
   `SetCamera` sequence (typed commands, never `raw`).

The `frozen_stubs_unimplemented` test in
`crates/mili-viz-server/tests/acceptance.rs` (the M1 acceptance
gate's frozen-stub assertion) is updated in place: agent/interrupt
are now `ok=true` when a backend is plugged in, and remain
`ok=false` with a clear error when none is; `capture_frame` is now
implemented (the M6 milestone names the offscreen-renderer
production swap explicitly). The other five M1 acceptance-gate
tests are untouched.

`cargo test --workspace --exclude mili-py` green at end of milestone;
every prior M1–M5 + MVP-polish + Phase 6 M1–M3 + Phase 4 M7/M8/M9 +
Phase 5 M7/M8/M9 gating test unchanged and green
(`bug-tracker.md` VB-001 byte-stability discipline).

`cargo clippy --workspace --all-targets` clean.

## Out of scope

- **Real LLM backend.** The trait + `MockAgent` land; an Anthropic /
  local-model backend is a separate follow-up (Decision 94 — it
  belongs behind a Cargo feature with its own dep tree and config
  contract). Research notes:
  [`agent-local-llm.md`](agent-local-llm.md),
  [`agent-local-llm-posttraining.md`](agent-local-llm-posttraining.md).
- **Server-side wgpu offscreen renderer for `CaptureFrame`.**
  Decision 96 lights up the RPC with a placeholder PNG; a real
  server-side renderer is the production swap path and a separate
  milestone (it needs an offscreen wgpu adapter plugged into the
  server crate, which today has no GPU surface area).
- **Floating AI panel / multi-window.** The
  `griz_wgpu_wireframes/README.md` §"Tweaks" lists "AI panel
  position: Right dock (default) vs. floating window" as a future
  preference; M6 ships only the right-dock variant.
- **Timeline strip above the viewport.** The wireframes' "P3"
  secondary provenance surface (off by default) is deferred — the
  primary inline turn-boundary marker covers the v1 need.
- **Real per-command undo.** Per-turn revert is the
  [`client.md`](client.md) Decision and the wireframes' resolution;
  per-command undo is explicitly out of scope there.
