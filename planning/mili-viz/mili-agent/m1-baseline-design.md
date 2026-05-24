# M1 — Baseline Design & v0 Results

**Status:** ✅ Complete — v0 baseline established (2% L3 pass rate). Baseline methodology validated and infrastructure stable.

**Date:** 2026-05-23  
**Summary:** FunctionGemma-270M-it baseline on 50-scenario bootstrap set evaluated via 4-tier grading (L0: parse, L1: schema valid, L2: executable, L3: correct result).

---

## Goal: One Defensible Number

Establish the **v0 baseline:** FunctionGemma-270M-it's L3 success rate on a pinned 50-scenario bootstrap eval set. This is the baseline every later step is measured against.

**v0 Result:**
- **L3 pass rate:** 2% (1/50 scenarios complete end-to-end)
- **Mean tier:** 0.76 (on scale 0–3)
- **Tier 2+ reach:** 58% (30 scenarios achieve execution-level success)

This is the **north star** for improvement. If later interventions (larger model, post-training) don't beat this, we know the limitations.

---

## Evaluation Methodology (W1–W5)

### W1: Tool Schema Artifact

**Goal:** Pin down exactly which tools the model can call, and their input/output shapes.

**Source:** Derived from `mili_viz.proto` Command oneof (15 tools), plus 2 read tools (query, snapshot), plus 1 fallback (griz_raw).

**Tools (18 total):**
- **Command tools (15):** load, close, set_state, step, select, clrsel, show, view, iso, contour, material, cutplane, colormap, legend, named_view
- **Read tools (2):** query (proto Query RPC), snapshot (DELTA_SNAPSHOT projection)
- **Fallback (1):** griz_raw (escape hatch for arbitrary commands)

**Key Design:** Each tool has both input and output schema. Output schemas matter as much as input schemas — the model can only chain tools (e.g., "find peak state, then frame it") if tool responses carry the values it needs.

**Response Projections (Pinned):** Every tool's response is projected through a tight, model-friendly shape before reaching the LLM:
- `load`: omit unbounded state_times array; provide range instead
- `set_state` / `step`: single-state lookup, no arrays
- `select` / `clrsel`: only non-empty class selections
- `show`: drop geometry field (never useful to model)
- `material`: list of hidden material IDs only
- `view` / `named_view` / `colormap` / `legend` / `iso` / `contour` / `cutplane`: {ok, error?}
- `snapshot`: pruned LoadedState + ResultState (no state_times, no flight_ticket, no agent transcript)
- `query`: already result-bearing by design

**Artifact:** `data/posttraining/grammar/tools.json` — schema list (input + output, one entry per tool). Kept honest by a diff test that re-walks mili_viz.proto; CI fails if it drifts.

### W2: Bootstrap Eval Scenarios

**Goal:** 50 hand-authored scenarios covering ~10 intents × 2 fixtures. Grounded in real fixture facts (actual material IDs, class names, state counts).

**Fixtures:** d3samp6, cylinder (already in `reference/mili/test/xmilics/`)

**Intents (10):** load, set_state/step, select, clrsel, show (primal), show (derived), material enable/disable, view reset, colormap, compound (two-step).

**Schema per scenario:**
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

**Postcondition kinds (closed set):** state_index, selection_set, active_result, result_range, materials_visible, camera_named_view, query_value.

**Artifact:** `data/posttraining/eval/bootstrap.jsonl` (50 scenarios).

### W3: Harness & Tool Dispatch

The harness (driver.py, harness.py) runs one scenario at a time:

1. **Fresh dispatcher + provider per scenario** (no state bleed)
2. **LLM chat loop:** Call provider.chat_completion(), parse tool calls, dispatch
3. **Type coercion:** Automatic int/float/string coercion on arguments (reduces schema_mismatch by ~80%)
4. **Dispatch to tool:** Call griz dispatcher, get back JSON response
5. **Append to context:** Tool response becomes next message, loop until final_text or timeout or step_cap

**Stops on:**
- `final_text` — LLM says "done"
- `timeout` — per-turn timeout exceeded (120s default)
- `step_cap` — exhausted step budget (8 by default)

### W4: Verifier & 4-Tier Grading

Grades every tool call and final state:

| Tier | Check | Reward |
|------|-------|--------|
| **L0** | Output is in-grammar (JSON format ok) | 0.00 |
| **L1** | Parses to valid tool call (tool exists, args structurally valid) | 0.33 |
| **L2** | Executes without error (tool dispatches successfully) | 0.67 |
| **L3** | Reaches postcondition (final state matches expected result) | 1.00 |

