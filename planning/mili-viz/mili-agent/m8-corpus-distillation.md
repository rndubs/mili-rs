# M8 — corpus distillation (teacher-model SFT data campaign)

**Status (2026-05-25):** Plan only. Sequenced after M7
([`m7-bench-live-parity.md`](m7-bench-live-parity.md)) Gate 7 — the
honest baseline run on the re-rendered 82-record corpus. M8 only
starts if that baseline confirms the existing corpus has hit a
content ceiling (i.e. structural fixes alone don't restore live-UX
quality).

---

## Goal

Scale the SFT corpus from 82 records (rev-22 v1) to **5–50k records**
via teacher-model distillation. The teacher (Gemma-4-31B per the
project's HF stack pin) generates candidate scenarios; a **three-layer
validation pipeline** rejects junk; only validated scenarios reach
`train.jsonl`. The student (FunctionGemma-270M-it) is then SFT'd on
the distilled corpus.

Specifically, M8 produces:

1. A new SFT corpus 50×–500× the current size, terminating-text
   compliant (M7 Delta 1 shape), covering the `list_results` lookup
   pattern (M7 Delta 5), and explicitly tiered easy / intermediate /
   hard.
2. A reproducible generation pipeline (`mili_llm_bench.synth.generate`)
   that can be re-run when the tool inventory expands or new mili
   fixtures land.
3. A model-in-the-loop validator (`mili_llm_bench.synth.validate`)
   that combines programmatic verification with semantic checks so
   bad teacher outputs are caught before they pollute training.

---

## Why teacher-distillation now

The M7 audit established three structural mechanisms that inflate the
bench score relative to live UX (see [`m7-bench-live-parity.md`](m7-bench-live-parity.md)
§"Root-cause analysis"). M7's Deltas 1–4 fix the *pipeline* — but they
don't fix corpus **content**. The v1 corpus is:

- Too small (82 train + 8 val + 81 heldout = 171 total) for a model to
  generalize across compound instructions, natural-language phrasings,
  unknown databases, or recovery patterns.
- Thin on natural-language ("first principal stress" → `prin_stress1`)
  variation.
- Missing tier coverage — every existing record is roughly the same
  difficulty, no graded curriculum.
- Missing `list_results` lookup-pattern examples (M7 Delta 5 adds the
  *runtime*; M8 adds the *training data* that exercises it).

We cannot fix these by manual rewrites at this scale. A 31B-class
teacher with grounded prompts and rigorous in-loop validation is the
right tool.

---

## Reading order (one-time orientation)

1. [`m7-bench-live-parity.md`](m7-bench-live-parity.md) — the
   structural fixes M8 builds on. M8 only starts after M7 Gate 7.
2. [`m5-sft-pipeline.md`](m5-sft-pipeline.md) — the existing SFT
   pipeline (Stages 1–7). M8 reuses Stages 6–7 verbatim; the
   generation pipeline replaces Stages 1–5.
3. `python/mili-llm-bench/src/mili_llm_bench/scenarios.py` — the
   scenario schema (`Postcondition` kinds, `Scenario` shape). M8's
   generator must produce this exact shape.
4. `python/mili-llm-bench/src/mili_llm_bench/verifier.py` — the W3
   verifier. M8's validator pipeline uses this verbatim as its
   programmatic layer.
5. `python/mili-llm-bench/src/mili_llm_bench/assemble.py` — the SFT
   record assembler. After M7 Delta 1, this is the consumer of M8's
   generated scenarios.

---

## Teacher model

**Choice:** `google/gemma-4-31B` from HuggingFace
(`https://huggingface.co/google/gemma-4-31B`).

**Why 31B is the right size:**
- Big enough to follow grounded few-shot prompts and emit
  schema-conforming JSON reliably.
- Open weights (no per-token API spend at generation time —
  cluster-amortized cost only).
- Stays in the Gemma family the student is already in, so the chat
  template, tokenizer, and JSON-tool-call conventions match.
- Fits on a single H100 80GB at BF16 (~62GB activations + KV cache),
  or two on the cluster for batched throughput.

**Setup checklist:**

- [ ] **Confirm the model identifier exists.** The exact URL/repo name
  the user specified is `google/gemma-4-31B`. Before launching the
  pull, run `huggingface-cli search google/gemma-4` and `huggingface-cli
  search Gemma-4` on the cluster (or check `https://huggingface.co/google`
  directly) to confirm the canonical identifier — Gemma family names
  have shifted historically (Gemma 2 → Gemma-2-27B etc.). If the
  canonical id is `google/gemma-4-31b` (lowercase) or
  `google/gemma-4-31B-it` (instruction-tuned), update this doc and
  the pull command before proceeding.
- [ ] **Decide instruction-tuned vs base.** For corpus generation, the
  IT variant is strongly preferred — it follows the
  generate-this-JSON-shape prompt without a separate fine-tune. If
  only the base is available, an additional in-context teaching pass
  is needed and the generation prompts get longer.
- [ ] **Pull the model to the cluster.** From the matrix41 H100:
  ```bash
  huggingface-cli download google/gemma-4-31B \
    --local-dir /p/vast1/whitmore/cadsat/models/gemma-4-31B
  ```
  Cache size ~62GB (BF16). Verify checksums.
- [ ] **Inference backend.** Use **vLLM** for batched generation
  throughput (vs `transformers.generate` which serializes). Install
  alongside the existing post-training env:
  ```bash
  uv pip install --directory python vllm
  ```
  Validate with a smoke `vllm.LLM(...).generate(["hello"])` call.
- [ ] **Wrap as a provider.** Add `python/mili-llm-bench/src/mili_llm_bench/providers/gemma_teacher.py`
  implementing the same `LlmProvider` protocol the existing providers
  use (anthropic, llamacpp, transformers, mock). This makes the
  teacher slot into the existing bench harness for self-grading.
- [ ] **Memory/parallelism budget.** Single H100 BF16 generation at
  ~30 t/s output. A 5k-scenario run with ~600 output tokens each =
  ~3M tokens = ~28 wall-clock hours single-GPU. Plan for multi-GPU
  tensor-parallel if pressed for time, or accept overnight runs.

**Costs:** Cluster-amortized. No per-token API fees. Wall-clock
hours, not dollars, are the budget.

---

## Three difficulty tiers

| Tier | Definition | Example instruction | Example tool sequence |
| --- | --- | --- | --- |
| **Easy** | Single tool call. Canonical names in the user prompt. Postcondition: single-call match. | "show prin_stress1" | `show({"result":"prin_stress1"})` |
| **Intermediate** | Either (a) single call where the user phrasing requires `list_results` → canonical mapping, or (b) two-step compound with canonical names. | "show the first principal stress" *or* "load cylinder and show vx" | `list_results({})` → `show({"result":"prin_stress1"})` *or* `load({"root":"cylinder"})` → `show({"result":"vx"})` |
| **Hard** | Multi-step compound *and* natural-language phrasing, *and/or* recovery after an error. Postcondition: multi-call sequence match. | "show the von Mises stress at the deformed configuration" *or* "select bricks 1 through 10 and color by max shear" *or* "show foo" (typo) | Variable; may include `list_results` lookup, `set_state` nav, recovery branches |

**Tier distribution (initial target):**

- 40% easy
- 40% intermediate
- 20% hard

Skewed toward easy/intermediate because the model fundamentals matter
most. Hard examples are the long tail that earn the highest-quality
gains per record.

Tiers are tagged on every scenario (`tier: "easy" | "intermediate" |
"hard"`) so the bench can report stratified pass rates.

---

## Validation pipeline — *the* design decision

Junk training data is the single biggest risk of teacher distillation.
A 31B model emits confident-looking JSON that may be subtly wrong
(typoed result names, wrong argument types, off-by-one state indices,
plausible but unverifiable post-conditions). Training on junk locks
the failure modes into the student's weights.

M8 validates every candidate scenario through **three independent
layers**. A scenario reaches `train.jsonl` only if it passes all
three.

### Layer 1 — Programmatic validation (the existing W3 verifier)

Reuses `mili_llm_bench.verifier.verify(messages, postcondition)`
verbatim. Pass criteria:

- Every tool call's name is in the registry (`tools.json`).
- Every tool call's `arguments` parses as JSON and conforms to the
  tool's `input_schema` (use `jsonschema.validate`).
- The rollout's final message is a content-only `assistant` (the M7
  Delta 1 + Delta 2 contract).
