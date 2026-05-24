# GEPA Integration Planning

**Date:** 2026-05-23  
**Current Branch:** `task4-baseline-measurement`  
**Status:** Pre-implementation planning phase — no code written yet

---

## Executive Summary

We have a **fully functional eval loop** (driver + harness + verifier) that generates scored rollouts with detailed failure diagnostics. GEPA's `optimize_anything` API can directly consume our rollout data and system prompt as artifacts, and use our tier-based scoring as the evaluator. This document maps what we have, what GEPA needs, and the glue code required to connect them.

**Key insight:** We don't need to rewrite our eval infrastructure — we need to wrap it so GEPA can propose system prompt variants and measure them against our existing rollout dataset or live runs.

---

## Part 1: Current Eval State (Post-Task 4b)

### 1.1 Architecture

```
┌─ Driver (driver.py) ─────────────────────────────────────┐
│                                                           │
│  Per-scenario loop:                                       │
│  - Fresh dispatcher (via factory)                        │
│  - Fresh provider (e.g. FunctionGemma)                   │
│  - Harness.run_turn() up to step_cap (default: 8)       │
│  - Stops on: final_text, timeout, step_cap_hit          │
│                                                           │
└─────────────────┬──────────────────────────────────────┘
                  │
┌─ Harness (harness.py) ──────────────────────────────────┐
│                                                           │
│  run_turn():                                             │
│  - Call provider.chat_completion()                       │
│  - Parse tool calls                                      │
│  - Type coercion on arguments                            │
│  - Schema validation                                     │
│  - Dispatch to tool (via dispatcher)                     │
│  - Collect response                                      │
│  - Append to messages                                    │
│                                                           │
└─────────────────┬──────────────────────────────────────┘
                  │
┌─ Verifier (verifier.py) ────────────────────────────────┐
│                                                           │
│  verify(messages, postcondition):                        │
│  - Grade tool calls (L0 parse → L1 schema → L2 dispatch)│
│  - Grade postcondition (L3)                              │
│  - Assign tier (0..3) and failure_mode                  │
│  - Return: VerifierResult(max_tier, reward, failure_mode)│
│                                                           │
│  reward = max_tier / 3.0  (0.0, 0.33, 0.67, 1.0)        │
│                                                           │
└─────────────────────────────────────────────────────────┘
```

### 1.2 Baseline Metrics (v0-llamacpp-baseline)

| Metric | Value |
|--------|-------|
| **L3 pass rate** | 2.0% (1/50) |
| **Tier 2+ (58%)** | 30 scenarios |
| **step_cap_hit** | 38 (76%) — primary blocker |
| **parse_error** | 4 (8%) |
| **schema_mismatch** | 4 (8%) |
| **dispatch_error** | 2 (4%) |
| **wrong_result** | 1 (2%) |
| **Mean turns (viable)** | 6.5 |

### 1.3 System Prompt (Content-Hashed for Falsifiability)

```
Current hash: 77b3f0659b28bfd5 (16-char SHA-256 prefix)
Location: driver.py::_DEFAULT_SYSTEM_PROMPT (79 lines)

Contents:
- Tool-use orientation (emit JSON function calls)
- Understanding tool responses (action_complete flags)
- Key tool mappings (load, show, material, select, etc.)
```

### 1.4 Rollout Dataset

**Location:** `data/posttraining/runs/v0-llamacpp-baseline/rollouts.jsonl`

**Per-record schema (posttraining-dataset.md §1):**
```python
{
  "id": str,                          # scenario ID
  "fixture": str,                     # test fixture name
  "intent_id": str,                   # scenario intent
  "instruction": str,                 # user instruction
  "instruction_source": str,          # "bootstrap-handauthored"
  "tools": [str],                     # sorted tool names in call sequence
  "tool_calls_flat": [{
    "name": str,
    "arguments": dict
  }, ...],
  "max_tier": int,                    # 0..3
  "failure_mode": str | None,         # closed set of 16 modes
  "postcondition_kind": str,          # scenario postcondition type
  "wall_ms": int,
  "provider": {
    "name": str,                      # e.g. "llamacpp"
    "config_hash": str                # 16-char system prompt hash
  }
}
```

