# mili-agent: Local LLM for griz Command Writing

Train and evaluate a local LLM to autonomously write griz visualization commands.

**Current Status (2026-05-24):** M1, M2, M4 complete. GEPA prompt ceiling
reached at 40 % L3 (FunctionGemma-270M floor). Claude Sonnet 4.5 ceiling
is 92 % L3. **SFT is now the active phase** — see [M5](m5-sft-pipeline.md).

---

## Where to start

**[M5: SFT Pipeline (live tracker)](m5-sft-pipeline.md)** — 🚧 Active.
The single entry point for SFT progress: pinned baselines, stage
status, v1 narrow-scope decisions, scale-up backlog, multi-step
coverage plan, gate thresholds.

The other documents in this directory are historical milestone
records; they pin past decisions and do not move once a milestone
completes. Read them for context, not for current status.

---

## Milestone history

**[M1: Baseline Design](m1-baseline-design.md)** — ✅ Complete
Establish v0 baseline methodology: FunctionGemma-270M on 50-scenario
bootstrap set, 4-tier grading (L0–L3). v0 absolute numbers later
invalidated by the M1-stub-fallback bug (fixed 2026-05-24); the
*methodology* survives, the original 2 % number does not. The
post-fix floor is 32 % (v4) / 40 % (v5 with GEPA-promoted tools).

**[M2: GEPA Optimization](m2-gepa-optimization.md)** — ✅ Complete
Evolutionary system-prompt search. 100 iterations (4 × 25-iter runs)
converged at **40 % L3**: prompt itself was unimproved, but four
tool descriptions (`clrsel`, `colormap`, `material`, `query`) were
promoted into `schemas.py:TOOL_DESCRIPTIONS`. Prompt engineering
alone cannot close the 52-point gap to the Claude ceiling — that is
SFT's job.

**[M3: Post-Training Strategy](m3-posttraining-strategy.md)** —
✅ Strategy locked, **superseded as a tracker by [M5](m5-sft-pipeline.md)**.
Kept for the design rationale (why zero-human-label data works, why
the verifier doubles as filter + reward + eval).

**[M4: Client Integration Status](m4-client-integration-status.md)** —
✅ Complete. FunctionGemma wired into the griz client via
`LlamaCppAgent` → `llama-server`. End-to-end signal path verified.

**[M5: SFT Pipeline](m5-sft-pipeline.md)** — 🚧 Active.
The live execution tracker. See above.

---

## Decision docs

**[GEPA vs Post-Training](GEPA-vs-POSTTRAINING.md)** — when to use
prompt optimization vs. model training, why GEPA hit a ceiling,
where SFT picks up. (Status section is intentionally lagging the M5
tracker; treat M5 as authoritative for *current* state.)

---

## Quick runs

Reproduce the v5 FunctionGemma floor:

```bash
llama-server -hf ggml-org/functiongemma-270m-it-GGUF:BF16 &
uv run --directory python/mili-llm-bench mili-llm-bench run \
  --provider llamacpp \
  --scenarios ../../data/posttraining/eval/bootstrap.jsonl \
  --out ../../data/posttraining/runs/v5-repro-$(date +%Y%m%d-%H%M%S) \
  --step-cap 8 --per-turn-timeout-s 120 --max-new-tokens 256
```

Reproduce the Claude ceiling (`ANTHROPIC_API_KEY` required):

```bash
uv run --directory python/mili-llm-bench mili-llm-bench run \
  --provider anthropic \
  --scenarios ../../data/posttraining/eval/bootstrap.jsonl \
  --out ../../data/posttraining/runs/v4-anthropic-repro-$(date +%Y%m%d-%H%M%S)
```

GEPA optimization loop (best artifact already promoted into
`schemas.py`; re-run only when adding new tools):

```bash
uv run --directory python/mili-llm-bench mili-llm-bench run-gepa \
  --scenarios ../../data/posttraining/eval/bootstrap.jsonl \
  --provider llamacpp \
  --out ../../data/posttraining/gepa-runs/gepa-run-$(date +%Y%m%d-%H%M%S)
```

---

## Related

- `python/mili-llm-bench/src/mili_llm_bench/verifier.py` — L0–L3 grader
- `python/mili-llm-bench/src/mili_llm_bench/gepa_integration.py` — GEPA loop
- `crates/mili-viz-server/src/llamacpp_agent.rs` — M4 client agent
- CLAUDE.md — project setup, parity test instructions
- `reference/baseline-setup-guide.md` — step-by-step environment setup
- `reference/functiongemma-debug-report.md` — tool-calling failure analysis
- `reference/` — historical docs and implementation details

---

**Last Updated:** 2026-05-24