- `verifier.verify(...).max_tier == 3` against the scenario's
  declared postcondition.

This layer catches:
- Syntactically malformed tool calls
- References to nonexistent tools (typos like `delete` when the model
  meant `clrsel`)
- Argument-type mismatches (e.g., `state` passed as a string)
- Unmet postconditions (rollout claimed to set state to 81 but never
  emitted `set_state` at all)

**Implementation:** wraps existing `verifier.verify`; ~30 LOC.

### Layer 2 — Semantic validation via teacher self-critique

The teacher generates a candidate scenario, then a **separate** teacher
call grades it. The critique prompt is independent of the generation
prompt — it gets just the instruction + rollout transcript, with no
prior context, and is asked:

> *Read this user instruction and the assistant's tool-call rollout.
> Did the assistant correctly fulfill the user's request? Answer
> "yes" or "no" first, then explain in one sentence why. Common
> failure modes to watch for: (a) the rollout solved a DIFFERENT
> request than what the user asked, (b) extra tool calls beyond what's
> needed, (c) wrong canonical name for the result the user described,
> (d) missing termination (rollout ends mid-call).*

A "no" verdict rejects the scenario. The critique reason is logged for
diagnostic review.

**Why a separate teacher call** (instead of trusting the original
generation): the original-generator is invested in its own output. A
fresh-context critique catches its own confabulations. This is the
same "judge from a clean slate" pattern Anthropic publishes for their
own eval rubrics.