**Count:** 50 scenarios × 1 baseline = 50 records (can grow)

### 1.5 Failure Mode Taxonomy (Closed Set)

**Parse/Schema (L0–L1):**
- `parse_error` — malformed JSON / JSON-adjacent
- `unknown_tool` — tool name not in registry
- `schema_mismatch` — arguments don't match schema (pre-coercion)

**Dispatch + Argument-Level (L2):**
- `dispatch_error` — generic tool execution failure
- `nonexistent_material`, `nonexistent_class`, `nonexistent_result`, `state_out_of_range` — domain-specific semantic errors

**Post-Condition (L3):**
- `wrong_final_state`, `wrong_selection`, `wrong_result`, `wrong_range`, `wrong_materials` — postcondition mismatch

**Driver-Level:**
- `step_cap_hit` — exhausted step budget
- `token_cap_hit` — (reserved, not emitted in v0)
- `timeout` — per-turn timeout exceeded

### 1.6 Scoring Semantics

```python
def reward(tier: int) -> float:
    return tier / 3.0

# Examples:
tier 0 (stuck)       → 0.00
tier 1 (schema ok)   → 0.33
tier 2 (dispatch ok) → 0.67
tier 3 (complete)    → 1.00
```

---

## Part 2: What GEPA Needs

### 2.1 Core API Shape

```python
from gepa import optimize_anything

result = optimize_anything(
    artifact=<str or dict>,           # thing to optimize
    evaluator=<callable>,              # (artifact) -> float | (float, diagnostic_info)
    objective=<str>,                   # optional: natural language goal
    dataset=<list>,                    # optional: multi-task examples
    valset=<list>,                     # optional: generalization target
    background=<str>,                  # optional: domain knowledge
    config=<dict>                      # optional: engine/reflection/tracking settings
)
```

### 2.2 Artifact Options (What Gets Optimized)

For our use case:

**Option A: System Prompt (String)**
```python
artifact = driver._DEFAULT_SYSTEM_PROMPT
# → GEPA proposes variants
# → We evaluate each variant
```

**Option B: Structured Config (Dict)**
```python
artifact = {
    "system_prompt": driver._DEFAULT_SYSTEM_PROMPT,
    "step_cap": 8,
    "temperature": 0.0,
    # ... other knobs
}
# → GEPA proposes variants
# → We evaluate each variant
```

**Option C: Natural Language (Bootstrap)**
```python
artifact = "A system prompt that teaches an LLM to operate Griz tools effectively"
# → GEPA generates candidates from scratch
```

### 2.3 Evaluator Requirements

GEPA passes the artifact to the evaluator and expects:

```python
def evaluate(artifact: str | dict) -> float | tuple[float, dict]:
    """
    Returns a score (higher is better) or (score, diagnostic_info).
    
    The evaluator is responsible for:
    1. Constructing an EvalConfig from the artifact
    2. Running scenarios (or a subset)
    3. Collecting tier/failure_mode results
    4. Aggregating to a single reward signal
    """
    pass
```

**Score choice:** We have multiple options:
- **L3 pass rate** (current) — binary: tier==3
- **Mean tier** — average tier across scenarios (0..3)
- **Weighted aggregate** — custom formula weighting tiers differently
- **Composite metric** — e.g., (0.7 * mean_tier) + (0.3 * inverse_dispatch_errors)

### 2.4 Optional Inputs

**dataset** — Multi-task search (GEPA learns patterns across scenarios):
```python
dataset = [
    {
        "scenario": scenario_obj,
        "postcondition": postcondition_obj,
        "input_features": {...}  # optional categorization
    },
    ...  # 50 scenarios
]
```

**valset** — Generalization mode (evaluate on held-out scenarios):
```python
valset = dataset[10:20]  # e.g., 10 held-out scenarios
# GEPA optimizes for transfer to unseen examples
```

