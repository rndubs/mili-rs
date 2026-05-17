# `mili-viz` — local lightweight LLM for command generation (optional, exploratory)

**Status: optional / not a priority / exploration-first.** This is a
*future* feature, not on the README/`client.md` critical path. It
refines two existing hooks in `client.md`: decision 4's `LlmProvider`
trait, and the "Offline model bar" open question. Nothing here should
be built until the agent loop (`client.md` Phase 5 M6) is stable and
the command vocabulary has settled. Revisit this doc before doing the
work — the numbers and model choices below are estimates to be
validated, not commitments.

Read `client.md` (the server-hosted agent, decision 4) and
`scripting.md` (Layer-0/Layer-1, the closed command vocabulary) first.
This doc obeys both.

## Scope: a command writer, not a replacement agent

`client.md`'s headline feature is an *autonomous, query-first
debugging agent*: multi-step tool use, scanning for NaN/Inf, reading
`Query` results back, deciding when to `Snapshot`, reasoning about the
simulation. A sub-1B model **does not** do that loop well — small
models are weak at long-context tool orchestration and multi-step
planning. That stays with the Claude-API backend (decision 4) or a
larger local model.

What a tiny fine-tuned model *can* do is the narrow slice: natural
language → a sequence of Layer-0 griz commands (closed ~321-command
vocabulary) or Layer-1 `griz` Python calls (small, regular API). This
is constrained NL→DSL translation, which is tiny-model-friendly,
especially with grammar-constrained decoding.

So the design is **two-tier behind the existing `LlmProvider` seam**:

- **Tiny local model** — the offline "command writer" / fast path for
  "translate this request into griz commands." No API key, runs on the
  cluster, the answer to the "Offline model bar" open question *scoped
  to command generation*.
- **Full reasoning agent** — Claude API (decision 4) or a larger local
  model for the autonomous debugging loop.

A small router (heuristic or a classifier) decides "pure translation"
(tiny model) vs. "needs reasoning/tools" (full agent). The tiny model
is a capability *fallback*, not a swap-in for the agent.

## Decisions (proposed, to revisit)

1. **Server-hosted, consistent with `client.md` decision 1.** The
   model ships/runs with `mili-viz-server` on the login/compute node,
   not the `egui` client. "Distribute with the client" only literally
   applies to the `griz.launch()` local-default mode.
2. **Rust stack: Candle behind `LlmProvider`.** Pure-Rust (no
   C++/Python/venv), single binary + a weights file, CPU/CUDA/Metal,
   quantized GGUF. Best distribution + air-gap story. `llama-cpp-2` is
   the alternative if we want llama.cpp's GBNF grammar-constrained
   decoding out of the box (we likely do — see below).
3. **Grammar-constrained decoding is mandatory, not optional.** The
   output is near-formal; constraining generation to the griz
   command/`griz`-API grammar (GBNF or logit masking) means the model
   only has to get *intent mapping* right, not syntax. This is what
   collapses the model-size requirement.
4. **CPU-only target.** Sub-1B 4-bit runs at tens of tok/s on a server
   CPU; outputs are short (~50–300 tokens); <1–2 GB RAM incl. KV
   cache. HPC login nodes are CPU-fat and GPU-contended — CPU-only is
   the natural fit. GPU only if we later scale to multi-billion params
   or high session concurrency.
5. **Model size: start at 0.5B–1.5B, fine-tuned.** Qwen2.5-0.5B/1.5B,
   Llama-3.2-1B, Gemma-class. ~0.3–0.7 GB at 4-bit on disk. Below
   ~0.3B is plausible for the tightest grammar-constrained slice but
   brittle on paraphrase variety — treat as a stretch experiment, not
   the plan.

## Post-training: SFT then (optionally) RL, grounded on fixtures

The key enabler: **outputs are automatically verifiable**. A candidate
command sequence either parses against the grammar and *runs* against
a real fixture session, or it doesn't. That gives a cheap reward
signal and a clean SFT-data filter without human labeling.

### Data sources we already have

- **Command/API schema** — the proto command vocabulary +
  `scripting.md` Layer-0/Layer-1 surface. This is the grammar and the
  space of valid outputs.
