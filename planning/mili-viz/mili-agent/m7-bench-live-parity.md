# M7 — close the bench-vs-live parity gap

**Status (2026-05-25):** Plan only. M6 wired the v1 SFT GGUF end-to-end
into the griz client AI panel ([m6-client-integration-v1.md](m6-client-integration-v1.md)).
Live smoke-test against `bar71.pltA` immediately exposed the headline
problem: **the rev-22 95.06% L3 bench number is not a faithful predictor
of live UX**. The model runs to `max_tokens=256` on every generation,
emits one well-formed `<start_function_call>` envelope followed by ~17
unpaired `call:X{…}<end_function_call>` fragments, and on the next step
follows up a correct `set_state(81)` with a destructive `show("81")`
that clobbers the prior result binding.

This is not a wiring bug — the M6 message-shape fixes (`role:
"developer"`, structured `tool_calls`, `tool_call_id`) all match the
training records — it is a **bench / training-data / corpus**
mismatch that the existing pipeline cannot detect.

---

## What this milestone delivers

After M7, the live griz panel behaves the way the bench score implies:
the model dispatches the correct tool call(s), emits one short
acknowledgment, and **stops**, without spurious follow-up calls that
clobber valid state. The bench score, when run after M7, is no longer
inflated by post-success runaway emissions.

Specifically, M7 lands:

1. **Terminating-assistant-text in every training record.** Re-render
   `train.jsonl` / `val.jsonl` / `heldout.jsonl` to end each rollout
   with `{"role": "assistant", "content": "<short ack>"}` — the loss
   mask includes this region (it falls inside the model-turn span per
   [`preflight-4-loss-mask.md`](../../../data/posttraining/sft/preflight-4-loss-mask.md)
   step 1), so the model receives positive training signal for
   *terminating after success*.

2. **Verifier tier-4 / clean-termination check.** Add a new
   `wrong_termination` failure mode in `verifier.py`. A rollout that
   reaches L3 on tool-call grading but fails to end on a content-only
   assistant message scores L2 max, not L3. This **forces the
   re-rendered corpus to actually contain clean terminations** for the
   bench number to hold, and exposes the bench-live gap in metrics
   going forward.

3. **Driver loop no longer auto-terminates on postcondition oracle.**
   Remove or gate the `verifier.verify(messages, postcondition).max_tier == 3`
   early-exit at `driver.py:282`. The bench should grade what the model
   actually does to completion, not what it does up until the oracle is
   satisfied. Optional: keep behind a `--allow-oracle-early-exit` flag
   for back-compat with prior reports.

4. **Server-side defensive: preserve `session.result` when `show`
   doesn't resolve.** `apply(Cmd::Show)` in `crates/mili-viz-server/src/lib.rs`
   currently overwrites `session.result` even when `geometry_ref`
   returns `None` (M3 "fallback to bare hull" semantics). M7 changes
   this so a failed `show` does *not* clobber a prior valid binding —
   the broadcast still carries the failure (geometry: None on the
   requested name) so the agent's `outcome_to_response` keeps
   reporting `ok: false` to the model. This is the **only fix in M7
   that protects the user from model misbehavior** instead of fixing
   the training pipeline itself.

Out of scope for M7: new Claude-generated training data (deferred — see
§"v2 deferrals"); GEPA re-tuning of the system prompt; tool-coverage
expansion; vision; streaming.

---

## Root-cause analysis

The 95.06% bench L3 and the live runaway co-exist because of **three
compounding mechanisms** in the existing pipeline. None is individually
a bug; together they form a measurement blind spot.

### 1. Training data has zero terminating-assistant-text examples

Audit of `data/posttraining/sft/sft/train.jsonl` (82 records,
2026-05-25):

| Role-sequence pattern | Count |
| --- | --- |
| `(developer, user, assistant, tool)` | 64 |
| `(developer, user, assistant, tool, assistant, tool)` | 11 |
| `(developer, user, assistant, tool, tool)` | 7 |
| **Records ending on `assistant` content** | **0 / 82** |