**objective** — Natural language goal (guides reflection):
```python
objective = "Increase L3 pass rate by reducing step_cap_hit cases through better tool selection guidance"
```

**background** — Domain context (for reflection/reasoning):
```python
background = """
Griz is a post-processor for Mili finite-element format.
Common failure modes:
- step_cap_hit: Model loops without progressing
- schema_mismatch: String args where ints expected (mostly fixed by Task 1 coercion)
- dispatch_error: Tool not found or state invalid
Tool mappings: load, show, material, select, set_state, step, ...
"""
```

### 2.5 Config Options

```python
config = {
    "engine": "claude-opus-4-7",       # LLM for proposals
    "reflection": "deep",              # amount of reasoning
    "max_iterations": 10,              # optimization iterations
    "tracking": "wandb",               # optional logging
}
```

---

## Part 3: Integration Gaps

### 3.1 Conceptual Gaps (What's Missing)

| Gap | Current State | GEPA Need | Impact |
|-----|---------------|-----------|--------|
| **Evaluator wrapper** | None | Function that takes artifact, returns score | Blocking |
| **Artifact representation** | System prompt is hardcoded | Artifact must be a mutable string/dict | Blocking |
| **Rollout caching** | Cold run every time | Cache vs. live eval trade-off decision | Design |
| **Score aggregation** | L3 pass rate only | Multiple aggregation strategies | Design |
| **Diagnostics pipeline** | Verifier emits per-scenario | Format for GEPA's ASI (Actionable Side Info) | Enhancement |
| **Config seam** | EvalConfig hardcoded defaults | Parameterize from artifact | Blocking |

### 3.2 Code Changes Required

#### **A. Artifact Abstraction Layer**

Currently: `EvalConfig` is a frozen dataclass with hardcoded defaults.

Needed:
```python
# driver.py or a new gepa_integration.py

def artifact_to_eval_config(artifact: str | dict) -> EvalConfig:
    """Convert GEPA's proposed artifact to EvalConfig."""
    if isinstance(artifact, str):
        # Plain string → system prompt
        return EvalConfig(system_prompt=artifact)
    elif isinstance(artifact, dict):
        # Structured config
        return EvalConfig(
            system_prompt=artifact.get("system_prompt", _DEFAULT_SYSTEM_PROMPT),
            step_cap=artifact.get("step_cap", 8),
            temperature=artifact.get("temperature", 0.0),
            max_new_tokens=artifact.get("max_new_tokens", 256),
            # ... etc
        )
    else:
        raise ValueError(f"Unexpected artifact type: {type(artifact)}")
```

#### **B. Evaluator Function**

```python
# gepa_integration.py (new file)

def evaluate_artifact(
    artifact: str | dict,
    *,
    provider_factory: Callable,
    dispatcher_factory: Callable,
    scenarios: list[Scenario],
    registry: Registry,
    tools: list[dict],
) -> float:
    """
    Run eval loop with artifact as system prompt, return aggregated reward.
    
    Implementation strategy:
    1. Parse artifact → EvalConfig
    2. Run driver.run_one_scenario() for each scenario
    3. Collect VerifierResult.max_tier for each
    4. Aggregate: mean_tier / 3.0
    5. Return float in [0, 1]
    """
    config = artifact_to_eval_config(artifact)
    
    results = []
    for scenario in scenarios:
        result = driver.run_one_scenario(
            provider=provider_factory(),
            dispatcher_factory=dispatcher_factory,
            scenario=scenario,
            tools=tools,
            config=config,
            registry=registry,
        )
        results.append(result.verifier_result)
    
    # Aggregate: mean tier
    mean_tier = sum(r.max_tier for r in results) / len(results)
    return mean_tier / 3.0  # normalize to [0, 1]
```

#### **C. GEPA Entry Point**

