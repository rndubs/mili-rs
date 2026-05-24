# M3 — Post-Training Strategy (Model Training Pipeline)

**Status:** 🧪 Exploratory — Design complete, ready to begin if quick wins plateau.

**Date:** 2026-05-24  
**Summary:** Long-term approach to train a better model via SFT/RL without human labeling. Data pipeline design verified; ready for implementation.

---

## When to Use This

Use post-training when:
1. Quick wins (step_cap increase, larger model) don't unlock major improvements
2. GEPA results show model capacity is the bottleneck (confirmed in M2)
3. You're willing to spend 1–2 weeks on data generation + training
4. You want a deployable model (not reliant on Claude API)

**Current status:** Conditions 1–2 not yet met. GEPA is still running; we haven't tried step_cap increase or Claude API yet. This document is here for planning purposes.

---

## The Problem We're Solving

From M1 and M2, we know:
- **FunctionGemma-270M has low success rate:** 2% L3 pass rate
- **Prompt optimization hits a ceiling:** Different prompts → same score (0.2533)
- **Bottlenecks are architectural:** 42% dispatch_error, 40% step_cap_hit
- **Lesson:** We need a better model, not better instructions

**Post-training goal:** Generate high-quality training data from our existing infrastructure (grammar, fixtures, verifier) and fine-tune a model to be better at command writing.

---

## Why This Works: Zero Human Labeling

The key insight: **we have machine oracles for the entire training pipeline.**

| Stage | Oracle | Source |
|-------|--------|--------|
| **Grammar** | `interpret.c` dispatch chain | griz source code |
| **Validity** | `parse_command()` function | griz source code |
| **Executability** | griz dispatcher + fixtures | mili-viz-server + test data |
| **Correctness** | Fixture facts + postconditions | existing test suite |

**Net:** We can generate training data, grade it, and optimize the model **without writing a single labeled example.**

---

## Pipeline Design (5 Stages)

### Stage 1: Grammar & Intent Extraction

**Input:** `reference/griz/Src/interpret.c` + `griz_manual.pdf` + `usage_text[]`

**Process:**
1. Parse `interpret.c`'s `strcmp(tokens[0], "...")` dispatch chain → extract keyword list
2. Extract aliases (e.g., `quit|done|exit|end`)
3. Extract argument arity per command
4. Pull command descriptions from `griz_manual.pdf` + usage strings
5. Build (intent prose ↔ canonical command) table

**Output:**
- `grammar.json` — keyword/alias/arity table
- `intents.jsonl` — (intent prose, canonical command) pairs
- GBNF grammar artifact (for constrained decoding)

**Cost:** ~2–4 hours (one-time, reusable)

### Stage 2: Scenario Synthesis

**Input:** Grammar, intents, fixtures (`reference/mili/test/xmilics/`)

**Process:**
1. For each fixture (d3samp6, cylinder, ml40, bar1, …) and each command/intent:
   - Generate paraphrased user requests (template + light LLM paraphrase)
   - Ground targets in fixture facts (real material IDs, state counts)
   - Create (instruction, fixture, postcondition) triples

**Output:** `scenarios.jsonl` — ~500–1000 synthetic scenarios (much larger than bootstrap)

**Cost:** ~4–8 hours (depends on number of fixtures and paraphrase diversity)

### Stage 3: Teacher Rollouts

**Input:** Scenarios, grammar, verifier

**Process:**
1. For each scenario:
   - Have Claude (or larger local model) propose N candidate command sequences
   - Run each through verifier (L0–L3 grading)
   - Keep all sequences (will be filtered by rejection sampling later)

**Output:** `rollouts.jsonl` — (scenario, proposed_sequence, tier, failure_mode) records

**Cost:** ~$1–10 depending on number of scenarios and N candidates. Cacheable (one-time).

### Stage 4: Rejection-Sampling SFT Data

**Input:** Rollouts, verifier

