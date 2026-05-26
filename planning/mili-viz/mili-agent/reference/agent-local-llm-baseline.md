# `mili-viz` — local LLM v0 baseline plan

**Status: In progress — W4a/W4b implementation with response enrichment.**
Concrete next milestone under the exploratory umbrella of
`agent-local-llm.md` / `agent-local-llm-posttraining.md` /
`_posttraining-dataset.md`. W4a (harness) and W4b (driver) are
completed with response enrichment enhancements (Task 3) to signal
completion and prevent looping. Ready for baseline run. Tracked in
`status.md` § "Local LLM agent (exploratory)".

## Goal — one defensible number

Produce one number: **stock FunctionGemma-270M-it's L3 success rate on
a 50-scenario bootstrap eval set under a pinned config.** That number
is the baseline every later step is measured against. If it already
clears the bar, the post-training work in
`agent-local-llm-posttraining.md` is **moot for v1** — the *good*
outcome that `_posttraining-dataset.md` Stage 8 calls out.

Non-goals for v0:

- No fine-tune, no teacher rollouts, no DPO, no GRPO.
- No in-process Rust adapter (`_posttraining-dataset.md` §0
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
diff test** — the typed-tool analogue of `_posttraining-dataset.md`
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
  `_posttraining-dataset.md` Stage 4).

**Each tool has both an input and an output schema.** Output schemas
matter as much as input schemas for v0 — the model can only chain
tools (e.g. "find peak state, then frame it") if the tool response
carries the values it needs.

**Pinned per-tool response projections.** A naive serialization of
the proto `Snapshot` is catastrophic for context budget — `Snapshot.
loaded.state_times` is `repeated double` and at production scale
(~10k states) JSON-encodes to ~150 KB / ~40K tokens, **larger than
FunctionGemma's full context window**, and the LLM cannot use a
length-N float array anyway. The harness therefore projects every
tool's response through a tight, model-friendly shape before it
reaches the model:

| Tool | Projected response |
|---|---|
| `load` | `{ok, num_states, num_classes, classes: [str], state_time_range: [t_min, t_max], current_time, error?}` — **no** `state_times` array, **no** `db` path |
| `set_state` / `step` | `{ok, state, num_states, current_time, error?}` — single-state lookup, no array |
| `select` / `clrsel` | `{ok, selection: {class: range_str, ...}, error?}` — only non-empty entries |
| `show` | `{ok, result, component, range: [min, max], error?}` — `geometry` field dropped entirely (the LLM never sees `flight_ticket`) |
| `material` | `{ok, hidden_materials: [int], error?}` — list of *off* IDs only (usually shorter than the full map; "everything else is visible" is the default) |
| `view` / `named_view` / `colormap` / `legend` / `iso` / `contour` / `cutplane` | `{ok, error?}` — model calls `snapshot` if it wants camera/result back |
| `query` | `{ok, table: ..., error?}` — already result-bearing by design |
| `snapshot` | `{state, num_states, current_time, classes: [str], selection: {...}, result: {result, component, range}, hidden_materials: [int], camera: {azimuth, elevation, distance, focus: [fx, fy, fz]}}` — pruned `LoadedState` + `ResultState`, `state_times` and `flight_ticket` stripped, `agent` stripped |
| `griz_raw` | `{ok, output: str, error?}` |

**Harness invariants (pinned, unit-tested).** Three fields must
never reach the LLM under any code path:

1. `Snapshot.loaded.state_times` — unbounded `repeated double`;
   replace with `state_time_range` + `current_time`.
2. `GeometryRef.flight_ticket` — opaque `bytes` for the Arrow Flight
   client; useless to the LLM, and base64 encoding adds ~33% bloat.
   `GeometryRef` is dropped wholesale from projected responses.
3. `Snapshot.agent` — the agent's own transcript. Echoing it back
   into the agent's tool-response context is a self-echo trap:
   quadratic growth and likely model confusion. Stripped
   unconditionally.

Each invariant gets one `test_no_state_times_in_response` /
`test_no_flight_ticket_in_response` / `test_no_agent_in_response`
test in `python/mili-llm-bench/tests/test_harness.py` against a
fabricated raw `Snapshot` carrying all three fields.

Effect: `snapshot` projects to a few hundred bytes regardless of sim
size; `show`/`set_state`/etc. become stable-sized responses
independent of the fixture.

Outputs:

- `data/posttraining/grammar/tools.json` — pinned schema list (input
  + output, one entry per tool).
- A schema-derivation script that re-walks `mili_viz.proto` and a
  pinned `python/mili-llm-bench/tests/test_schemas.py` honest-diff
  test — drift fails CI and forces a deliberate regenerate, matching
  `_posttraining-dataset.md` Stage 1's discipline.

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
- Closed post-condition kinds (mirror `_posttraining-dataset.md`
  Stage 4): `state_index`, `selection_set`, `active_result`,
  `result_range`, `materials_visible`, `camera_named_view`,
  `query_value`.
