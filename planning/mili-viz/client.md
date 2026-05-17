# `mili-viz` — client wireframe & AI-first design

The `wgpu` + `egui` viewer. This pins what the window *looks like* and
how the headline feature — an AI assistant that can drive, see, and
debug a session — fits the existing split without becoming a second
mechanism.

Read `README.md` (the split, why `wgpu`+`egui`) and `scripting.md`
(server-authoritative, the Python peer client) first. This doc obeys
both.

## Design principle: the agent is a peer of the command vocabulary

`scripting.md` forbids a second scripting mechanism: the Python layer
"lowers to the exact proto the `egui` client emits." The AI assistant
obeys the *same rule*. Its entire capability surface is already-planned
proto, not a privileged subsystem:

- **Drive** → the same `Command` variants the GUI and Python client emit.
- **Observe** → the `Subscribe` / `StateDelta` stream, so the agent sees
  the human's manual rotations and state steps live.
- **See** → a `Snapshot` capability (the frame behind `s.snapshot()` in
  scripting.md), handed to a multimodal model.
- **Debug the data** → the `Query` RPC (the structured-data payoff:
  per-element values, ranges, NaN/Inf scans), *not* pixel-peeping.

Because every agent action is an ordinary `Command` producing an
ordinary `StateDelta`, the agent inherits server-authoritative sync for
free: `state 47; show sx` moves every attached GUI window, and the agent
observes human edits the same way the Python client does. No new state
owner.

## Decisions (resolved 2026-05-17)

1. **Server-hosted agent service.** The agent's reasoning loop runs in
   `mili-viz-server`, not the client. Rationale, and why this is *more*
   consistent with the README than a client-side loop:
   - **Colocated with the data.** The README's whole reason for the
     split is "keep the heavy side colocated with the data." An agent
     debugging a 50 GB run issues dozens of `Query`/`Snapshot` calls;
     running them next to the dataset on the HPC node — not round-trip
     to a laptop — is the same argument that justified the server.
   - **One API key, server-side.** Centralized at the login node;
     never shipped to every client install. Air-gapped sites point it
     at a local model in one place.
   - **The conversation is shared session state.** Like the camera, the
     agent transcript broadcasts to *all* clients: open the GUI on the
     workstation and a second GUI on a laptop and both see the same
     live conversation. This is the server-authoritative model already
     in `scripting.md`, extended to the chat.
   The chat panel in the `egui` client is therefore a **thin
   subscriber + input box**, not the agent.

2. **Fully autonomous by default.** The agent runs commands —
   view/state/result/query/snapshot — without per-step approval. The
   user interrupts, not pre-approves. This *requires* the provenance
   and barge-in machinery below; it is not optional polish given the
   agent shares an authoritative session a human is watching.

3. **Scripting window: subprocess + `attach()`.** The in-GUI Python
   editor runs the pure-Python `griz` package in a managed venv
   subprocess that `attach()`es to the running session (scripting.md
   connection model). No in-process interpreter — the Visit problem
   scripting.md exists to avoid. The agent's "run code" tool reuses
   this exact runner.

4. **LLM backend: Claude API first, behind a thin provider trait.**
   No strong preference was expressed; this is the pragmatic default.
   Tool-use + streaming + multimodal (for `Snapshot`) against the
   Anthropic API, with a minimal `LlmProvider` trait so an offline
   model can slot in for air-gapped clusters without touching the
   agent loop. Not provider-agnostic up front (premature).

## Wireframe

Mirrors griz's shape so muscle memory transfers (`reference/griz/Src/`
in parentheses), with the AI panel as a first-class citizen.