```python
# gepa_integration.py (continued)

def run_gepa_optimization(
    dataset_path: Path,
    provider_name: str = "llamacpp",
    num_scenarios: int | None = None,
    artifact_mode: str = "system_prompt",  # or "config"
    max_iterations: int = 5,
) -> dict[str, Any]:
    """
    High-level entry point for GEPA loop.
    
    Returns dict with:
    - best_artifact
    - best_score
    - iteration_history
    - all_proposed_artifacts
    """
    
    # Load scenarios from dataset
    scenarios = load_scenarios(dataset_path)
    if num_scenarios:
        scenarios = scenarios[:num_scenarios]
    
    # Initialize provider & dispatcher factories
    provider_factory = {
        "llamacpp": lambda: providers.LlamacppProvider(...),
        "anthropic": lambda: providers.AnthropicProvider(...),
        # ...
    }[provider_name]
    
    dispatcher_factory = PygrizDispatcher  # or FakeDispatcher for tests
    
    # Load registry
    registry = Registry(...)
    tools = load_tools(schemas.default_artifact_path())
    
    # Seed artifact
    initial_artifact = driver._DEFAULT_SYSTEM_PROMPT
    
    # Call GEPA
    from gepa import optimize_anything
    
    result = optimize_anything(
        artifact=initial_artifact,
        evaluator=lambda art: evaluate_artifact(
            art,
            provider_factory=provider_factory,
            dispatcher_factory=dispatcher_factory,
            scenarios=scenarios,
            registry=registry,
            tools=tools,
        ),
        objective=(
            "Increase L3 pass rate by improving system prompt guidance. "
            "Current bottleneck: 76% hit step_cap_hit, suggesting model loops "
            "without progressing. Focus on better tool selection cues."
        ),
        background=(
            "Griz post-processor for Mili FEM format. Tools: load, show, material, "
            "select, set_state, step, view, colormap, etc. Common failures: "
            "step_cap_hit (loops), schema_mismatch (type coercion), dispatch_error "
            "(semantic validation). Model: FunctionGemma-270M."
        ),
        config={
            "engine": "claude-opus-4-7",
            "reflection": "medium",
            "max_iterations": max_iterations,
        },
    )
    
    return {
        "best_artifact": result.artifact,
        "best_score": result.score,
        "history": result.history,
    }
```

#### **D. CLI Integration**

```python
# cli.py (update run command to support GEPA mode)

@click.command(name="run-gepa")
@click.option(
    "--scenarios",
    type=click.Path(exists=True),
    required=True,
    help="Path to bootstrap.jsonl or dataset",
)
@click.option(
    "--num-scenarios",
    type=int,
    default=None,
    help="Limit to N scenarios for faster iteration",
)
@click.option(
    "--provider",
    type=click.Choice(["llamacpp", "anthropic"]),
    default="llamacpp",
)
@click.option(
    "--max-iterations",
    type=int,
    default=5,
)
@click.option(
    "--out",
    type=click.Path(),
    required=True,
    help="Output directory for GEPA results",
)
def run_gepa(scenarios, num_scenarios, provider, max_iterations, out):
    """Run GEPA optimization loop on system prompt."""
    result = gepa_integration.run_gepa_optimization(
        dataset_path=Path(scenarios),
        provider_name=provider,
        num_scenarios=num_scenarios,
        max_iterations=max_iterations,
    )
    
    output_dir = Path(out)
    output_dir.mkdir(parents=True, exist_ok=True)
    
    (output_dir / "best_artifact.txt").write_text(result["best_artifact"])
    (output_dir / "best_score.txt").write_text(str(result["best_score"]))
    (output_dir / "history.jsonl").write_text(
        "\n".join(json.dumps(entry) for entry in result["history"])
    )
```

### 3.3 Execution Flow

