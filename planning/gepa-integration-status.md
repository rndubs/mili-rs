# GEPA Integration Implementation Status

**Date:** 2026-05-23  
**Status:** ✅ Phase 1 & 2 Complete — Full integration deployed, **full baseline run in progress**

---

## 🚀 Current GEPA Run (gepa-run-1)

**Started:** 2026-05-23 10:27 PM  
**Elapsed Time:** ~34 minutes  
**Current Phase:** Evaluating baseline on 50 scenarios (Iteration 0 of 5)  

**Status:**
- ✅ Process running (PID 88225)
- ✅ Connected to llama-server (localhost:8080)
- ✅ Connected to Claude API for reflection
- ⏳ **First iteration** (baseline evaluation): ~1/3 complete
- 📍 **ETA for first results:** ~11:30 PM - 12:00 AM

**Expected Timeline:**
- Iteration 0 (baseline): ~100-250 minutes total
- Iterations 1-4 (optimization): ~20-30 minutes each
- **Total ETA:** 2-3 hours from start (1:30-2:30 AM)

**Output Directory:** `data/posttraining/gepa-runs/gepa-run-1/`
- Files will appear once iteration 0 completes
- Will contain: best_artifact.txt, best_score.txt, best_result.json, history.jsonl, metadata.json

---

## ✅ What's Done

### 1. Core Integration Module: `gepa_integration.py`

**Location:** `python/mili-llm-bench/src/mili_llm_bench/gepa_integration.py`

**Functions Implemented:**

#### `artifact_to_eval_config(artifact: str | dict) -> EvalConfig`
- Converts GEPA's proposed artifact (system prompt string or config dict) to `EvalConfig`
- Handles Phase 1 (string) and Phase 2+ (dict) seamlessly
- Falls back to sensible defaults for unspecified fields
- Fully type-hinted and docstring'd

#### `evaluate_artifact(artifact, **provider_setup) -> float`
- The evaluator function GEPA will call on each iteration
- Runs the full eval loop: load scenarios → run driver per scenario → collect verifier results
- Aggregates using mean_tier / 3.0 (dense signal for Phase 1)
- Returns float in [0, 1]
- **Key:** No changes to harness, driver, or verifier required — just a wrapper

#### `evaluate_artifact_detailed(artifact, **provider_setup) -> EvaluationResult`
- Same as `evaluate_artifact` but returns full metrics
- Includes failure mode breakdown, L3 pass rate, mean tier, wall-clock time
- Used for iteration logging and diagnostics
- Returns `EvaluationResult` dataclass with all metadata

#### `run_gepa_optimization(config: GepaRunConfig) -> dict`
- High-level orchestration entry point
- Loads scenarios from dataset file
- Instantiates provider factory (llamacpp or anthropic)
- Calls GEPA's `optimize_anything()` with proper configuration
- Serializes results to output directory
- Returns dict with best artifact, best score, and full history

### 2. Test Suite: `test_gepa_integration.py`

**Location:** `python/mili-llm-bench/tests/test_gepa_integration.py`

**Test Classes:**

#### `TestArtifactToEvalConfig`
- ✅ String artifact conversion
- ✅ Dict artifact (minimal and full)
- ✅ Default field fallback
- ✅ Invalid artifact type rejection

#### `TestEvaluateArtifact`
- ✅ Score is returned as float [0, 1]
- ✅ Custom system prompts are respected
- ✅ Detailed metrics include failure mode breakdown
- Uses `FakeDispatcher` + `MockLlmProvider` (no griz/llama-server required)

#### `TestGepaResultSerialization`
- ✅ String artifact roundtrips to disk
- ✅ Dict artifact serialized as JSON
- ✅ All metadata files created (best_artifact, best_score, best_result, metadata)

#### `TestSmokeIntegration`
- ✅ Artifact→config roundtrip
- ✅ Frozen dataclass immutability
- ✅ Type validation

**Run tests with:**
```bash
cd /Users/rwhit/Workspace/mili-rs
uv run --directory python/mili-llm-bench pytest tests/test_gepa_integration.py -v
```

### 3. Documentation

**Planning document:** `planning/gepa-integration-plan.md`
- Full design rationale
- Architecture diagrams
- Design decisions (artifact mode, scoring, scenarios)
- Success criteria
- Risk mitigation

**Implementation guide:** This file (`gepa-integration-status.md`)
- Current status
- What's next
- Pre-CLI integration checklist

---

## ✅ What's Complete (Since Initial Planning)

### CLI Integration: DONE

**File:** `python/mili-llm-bench/src/mili_llm_bench/cli.py`

