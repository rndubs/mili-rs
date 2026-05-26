# M8 — corpus distillation (teacher-model SFT data campaign)

**Status (2026-05-25):** **Stage A1–A5 landed (M7 code-only deltas);
Stage B2/B3/B4 drafted; Stage A6 + B1 + C + E + F gated on H100
access.** This doc is the single "go" pointer — read the
§"Runbook" below top-to-bottom on day-of-cluster-access and follow
the numbered steps. The rest of the document is the design rationale
the runbook links into.

| Stage | What | State |
| --- | --- | --- |
| A1–A5 | M7 Deltas 1–5 (terminating text, `wrong_termination`, no-oracle driver, `show` no-clobber, `list_results`) | ✅ landed on `setup-posttraining-m7` |
| A6 | M7 Gate 7 honest baseline measurement | ⛔ blocked on H100 |
| B1 | `result_aliases.json` content (~155 entries, human-reviewed) | ⛔ blocked on cluster (corpus enumeration) + 31B teacher draft |
| B2 | `synth/templates/generate.txt` few-shot prompt | ✅ drafted |
| B3 | `synth/templates/critic.txt` self-critique prompt | ✅ drafted |
| B4 | [`m8-tier-rubric.md`](m8-tier-rubric.md) per-intent_class exemplar table | ✅ drafted |
| C | Teacher infra (pull model, install vLLM, write `providers/gemma_teacher.py`) | ⛔ blocked on H100 |
| D | Synth pipeline scaffolding (`synth/generate.py`, `validate.py`, `critic.py`, `diversity.py`) | 🟡 not started — **pure code, can land off-cluster** |
| E | Pilot (90 scenarios, full validation, manual review) | ⛔ blocked on Stage C + D |
| F | Production go/no-go | ⛔ blocked on Stage E |

---

## Runbook — turn-key sequence to v2 student

Goal: take the codebase from "M7 deltas merged" to "v2 student trained
on the distilled corpus and bench-passed". Numbered phases below
distinguish off-cluster from on-cluster work so the H100 budget is
spent only on what needs the GPU.

### Phase 0 — pre-merge (now, off-cluster, ~hours)

Lands the remaining code so the cluster sequence below is pure-data /
pure-bench, no in-flight refactoring.

1. **Merge `setup-posttraining-m7` to `main`.** Carries M7 Deltas 1, 2,
   3, 5 + the M8 Stage B templates + tier rubric + empty alias-table
   stub. Gate: `cargo test -p mili-viz-server` (52/52) and
   `pytest python/mili-llm-bench/tests/` (252/252) both green;
   `cargo fmt --check` clean; `cargo clippy --workspace --no-deps -D
   warnings` clean.
2. **Land Stage D — synth pipeline scaffolding.** Pure Python, no GPU
   needed. Implement under `python/mili-llm-bench/src/mili_llm_bench/synth/`:
   - `generate.py` — orchestrate teacher inference for a batch of
     `(tier, intent_class, fixture)` tuples; write raw candidates to
     `data/posttraining/runs/synth-vN/candidates.jsonl`. Read the
     templates from `synth/templates/generate.txt`.
   - `validate.py` — three-layer validator (programmatic via
     `verifier.verify`, semantic via `critic.py`, diversity via
     `diversity.py`). Cheap-first order; rejected scenarios written
     with reason to `rejected.jsonl`.
   - `critic.py` — Layer 2; same vLLM-loaded teacher, fresh-context
     critique prompt from `synth/templates/critic.txt`.
   - `diversity.py` — Layer 3; sentence-transformers `all-MiniLM-L6-v2`
     embedding + cosine > 0.92 reject.
   - `pipeline.py` (or `synth/__main__.py` subcommand) — end-to-end
     runner: `python -m mili_llm_bench.synth pipeline --tier easy
     --n 30 --fixture d3samp6 --out data/posttraining/runs/synth-pilot-v1`.
   - Mock-provider unit tests so the pipeline is testable without the
     teacher loaded.