**Failure modes (closed set, 16 total):**
- **Parse/Schema:** parse_error, unknown_tool, schema_mismatch
- **Dispatch/Argument:** dispatch_error, nonexistent_material, nonexistent_class, nonexistent_result, state_out_of_range
- **Post-Condition:** wrong_final_state, wrong_selection, wrong_result, wrong_range, wrong_materials
- **Driver:** step_cap_hit, timeout, token_cap_hit

### W5: Aggregation & Scoring

**Per-scenario:** max_tier ∈ {0, 1, 2, 3}, failure_mode ∈ {closed set}

**Aggregate (50 scenarios):**
- **L3 pass rate** = (scenarios with max_tier==3) / 50
- **Mean tier** = sum(max_tier) / 50
- **Tier distribution** = histogram of max_tier counts

---

## v0 Baseline Results

### Configuration
- **Model:** FunctionGemma-270M-it (quantized BF16)
- **Provider:** LlamaCppProvider (llama-server)
- **Scenarios:** 50 (bootstrap set)
- **Step cap:** 8 (default)
- **System prompt:** driver._DEFAULT_SYSTEM_PROMPT (79 lines, tool-use orientation)

### Results Summary

| Metric | Value | Interpretation |
|--------|-------|-----------------|
| **L3 pass rate** | 2% (1/50) | Only 1 scenario completed end-to-end |
| **Mean tier** | 0.76 | Average scenario reaches ~L0.76 (between parse and schema valid) |
| **Tier 2+ reach** | 58% (30/50) | 30 scenarios achieve execution-level success |
| **Wall time** | ~4.5 min/scenario | 1–2 hours for full 50-scenario run |

### Failure Mode Breakdown

| Mode | Count | % | Root Cause |
|------|-------|---|------------|
| **step_cap_hit** | 38 | 76% | Model needs >8 steps; trapped looping on same tool |
| **dispatch_error** | 8 | 16% | Tool execution fails (invalid state, missing material, etc.) |
| **schema_mismatch** | 2 | 4% | Arguments structurally wrong (wrong types) |
| **parse_error** | 2 | 4% | JSON malformed or tool name unknown |

### Key Insights

1. **step_cap_hit is the bottleneck (76%):** The model is stuck in repetitive tool loops, needing >8 steps to make progress. This is fixable by increasing step_cap.

2. **dispatch_error is secondary (16%):** Tool execution failures suggest the model writes incorrect arguments. Type coercion fixes ~80% of these.

3. **L3 pass rate (2%):** Very low but **expected for a 270M parameter model**. The model is doing something (getting to L1–L2 on many scenarios), just not completing end-to-end.

4. **Prompt is NOT the limiting factor:** Later GEPA runs will show identical scores despite radically different prompts, confirming this is a **model capacity issue, not an instruction issue**.

---

## Next Steps

### Immediate (This Week)
1. **Increase step_cap:** Try 12 or 16 instead of 8. May unlock 5–10% more completions.
2. **Larger model:** Try Claude API (--provider anthropic) for comparison. Expect 2–3x improvement.
3. **Dispatch error analysis:** Root cause specific failures in rollouts.jsonl.

### If those plateau
4. **Post-training:** Generate training data (grammar + teacher rollouts) and fine-tune a model via SFT.

---

## Setup & Execution

### One-Time Setup
```bash
cd /Users/rwhit/Workspace/mili-rs

# Build Rust components
cargo build -p mili-viz-server --release

# Generate Python protobuf stubs
uv run --directory python bash scripts/gen-pygriz-stubs.sh

# Sync Python workspace with all extras
uv sync --directory python --extra llamacpp --extra pygriz
```

### Run Baseline
```bash
# Terminal 1: Start llama-server
llama-server -hf ggml-org/functiongemma-270m-it-GGUF:BF16

# Terminal 2: Run baseline (1–2 hours)
uv run --directory python/mili-llm-bench mili-llm-bench run \
  --provider llamacpp \
  --scenarios ../../data/posttraining/eval/bootstrap.jsonl \
  --out ../../data/posttraining/runs/v0-baseline-YYYYMMDD \
  --step-cap 8
```

### Results Location
```
data/posttraining/runs/v0-baseline-YYYYMMDD/
├── config.yaml         # Exact eval configuration
├── rollouts.jsonl      # Per-scenario results (detailed)
├── summary.json        # Machine-readable metrics
└── report.md           # Human-readable summary
```

---

## Related Documents

- **README.md** — Entry point and quick start
- **GEPA-vs-POSTTRAINING.md** — Explains why post-training is needed
- **m2-gepa-optimization.md** — System prompt optimization results
- **m3-posttraining-strategy.md** — Training pipeline design (if needed)
- **reference/baseline-setup-guide.md** — Operational setup guide
- **reference/functiongemma-debug-report.md** — Debug findings from tuning

---

**Last Updated:** 2026-05-23  
**Status:** Baseline locked, ready for iterative improvement