```
┌─ menu bar:  Control · Rendering · Picking · Results · Time · Plot · Help ─┐  (gui.c menubar)
│ ┌─ toolbar:  |◀  ◀  ▶  ▶|   stride[ 1 ]   ⏵ animate ⏹    view⟲ reset ───┐ │  (gui.c utility panel
├──────────────────┬──────────────────────────────────┬───────────────────┤   state-stepping)
│ LEFT DOCK        │                                    │  AI ASSISTANT     │
│  ▾ Runs/sessions │     3D RENDER VIEWPORT (wgpu)       │  (thin subscriber)│
│      d3samp6 ●   │                                    │                   │
│  ▾ Results       │   overlays: colormap legend,       │  ┌─ transcript ─┐ │
│      derived ▸   │   title, time/state, coord axes,   │  │ you: why does│ │
│      primal  ▸   │   bounding box                     │  │  state 47 …  │ │
│  ▾ Materials     │                                    │  │ ▸ ran: state │ │
│      [vis][col]  │   (server-authoritative: agent &   │  │   47; show sx│ │
│  ▾ Surfaces      │    human edits both land here)     │  │ ▸ queried sx │ │
│                  │                                    │  │   → [0,5.2e4]│ │
│  (gui.c Results  │   (draw.c render engine,           │  │ ▸ captured   │ │
│   menu + Material│    faces.c geometry)               │  │   frame      │ │
│   /Surface mgrs) │                                    │  │ assistant: … │ │
│                  │                                    │  └──────────────┘ │
│                  │                                    │  [📷 attach frame]│
│                  │                                    │  [ ask…      ][⏹]│
├──────────────────┴──────────────────────────────────┴───────────────────┤
│ BOTTOM TABS:  [ Command line ]   [ Scripting ]   [ Time-history plot ]    │
│   raw griz Layer-0   Python editor+Run   egui_plot (time_hist.c)          │
└───────────────────────────────────────────────────────────────────────────┘
 status bar:  attached <session-id>@host · proto v1 · pick: brick 4213       (scripting.md session file)
```

Panel-by-panel mapping from griz's ~321-command Motif UI:

| Legacy griz (Motif)                         | New client home |
|---------------------------------------------|-----------------|
| Results menus (derived/primal/time-indep)   | Left dock → Results tree |
| Material Manager, Surface Manager dialogs   | Left dock → Materials / Surfaces sections |
| Utility panel state stepping + animate      | Top toolbar |
| Render window + overlays                     | Center viewport (1:1) |
| Command line + history (`gui.c`)            | Bottom tab → Command line (Layer-0) |
| Time-history plot window (`time_hist.c`)    | Bottom tab → Time-history (`egui_plot`) |
| Picking modes / pick-class option menus     | Picking menu + status-bar readout |
| Session save/load                            | Control menu (uses scripting.md session file) |
| *(new)*                                      | Right dock → AI Assistant |

The ~321 commands collapse into the left dock + menus + the Layer-0
command-line tab; nothing from the inventory is dropped, it is
re-homed. Open question: which long-tail commands (e.g. `outvec`,
`refave`, traction visualization) get GUI affordances vs. live
command-line-only.

## AI Assistant panel

The headline feature. Thin client of a server-hosted agent.

**Transcript with inline tool calls.** Every tool call the agent makes
renders as a collapsed line in the transcript — `ran: state 47;
show sx`, `queried sx → [0, 5.2e4]`, `captured frame`. Non-negotiable:
the agent drives a session a human is watching, so what it did must be
legible without scrolling a log. Expanding a line shows the exact
`Command` / `Query` and its reply.

**Autonomy + barge-in (decision 2).** Fully autonomous, so the panel
*must* carry:
- A persistent **⏹ Stop** that cancels the in-flight agent turn (new
  proto: `Interrupt`). Always reachable.
- A **provenance journal**: agent-originated `StateDelta`s are tagged
  (`origin_client_id` already exists in the proto — the agent gets a
  stable id). The journal lists "agent changed: state → 47, result →
  sx" with a one-click **revert to before this turn**, implemented on
  the existing `NamedView`/command-journal machinery (server snapshots
  session state at each user turn boundary).
- Agent activity is visible to *all* clients (it is shared state), so a
  colleague attached from a laptop sees "agent is running" too.

**Vision is deliberate but agent-initiated.** Two paths: the user
clicks **📷 attach frame** to pin the current viewport to their next
message; and the agent can call `Snapshot` itself mid-turn when a
diagnosis needs it. Snapshots are server-side (cheap, next to the GPU),
encoded PNG/JPEG, handed to the multimodal model. Default to
agent-initiated-but-sparing to control token cost — the debugging
stance below makes that natural.