- Fixture-fact grounding: real material ids / class names / state
  counts pulled from the existing parity suite, not invented
  (`_posttraining-dataset.md` Stage 3 discipline).

Output: `data/posttraining/eval/bootstrap.jsonl`.

Honest scope: 50 is small. It is enough to detect "stock model is
broken on the chat template" vs. "stock model works on simple, fails
on compound", which is the v0 discrimination we need.

### W3 — Verifier (single source of truth for scoring)

The `_posttraining-dataset.md` Stage 4 L0–L3 verifier, implemented as
one Python module reused by v0 *and* by the future training pipeline.
Refold of the two-column table from `_posttraining-dataset.md` Stage 4:

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
  the L2-carries-more-weight load `_posttraining-dataset.md` Stage 4
  calls out)
- `wrong_final_state`, `wrong_selection`, `wrong_result`,
  `wrong_range`, `wrong_materials` (L3)
- `step_cap_hit`, `token_cap_hit`, `timeout` (driver-level)

Without this taxonomy, "the v0 baseline got 12 / 50" is not
actionable — we cannot tell whether to invest in better prompts,
fine-tuning, more macros, or richer tool responses.

### W4 — Agent harness + eval driver

W4 splits into a **factored harness** (W4a) reused by three
consumers — the eval driver here, the future teacher-rollout loop
(`_posttraining-dataset.md` Stage 5), and the live production
`AgentChat` handler (`client.md` decision 4 + the frozen
`AgentChat`/`Interrupt` RPCs already in `mili_viz.proto`) — and the
**v0-specific eval driver** (W4b) that wraps W4a, calls the verifier,
and writes rollouts.

#### W4a — Harness (the shared core)

Provider-agnostic, session-agnostic. Owns the "tool call JSON →
real session mutation → projected response JSON" translation. One
module: `python/mili-llm-bench/src/mili_llm_bench/harness.py`.

Components:

- **Tool registry** — declarative `dict[name, Tool(input_schema,
  output_schema, dispatch_fn, response_projection)]`. One source of
  truth; W1's `tools.json` is its serialized form.
- **Input validator** — `jsonschema` check before dispatch. A miss
  returns a structured `{ok: false, error: "...", error_kind:
  "schema_mismatch"}` response to the model, never raises.
- **Dispatcher** — typed-Command tools lower to the *existing pygriz
  typed helper* (`material → s.materials.enable/disable`,
  `set_state → s.state = n`, `show → s.show(...)`); `griz_raw`
  lowers to `s.command(raw)`; `query`/`snapshot` use the pygriz read
  paths. **Reuses the code path a human notebook user takes** — the
  alignment `agent-local-llm.md` "Surface choice" calls out.
- **Error wrapper** — pygriz exception or `CommandReply.ok == false`
  → `{ok: false, error: str, error_kind: <enum>}`. The model *sees*
  the error and can recover; a raw exception kills the rollout.
- **Response projection** — after every typed call, read a fresh
  snapshot and run each tool's pinned `response_projection` (the W1
  table) to build the model-facing dict. **The harness invariants
  pinned in W1 — no `state_times`, no `flight_ticket`, no
  `Snapshot.agent` — are enforced here**.
  - **ENHANCEMENT (Task 3 — Response Enrichment):** All tool
    responses now include `action_complete: true` for instant
    operations (load, show, step, select, material, etc.) to signal
    successful completion. State-changing tools like `set_state`
    include both `requested_state` and actual `state` for
    verification. Enhanced system prompt guides the model to
    recognize completion signals and avoid repeating identical tool
    calls. Commit `0baf291`.
- **Per-turn budget enforcement** — `step_cap`, `max_new_tokens`,
  wall-clock `timeout`. Lives in the harness, not the driver, so
  all three consumers get the same protections.

`error_kind` enum (closed; mirrors the verifier failure-mode
taxonomy in W3):

```
"parse_error"            # LLM emitted unparseable tool-call shape
"unknown_tool"           # name not in registry
"schema_mismatch"        # arguments fail jsonschema
"dispatch_error"         # pygriz call raised / server CommandReply.ok=false
"nonexistent_material"   # arg type valid but material id not in fixture
"nonexistent_class"      # ditto for class_name
"nonexistent_result"     # ditto for result name
"state_out_of_range"     # ditto for state index
"step_cap_hit"           # driver-level
"token_cap_hit"          # driver-level
"timeout"                # driver-level
```