Added new command `mili-llm-bench run-gepa`:
```bash
uv run --directory python/mili-llm-bench mili-llm-bench run-gepa \
  --scenarios data/posttraining/eval/bootstrap.jsonl \
  --provider llamacpp \
  --max-iterations 5 \
  --out data/posttraining/gepa-runs/gepa-run-1
```

**Options implemented:**
- `--scenarios` (required) — Scenario dataset path
- `--out` (required) — Output directory
- `--provider` — LLM provider (llamacpp, anthropic; default: llamacpp)
- `--num-scenarios` — Limit to N scenarios for faster iteration
- `--max-iterations` — GEPA iteration count (default: 5)
- `--gepa-engine` — GEPA proposer model (default: claude-opus-4-7)
- `--gepa-reflection` — Reflection depth (shallow/medium/deep; default: medium)

**Fixes applied:**
1. ✅ Provider class name: `LlamacppProvider` → `LlamaCppProvider`
2. ✅ GEPA import: `from gepa.optimize_anything import optimize_anything`
3. ✅ API compatibility: `seed_candidate=` (not `artifact=`), proper `GEPAConfig`
4. ✅ Result handling: `gepa_result.best_candidate` (not `artifact`)
5. ✅ Added `litellm` dependency for GEPA reflection

### Smoke Test: PASSED

- ✅ 3 scenarios evaluated successfully
- ✅ GEPA proposed variant system prompt
- ✅ Score: 0.333 (mean_tier 1.0, 0% L3 pass on small sample)
- ✅ All output files generated correctly
- ✅ API key integration working

### Dependencies: ADDED

Updated `python/mili-llm-bench/pyproject.toml`:
```toml
[project.optional-dependencies]
gepa = ["gepa>=0.1.1"]
```

Installed via: `uv sync --directory python --extra gepa`

---

## 🔄 What's Remaining

### After gepa-run-1 Completes (2-3 hours from now)

**Phase 3: Analysis & Results**

**File:** `python/mili-llm-bench/src/mili_llm_bench/cli.py`

Add one new command:
```python
@click.command(name="run-gepa")
@click.option("--scenarios", required=True, help="Path to bootstrap.jsonl")
@click.option("--num-scenarios", type=int, default=None, help="Limit to N scenarios")
@click.option("--provider", type=click.Choice(["llamacpp", "anthropic"]), default="llamacpp")
@click.option("--max-iterations", type=int, default=5)
@click.option("--out", required=True, help="Output directory for results")
def run_gepa(scenarios, num_scenarios, provider, max_iterations, out):
    """Run GEPA optimization loop on system prompt."""
    config = gepa_integration.GepaRunConfig(
        dataset_path=scenarios,
        output_dir=out,
        provider_name=provider,
        num_scenarios=num_scenarios,
        max_iterations=max_iterations,
    )
    result = gepa_integration.run_gepa_optimization(config)
    click.echo(f"Best score: {result['best_score']:.3f}")
    click.echo(f"Results saved to {out}")
```

**No changes needed to existing commands** — `run-gepa` is additive.

### Phase 2B: Smoke Test (30 minutes)

Run GEPA loop on 5 scenarios × 2 iterations:

```bash
llama-server -hf ggml-org/functiongemma-270m-it-GGUF:BF16 &

uv run --directory python/mili-llm-bench mili-llm-bench run-gepa \
  --scenarios ../../data/posttraining/eval/bootstrap.jsonl \
  --num-scenarios 5 \
  --provider llamacpp \
  --max-iterations 2 \
  --out /tmp/gepa-smoke-test
```

**Expected:** Completes in <10 minutes, best_score ≥ baseline (0.33–0.67 range).

### Phase 2C: Full Baseline Run (2–3 hours)

Once smoke test passes:

```bash
uv run --directory python/mili-llm-bench mili-llm-bench run-gepa \
  --scenarios ../../data/posttraining/eval/bootstrap.jsonl \
  --provider llamacpp \
  --max-iterations 5 \
  --out ../../data/posttraining/runs/gepa-run-1
```

**Expected:** 5 iterations on 50 scenarios, best artifact beats v0-baseline (2% L3).

---

## 📋 Pre-CLI Integration Checklist

Before modifying `cli.py`, confirm:

- [ ] Baseline evals in other session complete without interference
- [ ] `gepa_integration.py` imports cleanly in your dev environment
- [ ] Run test suite: `pytest tests/test_gepa_integration.py -v` (all pass)
- [ ] GEPA library installed: `pip list | grep gepa`
  - If missing: `pip install gepa` (check version compatibility)
- [ ] Smoke test succeeds (5 scenarios, 2 iterations)