```
User: uv run --directory python/mili-llm-bench mili-llm-bench run-gepa \
  --scenarios ../../data/posttraining/eval/bootstrap.jsonl \
  --num-scenarios 10 \
  --provider llamacpp \
  --max-iterations 3 \
  --out ../../data/posttraining/runs/gepa-run-1

  ↓

  gepa_integration.run_gepa_optimization()
    ↓
    [Setup: Load scenarios, providers, dispatcher, registry, tools]
    ↓
    optimize_anything(
      artifact=_DEFAULT_SYSTEM_PROMPT,
      evaluator=evaluate_artifact,
      ...
    )
    ↓
    [Iteration 1]
      - GEPA proposes new system prompt
      - evaluate_artifact() runs 10 scenarios with new prompt
      - driver.run_one_scenario() → harness.run_turn() × step_cap → verifier.verify()
      - Collect max_tier for each → mean_tier / 3.0 → return score
      - GEPA reflects on result, generates next variant
    ↓
    [Iteration 2, 3, ...]
    ↓
    [Return best variant + history]
    ↓
    Write results to output dir
```

---

## Part 4: Design Decisions

### 4.1 Artifact Mode: String vs. Dict

**Option 1: System Prompt String** ✓ **Recommended for Phase 1**
- Pros: Simple, focused, GEPA's sweet spot
- Cons: Can't tune step_cap, temperature, etc.
- Use case: Fast iteration on the primary lever (prompt)

**Option 2: Structured Config Dict**
- Pros: Tune multiple knobs simultaneously
- Cons: Larger search space, harder for GEPA to reason about
- Use case: Phase 2 after prompt is near-optimal

**Decision:** Start with **Option 1** (string). Once L3 pass rate plateaus, switch to **Option 2**.

### 4.2 Evaluator Scope: Cold vs. Cached Runs

**Option A: Cold Runs** ✓ **Recommended for Phase 1**
```python
# Every call to evaluate_artifact() spins up fresh providers/dispatchers
# Pros: Honest evaluation, no data leakage
# Cons: Slow (50 scenarios × 8 steps ≈ 5-15 min per iteration)
```

**Option B: Cached Dataset**
```python
# Pre-run all scenarios once; evaluator replays rollouts and re-scores
# Pros: Fast (seconds per iteration), but...
# Cons: Unfaithful (system prompt doesn't affect cached rollouts!)
```

**Decision:** Start with **Option A** (cold runs). Pre-cache scenarios only for prototyping/debugging.

**Optimization:** Run on a subset (e.g., `--num-scenarios 10`) for faster iteration, then full baseline at the end.

### 4.3 Score Aggregation Strategy

**Option 1: L3 Pass Rate** (Current)
```python
score = sum(1 for r in results if r.max_tier == 3) / len(results)
# Binary reward: 1.0 if complete, 0.0 otherwise
# Pro: Matches our stated goal
# Con: Sparse signal (only 1 out of 50 scenarios currently pass)
```

**Option 2: Mean Tier**
```python
score = sum(r.max_tier for r in results) / len(results) / 3.0
# Continuous reward: tiers weighted equally
# Pro: Dense signal, GEPA can find intermediate improvements
# Con: Doesn't directly measure L3 completeness
```

**Option 3: Weighted Composite** ✓ **Recommended**
```python
score = (
    0.5 * (sum(1 for r in results if r.max_tier == 3) / len(results))  # L3 pass rate
    + 0.3 * (sum(r.max_tier for r in results) / len(results) / 3.0)   # Mean tier
    + 0.2 * (1.0 - count_failure_mode(results, "step_cap_hit") / len(results))  # Avoid step_cap
)
```

**Decision:** Start with **Option 2** (mean tier) for denser signal; graduate to **Option 3** (weighted) if needed.

### 4.4 Baseline vs. Held-Out Scenarios

**Option 1: All Bootstrap Scenarios**
- Train and test on the same 50 scenarios
- Pros: Fast iteration, maximizes signal
- Cons: GEPA may overfit to bootstrap set

**Option 2: Train/Val Split**
```python
scenarios = load_scenarios(bootstrap_path)  # 50 total
train = scenarios[:40]
val = scenarios[40:]

# GEPA optimizes on train, evaluates on val for each iteration
# Reduces overfitting, more honest generalization estimate
```

**Decision:** Start with **Option 1** (all bootstrap), then fold in **Option 2** (train/val split) if we see overfitting signals.

---

## Part 5: Pre-Implementation Checklist

### 5.1 Code Infrastructure

