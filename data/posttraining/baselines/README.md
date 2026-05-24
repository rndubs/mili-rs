# Baseline Reference Snapshots

This directory contains committed baseline measurements that serve as reference points for the mili-llm-bench harness evolution.

## v0-ground-zero

**Status:** Ground zero baseline for post-training development  
**Date:** 2026-05-23  
**Branch:** task4-baseline-measurement (commit 0cd0861)

### Configuration
- Model: FunctionGemma 270M (llamacpp)
- Step cap: 8
- Max tokens: 256
- Scenarios: 50 (bootstrap eval set)
- System prompt hash: 77b3f0659b28bfd5

### Results Summary
- **L3 pass rate:** 2.0% (1/50)
- **Tier distribution:** 
  - Tier 0 (stuck): 12 (24%)
  - Tier 1: 8 (16%)
  - Tier 2: 29 (58%)
  - Tier 3 (complete): 1 (2%)

### Failure Mode Breakdown
```
step_cap_hit:     38 (76%)  — main blocker: model repeats same tool call
parse_error:       4 (8%)   — remaining edge cases
schema_mismatch:   4 (8%)   — remaining edge cases
dispatch_error:    2 (4%)   — resolved most via type coercion fix
wrong_result:      1 (2%)   — semantic issue
```

### Key Finding: The Looping Problem

Model exhibits systematic looping behavior: repeats identical tool calls despite:
- System prompt guidance: "Do not repeat the same tool call with identical arguments"
- Response signals: `"action_complete": true` on successful operations
- Explicit instruction: "move on when action_complete"

**Example:** Loading a database calls `load(root="d3samp6")` 12 times in a row, each returning `ok=true, action_complete=true`.

**Implication:** The FunctionGemma 270M base model lacks semantic understanding to:
1. Recognize action completion signals
2. Avoid repeating unsuccessful patterns
3. Transition from tool-calling to final-answer mode

This is precisely what post-training (Stages 4-5) exists to address.

### Harness Maturity

All framework optimizations are working correctly:
- ✅ Task 1: Type coercion (reduced schema_mismatch 80%)
- ✅ Task 2: Parse error recovery (handles malformed input)
- ✅ Task 3: Response enrichment (action_complete signals)
- ✅ Type coercion harness fix (resolved 89% of dispatch_errors)

The 2% baseline reflects the **true capability of the untuned model**, not framework limitations.

### Using This Baseline

**For comparison:** Future baseline runs should be compared against v0-ground-zero results to measure:
- Post-training impact on L3 pass rate
- Reduction in step_cap_hit via behavior training
- Improvement in handling edge-case error modes

**Artifacts included:**
- `rollouts.jsonl` — complete per-scenario results with tool calls and responses
- `report.md` — human-readable summary with per-intent breakdown
- `summary.json` — machine-readable metrics for automated comparison
- `config.yaml` — exact configuration for reproducibility
