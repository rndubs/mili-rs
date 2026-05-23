# `mili-viz` — local LLM v0 baseline plan

**Status: drafted, not yet started.** Concrete next milestone under
the exploratory umbrella of `agent-local-llm.md` /
`agent-local-llm-posttraining.md` / `posttraining-dataset.md`. Read
`agent-local-llm.md` "Surface choice" first — this doc operationalizes
that decision (typed-`Command` JSON tool calls) into a runnable v0.
Tracked in `status.md` § "Local LLM agent (exploratory)".

## Goal — one defensible number

Produce one number: **stock FunctionGemma-270M-it's L3 success rate on
a 50-scenario bootstrap eval set under a pinned config.** That number
is the baseline every later step is measured against. If it already
clears the bar, the post-training work in
`agent-local-llm-posttraining.md` is **moot for v1** — the *good*
outcome that `posttraining-dataset.md` Stage 8 calls out.

Non-goals for v0:

- No fine-tune, no teacher rollouts, no DPO, no GRPO.
- No in-process Rust adapter (`posttraining-dataset.md` §0
  `GrizSession`) — v0 drives the existing `mili-viz-server` via
  pygriz, which works *today*.
- No GUI pixel snapshots in the scoring path (`agent-local-llm.md`
  "Surface choice" closed this out — all post-conditions in the closed
  kind set are checkable from `snapshot`/`Query`).
- No analysis-macro tools (`query_extreme`, `scan_states`, …); typed
  Commands + `query`/`snapshot` + `griz_raw` only. Macros only earn a
  slot after v0 failure modes justify them
  (`agent-local-llm.md` open Q on macro inventory).

## Workstreams (build order)

### W1 — Tool-schema artifact (no LLM, no server)

JSON Schemas for each tool the model is shown, **derived from the
frozen `mili_viz.proto` `Command` oneof, pinned, and kept honest by a
diff test** — the typed-tool analogue of `posttraining-dataset.md`
Stage 1's grammar artifact.

Inventory (≈18 tools):

- **One per typed `Command` oneof variant** (15 — from
  `crates/mili-viz-proto/proto/mili_viz.proto`): `load`, `close`,
  `set_state`, `step`, `select`, `clrsel`, `show`, `view`, `iso`,
  `contour`, `material`, `cutplane`, `colormap`, `legend`,
  `named_view`. (`render` excluded from v0 — it is offscreen capture,
  not session manipulation.)