3. **Decision: provider abstraction for the teacher.** Add
   `providers/gemma_teacher.py` as a *stub* that conforms to the
   `LlmProvider` protocol but raises on `generate()`. Stage C below
   fills in the real vLLM impl. Lets the rest of the synth code be
   typed against the real interface without standing up the teacher.

**Gate to Phase 1:** the synth pipeline scaffolding lands, passes its
mock-provider tests, and `python -m mili_llm_bench.synth pipeline
--provider mock --n 10` produces 10 candidates + a non-empty
`validated.jsonl` against a hand-fabricated mock teacher.

### Phase 1 — H100 day-0 setup (~1 hour)

First contact with the GPU. Everything below requires
`source scripts/setup-gpu-env.sh` and cluster filesystem access.

4. **Confirm the canonical Gemma id.** From the cluster:
   ```bash
   huggingface-cli search google/gemma
   ```
   Pin the exact id (`google/gemma-4-31B`, `google/gemma-4-31b`,
   `google/gemma-4-31B-it`, etc.) and write it into `gemma_teacher.py`
   and this doc. Prefer the **-it** (instruction-tuned) variant —
   §"Teacher model" item 2.
5. **Pull the model.** Cache size ~62GB at BF16:
   ```bash
   huggingface-cli download <pinned-id> \
     --local-dir /p/vast1/whitmore/cadsat/models/gemma-4-31B
   ```
   Verify checksums; this is a one-time download.
6. **Install vLLM and smoke it.** From the M5 env on the cluster:
   ```bash
   uv pip install --directory python vllm
   python -c "from vllm import LLM; llm = LLM(model='/p/vast1/whitmore/cadsat/models/gemma-4-31B'); print(llm.generate(['hello'])[0].outputs[0].text)"
   ```
   Expected memory: ~70–80 GB on a single H100 80GB at BF16 (see
   chat-history memory table). If OOM, drop
   `--gpu-memory-utilization=0.80`.
7. **Fill in `gemma_teacher.py`.** Replace the Phase 0 stub with the
   real vLLM-backed `generate()` — see §"Teacher model" item 4.
   Smoke: `python -m mili_llm_bench.synth pipeline --provider
   gemma_teacher --n 10 --fixture d3samp6` produces 10 real
   teacher-generated candidates.

**Gate to Phase 2:** `gemma_teacher.generate()` round-trips a
fabricated prompt and returns sensible output. Single-call latency +
throughput measured and recorded in
`planning/mili-viz/mili-agent/m8-corpus-distillation.md` § "Teacher
model" item 5.

### Phase 2 — H100 day-1: M7 Gate 7 + B1 alias enumeration (~half-day)

8. **M7 Gate 7 honest baseline.** Retrain the v1 corpus through the
   landed `assemble.py` (terminating text appended) on the SFT
   cluster — see [`m5-sft-pipeline.md`](m5-sft-pipeline.md) §"Stage 6".
   ~5–10 min wall clock. Then re-run the bench against the heldout:
   ```bash
   uv run --directory python/mili-llm-bench mili-llm-bench run \
     --provider llamacpp \
     --scenarios data/posttraining/sft/eval/heldout.jsonl \
     --out data/posttraining/runs/v2-llamacpp-baseline \
     --step-cap 8 --per-turn-timeout-s 120 --max-new-tokens 256
   ```
   Record: L3 % under the new verifier, `failure_mode` histogram with
   `wrong_termination` and `step_cap_hit` bucketed separately, mean
   turns to completion. **Expected drop from rev-22's 95.06% L3** —
   30%–80% is plausible per the M7 plan. Whatever number lands IS the
   M8 baseline.

