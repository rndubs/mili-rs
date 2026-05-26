# GEPA vs Post-Training: What's the Difference?

Quick guide to understand when to use each approach and how they fit together.

---

## One-Sentence Summary

- **GEPA:** Find a better system prompt for the current model (fast, no training)
- **Post-Training:** Train a better model on better data (slow, but better long-term results)

---

## Side-by-Side Comparison

| Aspect | GEPA | Post-Training |
|--------|------|---------------|
| **What it optimizes** | System prompt (LLM instructions) | Model weights (trained from data) |
| **Time to run** | 4-5 hours per iteration | Days to weeks (depends on data size) |
| **What you get back** | Better system prompt (text file) | Better model (checkpoints) |
| **Can it fix tool-calling bugs?** | No (prompt can't fix model bugs) | Yes (model learns from data) |
| **Can it improve reasoning?** | Slightly (better instructions) | Yes (model learns strategies) |
| **Upfront cost** | $0-100 (GEPA's optimizer calls Claude) | ~$0 (teacher model is Claude, but one-time) |
| **Can reuse results?** | Yes (prompt works with any model) | Yes (model can be deployed anywhere) |
| **Dependency on data** | Low (no training data needed) | High (quality data == quality model) |
| **Status** | ✅ Deployed, running now | 🧪 Exploratory, not started |

---

## The Workflow

```
Current State:
┌──────────────────────────┐
│ FunctionGemma-270M       │
│ + system prompt (v0)     │
│ = 2% L3 pass rate        │
└──────────────────────────┘

↓ Try this NOW
→ GEPA (prompt optimization)
  · Takes 4-5 hours
  · Output: slightly better prompt
  · Expected: +1-2% on L3 (if limited by prompt)
  · Reality: Found prompt is NOT the limit (0% → 0%)
  ↓

Decision Point:
┌──────────────────────────┐
│ If prompt is not limiting:        │
│ Do post-training instead          │
├──────────────────────────┤
│ Steps:                            │
│ 1. Generate training data         │
│    (teacher model writes examples)│
│ 2. SFT (supervised fine-tune)     │
│ 3. Evaluate new model             │
│ 4. Optional: DPO (preference opt) │
│ Time: days to weeks               │
└──────────────────────────┘
```

---

## When to Use Each

### Use GEPA When:

1. ✅ You have a model that's mostly working but the prompt could be better
2. ✅ You want results in hours, not days
3. ✅ You're not sure if the problem is the prompt or the model
4. ✅ You want to explore the design space quickly (5 iterations = different approaches)
5. ✅ The failure modes suggest instruction-following issues

**Example:** "The model understands the task but needs clearer examples in the prompt."

### Use Post-Training When:

1. ✅ You have clear evidence the model is the bottleneck (GEPA didn't help)
2. ✅ You have a way to generate training data (we do: grammar + teacher rollouts)
3. ✅ You're willing to wait days for results
4. ✅ You want a model that works better across ALL prompts, not just this one
5. ✅ The failure modes suggest the model can't do the task (tool-calling bugs, reasoning gaps)

**Example:** "The model can't call tools correctly; this needs training data to fix."

---

## Current Status (2026-05-24)

### Phase 1: GEPA (NOW ✅)
- **What happened:** Ran 5 GEPA iterations on system prompt
- **Result:** No improvement (0.2533 → 0.2533)
- **Lesson:** System prompt is NOT the limiting factor
- **Bottleneck:** 42% tool-calling errors (dispatch_error), 40% step budget exceeded (step_cap_hit)
- **Conclusion:** Need a better model, not a better prompt

### Phase 2: Quick Wins (THIS WEEK ⏳)
- Increase `step_cap` from 8 to 12-16 (may unlock 5-10%)
- Try Claude API instead of FunctionGemma (baseline comparison)
- Analyze specific dispatch_error cases

### Phase 3: Post-Training (NEXT SPRINT 🔮)
- If Phase 2 doesn't unlock major improvements, begin post-training
- Generate training data (teacher model proposes sequences)
- SFT on FunctionGemma-270M or larger base model
- Evaluate on held-out scenarios

---

## Why We Tried GEPA First

1. **Fast iteration:** Results in hours, not days
2. **Diagnostic value:** Proved prompt is not the problem
3. **No data needed:** GEPA works with zero labeled examples
4. **Gives us a baseline:** Now we know what GOOD looks like (score > 0.25)

**Result:** We learned that we need to improve the model (training) not the instructions (prompting).

---

## Data Flow: GEPA

```
System Prompt (text)
    ↓
GEPA Optimizer (Claude)
    ↓ proposes variant prompts (iteration 1, 2, 3...)
    ↓
Evaluate Each Prompt:
    ├─ Load 50 scenarios
    ├─ Run FunctionGemma with THIS prompt
    ├─ Grade (L0: parse, L1: valid, L2: execute, L3: correct)
    └─ Return score
    ↓
Best Prompt Found (goes to best_artifact.txt)
    ↓
✅ Done (can use immediately)
```

**Key:** Same model, different prompt. No training data needed.

---

## Data Flow: Post-Training

```
Grammar (from interpret.c)
    + Usage strings (from source)
    + Manual (from griz_manual.pdf)
    + Fixtures (test scenarios)
    ↓
Scenario Synthesis
    (Create pairs: intent + fixture state)
    ↓
Teacher Model (Claude API)
    (Propose command sequences)
    ↓
Verifier (L0-L3 grading)
    ├─ Filter for L2+ examples (valid & executable)
    └─ Generate preference pairs
    ↓
Training Data (JSONL)
    ↓
SFT (QLoRA)
    (Fine-tune FunctionGemma on good examples)
    ↓
New Model Checkpoint
    ↓
Evaluate New Model
    (Same 50 scenarios, see if L3 pass rate improved)
    ↓
✅ Done (deploy new model)
```

**Key:** Same prompt, better model. Requires training data generation.

---

## Why Prompt Alone Didn't Work

From `gepa-integration-status.md` § "Critical Finding":

**Run 1 vs Run 2:**
- Run 1: Explicit JSON format examples + type guidance → Score 0.2533
- Run 2: Highly structured CORE LOOP + anti-patterns + tool cheat sheet → Score 0.2533

**Same score despite radically different prompts means:**
- The prompt is not the bottleneck
- The model itself has limitations (can't execute the task correctly)
- We need better training data, not better instructions

**Breakdown of failures:**
- 42% `dispatch_error` — Tool execution bugs (model writes wrong parameters)
- 40% `step_cap_hit` — Model runs out of steps (needs better strategy or larger model)
- 0% `L3_pass_rate` — No scenarios complete end-to-end

**None of these are fixable by prompt alone.**

---

## What Should You Do Right Now?

### If you want to improve the agent TODAY:

1. **Try increasing step_cap** (5-min change, 2-hour test)
   - In `python/mili-llm-bench/src/mili_llm_bench/schemas.py`, change:
     ```python
     step_cap: int = 12  # was 8
     ```
   - Run a baseline with `--step-cap 12`
   - See if dispatch_error or step_cap_hit rates improve

2. **Try Claude API** (no code change needed, 2-hour test)
   - Same command but with `--provider anthropic`
   - Compare against FunctionGemma baseline
   - See if a better model has better success rate

### If step_cap increases don't help AND Claude API doesn't help:

**Then start post-training** (days of work, but high ROI):
- Generate training data (grammar + teacher rollouts)
- Fine-tune a larger model (or FunctionGemma-1B)
- Verify L3 pass rate improves

---

## FAQ

**Q: Can I run GEPA and post-training in parallel?**  
A: Yes, but GEPA is faster to debug. Do GEPA first, then decide if post-training is needed.

**Q: What if GEPA had worked (found a better prompt)?**  
A: Then we'd keep that prompt, maybe combine with post-training later. But it didn't, so we move to plan B.

**Q: Why not just use Claude API directly?**  
A: We're trying to build a small, deployable model. Claude API is great for testing/comparison, but we want a 270M parameter model for resource efficiency.

**Q: How do I know when post-training is worth it?**  
A: When:
1. Simple fixes (step_cap increase, prompt tuning) don't work
2. You have 1-2 days to spare on training
3. The failure modes are model-specific (dispatch_error, reasoning gaps)

**Q: Can I reuse the training data across models?**  
A: Yes! Training data is model-agnostic. SFT data from teacher rollouts works for any base model.

**Q: What if post-training also doesn't work?**  
A: Use a larger base model (FunctionGemma-1B, LLaMA-405B) and re-train. Or use Claude API directly (not a local model).

---

## Timeline

| Date | Work | Status | Decision Point |
|------|------|--------|-----------------|
| 2026-05-23 | GEPA runs 1-5 | ✅ Complete | Prompt is not the limit |
| 2026-05-24 | Step_cap increase test | ⏳ Pending | Do we unlock 5-10%? |
| 2026-05-24 | Claude API comparison | ⏳ Pending | Is model the bottleneck? |
| 2026-05-25 | Dispatch_error analysis | ⏳ Pending | Root cause analysis |
| **2026-05-26** | **Decision:** Post-training or other | 🔮 | Start training pipeline if needed |

---

## Key Takeaway

**GEPA is the first step to confirm the diagnosis. We've confirmed: the model, not the prompt, is limiting performance.**

Next steps are either:
1. **Short-term:** Increase step budget or try a better model (no training needed)
2. **Long-term:** Train a better model on better data (post-training)

The good news: We have a clear, data-driven roadmap. The bad news: Quick wins might be limited. The opportunity: Post-training gives us full control over model behavior.

---

**See Also:**
- [gepa-integration-status.md](gepa-integration-status.md) — Current GEPA results
- [agent-local-llm-posttraining.md](agent-local-llm-posttraining.md) — Post-training pipeline design
- [README.md](README.md) — Quick start and next actions