**Process:**
1. Filter rollouts: keep only L2+ (ideally L3) sequences
2. Format as (instruction → command_sequence) pairs
3. Create balanced train/val split
4. Deduplicate

**Output:** `sft_data.jsonl` — ~100–500 high-quality training pairs

**Cost:** Free (deterministic filtering)

### Stage 5: Fine-Tuning & RL (Optional)

**Input:** SFT data, base model (FunctionGemma or Qwen)

**Process:**

#### 5a: Supervised Fine-Tuning (SFT)
```bash
# QLoRA fine-tune on single GPU
python train_sft.py \
  --model_id ggml-org/functiongemma-270m-it-GGUF \
  --data sft_data.jsonl \
  --epochs 3 \
  --lora_r 16
```

**Time:** ~4–8 hours (single GPU)  
**Cost:** ~$0–2 (cloud GPU time)

#### 5b: Optional RL (DPO/GRPO)
If SFT plateaus, add preference learning:
- **DPO (Direct Preference Optimization):** Pair L3-pass vs. L1/L2-fail sequences
- **GRPO (Graded Reward Policy Optimization):** Use verifier's tier-based reward

**Time:** ~2–4 hours (single GPU)  
**Cost:** ~$0–1 (cloud GPU time)

**When to use:** Only if SFT doesn't reach 10%+ L3 pass rate on held-out set.

### Stage 6: Evaluation

**Input:** Trained model, held-out scenarios

**Process:**
1. Run eval loop on held-out test set (same verifier as baseline)
2. Compare L3 pass rate to v0 baseline (2%)
3. Success criteria: >5% L3 pass rate (2.5x improvement) or >20% L2+ reach

**Output:** Model checkpoint + evaluation report

---

## Why This Is Now Low-Risk

1. **Zero human labeling.** Grammar, validity, execution, postconditions are all machine oracles.

2. **One thing to build.** Teacher filter, RL reward, and eval are the same verifier. Most engineering is grammar extraction + fixture-execution harness — both reusable by main agent regardless of outcome.

3. **Graceful failure.** If the small model underperforms, we already have a fallback: use Claude API directly (--provider anthropic).

4. **Honest unknown.** We don't know if synthetic intent diversity (from template + teacher paraphrase) is good enough. The first experiment is: **pre-test grammar-constrained decoding with stock 0.5–1B model (no fine-tune) and measure L3 on eval set.** If it clears the bar, post-training is moot for v1.

---

## Implementation Checklist

### Pre-Implementation (Now)
- [ ] Confirm step_cap increase doesn't unlock major gains (Option 1, M2)
- [ ] Confirm Claude API doesn't solve it (Option 2, M2)
- [ ] Decision: "Yes, we need post-training"

### Stage 1: Grammar Extraction (~2–4 hours)
- [ ] Script to parse `interpret.c` dispatch chain
- [ ] Extract aliases + arity
- [ ] Pull descriptions from `griz_manual.pdf` + usage_text[]
- [ ] Output grammar.json + intents.jsonl
- [ ] Test: Re-derive grammar, diff against baseline (CI gate)

### Stage 2: Scenario Synthesis (~4–8 hours)
- [ ] Load intents + fixtures
- [ ] Generate template-based user requests
- [ ] Paraphrase via Claude (or skip for v0 and just use templates)
- [ ] Ground postconditions in fixture facts
- [ ] Output scenarios.jsonl

### Stage 3: Teacher Rollouts (~$1–10, varies)
- [ ] For each scenario: have Claude propose N sequences
- [ ] Run through verifier
- [ ] Output rollouts.jsonl

### Stage 4: Rejection Sampling (~1 hour)
- [ ] Filter rollouts: keep L2+
- [ ] Format as (instruction, sequence) pairs
- [ ] Train/val split + dedupe
- [ ] Output sft_data.jsonl