Every training record terminates on a `tool` message. The system prompt
explicitly instructs `When you have completed ALL sub-tasks… emit the
final text message and STOP`, but the model has no positive example of
doing so. Loss-bearing tokens cluster at 13–17 per single-call
scenario, 39 for compound (per
[`preflight-4-loss-mask.md`](../../../data/posttraining/sft/preflight-4-loss-mask.md)
§"Full-corpus scan") — **all of which fall inside the
`<start_function_call>…<start_function_response>` window**, not after
the tool response.

At inference time the model:
- emits the correct first envelope (well-formed `<start_function_call>…<end_function_call>`)
- emits `<start_function_response>` as the trained handoff cue
- (no system-side response insertion because we don't actually use the
  `<...response>` framing on the wire — the OpenAI-shape `tool` message
  is rendered into the next turn's prompt by the jinja template)
- has *no signal* for what to emit next, runs free until `max_tokens`

### 2. Verifier grades `max_call_tier` across all calls

`verifier.py:533-541`:

```python
per_call = [_grade_call(c, tools) for c in calls]
max_call_tier = max(tiers)
```

The grade is the **maximum tier reached across all tool calls in the
rollout**, not the tier of the *last* call or a count of bad calls.
A rollout containing `set_state(81)` plus seventeen `show("81")` /
`delete{"class_name": "81"}` follow-ups still scores L3 if the
postcondition (`state == 81`) holds — the seventeen bad calls dock
nothing.

### 3. Driver auto-terminates on oracle postcondition

`driver.py:282`:

```python
if verifier.verify(messages, postcondition).max_tier == 3:
    terminated_cleanly = True
    break
```

Once the postcondition is met, the driver breaks out of the multi-step
loop **before the model has a chance to emit follow-up calls**. The
explicit comment at `driver.py:278-281` acknowledges this:

> Weak open-weight models often emit repeat-the-call-N-times
> trajectories and never produce a final_text; this auto-terminate
> keeps the harness from converting a correct first call into a
> `step_cap_hit`.

The bench *knows* the model doesn't terminate cleanly and works around
it with an oracle. Live use has no oracle — the user's intent is in
natural language — so the agent loop continues past the user's
success criterion and the runaway happens.

### The bench-live equation

| Mechanism | Effect in bench | Effect in live agent |
| --- | --- | --- |
| No terminating-text training | Model never emits final_text; harness auto-terminates anyway | Model runs to `max_tokens`; emits runaway envelopes |
| `max_call_tier` grading | One correct call ⇒ L3 regardless of follow-ups | Each follow-up call dispatches; clobbers state |
| Oracle early-exit | Loop ends as soon as postcondition holds | No oracle; loop runs to `MAX_STEPS=4`, every step dispatches |

L3 95.06% = "the model emitted the correct call somewhere in a rollout
the harness silently truncated." It is **not** "the model behaves well
end-to-end." The two diverge most sharply on scenarios where the
correct call comes first and the runaway is what would have come next.

---

## Reading order (one-time orientation)

1. [`m6-client-integration-v1.md`](m6-client-integration-v1.md) — the
   M6 deltas (developer-role, structured `tool_calls`, JSON-literal
   envelope parser). M7 layers on top of M6's wiring; the wiring
   itself is correct.
2. [`m5-sft-pipeline.md`](m5-sft-pipeline.md) — the SFT pipeline that
   produced the 95.06% number. Skim for context on rev-22 training
   parameters.
3. [`../../../data/posttraining/sft/preflight-4-loss-mask.md`](../../../data/posttraining/sft/preflight-4-loss-mask.md)
   — what the loss mask covers. Critical: the assistant-content-only
   region we're about to add **does** fall inside the model-turn span
   and **will** be loss-bearing.
4. `python/mili-llm-bench/src/mili_llm_bench/assemble.py` — the
   training-record generator. M7's biggest training change lives here.
5. `python/mili-llm-bench/src/mili_llm_bench/verifier.py` — the
   grading logic. M7's tightening lives here.
6. `python/mili-llm-bench/src/mili_llm_bench/driver.py:240-290` —
   the multi-step driver loop. M7 removes / gates the oracle
   early-exit here.
7. `crates/mili-viz-server/src/lib.rs:797-820` —
   `apply(Cmd::Show)`. M7's defensive server fix lives here.

---

## Deltas in order of impact

### Delta 1 — Training records gain a terminating assistant text

**Today:** Every record in `train.jsonl` / `val.jsonl` / `heldout.jsonl`
ends on a `tool` response. The assembler in `assemble.py` appends the
tool messages and stops.

**Change:** After the final `tool` message, append one
`{"role": "assistant", "content": "<short ack>"}` message. The
content can be:
- A templated literal (e.g. `"Done."` — simplest, single token, no
  drift)
- A per-intent acknowledgment (e.g. `"Loaded {db}."`, `"Showing
  {result}."`, `"State {state}."`) — more informative, more variance,
  more authentic-looking termination

Recommend: start with the templated literal (`"Done."`) to verify the
mechanic. Move to per-intent if metric / live-UX needs warrant.

**Loss-mask interaction:** `MaskAssistantOnlyCollator` walks model-turn
spans (`<start_of_turn>model\n … <end_of_turn>`) and only subtracts
tool-response payloads inside the span. The new assistant-content
message becomes its own model-turn span — fully loss-bearing,
including `<end_of_turn>`. No collator change needed; the existing mask
logic handles it. **Verify** with a single-row probe after re-rendering
that the new termination span is decoded as visible (mirror
`preflight-4-loss-mask.md` §"Single-row probe").

**Expected outcome:** The model learns "after the last tool response,
emit a short content message and stop." Inference behavior at
`<end_of_turn>` becomes reliable. Step 1 in the live agent emits final
text, the loop terminates, no follow-up `show("81")` to clobber state.

### Delta 2 — Verifier `wrong_termination` failure mode

**Today:** `verifier.verify` returns L3 if `max_call_tier >= 2` and the
postcondition holds. The final message in the rollout is not inspected.

**Change:** Add a new failure mode to `FAILURE_MODES`:

```python
"wrong_termination",  # max_tier=2 if tool calls graded ok but
                      # rollout didn't end on a content-only assistant
                      # message
```

In the public `verify`:

```python
last_msg = messages[-1] if messages else {}
clean_termination = (
    last_msg.get("role") == "assistant"
    and isinstance(last_msg.get("content"), str)
    and last_msg["content"].strip()
    and not last_msg.get("tool_calls")
)
if pc_ok and not clean_termination:
    return VerifierResult(
        max_tier=2, reward=2/3,
        failure_mode="wrong_termination",
    )
```

This **forces** the re-rendered corpus to actually contain clean
terminations for the bench number to hold. The current 95.06% will
likely drop sharply on the first M7 run; that's a feature, not a
regression — the rev-22 number was inflated.

**Expected outcome:** Bench L3 becomes a faithful predictor of live UX
quality. M7+ corpus iterations are graded against the harder, more
honest target.

### Delta 3 — Drop the oracle early-exit in the driver loop

**Today:** `driver.py:282` breaks the loop the moment the postcondition
is met, regardless of whether the model emitted final_text.

**Change:** Remove the early-exit, OR gate it behind a
`--allow-oracle-early-exit` flag (default off). The default loop now
runs until `final_text` or `step_cap` — exactly what the live agent
does.

**Caveat:** Without the oracle early-exit, a v1-style model that never
emits final_text **will** hit `step_cap_hit` on every scenario. The L3
number will drop further (compounding with Delta 2). This is the
honest measurement; we should not hide it.

**Expected outcome:** Bench / live behavior parity. Whatever number the
bench reports is the number the user will see in the live panel.

### Delta 4 — `apply(Cmd::Show)` preserves prior result on resolution failure

**Today:** `lib.rs:797-820`:

```rust
Cmd::Show(show) => {
    let svar = ...;
    let (geometry, min, max) = match s.geometry_ref(&svar) {
        Some((g, lo, hi)) => (Some(g), lo, hi),
        None => (None, 0.0, 0.0),
    };
    let r = pb::ResultState {
        result: show.result,
        component: show.component,
        min, max,
        geometry,
    };
    s.result = Some(r.clone());  // ← clobbers prior valid result
    (D::DeltaResult, P::Result(r))
}
```

Even when `geometry_ref` returns `None`, the session's `result` is
overwritten with the requested name and `geometry: None`. The agent's
`outcome_to_response` correctly reports `ok: false` to the model
afterward, but the user-visible state damage is already done.

**Change:** When `geometry_ref` returns `None`, build the
`ResultState` for the broadcast (so `outcome_to_response` still sees
`geometry: None` and signals failure) but **do not mutate
`s.result`** — leave the prior binding intact.

```rust
Cmd::Show(show) => {
    let svar = if show.component.is_empty() { show.result.clone() }
               else { show.component.clone() };
    match s.geometry_ref(&svar) {
        Some((geometry, min, max)) => {
            let r = pb::ResultState { result: show.result, component: show.component, min, max, geometry: Some(geometry) };
            s.result = Some(r.clone());
            (D::DeltaResult, P::Result(r))
        }
        None => {
            // Failed to resolve — do NOT clobber session.result.
            // Broadcast carries the failure so the agent sees it.
            let r = pb::ResultState {
                result: show.result, component: show.component,
                min: 0.0, max: 0.0, geometry: None,
            };
            (D::DeltaResult, P::Result(r))
        }
    }
}
```

**Subtle:** this changes typed `Execute`/`Cmd::Show` semantics for any
caller (CLI, scripting). The current behavior was "fallback to bare
hull" per `phase-4-m3.md` Decision 13; the new behavior is "no-op on
unresolvable, prior binding preserved." Argument for the change: griz's
typed command set has no other place to communicate "this svar didn't
resolve" — the M3 fallback was a UI affordance, not a contract. The
new behavior matches what a careful UI would do: don't lose state on
typo. Document the change in `phase-4-m3.md` "Resolved questions log"
when it lands.

**Expected outcome:** Even if the model fires `show("81")` after
`set_state(81)`, the prin_stress1 binding survives. The user keeps
their colored bar.

### Delta 5 — Bench regression baseline

**Today:** The headline metric is rev-22's 77/81 L3 (95.06%) under the
inflating-mechanisms.

**Change:** After Deltas 1–3 land, re-run the bench against the
re-rendered corpus + new verifier + no-oracle driver. Record:

- L3 % under the new grading (expected to drop, possibly substantially
  — anywhere from 30% to 80% is plausible)
- `failure_mode` histogram with `wrong_termination` and `step_cap_hit`
  bucketed separately
- Mean turns to completion (`mean_turns_to_completion` should now be
  the *true* turns, not the oracle-truncated value)

This becomes the M7 baseline. Subsequent corpus/training changes are
measured against it.

---

## Validation plan

### A — Re-render audit (cheap, mechanical)

After modifying `assemble.py`:

```bash
uv run --directory python/mili-llm-bench -m mili_llm_bench.assemble \
    --scenarios data/posttraining/scenarios/synth.jsonl \
    --out data/posttraining/sft/

# Confirm every record now ends on an assistant content message:
python3 -c "
import json
with open('data/posttraining/sft/sft/train.jsonl') as f:
    for line in f:
        d = json.loads(line)
        last = d['messages'][-1]
        assert last['role']=='assistant' and last.get('content'), d['scenario_id']
print('all', sum(1 for _ in open('data/posttraining/sft/sft/train.jsonl')), 'records terminate on assistant content')
"
```

### B — Loss-mask single-row probe

Re-run `python/scripts/sft_loss_mask_check.py` and confirm the
"visible / non-pad fraction" rises (since the new termination region is
loss-bearing). New target: ~25–35 visible tokens per single-call
scenario (was 13–17), reflecting the added termination. The decoded
visible region should now include `<start_of_turn>model\nDone.<end_of_turn>`
(or whichever literal is chosen).

### C — Retrain on the cluster

Stage 6 SFT (see `m5-sft-pipeline.md` §"Stage 6") with the re-rendered
corpus. Same TRL config, same hparams. ~5 min on `matrix41`.

### D — Bench re-run with new verifier and no-oracle driver

```bash
uv run --directory python/mili-llm-bench mili-llm-bench run \
  --provider llamacpp \
  --scenarios data/posttraining/sft/eval/heldout.jsonl \
  --out data/posttraining/runs/v2-llamacpp-baseline \
  --step-cap 8 --per-turn-timeout-s 120 --max-new-tokens 256
```

Record the new L3 % and failure-mode histogram. Compare to rev-22.

### E — Live griz smoke

Bring up the M6 stack with the v2 GGUF, type the same prompts that
failed in M6 smoke:

- "set state to 81"
- "show prin_stress1"
- "show the prin_stress1 value for the last plot state"

Look for: model emits 1–2 well-formed envelopes, then short final
text ("Done." or similar), loop terminates, plot state changes
correctly without follow-up clobbering.

### F — Server-side `Cmd::Show` regression

After landing Delta 4: add a unit test in `mili-viz-server` that
typing `show foo_nonexistent` after a valid `show vx` preserves the
`vx` binding instead of clearing it. Update the `phase-4-m3.md` log.

---

## Gates

- **Gate 1 (re-render):** Validation §A — every record ends on
  `assistant` content. Loss probe §B shows the new region is visible
  in the mask.
- **Gate 2 (retrain):** §C completes without TRL errors; loss
  trajectory looks healthy (resembles rev-22).
- **Gate 3 (honest bench):** §D's new L3 number is *whatever it is* —
  no quality bar at this gate. The point is to have an honest
  baseline. Likely drops; that's expected.
- **Gate 4 (live):** §E shows clean termination on at least the simple
  single-action prompts ("set state to 81", "show prin_stress1"). The
  compound prompt may still fail on the model side; that's a v2
  corpus problem, not an M7 problem.
- **Gate 5 (server):** §F — typed `show foo` no longer clobbers prior
  valid result. Existing M3 tests in `mili-viz-server` updated to
  reflect the new semantics.

---

## v2 deferrals

| Item | Why deferred |
| --- | --- |
| Claude-generated corpus expansion | M7 is structural; need to know what the honest baseline is before paying for more data. After M7 Gate 3, if the bench L3 drops below ~60% and the failure-mode histogram concentrates on `wrong_termination` even after retraining, the existing corpus has hit its ceiling and Claude rollouts become the next move. |
| Compound-instruction coverage ("last plot state", "first state", "next 10 states") | The v1 model failed on these because the corpus has thin coverage. Earned-when after M7 lands and we have an honest gap to close. |
| Tool coverage (7 unmapped tools in the M4 list) | Independent of model; this is griz surface area, not training. |
| Streaming / cancellation polish | Independent of M7; carries over from M4 "next moves." |
| GEPA re-tuning of the system prompt | The current prompt's sha256 prefix `9f36d0deb5e98a89` is pinned. Re-tuning is its own milestone with its own measurement. Don't combine. |

---

## Path forward

1. **Delta 4 first (server, defensive).** Smallest risk, biggest
   immediate UX win for the existing v1 GGUF without retraining. Lets
   us continue smoke-testing while the training-side fixes are in
   flight.
2. **Delta 1 (assemble.py + re-render).** Re-tokenize, re-run loss
   probe (§B). Confirm visibility rises and decoded region contains
   the termination.
3. **Delta 2 (verifier `wrong_termination`).** Land before retraining
   so the first re-trained run is graded under the new rule.
4. **Delta 3 (driver no-oracle).** Land alongside Delta 2; they
   compose.
5. **Retrain (§C → SFT cluster run).** Probably 5–10 min wall clock.
6. **Validation §D → §E.** Record the v2 baseline. Update the M5/M6
   "Resolved questions log" with the new numbers.
7. **Decide on v2 deferrals based on the §D failure-mode histogram.**

Each step is independently revertable. Deltas 1–3 can land in any
order on the Python side; Delta 4 is independent of all of them.