**Implementation:** new `synth/critic.py`; ~80 LOC. Uses the same
vLLM-loaded teacher.

### Layer 3 — Diversity filter

For each batch of N candidates, embed the `instruction` strings
(sentence-transformers / `all-MiniLM-L6-v2`) and reject candidates
whose cosine similarity to a prior-accepted instruction is > 0.92.
Prevents the teacher from emitting the same templated phrasing
hundreds of times.

The threshold is tunable; 0.92 is conservative for short imperatives.

**Implementation:** new `synth/diversity.py`; ~40 LOC with
`sentence-transformers` as a new dep.

### Gate: all three layers, in order

Order matters: programmatic is cheapest (microseconds), critic is
expensive (one teacher call per candidate), diversity is mid-cost
(embedding lookup). Run cheap-first so most rejections happen before
the critic spend.

A scenario that fails *any* layer is logged with its rejection reason
and discarded. The acceptance rate is a key metric (target: ≥ 30%; if
<10% the generation prompt needs revision before scaling).

---

## Static content artifacts

Before generation can start, three hand-curated artifacts must land:

### Artifact 1 — `data/posttraining/grammar/result_aliases.json`

Canonical svar → list of natural-language aliases. Covers every
queriable name across the test corpora (~155 entries).

**Schema:**

```json
[
  {
    "name": "prin_stress1",
    "type": "derived",
    "description": "First principal stress eigenvalue",
    "aliases": [
      "first principal stress",
      "principal stress 1",
      "max principal stress",
      "σ₁",
      "sigma_1"
    ]
  },
  {
    "name": "vel_x",
    "type": "primal",
    "description": "Velocity x component",
    "aliases": ["x velocity", "velocity x", "vx", "u-velocity"]
  },
  "..."
]
```

**Build process:**

- [ ] Enumerate the canonical svar set across all test corpora
  (`d3samp6`, `basic1`, `cylinder`, `bar71`, etc. — walk
  `db.queriable_svars()` plus `db.derived_variables_of_class()` for
  each fixture and union the results).
- [ ] One-shot Gemma-4-31B (or Claude) draft pass: feed the canonical
  names and ask for ~5 plausible aliases each.
- [ ] **Human review** — at least one engineer with mili / griz
  domain knowledge eyeballs the entire output. Aliases are the seed
  for the corpus; bad aliases poison every scenario that uses them.
- [ ] Unit test: every key is unique, every name is in the test-corpus
  catalog (no orphans).

### Artifact 2 — Generation prompt template

The few-shot prompt sent to the teacher per scenario. Lives at
`python/mili-llm-bench/src/mili_llm_bench/synth/templates/generate.txt`.

**Inputs (filled at generation time):**
- `tier` — easy / intermediate / hard
- `intent_class` — load / show / set_state / step / select / clrsel /
  material / colormap / view / named_view / iso / contour / cutplane /
  legend / query / close / griz_raw
- `fixture` — the database name + a precomputed availability summary
  (`num_states`, available result names sample)
- `tool_inventory` — the `tools.json` content
- `result_aliases` — the relevant subset of `result_aliases.json`

**Output:**
A JSON object matching the `scenarios.Scenario` schema:

```json
{
  "scenario_id": "synth-tier-xxxxx",
  "tier": "easy" | "intermediate" | "hard",
  "fixture": "d3samp6",
  "intent_id": "show",
  "instruction": "show the first principal stress",
  "instruction_source": "gemma-4-31B-distill",
  "messages": [
    {"role": "developer", "content": "<system prompt>"},
    {"role": "user", "content": "<instruction>"},
    {"role": "assistant", "tool_calls": [...]},
    {"role": "tool", "tool_call_id": "...", "name": "...", "content": "..."},
    {"role": "assistant", "content": "<terminating text>"}
  ],
  "postcondition": {"kind": "...", "expect": {...}}
}
```

