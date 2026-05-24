# v0 Baseline Results — FunctionGemma-270M-it via llama.cpp

**Status:** ❌ **CRITICAL BLOCKER — v0 baseline is 0% (L3 pass rate = 0/50)**

This document captures the headline L3 pass-rates and failure-mode breakdown
for the **v0 FunctionGemma local baseline**. The v0 baseline is **completely
broken**: the model generates malformed tool calls that our parser cannot
extract. This is not a provider bug; it's a fundamental mismatch between the
FunctionGemma format we constructed and what the model actually outputs.

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

**L3 pass rate: 0 / 50 scenarios (0%)**

Critical failure: the model is generating malformed tool calls (see "Root Cause" below).

## By Max Tier

| Tier | Count | % |
|------|-------|-------|
| L3   | 0     | 0%   |
| L2   | 18    | 36%  |
| L1   | 0     | 0%   |
| L0   | 32    | 64%  |

## By Failure Mode

| Failure Mode | Count |
|---|---|
| parse_error | 32 |
| step_cap_hit | 18 |
| (all others) | 0 |

## Root Cause — Malformed Tool Calls

The 32 L0 scenarios fail on `parse_error`: the model generates garbled output like:

```
<start_function_call>call:load
<start_function_call><start_function_response>
<start_function_call>call:load
<start_function_call><start_function_response>call:load
...
```

This does not match the expected FunctionGemma format. The issue is not in the
provider code; it's a **fundamental mismatch between the prompt format we
constructed and what the model actually outputs**.

The model card and Jinja template documentation did not provide sufficient detail
to reconstruct the correct format for tool use in FunctionGemma-270M-it when
used via llama-server's `/completion` endpoint. The `/v1/chat/completions`
endpoint would have applied the model's baked-in template, but llama-server's
implementation does not support tools on that endpoint.

## Per-Intent Breakdown

No L3 passes to break down.

## Timing

**Wall-clock time**: 4980.39 seconds (~83 minutes)  
**Mean turns per scenario**: 3.52 (23/50 scenarios hit step_cap; 32 failed on turn 1 with parse_error)  
**Per-turn timeout**: 120s (no timeouts observed)

## After-v0 Branch Decision

The L3 pass-rate is **0%** with **64% L0** (all parse_error). This falls squarely
into the "L0/L1 mostly red" bucket.

**Decision: Re-baseline against teacher model (Qwen2.5-Coder, not FunctionGemma)**

The reason: FunctionGemma is a 270M quantized model designed for on-device
inference. It was never expected to work without heavy prompt engineering,
and attempting to reverse-engineer the correct format has hit diminishing returns.

**Next steps:**
1. Update the baseline to use Qwen2.5-Coder (a larger, instruction-tuned model)
2. Teacher model should provide a solid anchor for post-v0 branch decisions
3. Once teacher baseline is established, decide whether to:
   - Keep FunctionGemma as a fallback/experimental arm (likely no)
   - Abandon the local-only constraint and use Anthropic (likely yes)
   - Pursue other approaches

---

**Run artifacts:**
- Rollouts: `data/posttraining/runs/v0-llamacpp-20260524_003947Z/rollouts.jsonl`
- Config: `data/posttraining/runs/v0-llamacpp-20260524_003947Z/config.yaml`
- Report: `data/posttraining/runs/v0-llamacpp-20260524_003947Z/report.md`
- Summary: `data/posttraining/runs/v0-llamacpp-20260524_003947Z/summary.json`

**Document updated:** 2026-05-23 (run completed, baseline failed)