The harness emits `parse_error` / `unknown_tool` / `schema_mismatch`
/ `dispatch_error` itself; argument-level `nonexistent_*` /
`state_out_of_range` are classified by the dispatcher from pygriz's
returned error string (best-effort pattern match — when the
dispatcher can't classify, it falls back to `dispatch_error`). The
driver emits `step_cap_hit` / `token_cap_hit` / `timeout`.

Public surface:

```python
def run_turn(
    provider: LlmProvider,
    session: pygriz.Session,
    messages: list[dict],   # mutated in place; appended to
    tools: list[dict],
    *,
    step_index: int,
    max_new_tokens: int = 256,
    temperature: float = 0.0,
    seed: int = 0,
    timeout_s: float = 60.0,
) -> TurnResult: ...

@dataclass
class TurnResult:
    kind: Literal["tool_calls", "final_text", "error"]
    tool_calls: list[ExecutedCall]      # 0..N: see "N tool calls" below
    final_text: str | None              # populated when kind == "final_text"
    error_kind: str | None              # populated when kind == "error"
    tokens_used: int
    wall_ms: int

@dataclass
class ExecutedCall:
    name: str
    arguments: dict
    response: dict                       # projected, harness-invariant-safe
    error_kind: str | None               # None on L2+ success
    dispatch_ms: int
```

A driver calls `run_turn` in a loop, deciding when to stop. The
harness owns per-turn semantics.

**0 / 1 / N tool calls per turn.** FunctionGemma emits at most one
per turn; OpenAI/Anthropic can emit N. The harness dispatches all
N in their declared order in a single turn and returns one
`ExecutedCall` per slot, each with its own projected response. The
driver appends one `assistant` message (with all N `tool_calls`)
and N `tool` messages (one per call, in order) before the next
`run_turn`. Simpler than forcing one-call-per-turn at the prompt
level; matches the OpenAI / Anthropic tool-use convention; reduces
to one slot for FunctionGemma.

**Parse-error recovery (option (b), pinned).** If `provider.generate`
returns text the harness cannot normalize to a canonical
`{name, arguments}` tool call, the harness emits one `ExecutedCall`
with `name="<parse_error>"`, `arguments={}`, and `response=
{ok: false, error: "...", error_kind: "parse_error"}`. The driver
appends that as a `tool` message and re-enters `run_turn` — the
model sees the error and can self-correct. This is the same shape
as L2 dispatch errors; gives the model a recovery loop; matches
what trained tool-use models expect. Pinned alternative ((a)
silent retry of `generate`, (c) terminate) explicitly rejected:
(a) doesn't teach the model anything; (c) kills rollouts on
recoverable failures.

**Replay mode (pinned, ships in v0).** `harness.run_turn` accepts a
`ReplayLlmProvider` that yields pre-recorded provider outputs from
a rollouts file instead of calling generate. Two uses:

1. **Deterministic regression of the verifier.** Re-grade a stored
   `rollouts.jsonl` under a new post-condition or a new
   failure-mode taxonomy without re-running the LLM.
2. **Dataset validation.** Round-trip a training set through the
   harness to confirm every recorded tool call still parses,
   dispatches, and produces the recorded response — catches
   schema drift or fixture-fact drift before it pollutes a fine-tune.

`ReplayLlmProvider` is one file in `providers/replay.py`. Same
`LlmProvider` Protocol; no driver change. Counts as part of W5's
provider seam, listed here for completeness.

**Conversation truncation.** v0 stretches the context window as
far as the model allows (Gemma 3 270M ≈ 32K tokens) and does not
truncate. With `step_cap=8`, `max_new_tokens=256`, the projected
responses in the W1 table, and ~3–5K tokens of tools-list overhead,
typical rollouts stay well under 16K tokens. Truncation becomes
relevant when (a) traces grow beyond `step_cap=8`, or (b) we add a
tool whose response cannot be bounded. Not v0 work; tracked as
an open question for after the baseline run.

#### W4b — Eval driver

The v0-specific loop on top of W4a. One module:
`python/mili-llm-bench/src/mili_llm_bench/driver.py`.

```
for scenario in scenarios:
    session = pygriz.launch()
    session.open(scenario.fixture)
    messages = [
        {"role": "developer", "content": <pinned system prompt>},
        {"role": "user", "content": scenario.instruction},
    ]
    step_index = 0
    while step_index < step_cap:
        turn = harness.run_turn(provider, session, messages, tools,
                                step_index=step_index, ...)
        if turn.kind == "final_text":
            break
        if turn.kind == "error" and turn.error_kind in {"timeout"}:
            break
        # tool_calls — N executed calls, all already projected + safe
        messages.append({"role": "assistant",
                         "tool_calls": [...]})
        for ec in turn.tool_calls:
            messages.append({"role": "tool", "name": ec.name,
                             "content": json.dumps(ec.response)})
        step_index += 1
    else:
        # step_cap_hit
        ...
    result = verifier.verify(messages, scenario.postcondition)
    write_rollout(scenario, messages, result)
    session.close()
```

Caps and determinism (pinned):

- `step_cap = 8` (most v0 scenarios are 1–3 turns; the cap catches
  loops).
- `max_new_tokens` per generate call: 256.
- `temperature = 0`, `seed = 0` — eval must be deterministic.
- Per-turn wall timeout: 60 s.

Pure-logic tests (no LLM, no GPU) via a `MockLlmProvider` that
emits a scripted tool-call sequence. The same Mock is the test
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
  report and as the future teacher (`_posttraining-dataset.md` Stage
  5). Standard `tool_use` / `tool_result` blocks.
- `MockLlmProvider` — scripted, deterministic, for tests.
- `ReplayLlmProvider` — yields pre-recorded outputs from a
  `rollouts.jsonl`, for re-grading a stored run under a new verifier
  or for dataset validation (W4a "Replay mode").

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
  as `_posttraining-dataset.md` §1 (so v0 rollouts can be reused as
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
    harness.py        # W4a: run_turn, tool registry, dispatch, projection, error_kind enum
    driver.py         # W4b: eval-specific loop on top of harness, caps, rollout writer
    providers/
      __init__.py
      base.py         # LlmProvider Protocol
      functiongemma.py
      anthropic.py
      mock.py
      replay.py       # W4a/W5: ReplayLlmProvider (re-grade + dataset validation)
    cli.py            # W6: `mili-llm-bench {derive-schemas,run,replay}`
  tests/
    test_schemas.py   # W1: honest-diff vs proto
    test_verifier.py  # W3: L0..L3 tiers + failure_mode taxonomy
    test_harness.py   # W4a: invariants (no state_times / flight_ticket / agent), N-tool-calls, parse-error feedback, replay round-trip
    test_driver.py    # W4b: MockLlmProvider; no LLM, no GPU required

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
2. The harness invariants — no `state_times`, no `flight_ticket`,
   no `Snapshot.agent` ever reach the LLM — are pinned by the
   three `test_no_*_in_response` unit tests against a fabricated
   raw `Snapshot` carrying all three fields.
3. `mili-llm-bench run --provider mock --scenarios bootstrap.jsonl`
   completes deterministically end-to-end on a laptop with no GPU
   and no live LLM. (This is the always-on test path; everything
   below this point is skip-on-absent.)
4. `mili-llm-bench replay --rollouts <path>` re-grades a stored
   `rollouts.jsonl` against the current verifier deterministically
   (round-trip identity on an unchanged verifier; a deliberate
   verifier change shifts at least one row's `max_tier`).
5. `mili-llm-bench run --provider functiongemma` completes in <10
   min on a developer laptop and writes a valid `summary.json` and
   `report.md`.
6. `summary.json` carries non-trivial values for every entry in the
   failure-mode taxonomy (the eval set is balanced enough to exercise
   them) — so "we don't know which failure mode dominates" is
   structurally impossible.
7. `status.md` § "Local LLM agent (exploratory)" rows W1–W6 (with
   W4 split into W4a/W4b) all flipped to ✅ with the gating tests
   named.

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
  Proceed to `_posttraining-dataset.md` Stage 5 (teacher rollouts)
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

- Tool-response richness vs. context bloat: the W1 projection table
  pins the *fields* per response and the harness invariants pin what
  is never echoed, but the right number of fields per turn for a
  270M model is empirical. Settled by the v0 run + the
  tokens-per-rollout breakdown the report includes.
- Should `query`/`snapshot` be **two tools** or a single
  `read(kind=...)` tool? Two tools is closer to FunctionGemma's
  one-purpose-per-tool convention; settle in W1 before pinning
  `tools.json`.
- When the `griz_raw` fallback is used, do we count it as a "miss"
  for the typed-tool benchmark or as a fair pass? v0 treats it as a
  fair pass; the report includes a separate "raw-fallback rate"
  metric so the question is visible.
- Whether to also run FunctionGemma under llama.cpp + GBNF
  grammar-constrained decoding on the `griz_raw` argument (as
  `agent-local-llm.md` Decision 3 envisions). Out of scope for v0
  unless the HF-transformers path is too slow on the dev laptop.
- Conversation truncation: not v0 work (the W4a §"Conversation
  truncation" paragraph stretches the window instead). Becomes
  relevant when traces exceed `step_cap=8` or a tool's response
  cannot be bounded.
- Dispatcher → `error_kind` classification confidence: the
  argument-level `nonexistent_material` / `nonexistent_class` /
  `nonexistent_result` / `state_out_of_range` labels are inferred
  from pygriz error strings (best-effort pattern match). If
  misclassification rate is high in the v0 run, promote a
  structured error path in pygriz / `mili-viz-server` —
  cross-cutting with `client.md`'s provenance journal.