- **Read tools** (2): `query` (the proto `Query` RPC) and `snapshot`
  (a one-shot `Subscribe` opening `DELTA_SNAPSHOT` projection — what
  pygriz's `_snapshot()` already does).
- **Fallback** (1): `griz_raw(line: str)` — the long-tail escape
  hatch; lowers to `Command{raw}`. Its argument is graded against the
  Stage-1 grammar artifact at L0/L1 (verifier two-column table in
  `posttraining-dataset.md` Stage 4).

**Each tool has both an input and an output schema.** Output schemas
matter as much as input schemas for v0 — the model can only chain
tools (e.g. "find peak state, then frame it") if the tool response
carries the values it needs.

Recommended tool-response shape, pinned per-tool:

| Tool | Response (minimum) |
|---|---|
| `load` | `{ok, num_states, classes: [str], error?}` |
| `set_state`/`step` | `{ok, state, num_states, error?}` |
| `select`/`clrsel` | `{ok, selected: {class: [ids]}, error?}` |
| `show` | `{ok, result, component, range: [min,max], error?}` |
| `material` | `{ok, materials_visible: {id: bool}, error?}` |
| `view`/`named_view`/`colormap`/`legend`/`iso`/`contour`/`cutplane` | `{ok, error?}` (state read back via `snapshot` next turn) |
| `query` | `{ok, table: {...}, error?}` (mirror of proto `Query` reply) |
| `snapshot` | `{ok, state, selection, result, materials, camera}` |
| `griz_raw` | `{ok, output: str, error?}` |

Outputs:

- `data/posttraining/grammar/tools.json` — pinned schema list (input
  + output, one entry per tool).
- A schema-derivation script that re-walks `mili_viz.proto` and a
  pinned `python/mili-llm-bench/tests/test_schemas.py` honest-diff
  test — drift fails CI and forces a deliberate regenerate, matching
  `posttraining-dataset.md` Stage 1's discipline.

No interface dependency — runs entirely off the proto.

### W2 — Bootstrap eval scenarios (no LLM, no server)

50 hand-authored scenarios covering ~10 intents × 2 fixtures.

- Fixtures: `d3samp6`, `cylinder` (already in
  `reference/mili/test/xmilics/`, used by parity tests).
- Intent slice (v0): `load`, `set_state`/`step`, `select`, `clrsel`,
  `show` (primal), `show` (derived — pick one from the M5 set, e.g.
  `eff_stress`), `material` enable/disable, `view reset`, `colormap`,
  one *two-step* compound (e.g. "disable material 3 and show
  `eff_stress`") to stress multi-turn chaining.
- Each scenario:
  ```json
  {
    "id": "bs-001",
    "fixture": "d3samp6",
    "intent_id": "show-derived",
    "instruction": "color the mesh by effective stress",
    "postcondition": {
      "kind": "active_result",
      "expect": {"result": "eff_stress"}
    }
  }
  ```
- Closed post-condition kinds (mirror `posttraining-dataset.md`
  Stage 4): `state_index`, `selection_set`, `active_result`,
  `result_range`, `materials_visible`, `camera_named_view`,
  `query_value`.
- Fixture-fact grounding: real material ids / class names / state
  counts pulled from the existing parity suite, not invented
  (`posttraining-dataset.md` Stage 3 discipline).

Output: `data/posttraining/eval/bootstrap.jsonl`.

Honest scope: 50 is small. It is enough to detect "stock model is
broken on the chat template" vs. "stock model works on simple, fails
on compound", which is the v0 discrimination we need.

### W3 — Verifier (single source of truth for scoring)

The `posttraining-dataset.md` Stage 4 L0–L3 verifier, implemented as
one Python module reused by v0 *and* by the future training pipeline.
Refold of the two-column table from `posttraining-dataset.md` Stage 4:

| Tier | Typed tool call | `griz_raw` |
|---|---|---|
| L0 | output parses as `{name, arguments}` | inner `line` ∈ Stage-1 grammar |
| L1 | `name` known **and** `arguments` matches input schema | `parse_command` accepts |
| L2 | dispatch returns `ok=true` | raw runs without error |
| L3 | post-condition met (snapshot/query equals expected) | same |

**Failure-mode taxonomy** — emit alongside `max_tier` and `reward`,
not in place of them. Closed set:

- `parse_error` (L0)
- `unknown_tool`, `schema_mismatch` (L1)
- `dispatch_error`, `nonexistent_material`, `nonexistent_class`,
  `nonexistent_result`, `state_out_of_range` (L2 — argument-level
  semantic failures the schema check cannot catch; this is exactly
  the L2-carries-more-weight load `posttraining-dataset.md` Stage 4
  calls out)
- `wrong_final_state`, `wrong_selection`, `wrong_result`,
  `wrong_range`, `wrong_materials` (L3)
- `step_cap_hit`, `token_cap_hit`, `timeout` (driver-level)

Without this taxonomy, "the v0 baseline got 12 / 50" is not
actionable — we cannot tell whether to invest in better prompts,
fine-tuning, more macros, or richer tool responses.

### W4 — Driver loop (multi-turn agent harness)

Pure Python, on top of pygriz. One file. The same code is reused for
later teacher rollouts and for the eval harness — it is the "driver
loop" Stage 5 of `posttraining-dataset.md` implicitly requires.

```
init session (pygriz.launch() → Session)
load fixture
build initial messages: [
  {"role": "developer", "content": <pinned system prompt>},
  {"role": "user", "content": scenario.instruction}
]
loop:
  emit = provider.generate(messages, tools=tools.json)
  if emit is a tool_call:
    response = dispatch(tool_call, session)   # → tool-response JSON
    messages.append({"role": "assistant", "tool_calls": [emit]})
    messages.append({"role": "tool", "name": emit.name,
                     "content": json.dumps(response)})
    if step_count >= step_cap: break with step_cap_hit
  else:
    final_assistant_message = emit; break
verify(messages, postcondition) → {max_tier, failure_mode, reward}
session.reset()  # for next scenario
```

Pinned dispatch table:

- Each typed-Command tool lowers to the *existing pygriz typed
  helper* (e.g. `material → s.materials.enable/disable`,
  `set_state → s.state = n`, `show → s.show(...)`). **Reuses
  the same code path a human notebook user takes** — the alignment
  `agent-local-llm.md` "Surface choice" calls out.
- `griz_raw` lowers to `s.command(raw)`.
- `query` / `snapshot` use pygriz's existing `_snapshot()` /
  forthcoming `s.query()` (Phase 6 M5; until M5 lands, `query` is a
  stub that returns `{ok: false, error: "query unimplemented"}` —
  that itself is a measurement, not a blocker).
- After every typed call, the dispatcher reads back a fresh snapshot
  to populate the response fields (`range`, `materials_visible`,
  `state`, etc.).

Caps and determinism:

- `step_cap = 8` (most v0 scenarios are 1–3 turns; the cap catches
  loops).
- `max_new_tokens` per generate call: 256.
- `temperature = 0`, `seed = 0` — eval must be deterministic.
- Per-turn wall timeout: 60 s.

Pure-logic tests (no LLM, no GPU) via a `MockLlmProvider` that
replays a scripted tool-call sequence. The same Mock is the test
harness for `verifier.py` and for future training-data validators.

### W5 — Inference provider seam

```python
class LlmProvider(Protocol):
    def generate(
        self,
        messages: list[dict],
        tools: list[dict],
        *,
        temperature: float,
        max_new_tokens: int,
        seed: int,
    ) -> ToolCallOrText: ...
```

v0 implementations:

- `FunctionGemmaProvider` — HF `transformers`, the documented path
  in the FunctionGemma model card (`processor.apply_chat_template(
  tools=…)` → `model.generate(...)` → parse the
  `<start_function_call>…<end_function_call>` block). CPU OK; GPU if
  available. Same seam as the eventual local-runtime decision in
  `agent-local-llm.md` Decision 2 — Candle/llama-cpp swap is later.
- `AnthropicProvider` — for the *frontier baseline* line in the
  report and as the future teacher (`posttraining-dataset.md` Stage
  5). Standard `tool_use` / `tool_result` blocks.
- `MockLlmProvider` — scripted, deterministic, for tests.

Future: `VLLMProvider`, `LlamaCppProvider`, `CandleProvider` — swap
without touching the driver loop.

### W6 — Bootstrap run + report

```
mili-llm-bench run \
  --provider functiongemma \
  --scenarios data/posttraining/eval/bootstrap.jsonl \
  --tools data/posttraining/grammar/tools.json \
  --out data/posttraining/runs/<timestamp>-fg-<config-hash>/
```

Outputs in the run dir:

- `config.yaml` — pinned (model id, temp, seed, step_cap, prompt
  hash, tools.json hash, scenarios.jsonl hash). **Without
  config.yaml, the number is unfalsifiable.**
- `rollouts.jsonl` — one canonical record per scenario, same shape
  as `posttraining-dataset.md` §1 (so v0 rollouts can be reused as
  training data later if they happen to pass L3).
- `summary.json` — counts by `max_tier`, counts by `failure_mode`,
  mean turns to completion, total wall time, L3 pass-rate.
- `report.md` — human-readable summary; the publishable number.

Comparison baselines to run alongside (each is one `bench run`
invocation with a different `--provider`):

1. **FunctionGemma-270M-it, no fine-tune** — the v0 baseline. The
   number.
2. **Claude (Anthropic API)** — frontier ceiling; tells us how
   much headroom there is.
3. *(Optional)* **Random tool-call provider** — floor; sanity check
   that L3 isn't accidentally easy.

## Repo layout

```
python/mili-llm-bench/                 # new package
  pyproject.toml                       # depends on pygriz, transformers, anthropic, jsonschema
  src/mili_llm_bench/
    __init__.py
    schemas.py        # W1: derive tools.json from proto, load+validate
    scenarios.py      # W2: load bootstrap.jsonl, render prompts
    verifier.py       # W3: L0..L3 + failure_mode taxonomy
    driver.py         # W4: multi-turn loop, dispatch table, caps
    providers/
      __init__.py
      base.py         # LlmProvider Protocol
      functiongemma.py
      anthropic.py
      mock.py
    cli.py            # W6: `mili-llm-bench {derive-schemas,run}`
  tests/
    test_schemas.py   # honest-diff vs proto
    test_verifier.py  # L0..L3 tiers + failure_mode taxonomy
    test_driver.py    # MockLlmProvider; no LLM, no GPU required

data/posttraining/                     # gitignored except generators/pinned
  grammar/
    tools.json                         # pinned, regenerated by `derive-schemas`
    griz.gbnf                          # Stage-1 artifact (for griz_raw constraint)
  eval/
    bootstrap.jsonl                    # checked in (50 scenarios is small)
  runs/<timestamp>-<provider>-<hash>/  # gitignored
```

`pygriz` already depends on the running `mili-viz-server`; the bench
package adds `transformers` and `anthropic`. No new Rust crate is
required for v0.

## Acceptance gate

v0 ships when:

1. `mili-llm-bench derive-schemas` regenerates `tools.json` from
   `mili_viz.proto` and the honest-diff test passes.
2. `mili-llm-bench run --provider mock --scenarios bootstrap.jsonl`
   completes deterministically end-to-end on a laptop with no GPU
   and no live LLM. (This is the always-on test path; everything
   below this point is skip-on-absent.)
3. `mili-llm-bench run --provider functiongemma` completes in <10
   min on a developer laptop and writes a valid `summary.json` and
   `report.md`.
4. `summary.json` carries non-trivial values for every entry in the
   failure-mode taxonomy (the eval set is balanced enough to exercise
   them) — so "we don't know which failure mode dominates" is
   structurally impossible.
5. `status.md` § "Local LLM agent (exploratory)" rows W1–W6 all
   flipped to ✅ with the gating tests named.

## What v0 explicitly does *not* do

- Define a passing score. The v0 number *is* the report; whether it
  is "good enough" is a follow-on decision that needs the failure-mode
  breakdown in hand.
- Touch the Phase 4/5/6 critical path. Server `mili_viz.proto` is
  read-only; pygriz is read-only; no new Rust crate. This is the
  same `phase-4-m1.md` Decision 6 capability-gated discipline — the
  agent contract exists, the impl is off the critical path.
- Commit to a model. FunctionGemma is the v0 choice because of its
  documented function-calling chat template; the `LlmProvider` seam
  is what lets us re-baseline against Qwen2.5-Coder / Llama-3.2 /
  Gemma-class without rewriting the driver.

## After v0

Decision tree once the baseline number is in hand:

- **L3 pass-rate ≥ target on bootstrap eval** → declare the v1
  pre-experiment passed; the post-training pipeline is moot for v1.
  Wire the bench as a regression test for the agent layer. Stop.
- **L3 pass-rate inadequate, but L0/L1 mostly green** → the model
  understands the schema; the gap is intent/argument grounding.
  Proceed to `posttraining-dataset.md` Stage 5 (teacher rollouts)
  with the dispatch + verifier already built.
- **L0/L1 mostly red** → the chat template is wrong for the model,
  or schemas are too rich. Try a code-specialist base
  (Qwen2.5-Coder 1.5B/3B per `agent-local-llm.md` Decision 5) before
  spending teacher tokens.
- **L3 green on simple, red on compound** → the tool-response shape
  is not feeding enough state back; redesign per-tool responses or
  add the first analysis macro (`query_extreme`). Same data set;
  no retrain needed yet.

Each of these branches reuses the v0 plumbing — the bench, the
verifier, the schemas, the scenarios. None of v0 is throwaway.

## Open questions (carried, not resolved here)

- Tool-response richness vs. context bloat: each `snapshot` projection
  costs tokens. How many fields per tool response is the sweet spot
  for a 270M model? Empirical, settled by the v0 run.
- Should `query`/`snapshot` be **two tools** or a single
  `read(kind=...)` tool? Two tools is closer to FunctionGemma's
  one-purpose-per-tool convention; settle in W1 before pinning.
- When the `griz_raw` fallback is used, do we count it as a "miss"
  for the typed-tool benchmark or as a fair pass? v0 treats it as a
  fair pass; the report includes a separate "raw-fallback rate"
  metric so the question is visible.
- Whether to also run FunctionGemma under llama.cpp + GBNF
  grammar-constrained decoding on the `griz_raw` argument (as
  `agent-local-llm.md` Decision 3 envisions). Out of scope for v0
  unless the HF-transformers path is too slow on the dev laptop.
