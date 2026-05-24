# mili-agent: Local LLM for griz Command Writing

Train and evaluate a local LLM to autonomously write griz visualization commands.

**Current Status (2026-05-24):** M1 & M2 complete. v0 baseline established (2% L3 pass rate). GEPA optimization shows prompt is not the limiting factor. Ready for architectural improvements or post-training.

---

## Three Milestones

**[M1: Baseline Design](m1-baseline-design.md)** — ✅ Complete  
Establish v0 baseline: FunctionGemma-270M on 50-scenario bootstrap set. 4-tier grading (L0: parse, L1: schema, L2: execute, L3: correct). Result: 2% L3 pass, 76% blocked by step_cap_hit. Infrastructure validated.

**[M2: GEPA Optimization](m2-gepa-optimization.md)** — ✅ Complete  
Evolve system prompt via iterative search. 5 runs show prompt is not limiting (identical scores despite different prompts). Bottlenecks: 42% dispatch_error, 40% step_cap_hit, 0% L3 completion. Confirmed: model capacity, not instructions, is the constraint.

**[M3: Post-Training Strategy](m3-posttraining-strategy.md)** — 🧪 Design Complete, Blocked  
Train a better model via SFT/RL without human labeling. Grammar, fixtures, and verifier provide zero-human-label data pipeline. Start only if quick wins (step_cap increase, larger model) plateau.

---

## Key Decision Doc

**[GEPA vs Post-Training](GEPA-vs-POSTTRAINING.md)**  
When to use prompt optimization vs. model training. Why GEPA alone hit a ceiling. Timeline and next decision points.

---

## What's Next

1. **This week:** Increase step_cap 8→12-16, test in 2-3 hours
2. **This week:** Try Claude API (--provider anthropic), test in 2-3 hours  
3. **If those plateau:** Analyze dispatch_error root causes or begin post-training

---

## Quick Run

GEPA optimization loop (finds best system prompt):
```bash
llama-server -hf ggml-org/functiongemma-270m-it-GGUF:BF16 &
uv run --directory python/mili-llm-bench mili-llm-bench run-gepa \
  --scenarios ../../data/posttraining/eval/bootstrap.jsonl \
  --provider llamacpp \
  --out ../../data/posttraining/gepa-runs/gepa-run-$(date +%Y%m%d-%H%M%S)
```

Results in `data/posttraining/gepa-runs/`. Check `best_result.json` for metrics.

---

## Related

- **CLAUDE.md** — Project setup and parity test instructions
- **`reference/baseline-setup-guide.md`** — Step-by-step environment setup
- **`reference/functiongemma-debug-report.md`** — Tool-calling failure analysis
- **`reference/`** — Historical docs and implementation details

---

**Last Updated:** 2026-05-24