### Stage 5: Fine-Tuning (~4–8 hours)
- [ ] Set up training environment (huggingface transformers, peft for QLoRA)
- [ ] Train on sft_data.jsonl
- [ ] Save checkpoint

### Stage 6: Evaluation (~30 min)
- [ ] Run eval loop on held-out scenarios
- [ ] Compare L3 pass rate to v0 baseline (2%)
- [ ] Decision: Keep model, or iterate

---

## Expected Outcomes

### Conservative Estimate
- **L3 pass rate:** 5–10% (2.5–5x improvement)
- **Training time:** 1–2 weeks (including experimentation)
- **Model size:** 0.5–1.5B parameters (deployable)
- **Cost:** ~$5–20 (teacher rollouts + GPU time)

### Optimistic Estimate
- **L3 pass rate:** 15–25% (7.5–12.5x improvement)
- **Training time:** 1 week (smooth pipeline)
- **Model size:** Same (1.5B max)
- **Cost:** Same

### What Could Go Wrong
- Synthetic intent diversity is too narrow (template collapse)
- Grammar extraction misses edge cases
- Teacher model (Claude) generates low-quality rollouts
- Model underfits on edge cases despite good in-distribution performance

**Mitigation:** Each stage outputs artifacts that can be inspected. If Stage 2 looks bad, abort before investing in Stage 3.

---

## Timeline

**Option A: Start immediately (if quick wins plateau)**
- Week 1: Stages 1–2 (grammar extraction, scenario synthesis)
- Week 2: Stage 3 (teacher rollouts, rejection sampling)
- Week 3: Stages 5–6 (fine-tuning, evaluation)
- **Total:** 2–3 weeks

**Option B: Start after trying quick wins (likely)**
- This week: Step_cap increase + Claude API tests
- Next week: Decision on post-training
- If yes: 2–3 weeks starting then

---

## Key Files & References

- **`reference/griz/Src/interpret.c`** — Grammar oracle (11k lines)
- **`reference/griz/Src/viewer.c`** — Usage strings
- **`reference/griz/Src/Doc/griz_manual.pdf`** — NL descriptions
- **`reference/mili/test/xmilics/`** — Test fixtures
- **`python/mili-llm-bench/src/mili_llm_bench/verifier.py`** — Grading oracle (reused as-is)
- **`data/posttraining/eval/bootstrap.jsonl`** — Bootstrap scenarios (reference)

---

## Open Questions (Don't Resolve Yet)

These questions will be answered during implementation:

1. Can `interpret.c`'s dispatch be parsed robustly, or is it too irregular?
2. How much does L3 need to carry? Is fixture coverage wide enough?
3. What's the teacher cost at full rollout volume?
4. Does grammar-constrained decoding alone make fine-tune unnecessary?
5. Can we source real `grizinit` files (LLNL/site users) for authentic sequences?

---

## Decision Tree

```
Are quick wins (M2 Option 1–2) sufficient?
├─ YES → STOP, deploy improved model/prompt
└─ NO → GEPA hits ceiling
      │
      └─ Start post-training pipeline
         ├─ Stage 1–2 slow/painful?
         │  └─ YES → Abort, use Claude API directly
         └─ Stage 3 rollouts low quality?
            └─ YES → Adjust teacher, retry
         └─ Stage 5–6 low L3 gain?
            └─ YES → Try larger base model, retry
                   OR abort and use Claude API
```

---

## Related Documents

- **README.md** — Entry point and quick start
- **GEPA-vs-POSTTRAINING.md** — Explains why post-training is needed and when to use it
- **m1-baseline-design.md** — Baseline methodology (post-training measures against this)
- **m2-gepa-optimization.md** — Why prompt alone isn't enough
- **reference/functiongemma-debug-report.md** — Earlier debug findings

---

**Last Updated:** 2026-05-24  
**Status:** Design complete, implementation blocked on quick-wins testing  
**Next Decision Point:** After trying step_cap increase + Claude API comparison