- [ ] **artifact_to_eval_config()** — Parse artifact to EvalConfig
- [ ] **evaluate_artifact()** — Run eval loop, return float score
- [ ] **run_gepa_optimization()** — High-level orchestration
- [ ] **CLI integration** — `run-gepa` command
- [ ] **Result serialization** — Save best artifacts, history, diagnostics

### 5.2 Runtime Dependencies

- [ ] **GEPA library** — Install `gepa` (check version / availability)
- [ ] **LLM provider** — Ensure `llama-server` running (or switch to `--provider anthropic`)
- [ ] **Griz subprocess** — Dispatcher must be able to spawn griz.Session()
- [ ] **Timeout handling** — Robust evaluation under slow/hanging scenarios

### 5.3 Instrumentation

- [ ] **Logging** — Track which iteration we're on, which artifact being evaluated, current best score
- [ ] **Rollout recording** — Optionally save each iteration's rollouts for post-hoc analysis
- [ ] **Config hashing** — Each proposed artifact should be content-hashed for traceability

### 5.4 Testing Strategy

- [ ] **Smoke test** — GEPA loop with 2 scenarios × 2 iterations (should complete in <1 min)
- [ ] **Score monotonicity** — Verify scores are reproducible across runs
- [ ] **Artifact recovery** — Best artifact should be serializable and executable
- [ ] **Backward compatibility** — Existing `run` command unchanged

### 5.5 Validation Milestones

1. **Phase 1 (Prototype):** GEPA loop runs end-to-end on 5 scenarios, 2 iterations
2. **Phase 2 (Validation):** Full 50 scenarios, 3 iterations; best artifact scores ≥ baseline
3. **Phase 3 (Tuning):** 5+ iterations; measure L3 pass rate improvement vs. v0-baseline
4. **Phase 4 (Analysis):** Collect iteration history; visualize score progression; extract insights

---

## Part 6: Output Artifacts

### 6.1 Per-Run Directory Structure

```
data/posttraining/runs/gepa-run-1/
├── best_artifact.txt          # Winning system prompt
├── best_score.txt             # Best score achieved
├── history.jsonl              # Line-delimited iteration history
├── all_proposed_artifacts/
│   ├── iteration_1.txt
│   ├── iteration_2.txt
│   └── ...
├── iteration_rollouts/        # Optional: per-iteration rollouts
│   ├── iteration_1_rollouts.jsonl
│   └── ...
└── metadata.json              # Run config: num_scenarios, provider, max_iterations, etc.
```

### 6.2 History Record Schema

```json
{
  "iteration": 1,
  "artifact": "You are an assistant...",
  "score": 0.52,
  "mean_tier": 1.56,
  "l3_pass_rate": 0.0,
  "failure_modes": {
    "step_cap_hit": 38,
    "parse_error": 4,
    "schema_mismatch": 4,
    "dispatch_error": 2,
    "wrong_result": 1
  },
  "wall_s": 425.3,
  "timestamp": "2026-05-23T14:32:18Z"
}
```

---

## Part 7: Success Criteria

| Milestone | Criterion | Target |
|-----------|-----------|--------|
| **Smoke Test** | 5 scenarios, 2 iterations complete | <10 min |
| **Full Baseline** | 50 scenarios, 3 iterations, no crashes | <2 hours |
| **Score Improvement** | Best artifact beats v0-baseline | ≥ 2.0% L3 (current: 2.0%) |
| **Convergence** | Score plateau across final 2 iterations | <1% delta |
| **Reproducibility** | Re-run with same config produces same best artifact | ✓ Deterministic |

---

## Part 8: Known Constraints & Risks

### Risks

| Risk | Mitigation |
|------|-----------|
| **Slow eval loop** | Start with `--num-scenarios 10`, measure wall-clock time |
| **GEPA not installed** | Verify `pip list \| grep gepa` before starting |
| **llama-server crashes** | Switch to `--provider anthropic` or add subprocess restart logic |
| **Dispatcher hangs** | Increase `per_turn_timeout_s`; add process-level timeouts |
| **GEPA overfits to bootstrap** | Implement train/val split in Phase 2 |
| **Score signal is too sparse** | Switch to mean_tier aggregation for Phase 1 |