9. **B1 alias-table content.** Two steps:
   - **Enumerate** the canonical svar set across every test fixture.
     Walk `db.queriable_svars(false, false)` and
     `db.derived_variables_of_class` per mesh class, union and dedup.
     Target: ~155 entries. Write a one-shot script (or inline in
     `synth/catalog.py`) — output goes to a scratch file.
   - **Draft aliases** via Gemma-4-31B: feed the canonical names in
     batches of ~20 with the prompt "give 5 plausible
     natural-language phrasings each". Capture into
     `data/posttraining/grammar/result_aliases.json` matching the
     schema in `result_aliases.README.md`.
   - **Human review.** Mili / griz domain knowledge required. Bad
     aliases poison every M8 scenario that uses them, so do not skip.
     Unit test: every key unique; every name present in the catalog
     enumeration above (no orphans).

**Gate to Phase 3:** baseline number recorded; `result_aliases.json`
populated and human-reviewed; `cargo test -p mili-viz-server` still
green (the alias table parses at compile time — a malformed file
panics at first use).

### Phase 3 — H100 day-2: pilot run (~few hours)

10. **Run the 90-scenario pilot.** Per the M8 plan Stage E:
    ```bash
    python -m mili_llm_bench.synth pipeline \
      --provider gemma_teacher \
      --tiers easy=30,intermediate=30,hard=30 \
      --out data/posttraining/runs/synth-pilot-v1
    ```
    Records per-layer acceptance rates. Target: ≥30% overall
    acceptance. If <10%, **stop** — return to the templates
    (`synth/templates/generate.txt` + `critic.txt`) and tune the
    prompts; do **not** scale up under low acceptance.

11. **Manual review.** Eyeball ~15 accepted + ~15 rejected scenarios.
    Pass criteria: ≥90% of accepteds look natural and correct;
    rejections are reasonable (not over-strict).

12. **Regression sanity.** Run the existing 82-record v1 corpus
    through the same validator pipeline. The human-written records
    should mostly pass; if many false-reject, the validator is too
    strict — tune Layer 2/3 thresholds.

**Gate to Phase 4 (F1–F4 from §"Production go/no-go"):** pilot
acceptance ≥30%, accepted-scenario quality ≥90%, loss-mask probe
healthy on the new shape, wall-clock projection acceptable.

### Phase 4 — H100 day-3+: production + retrain (~half-day to overnight)

13. **Production run.** Default 5000 scenarios, 40/40/20 tier split,
    intent split overweighting `show` and `set_state`. Wall-clock
    projection ~28 hours on a single H100 BF16 at 30 t/s output; cut
    via tensor-parallel = 2 on two H100s for ~14 hours. Output:
    `data/posttraining/runs/synth-prod-v1/{accepted,rejected}.jsonl`.

14. **Retrain student on the production corpus.** Reuses Stage 6/7 of
    [`m5-sft-pipeline.md`](m5-sft-pipeline.md) verbatim. ~5–10 min
    wall clock on `matrix41`. Output: `functiongemma-v2.bf16.gguf`.