---

## 🏗️ Architecture Summary

```
┌─────────────────────────────────────────────────────────────┐
│                     GEPA Integration                         │
│                (gepa_integration.py)                         │
├─────────────────────────────────────────────────────────────┤
│                                                               │
│  run_gepa_optimization()   ← High-level entry point         │
│       ↓                                                       │
│  optimize_anything()       ← GEPA library                    │
│       ↑                                                       │
│  evaluator callback        ← evaluate_artifact()            │
│       ↓                                                       │
│  artifact_to_eval_config() ← Parse artifact to knobs        │
│       ↓                                                       │
│  driver.run_one_scenario() ← Run eval (unmodified)          │
│       ↓                                                       │
│  harness.run_turn()        ← LLM + dispatch (unmodified)    │
│       ↓                                                       │
│  verifier.verify()         ← Grade + tier (unmodified)      │
│       ↓                                                       │
│  Collect: max_tier, failure_mode → aggregate score          │
│       ↓                                                       │
│  Return float [0, 1] to GEPA                                │
│                                                               │
└─────────────────────────────────────────────────────────────┘

No changes to: harness.py, driver.py, verifier.py, schemas.py
```

---

## 🎯 Design Decisions Locked In

| Decision | Choice | Why |
|----------|--------|-----|
| **Artifact mode** | String (system prompt) | Simpler, faster iteration (Phase 2: dict) |
| **Evaluator** | Cold runs | Honest feedback, not cached |
| **Aggregation** | Mean tier / 3.0 | Dense signal (Phase 2: weighted composite) |
| **Scenarios** | All 50 bootstrap | Full baseline (Phase 2: train/val split) |
| **GEPA engine** | claude-opus-4-7 | Most capable, available |

---

## 🚀 Ready to Use

The integration module is **ready to use standalone** (for testing/debugging) or **ready for CLI integration** when the baseline evals finish.

### Standalone usage (for testing):

```python
from mili_llm_bench.gepa_integration import (
    artifact_to_eval_config,
    evaluate_artifact,
)

# Test artifact parsing
config = artifact_to_eval_config("Your custom prompt here")

# Quick eval on mock setup (if needed)
score = evaluate_artifact(
    "Your prompt",
    provider_factory=...,
    dispatcher_factory=...,
    scenarios_list=...,
    registry=...,
    tools=...,
)
```

### Once baseline evals complete:

1. Update `cli.py` with `run-gepa` command (~20 lines)
2. Run smoke test (5 scenarios, 2 iterations)
3. Run full baseline (50 scenarios, 5 iterations)
4. Analyze results + iteration history

---

## 📊 Expected Outputs

After a GEPA run completes, you'll have:

```
data/posttraining/runs/gepa-run-1/
├── best_artifact.txt          # Best system prompt found
├── best_score.txt             # Best score (e.g., 0.666667)
├── best_result.json           # Full metrics:
│   ├── score                  #   Aggregated score
│   ├── mean_tier              #   Average tier (0–3)
│   ├── l3_pass_rate           #   Proportion with max_tier==3
│   ├── failure_modes          #   Count per mode (step_cap_hit, etc.)
│   └── wall_s                 #   Total evaluation time
├── history.jsonl              # Per-iteration records
│   └── Each line: {iteration, score, artifact_preview, ...}
└── metadata.json              # Run config (provider, num_scenarios, etc.)
```

You can then:
- Compare best_artifact to v0-baseline (driver._DEFAULT_SYSTEM_PROMPT)
- Plot iteration history to see convergence
- Analyze failure_modes to see what improved
- Re-baseline with best_artifact as new seed

---

## 🔐 No Breaking Changes

✅ **All existing code remains unchanged:**
- `driver.py` — no modifications
- `harness.py` — no modifications
- `verifier.py` — no modifications
- `cli.py` — will only add `run-gepa`, existing `run` command unchanged
- All tests continue to pass

✅ **Additive only:**
- New file: `gepa_integration.py`
- New file: `test_gepa_integration.py`
- New CLI command: `run-gepa`
- New planning doc: `gepa-integration-plan.md`

---

## 🤝 Integration Timeline

1. **Now (2026-05-23):** Core module + tests complete ✅
2. **After baseline evals finish:** Smoke test (30 min)
3. **Same day:** CLI integration (1–2 hours)
4. **Next:** Full GEPA run (2–3 hours)
5. **Analysis:** Compare results to baseline, iterate

**Total hands-on time (once evals complete):** ~4–5 hours

---

**Next Action:** Wait for baseline evals to complete, then call out and we'll do the CLI integration + smoke test.