**Build process:**
- [ ] Draft the prompt with ~3 hand-written exemplars per tier.
- [ ] Pilot-generate 30 scenarios with this prompt (10 per tier).
- [ ] Run validation; tune the prompt based on what fails most.
- [ ] Lock the prompt with a content hash (mirroring the system-prompt
  pin pattern) before production.

### Artifact 3 — Critic prompt template

Same idea, lives at `synth/templates/critic.txt`. Inputs: the
instruction + rollout transcript. Output: `{"verdict": "yes"|"no",
"reason": "..."}`. See Layer 2 above.

---

## Detailed prerequisite checklist

**No scenario generation runs until every box is ticked.** This is the
"are we ready to spend tokens" gate.

### Stage A — M7 must be complete

- [ ] **A1.** M7 Delta 1 (`assemble.py` terminating-text) landed and
  re-rendered 82-record corpus validated.
- [ ] **A2.** M7 Delta 2 (`verifier.wrong_termination`) landed and
  unit-tested.
- [ ] **A3.** M7 Delta 3 (driver no-oracle) landed.
- [ ] **A4.** M7 Delta 4 (server `show` no-clobber) landed ✅ (commit
  679bd48).
- [ ] **A5.** M7 Delta 5 (`list_results` tool registered in
  `tools.json`, agent handler implemented in
  `llamacpp_agent_v1.rs`, `AgentTurnCtx.catalog_provider` plumbed,
  system_prompt.txt updated, Rust unit tests added) landed.
- [ ] **A6.** M7 Gate 7 baseline measurement taken — bench L3 +
  failure-mode histogram recorded under the new verifier/driver. This
  is the *honest* baseline M8 is measured against.

### Stage B — Content artifacts

- [ ] **B1.** `data/posttraining/grammar/result_aliases.json` exists,
  covers every queriable svar in test corpora, human-reviewed.
- [ ] **B2.** `synth/templates/generate.txt` drafted, with ≥3 exemplars
  per tier.
- [ ] **B3.** `synth/templates/critic.txt` drafted.
- [ ] **B4.** Tier rubric written down with concrete examples per
  intent_class.

### Stage C — Teacher infrastructure

- [ ] **C1.** `google/gemma-4-31B` (or its canonical name) pulled to
  the cluster at a known path.
- [ ] **C2.** vLLM installed and smoke-tested with the teacher model
  (one inference call returns sensible output).
- [ ] **C3.** `providers/gemma_teacher.py` implemented as a clean
  `LlmProvider`.
- [ ] **C4.** Single-call latency + throughput measured. Project the
  wall-clock budget for the target corpus size.

### Stage D — Pipeline implementation

- [ ] **D1.** `synth/generate.py` — orchestrates teacher inference for
  a batch of (tier, intent, fixture) tuples, writes raw candidates to
  `data/posttraining/runs/synth-vN/candidates.jsonl`.
- [ ] **D2.** `synth/validate.py` — runs Layer 1 / 2 / 3 in order on
  the candidates file; writes accepted scenarios to
  `data/posttraining/runs/synth-vN/accepted.jsonl` and rejections
  with reasons to `data/posttraining/runs/synth-vN/rejected.jsonl`.
- [ ] **D3.** `synth/critic.py` — Layer 2 implementation; loads the
  same teacher via vLLM.
- [ ] **D4.** `synth/diversity.py` — Layer 3 implementation.
- [ ] **D5.** End-to-end pipeline script
  (`python -m mili_llm_bench.synth pipeline --tier easy --n 30 ...`)
  exists and runs without errors on a 10-scenario smoke.

### Stage E — Pilot

- [ ] **E1.** Generate **30 scenarios per tier (90 total)** through the
  full pipeline.
- [ ] **E2.** Layer-1 / 2 / 3 acceptance rates recorded. Target: ≥30%
  overall acceptance. If <10%, return to Stage B and revise prompts;
  do **not** scale up.
- [ ] **E3.** Manually eyeball ~15 accepted scenarios (5 per tier).
  Pass criteria: instruction reads natural, tool sequence is the
  obvious right one, termination is clean.
- [ ] **E4.** Manually eyeball ~15 rejected scenarios with their
  rejection reasons. Confirm rejections are reasonable; if many false
  positives, tune validator thresholds.
- [ ] **E5.** Run the existing 82-record corpus through the pipeline
  *as if it were teacher output* (a regression check — the human-
  written records should mostly pass validation). If they don't, the
  validator is too strict.
- [ ] **E6.** Re-tokenize one accepted scenario; verify the loss
  mask covers all expected spans (mirror M5 preflight-4-loss-mask.md
  §"Single-row probe").

