# mili-agent: Local LLM for griz command writing

Train and evaluate a local LLM to write griz visualization commands autonomously.

**Status (2026-05-25):** v1 SFT ships. Winner GGUF at **95.06 % L3** on the 81-row heldout. Active phase: [M6 client integration](m6-client-integration-v1.md).

---

## Results

| Run | Model | Eval set | L3 | Notes |
|-----|-------|----------|----|-------|
| v4 floor (pre-GEPA) | FunctionGemma-270M | bootstrap (50) | 32 % | Historical floor |
| v5 floor (post-GEPA) | FunctionGemma-270M | bootstrap (50) | 40 % | GEPA-promoted tool descriptions |
| v7 floor (canonical) | FunctionGemma-270M | bootstrap (50) | 26 % | `--jinja` path, rev-10 fallback |
| v4 ceiling | Claude Sonnet 4.5 | bootstrap (50) | 92 % | Pre-promotion tools |
| v7 ceiling | Claude Sonnet 4.5 | synth (175) | 97.7 % | Post-promotion tools |
| **v1 SFT winner** | functiongemma-v1 (SFT) | heldout (81) | **95.1 %** | HF and GGUF paths identical |

GEPA ran 100 iterations (4 × 25); converged at 40 % L3 — the prompt-engineering ceiling. System prompt never improved; only four tool descriptions (`clrsel`, `colormap`, `material`, `query`) moved the number. SFT closed the 52-point gap: **checkpoint-126** at 95.1 % L3 on the heldout split, 13/14 intents at 100 %. Four residual `select` semantic-disambiguation misses are the v2 paraphrase-multiplier target.

Artifacts: `data/posttraining/checkpoints/v1/winner → checkpoint-126` (HF) + `data/posttraining/checkpoints/v1/functiongemma-v1.bf16.gguf` (GGUF).

---

## Milestones

| Milestone | Status | Key finding |
|-----------|--------|-------------|
| [M1: Baseline Design](m1-baseline-design.md) | ✅ | Eval methodology established: bootstrap.jsonl (50 scenarios), L0–L3 grading, 4 failure-mode families |
| [M2: GEPA Optimization](m2-gepa-optimization.md) | ✅ | 100 iters → 40 % L3 ceiling; system prompt never moved; 4 tool descriptions rewritten and promoted |
| [M3: Post-Training Strategy](m3-posttraining-strategy.md) | ✅ superseded | Zero-human-label SFT design rationale; superseded as tracker by M5 |
| [M4: Client Integration](m4-client-integration-status.md) | ✅ | FunctionGemma wired via `LlamaCppAgent`; load-bearing gap: no dispatch feedback |
| [M5: SFT Pipeline](m5-sft-pipeline.md) | ✅ v1 ships | 175-scenario synth corpus → Claude teacher → H100 training → 95.06 % L3 GGUF |
| [M6: v1 Client Integration](m6-client-integration-v1.md) | 📋 planned | Swap model, fix prompt path, port rev-21 parser to Rust, unify system prompt |

---

## Quick commands

Reproduce the v7 FunctionGemma floor:

```bash
llama-server -hf ggml-org/functiongemma-270m-it-GGUF:BF16 --jinja &
uv run --directory python/mili-llm-bench mili-llm-bench run \
  --provider llamacpp \
  --scenarios ../../data/posttraining/eval/bootstrap.jsonl \
  --out ../../data/posttraining/runs/v7-repro-$(date +%Y%m%d-%H%M%S) \
  --step-cap 8 --per-turn-timeout-s 120 --max-new-tokens 256
```

Serve the v1 SFT winner (cluster only):

```bash
source scripts/setup-gpu-env.sh
llama-server \
  -m data/posttraining/checkpoints/v1/functiongemma-v1.bf16.gguf \
  --port 8080 --jinja
```

Reproduce the Claude ceiling (`ANTHROPIC_API_KEY` required):

```bash
uv run --directory python/mili-llm-bench mili-llm-bench run \
  --provider anthropic \
  --scenarios ../../data/posttraining/eval/bootstrap.jsonl \
  --out ../../data/posttraining/runs/claude-repro-$(date +%Y%m%d-%H%M%S)
```

---

## Files

**Active tracker:**
- [m5-sft-pipeline.md](m5-sft-pipeline.md) — single source of truth for SFT status, baselines, gates, changelog
- [m6-client-integration-v1.md](m6-client-integration-v1.md) — planned client wiring (model swap + parser port)

**SFT pipeline companions:**
- [_posttraining-dataset.md](_posttraining-dataset.md) — dataset construction plan (stage-by-stage build order)
- [_cluster-setup.md](_cluster-setup.md) — H100 cluster setup, training recipe, §6 launch instructions
- [_sft-preflight-gpu.md](_sft-preflight-gpu.md) — pre-flight checklist (all 6 checks cleared for v1)

**Completed milestones:**
- [m1-baseline-design.md](m1-baseline-design.md), [m2-gepa-optimization.md](m2-gepa-optimization.md), [m3-posttraining-strategy.md](m3-posttraining-strategy.md), [m4-client-integration-status.md](m4-client-integration-status.md)

**Background / reference (`reference/`):**
- `agent-local-llm*.md` — early design docs (pre-milestone)
- `baseline-setup-guide.md` — step-by-step environment setup
- `functiongemma-debug-report.md` — tool-calling failure analysis
- `gepa-runs-summary.md` — per-run GEPA data (all 4 runs, failure-mode breakdown)
- `gepa-vs-posttraining.md` — conceptual guide: when to use GEPA vs. post-training

---

**Code pointers:**
- `python/mili-llm-bench/src/mili_llm_bench/verifier.py` — L0–L3 grader
- `python/mili-llm-bench/src/mili_llm_bench/schemas.py:TOOL_DESCRIPTIONS` — GEPA-promoted tool descriptions
- `crates/mili-viz-server/src/llamacpp_agent.rs` — M4 client agent (M6 target)
- `data/posttraining/eval/bootstrap.jsonl` — 50-scenario bootstrap eval (do not edit without re-pinning baselines)

**Last Updated:** 2026-05-25
