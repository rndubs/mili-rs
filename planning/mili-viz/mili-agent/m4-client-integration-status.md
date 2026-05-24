# M4 — FunctionGemma in the griz client: status

Date: 2026-05-24
Branch: `task4-baseline-measurement`
Commits: `6fc9a50` (initial wiring), plus uncommitted fixes for cycle
detection / synth-response cleanup.

## What we built

A `LlamaCppAgent` (`crates/mili-viz-server/src/llamacpp_agent.rs`)
that implements `AgentBackend` and connects the running griz client to
a locally-running `llama-server` hosting FunctionGemma-270M. Activated
by `--agent llamacpp` on the server binary. ~700 LOC + 23 unit tests,
all passing.

End-to-end signal path:

```
griz client UI (AI panel)
   → agent_chat RPC
      → LlamaCppAgent::run_turn
         → build FunctionGemma prompt (tools + system prompt + history)
         → POST http://localhost:8080/completion
         → parse tool calls (JSON + FunctionGemma text format)
         → ctx.dispatch(pb::command::Cmd)  ← broadcasts to client
         → synthesize tool response, loop
```

## What works

- Connection / wiring: server starts cleanly with the flag, broadcasts
  `CAP_AGENT`, client AI panel opens against it.
- Prompt building matches FunctionGemma's expected format (developer
  turn with tool declarations, message history, model turn priming).
- Tool-call parsing handles JSON (`{"name": "...", "arguments": {...}}`),
  FunctionGemma text format (`<start_function_call>call:name{...}<end>`),
  type coercion (string `"5"` → int 5; `"true"` → bool true), and
  dotted-name tolerance (`material.disable` → `material`).
- 11 typed griz tools map cleanly to `pb::command::Cmd`: `load`,
  `show`, `set_state`, `step`, `material`, `select`, `clrsel`,
  `colormap`, `named_view`, `close`, `griz_raw`.
- Unknown tools properly return `ok=false` + `unknown_tool` error_kind.
- Runaway-loop guards: `MAX_STEPS=4`, name-based cycle detection
  (window=4) that fires *before* dispatch.

## What's broken — the load-bearing limitation

**The agent has no idea whether the commands it dispatches actually
worked.** `ctx.dispatch()` returns a `u64` seq, not the griz response.
So `LlamaCppAgent` synthesizes generic `{"ok": true, "action_complete":
true}` responses regardless of what griz actually did.

Concrete symptoms:
- User: "show prin_stress1" → model emits `show {result:
  "prin_stress1"}` → dispatched → if it works, colormap renders; if it
  silently fails (typo, missing result, no active database), model
  thinks it succeeded and stops.
- Today's session: a single `show` call appeared in the UI as
  `▸ ran: show → ok` and **the mesh stayed gray** — the dispatch
  reached griz but produced no visible result, and there's no signal
  back to the model or the user about why.

This is the architectural gap. Without it, every other surface bug
(model retries unnecessarily, model can't self-correct, etc.) gets
papered over with heuristics that work some of the time.

## What's wired but limited

- **No streaming**: `post_completion` waits for the full llama-server
  response before emitting tokens. Acceptable for short turns; will
  feel laggy on multi-sentence final replies.
- **No vision**: `AgentChatRequest.image` is ignored by the agent
  even when the user clicks 📷 in the panel.
- **No mid-flight cancellation**: `Interrupt` RPC only takes effect
  between loop iterations, not during an in-flight HTTP request.
- **Tool registry hard-coded**: tools.json is `include_str!`-ed at
  compile time. Editing requires rebuild.
- **Single tool family**: only `view` / `iso` / `contour` / `legend`
  / `cutplane` / `query` / `snapshot` are unmapped (return
  unknown_tool error). 7 of 18 tools.

## Heuristic guards in place

These exist because the model is 270M params and prone to over-calling:

- `MAX_STEPS = 4` — hard cap on iteration count per user turn.
- Name-based cycle detection — if the same *tool name* appears twice
  in the last 4 iterations, stop. Catches show/clear-show oscillation,
  step-spam, etc. Trade-off: legitimate "disable material 2 and
  material 3" gets stopped after the first call.
- Synthesized responses are deliberately spartan
  (`{"ok":true,"action_complete":true}`) — earlier attempts to echo
  args back (`{"result": "x", "component": ""}`) triggered the model
  to "fix" the empty component by re-calling show with different args.

## What the user observes today

For simple, single-tool requests with correct argument names:
- Model emits one tool call, gets dispatched, says "ok," stops. ✓

For requests with typos or missing-on-server cases (e.g.
`princ_stress1` vs `prin_stress1`):
- Tool call dispatched, no visible effect, model says ok and stops. The
  UI gives the user no clue what went wrong.

For multi-step requests:
- Cycle detection often fires mid-task, leaving the user with a
  half-done state.

For conversational requests ("are we up?"):
- Model hallucinates ("I cannot assist with home appliances..."). This
  is a small-model capability issue, not an integration bug.

## Next moves, ranked by impact

1. **Plumb dispatch results back into `AgentTurnCtx`.** This is the
   architectural unblock. Today `Dispatcher` is `Fn(Cmd, &str) -> u64`.
   Make it `Fn(Cmd, &str) -> (u64, DispatchOutcome)` where
   `DispatchOutcome` carries success/error and (for read-style commands)
   structured result data. Then `LlamaCppAgent` can pass real responses
   to the model instead of synthesized ones, and the UI can render
   actual error reasons in the tool-end row. **One change unblocks
   four downstream limitations.**

2. **Surface tool args in the UI summary** (~5 lines): change
   `▸ ran: show → ok` to `▸ ran: show {result: "princ_stress1"} → ok`.
   Today there's no way to debug "why didn't it work" without server-side
   logs.

3. **Add server-side request/response logging** behind a flag (`--agent-log`).
   Today the only way to see what the model actually emitted is by
   reading llama-server stdout.

4. **Try a larger model.** FunctionGemma-270M is at the edge of
   capability for griz's tool surface. A 2-7B function-calling model
   (e.g. Qwen2.5-Coder-7B-Instruct) would dramatically improve the
   error rate and let us relax the heuristic guards. The llama-server
   integration doesn't care about model size — this is purely a model
   swap.

5. **Real cancellation propagation** via `tokio::select!` on the HTTP
   call. Today `Interrupt` is honored only between iterations.

6. **Mapping coverage for the remaining 7 tools** (`view`, `iso`,
   `contour`, `legend`, `cutplane`, `query`, `snapshot`). Some are
   trivial; `view` is the only complex one (oneof with 6 sub-ops).

7. **Vision support** (`CaptureFrame` integration). Architecturally
   straightforward; requires multi-part request to llama-server which
   FunctionGemma probably doesn't handle well anyway.

## Bottom line

The integration is **plumbed correctly** and the **infrastructure is
sound** — what's left isn't broken wiring, it's missing semantic
feedback. Until `ctx.dispatch()` can tell the agent what actually
happened, FunctionGemma is working blind, and no amount of prompt
tuning will reliably fix that. Item 1 above is the single most
valuable next thing to build.
