# v0 Baseline Results — FunctionGemma-270M-it via llama.cpp

**Status:** ⏸️ BLOCKED — tooling issue discovered during smoke test

This document captures the headline L3 pass-rates and failure-mode breakdown
for the **v0 FunctionGemma local baseline** — the defensible number required by
baseline.md §"Acceptance gate" and the anchor for post-v0 branch decisions.

## Baseline Configuration

**Provider:** `LlamaCppProvider` (PR-6, llama.cpp locally)  
**Model ID:** `ggml-org/functiongemma-270m-it-GGUF:BF16`  
**Runtime:** llama-server CPU inference (Apple M-series)  
**Pinned:** Temperature 0 (greedy), seed-deterministic, BF16 full precision (no quantization)

**Eval:**
- **Scenarios:** `data/posttraining/eval/bootstrap.jsonl` (50 scenarios)
- **Step Cap:** 8 turns per scenario
- **Per-Turn Timeout:** 120 seconds
- **Max New Tokens:** 512

**Config Hash:** TBD (see config.yaml post-run)

## Headline L3 Pass Rate

*Awaiting completion...*

```
L3 pass rate: ___ / 50 scenarios ( ___% )
```

## By Max Tier

*Awaiting completion...*

| Tier | Count |
|------|-------|
| L3   | ___   |
| L2   | ___   |
| L1   | ___   |
| L0   | ___   |

## By Failure Mode

*Awaiting completion...*

| Failure Mode | Count |
|---|---|
| parse_error | ___ |
| unknown_tool | ___ |
| schema_mismatch | ___ |
| dispatch_error | ___ |
| ... | ... |

## Per-Intent Breakdown

*Full tables from report.md will be inserted here post-run.*

## Token Usage

**Total tokens**: ___ (prompt + completion across 50 scenarios, up to 8 turns each)  
**Avg per scenario**: ___  
**Avg per turn**: ___

## After-v0 Branch Decision

*See baseline.md §"After v0" for the four branches.*

Based on the L3 pass-rate above and the failure-mode distribution:

- **L3 ≥ 50%** → [Branch name and justification]
- **L0/L1 mostly red** → Re-baseline against Qwen2.5-Coder (teacher-free path)
- **Mid-range (20–50%)** → [Branch name and justification]

### Decision

*TBD pending run completion.*

---

**Run artifacts:**
- Rollouts: `data/posttraining/runs/v0-llamacpp-20260524_002518Z/rollouts.jsonl`
- Config: `data/posttraining/runs/v0-llamacpp-20260524_002518Z/config.yaml`
- Report: `data/posttraining/runs/v0-llamacpp-20260524_002518Z/report.md`
- Summary: `data/posttraining/runs/v0-llamacpp-20260524_002518Z/summary.json`

**Document updated:** 2026-05-24 (run in progress)