15. **Bench + live smoke.** Validation gates G1–G4 from §"Validation
    plan":
    - G1: pilot quality (already passed in Phase 3)
    - G2: pipeline rerun reproducibility (same seeds → byte-identical
      output)
    - G3: bench L3 on heldout ≥ Phase 2 baseline + 15pp (absolute)
    - G4: live griz smoke on the compound prompts that failed in M6
      ("show the prin_stress1 values for the last time step", "show
      the von Mises stress at the deformed configuration", etc.)

**Done when:** all four §"Success criteria" hold: bench L3 ≥
baseline+15pp, `wrong_termination` < 5%, live smoke passes ≥ 8/11
per-intent prompts, `list_results` lookup rate on intermediate-tier
≥ 60% of natural-language references.

---

## Original sequencing context (pre-runbook)

Sequenced after M7
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

- [x] **A1.** M7 Delta 1 (`assemble.py` terminating-text) landed
  (`setup-posttraining-m7`); re-rendered 82-record corpus validated
  via the §A audit test in `test_assemble.py`. Cluster retrain is
  Phase 2 / step 8 of the §"Runbook".
- [x] **A2.** M7 Delta 2 (`verifier.wrong_termination`) landed and
  unit-tested (3 new tests + 10 happy-path tests updated).
- [x] **A3.** M7 Delta 3 (driver no-oracle) landed; legacy behavior
  preserved behind `EvalConfig.allow_oracle_early_exit=False` +
  `--allow-oracle-early-exit` CLI flag.
- [x] **A4.** M7 Delta 4 (server `show` no-clobber) landed ✅ (commit
  679bd48 on `main`).
- [x] **A5.** M7 Delta 5 (`list_results` tool registered in
  `tools.json`, agent handler in `llamacpp_agent_v1.rs`,
  `AgentTurnCtx.catalog` closure plumbed via
  `crate::agent::CatalogProvider` + `Session::list_results_catalog`,
  `system_prompt.txt` updated to sha256[:16] `34cc473118246dfb` /
  2415 bytes, 4 new Rust unit tests). Schemas regenerated to 19
  entries.
- [ ] **A6.** M7 Gate 7 baseline measurement taken — bench L3 +
  failure-mode histogram recorded under the new verifier/driver.
  **Cluster-gated (Phase 2 / step 8 of the §"Runbook").** This is the
  *honest* baseline M8 is measured against.

### Stage B — Content artifacts

- [ ] **B1.** `data/posttraining/grammar/result_aliases.json` exists,
  covers every queriable svar in test corpora, human-reviewed. Stub
  (`[]`) + schema doc (`result_aliases.README.md`) landed; content
  blocked on Phase 2 / step 9 of the §"Runbook" (corpus enumeration
  + teacher draft + human review).
- [x] **B2.** `synth/templates/generate.txt` drafted with one exemplar
  per tier (easy / intermediate / hard).
- [x] **B3.** `synth/templates/critic.txt` drafted with the
  failure-mode rubric.
- [x] **B4.** Tier rubric written: [`m8-tier-rubric.md`](m8-tier-rubric.md)
  pins the `tier × intent_class` exemplar table.

### Stage C — Teacher infrastructure

- [ ] **C1.** `google/gemma-4-31B` (or its canonical name) pulled to
  the cluster at a known path. **Phase 1 / step 5.**
- [ ] **C2.** vLLM installed and smoke-tested with the teacher model
  (one inference call returns sensible output). **Phase 1 / step 6.**
- [ ] **C3.** `providers/gemma_teacher.py` implemented as a clean
  `LlmProvider`. **Phase 0 / step 3 (stub) → Phase 1 / step 7 (real
  vLLM-backed impl).**
- [ ] **C4.** Single-call latency + throughput measured. Project the
  wall-clock budget for the target corpus size. **Phase 1 / Gate to
  Phase 2.**

### Stage D — Pipeline implementation

Pure code, **no GPU needed.** Lands in Phase 0 / step 2 of the
§"Runbook" so the cluster sequence is data/bench-only.

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
  exists and runs without errors on a 10-scenario smoke against the
  `mock` provider (so the pipeline is testable before C3 is real).

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

See the top-of-doc §"Runbook — turn-key sequence to v2 student" for
the operational sequence. The runbook is the GO doc; the design
rationale above (§"Validation pipeline", §"Static content artifacts",
§"Detailed prerequisite checklist") is what the runbook steps link
into.

Phase summary:

- **Phase 0 — off-cluster.** Merge `setup-posttraining-m7`; land
  Stage D (synth pipeline scaffolding, pure Python); stub
  `gemma_teacher.py`.
- **Phase 1 — H100 day-0.** Confirm Gemma id; pull model; vLLM smoke;
  fill in `gemma_teacher.py`.
- **Phase 2 — H100 day-1.** M7 Gate 7 baseline (retrain v1 corpus on
  the new `assemble.py`, bench under the new verifier/driver);
  populate B1 alias content + human review.
- **Phase 3 — H100 day-2.** 90-scenario pilot; layer acceptance
  measurement; manual review; regression sanity on the v1 corpus.
- **Phase 4 — H100 day-3+.** Production run (5000 scenarios); retrain
  student; gates G1–G4.

Each phase's gate is concrete and falsifiable; nothing commits to the
next phase's scope until the prior gate passes.