**Debugging stance: data-first, pixels-second.** The motivating use
case ("help debug issues about the simulation"). The strong capability
is not vision — it is that the agent calls the same `Query` layer the
analyst uses: scan a result for NaN/Inf across all states, find the
state where von Mises spikes, pull the per-element outliers, diff two
runs. The `Snapshot` *corroborates* ("yes, that's the red blob at the
top") but the diagnosis is structured data. The system prompt and tool
descriptions push query-first; this also keeps token cost down.

## Bottom tabs

- **Command line** — Layer-0 raw griz / `grizinit` stream, verbatim
  parity (`Command.raw` in the proto). Power users and migration; the
  scripting.md "Layer 0 ≡ raw stream" integration test covers it.
- **Scripting** — lightweight editor + Run + output pane. *Not* the
  primary scripting surface (that is an external IDE per the task);
  this is for quick in-context snippets. Runs via the
  subprocess+`attach()` runner (decision 3): a managed venv
  (detect/create, `pip install` the `griz` wheel), a child `python`
  that `attach()`es to *this* session via the session file, output
  streamed back to the pane. The agent's "run code" tool is the same
  runner — one code path, one venv.
- **Time-history plot** — `egui_plot`, porting `time_hist.c`'s X-Y
  series/glyph model. README already nominates `egui_plot` for v1.

## Proto impact (expands Phase 4 M1)

The command *vocabulary* is unchanged (still the griz set). The
server-hosted agent + vision add surface to `proto/mili_viz.proto`:

- **`AgentChat`** — client→server user turn. Server runs the loop;
  the agent's commands flow through the *existing* internal dispatch
  (so they broadcast as ordinary `StateDelta`s, tagged with the
  agent's `origin_client_id`).
- **Agent events as broadcast deltas** — assistant token deltas,
  tool-call begin/end, agent status (thinking/running/idle). Add a
  `DELTA_AGENT` kind so the conversation rides the existing `Subscribe`
  stream and every client mirrors it (consistent with camera being
  broadcast state). A late-joining client gets the transcript in the
  opening `DELTA_SNAPSHOT`.
- **`Snapshot`** — `(width, height, format) → encoded frame bytes`.
  Backs both the agent's vision tool and Python `s.snapshot()`; one
  capability, two callers.
- **`Interrupt`** — cancel the in-flight agent turn (barge-in).
- **Handshake capability flag** — `agent` in `HelloReply.capabilities`;
  a server built without an LLM backend (or air-gapped with none
  configured) advertises it absent and the client hides the panel
  rather than erroring. This is the scripting.md
  capability-negotiation pattern, reused.

These are additive to the M1 multi-client surface scripting.md already
expanded; they do not change the geometry/Flight path.

## Phasing (refines README Phase 5)

README Phase 5 M1–M5 stand. Inserted:

- **Phase 5 M3.5 — bottom tabs.** Command-line (Layer-0) and the
  subprocess scripting runner. Depends on the scripting.md Python
  client existing.
- **Phase 5 M6 — AI assistant.** After M3 controls + the M3.5 runner.
  Sub-steps: (a) server agent loop + `AgentChat`/`DELTA_AGENT` over
  the in-process transport; (b) `Snapshot` + multimodal; (c) provenance
  journal + `Interrupt`; (d) `LlmProvider` trait + offline backend.
  Server-side, so it tracks Phase 4, not blocked on the `wgpu`
  renderer.

## Open questions

- **Agent ↔ Flight geometry.** The agent reasons over `Query` (data)
  and `Snapshot` (pixels); it never needs raw vertex buffers. Confirm
  the agent tool surface deliberately excludes `GeometryRef`/Flight so
  it cannot foot-gun on huge buffers.
- **Provenance granularity.** Revert per *turn* (snapshot at user-turn
  boundary) is the proposal. Per-*command* undo is finer but needs a
  full inverse-command journal — probably not v1.
- **Long-tail command affordances.** Which of griz's ~321 commands get
  dock/menu GUI vs. command-line-only. Inventory exists; triage by
  usage is its own pass.
- **Multi-user conflict.** Two humans + an autonomous agent on one
  server-authoritative session: last-writer-wins is the current model
  (camera already works this way). Is an explicit "agent has the
  floor" lock ever wanted, or is barge-in enough? Lean: barge-in
  enough for v1.
- **Offline model bar.** What local model is good enough for the
  query-first debugging loop on an air-gapped cluster — affects how
  hard the `LlmProvider` boundary must work. A scoped, optional
  exploration of a *tiny* fine-tuned command-generation model (not
  the full agent) is sketched in `agent-local-llm.md` — non-priority,
  revisit before building.
```