### Stage F — Production go/no-go

- [ ] **F1.** Pilot Layer 2 acceptance rate ≥ 30%.
- [ ] **F2.** Pilot manual review shows ≥ 90% of accepted scenarios
  look correct.
- [ ] **F3.** Loss-mask probe is healthy on the new shape.
- [ ] **F4.** Wall-clock projection for the target corpus size is
  acceptable.

**Only when all of F1–F4 are met does the production generation
launch.**

---

## Production parameters (decide at F4)

| Knob | Default | Notes |
| --- | --- | --- |
| Corpus size | 5000 scenarios | Easy to 5×; conservative starting point. |
| Tier split | 40 / 40 / 20 (E/I/H) | Skewed easy/intermediate. Hard is the long tail. |
| Intent split | Proportional to `tools.json` surface area, with `show` and `set_state` overweighted | These are the high-leverage tools. |
| Fixture diversity | All available test fixtures | Don't overfit to one database. |
| Seed strategy | Per-batch seed for reproducibility | Re-run by seed if a batch goes bad. |

---

## Validation plan

### Gate G1 — Pilot quality (Stage E)

Manual review of pilot output is the most important quality signal.
A bad pilot doesn't get scaled.

### Gate G2 — Pipeline rerun reproducibility

Re-running the pipeline with the same seeds produces byte-identical
output. Reproducibility is non-negotiable for the SFT campaign — if
the corpus changes between runs we can't bisect regressions.

### Gate G3 — End-to-end smoke

Train an FG-270M student on the production corpus. Run the bench
under M7's verifier/driver. The L3 number on the heldout (and
ideally a held-out subset of *the new corpus*) should exceed M7
Gate 7's honest baseline by a meaningful margin (target: ≥ 15
percentage points absolute).

If G3 fails, the corpus generation has issues we missed in pilot.
Diagnose via the failure-mode histogram: which tiers / which
intents are still failing?

### Gate G4 — Live griz smoke

Same smoke prompts as M7 Gate 4. Specifically the compound prompts
that failed on v1:
- "show the prin_stress1 values for the last time step"
- "show the von Mises stress at the deformed configuration"
- "select bricks 1 through 10 and display max shear stress"

Pass criteria: model emits the correct multi-tool sequence (with
`list_results` lookups where natural-language phrasing requires
it), terminates cleanly, no runaway.

---

## Success criteria

M8 is a success if all of the following hold after the production
corpus + retrain:

1. Bench L3 on heldout ≥ M7 Gate 7 + 15 pp (absolute).
2. `failure_mode` histogram: `wrong_termination` < 5% of rollouts
   (was the dominant mode under M7's verifier alone).
3. Live griz smoke: compound prompts dispatch the correct multi-tool
   sequence, terminate cleanly. Quantitative: ≥ 8 / 11 per-intent
   heldout prompts hit the right tool with the right arguments,
   matching M6's per-intent floor.
4. `list_results` lookup rate on intermediate tier: ≥ 60% of model
   responses to natural-language result references emit a
   `list_results` call before `show`.

---

## v3 deferrals

| Item | Why deferred |
| --- | --- |
| Multi-turn conversation corpus (the user replies to the model's clarification) | Single-turn first; multi-turn is a separate corpus shape. |
| Per-fixture catalog availability in the generation prompt (the teacher knows what `d3samp6` actually has) | First pass uses a global alias table; per-fixture is a quality refinement once we know it's needed. |
| RLHF / DPO on top of SFT | Cannot start without a working SFT baseline. M9 territory. |
| Vision / multimodal scenarios (screenshots → tool calls) | Independent of M8; the v1 model can't see anyway. |
| Curriculum learning (easy-first training schedule) | Tag tiers in the corpus now; curriculum schedule is an orthogonal training-recipe knob. |

---

## Path forward

1. **Hold until M7 Gate 7.** No M8 work begins before the honest
   baseline is recorded.
2. **Stage B in parallel with the tail of M7.** Content artifacts
   (`result_aliases.json`, prompt templates) can be drafted while
   Deltas 1–3 land.
3. **Stage C as M7 winds down.** Pull the teacher model, install
   vLLM, validate single inference.
4. **Stage D — pipeline implementation.** All Python; no model spend.
5. **Stage E — pilot.** 90 scenarios, full validation. Gate F1–F4.
6. **Stage F — production.** 5000 scenarios. Wall-clock budget
   permitting.
7. **Retrain student on the production corpus.**
8. **Gates G1–G4.** Bench + live smoke + manual review.

Each stage's deliverables are reviewable in isolation; nothing
commits to the next stage's scope until the prior one's gates pass.
