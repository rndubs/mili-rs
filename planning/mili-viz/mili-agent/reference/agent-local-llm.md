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
language → a sequence of structured calls into the griz surface. That
slice is constrained NL → action translation, which is
tiny-model-friendly with constrained decoding. The choice of *which*
projection of the griz surface the model emits — raw Layer-0 DSL,
typed `Command` tool calls, or free-form pygriz Python — is decided
in "Surface choice" below; the rest of this doc assumes the typed
tool-call surface that section settles on.

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

## Surface choice: typed tool calls, not raw DSL or free-form Python

Griz exposes the *same* operations through three projections:

1. **Layer-0 raw DSL** — `disable mat 3; rfit` strings, parsed by
   `interpret.c`'s `parse_command` dispatch (~318 keywords; see
   `posttraining-dataset.md` Stage 1).
2. **Layer-1 typed `Command`** — the `mili-viz-proto` `Command` oneof:
   ~15 variants (`load`, `set_state`, `step`, `select`, `clrsel`,
   `show`, `iso`, `contour`, `material`, `cutplane`, `colormap`,
   `legend`, `view`, `named_view`, plus `raw`) with `Query` and
   `Subscribe` as the read paths. This is also what `pygriz`'s
   `Session` already lowers every typed call to
   (`python/pygriz/src/griz/__init__.py`).
3. **Layer-1 Python (pygriz)** — `s.materials.disable(mat=3);
   s.view.reset()` against the `Session` object directly.

**Decision: the tiny model emits surface (2)** — one JSON-schema tool
per typed `Command` oneof variant, plus `query`/`snapshot` as read
tools, plus a small handful of *analysis macros* (e.g. `query_extreme`,
`scan_states`) implemented server-side as pygriz functions, plus
`griz_raw(line: str)` as the long-tail fallback. Approximate
inventory:

- ~15 typed-`Command` tools (one per oneof variant),
- ~2 read tools (`query`, `snapshot`),
- ~3 analysis macros (TBD; see "tool inventory" question below),
- 1 fallback (`griz_raw`).

**≈ 20 tools total** — inside the tested footprint for
FunctionGemma-class (270M) and APIGen-MT / xLAM / ToolACE-class (1B–3B)
function-calling models. Crucially, the long tail of ~318 griz
keywords does **not** inflate the schema; the model just learns to
fall back to `griz_raw` when the typed tools don't cover the request.

### Why typed tools, not raw DSL alone

The original Decision 3 below targeted "griz command lines under GBNF
constraint". That works for single-turn translation, but does not
naturally express *multi-step plans whose later steps depend on
intermediate values* — e.g. "find the state where `sx` peaks on bricks
1–100, then frame it and show `evm`". Tool calling's
`function_call → function_response → next function_call` protocol is
exactly what FunctionGemma's chat template (and APIGen-MT, ToolACE,
Magnet, xLAM training data) is shaped to. Surface (2) gets multi-turn
intermediate-value chaining for free; surface (1) does not.

### Why not free-form Python

For a 270M–1.5B model, free-form Python over the `Session` object is
the wrong target:

- No equivalent of grammar-constrained decoding (Python's grammar is
  large; semantic validity over a live `Session` is harder still).
- Verifier L0/L1 (see `agent-local-llm-posttraining.md` §2) would
  collapse to "did `ast.parse` succeed and was every attribute access
  on a real `Session` member?" — strictly weaker than the per-call
  JSON-schema check on a typed `Command`.
- Recent open work on small tool-use models (Magnet, xLAM, APIGen-MT,
  FunctionGemma) consistently sits at the tool-call layer, not
  free-form code, for sub-3B models.

Python via pygriz is the right surface for **humans** (notebooks,
scripts) and for the **Claude-API reasoning tier** (`client.md`
decision 4) — not for the tiny command-writer. The two-tier
architecture above already separates these.

### Pleasing alignment with pygriz

Compound macros that earn a tool slot are *implemented* as pygriz
functions on top of `Session` — the same code a user would write in a
notebook. So pygriz is not the LLM's emit target, but it is the LLM's
tool-implementation language. "What a user types in a notebook" ≡
"what backs the agent's `query_extreme`." This keeps a single
authoritative library of analysis macros, callable from both humans
and the model.

### Tradeoff this introduces

JSON-schema validation gives "argument *type* is valid"; it does not
give "argument *exists for this fixture*" (e.g. `material: 7` when only
1–4 exist). Layer-0 DSL had the same semantic gap, but its dispatcher
already returns rich runtime errors. So **L2-execution carries more
weight than the original plan assumed**: schema validity covers less
ground than grammar-parse did. The verifier's L1 tier accordingly
thins; L2 must catch most argument-level errors. This matches the
vLLM caveat the deep-research report cites — structured decoding
guarantees parseable, not semantically correct. See
`posttraining-dataset.md` Stage 4 for the verifier-side fold-in.

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
3. **Constrained decoding is mandatory, not optional — but the
   constraint follows "Surface choice" above.** Primary constraint:
   the FunctionGemma-style chat template + per-tool JSON schema, so
   tool-call output is parseable by construction and the model only
   has to get *intent and argument mapping* right, not syntax.
   Secondary constraint, scoped to the `griz_raw` fallback only: the
   griz GBNF derived from `interpret.c` (Stage-1 artifact in
   `posttraining-dataset.md`) — kept because it is independently
   useful and because it constrains the one tool whose argument is a
   free-form command string. This is what collapses the model-size
   requirement.
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

- **Tool inventory: which analysis macros earn a tool slot vs. stay as
  multi-turn orchestration?** First-pass candidates: `query_extreme`
  (find the state index where a result peaks/troughs over a class +
  range), `scan_states` (sweep state indices and report a vector of
  per-state values), `diff_states` (compare result fields at two
  states). Each macro shrinks the model's required reasoning depth at
  the cost of one more schema to learn. The pre-experiment (decision 6
  / Stage 8) is the right place to settle this — run with **typed
  tools only** first, then add macros only where the eval set's
  failure modes demand them.
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