- **Fixture databases** — `reference/mili/test/xmilics/{d3samp6,
  cylinder, ml40, bar1, ...}` `.plt` runs. These are *executable
  environments*: load one, run candidate commands, observe whether the
  session reaches the intended state / returns sane `Query` data.
- **The `mili`/mili-rs test suite** — exercises the result/query API
  and pins expected values; a source of realistic "ask → expected
  data" anchors and of regression-grade verification.
- **Griz source (`reference/griz`, now checked out).**
  `Src/interpret.c`'s `parse_command()` is the command vocabulary as a
  `strcmp` dispatch — a mechanically-extractable grammar *and* a free
  validity oracle; `Src/viewer.c` `usage_text[]` and
  `Src/Doc/griz_manual.pdf` are a natural-language command corpus.
  This **supersedes the earlier assumption of "no natural corpus"**:
  the training set is still synthesized + execution-filtered, but it
  is *grounded* in real command descriptions, not invented from
  schema alone.

> **Deep dive:** the full post-training design — graded verifier
> tiers, grammar extraction from `interpret.c`, teacher-rollout loop,
> SFT→DPO/GRPO, and a no-fine-tune pre-experiment — is in
> `agent-local-llm-posttraining.md`. The sketch below is the summary.

### Minimal pipeline (sketch — revisit before building)

1. **Schema → seed grammar.** Generate the GBNF/grammar from the proto
   command set + Layer-1 API. This is reused at inference (decision 3)
   and to validate generated data.
2. **Teacher rollouts.** Use a larger model (Claude API, or a strong
   local 7–14B) as a *teacher*. Programmatically enumerate task
   intents over the fixtures ("disable material 3 then frame it",
   "find the state where sx peaks on bricks 1–100", paraphrased N
   ways) and have the teacher emit candidate griz command sequences.
   This is the "use existing test data to generate rollouts" step: the
   fixtures define the executable scenarios; the teacher proposes
   solutions.
3. **Execution filter (the reward).** Run each candidate against the
   actual fixture session via the server's command dispatch. Keep only
   rollouts that (a) parse, (b) execute without error, and (c) reach a
   checkable post-condition (expected state index, result range,
   query-value match against the test suite's known answers). This
   turns noisy teacher output into a clean, *verified* SFT set —
   rejection sampling.
4. **SFT.** QLoRA fine-tune the 0.5–1.5B base on the verified
   (instruction → verified command sequence) pairs. Single consumer
   GPU, hours. This alone likely covers most of the value.
5. **RL only if SFT plateaus.** If SFT leaves a gap on
   compositional/free-form requests, do a lightweight preference or
   policy step (DPO from execution-pass vs. -fail pairs we already
   have for free from step 3; or GRPO/PPO with the step-3 execution
   check as the reward). RL is explicitly *phase 2 of phase 2* — only
   if measured need.
6. **Eval harness.** A held-out set of (intent, fixture,
   post-condition) triples; metric = execution-verified success rate
   under grammar-constrained decoding. Same machinery as step 3, so
   eval is nearly free once that exists.

### Why this is cheap and low-risk to *explore*

- No human labeling: the fixtures + test suite are the oracle.
- The teacher, the data filter, and the eval harness are the *same*
  execution check — build it once.
- Every artifact (grammar, harness) is reusable by the main agent's
  testing even if the tiny-model idea is dropped.
- Failure mode is graceful: if the tiny model underperforms, the
  router just sends more traffic to the full agent (decision 4
  already handles the no-local-model case).

## Open questions (do not resolve until exploration)

- Is there enough *intent diversity* to synthesize without a natural
  corpus, or does teacher-generated data collapse to a narrow style?
- Router design: heuristic, a tiny classifier, or "let the small model
  try and escalate on grammar/exec failure"?
- Does grammar-constrained decoding alone (no fine-tune) on a stock
  0.5–1B model already clear the bar, making post-training moot for
  v1?
- How much does paraphrase robustness actually need >0.5B?
- Token-cost / latency budget for teacher rollouts at the data scale
  we'd need.