### Constraints

- **No code changes to harness/verifier** — These stay frozen; only driver/new integration layer
- **Backward compatible** — Existing `run` command must work unchanged
- **Determinism** — Temperature=0.0, seed pinned; content-hashing on artifacts for reproducibility

---

## Part 9: Next Steps

### To Move to Implementation

1. **Get GEPA approval:** Confirm GEPA library is accessible and has expected API
2. **Finalize artifact mode:** Decide string vs. dict (recommend: string for Phase 1)
3. **Finalize score aggregation:** Decide L3 pass rate vs. mean tier (recommend: mean tier)
4. **Write artifact_to_eval_config()** — The core seam that unlocks everything
5. **Write evaluate_artifact()** — Thin wrapper around existing driver loop
6. **Write CLI + orchestration** — Glue it all together

### Structure for Code Phase

**New file:** `python/mili-llm-bench/src/mili_llm_bench/gepa_integration.py`
- `artifact_to_eval_config()`
- `evaluate_artifact()`
- `run_gepa_optimization()`

**Modified file:** `python/mili-llm-bench/src/mili_llm_bench/cli.py`
- Add `@click.command(name="run-gepa")`
- Wire to `gepa_integration.run_gepa_optimization()`

**Test file:** `python/mili-llm-bench/tests/test_gepa_integration.py`
- Smoke test with `FakeDispatcher` + `MockLlmProvider`
- Verify score reproducibility

---

## Appendix A: GEPA API Reference

From https://gepa-ai.github.io/gepa/blog/2026/02/18/introducing-optimize-anything/ :

```python
from gepa import optimize_anything

# Minimal
result = optimize_anything(
    artifact="string or dict",
    evaluator=lambda x: score(x)  # returns float
)

# Full-featured
result = optimize_anything(
    artifact="seed or description",
    evaluator=lambda x: (score(x), diagnostic_info),
    objective="natural language goal",
    dataset=[{"input": ..., "target": ...}, ...],
    valset=[...],  # hold-out set for generalization
    background="domain knowledge",
    config={
        "engine": "claude-opus-4-7",
        "reflection": "deep",  # "shallow", "medium", "deep"
        "max_iterations": 10,
        "tracking": "wandb",
    }
)

# Result attributes
result.artifact      # Best artifact found
result.score         # Best score achieved
result.history       # Iteration log
result.diagnostics   # Reflection / reasoning traces
```

---

## Appendix B: Current System Prompt (Baseline)

```
You are an assistant that operates the Griz post-processor for the 
Mili finite-element format. You drive Griz by emitting JSON function 
calls into the supplied tool inventory. Inspect the user's request, 
call exactly the tools that satisfy it, and reply with one short 
final text message only after the request is fully complete. Do not 
narrate plans; emit a tool call instead. Prefer the typed tools 
over the `griz_raw` fallback when a typed tool exists for the task.

UNDERSTANDING TOOL RESPONSES:
When a tool response includes 'action_complete': true, the action has succeeded and you should move on.
For state-changing tools (set_state): compare 'requested_state' with 'state' to verify completion.
Do not repeat the same tool call with identical arguments if you already received a successful response.
Only call a tool again if you need to verify something or if the previous response indicated an error (ok: false).

KEY TOOL MAPPINGS:
- Load/open a database: use `load` with root parameter (e.g., root='cylinder')
- Display/show/color a result: use `show` with result parameter (e.g., result='vx')
- Enable/disable materials: use `material` with enable (true/false) and material/class_name
- Select elements: use `select` or `clrsel` (clear selection)
- Change states: use `set_state` or `step`
- Adjust view: use `colormap`, `view`, `named_view`, `legend`
```

**Hash:** `77b3f0659b28bfd5` (SHA-256, 16-char prefix)

---

**Document Version:** 1.0  
**Last Updated:** 2026-05-23  
**Status:** Ready for implementation planning review
