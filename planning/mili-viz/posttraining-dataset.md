# `mili-viz` — post-training dataset construction plan

**Status: plan only. No dataset code exists yet.** This is the
build-order companion to `agent-local-llm.md` and
`agent-local-llm-posttraining.md` (read those first for *why*; this
doc is *how* and *in what order*). It commits to nothing about model
choice or whether we fine-tune at all — it specifies how to produce
the corpus so that, the moment the Griz-python / API interface lands,
we can run the pipeline end to end without redesign.

## 0. The one hard dependency, isolated

Everything that needs the interface is funnelled through a single
seam so the rest can be built and tested now against a stub:

```
trait GrizSession {                       # the only interface contract
    fn load(&mut self, root: &str) -> Result<LoadedInfo>;
    fn execute(&mut self, line: &str) -> ExecOutcome;   # Layer-0 raw stream
    fn query(&self, q: QuerySpec) -> Result<QueryTable>; # data back
    fn snapshot(&self) -> SessionState;   # state/selection/result/camera/materials
    fn reset(&mut self);                  # cheap rollback between rollouts
}
```

This is *exactly* the `mili-viz-proto` surface already drafted
(`proto/mili_viz.proto`: `Execute(Command{raw})`, `Query`,
`Subscribe`'s `Snapshot`). The dataset pipeline depends on **only**
these five operations — not on the typed `Command` variants, not on
Arrow Flight, not on the GUI. When the interface is real, `GrizSession`
is a thin adapter over the in-process server; until then it is a
`MockGrizSession` good enough to develop and unit-test the verifier
and the assembler.

**Implication for "hit the ground running":** the interface team only
has to deliver `load` + raw-line `execute` + `query` + `snapshot` +
`reset` (in-process, no network) for the dataset pipeline to start
producing real data. Everything else here (grammar, intent corpus,
scenario synthesis, teacher rollouts, dataset format, splits, eval
harness) is built **before and independent of** that delivery.

## 1. What we ship — the dataset artifacts

Two consumers, one corpus. Generate a single canonical record set;
project it into the two training formats.

**Canonical rollout record** (`rollouts/*.jsonl`, one JSON per line)
— under the typed-tool surface (`agent-local-llm.md` "Surface
choice"), the `messages` array is FunctionGemma / OpenAI-style with
`tool_calls` and `tool` role responses, not raw griz strings:

```json
{
  "id": "rs-000123",
  "fixture": "d3samp6",
  "intent_id": "disable-mat-then-frame",
  "instruction": "hide material 3 and zoom to fit",
  "instruction_source": "template|manual-paraphrase|teacher-paraphrase",
  "tools": ["material", "view", "snapshot", "griz_raw", "..."],
  "messages": [
    {"role": "developer", "content": "You are a model that can do function calling with the following functions."},
    {"role": "user", "content": "hide material 3 and zoom to fit"},
    {"role": "assistant", "tool_calls": [
      {"type": "function", "function": {"name": "material",
        "arguments": {"enable": false, "material": 3}}}
    ]},
    {"role": "tool", "name": "material", "content": "{\"ok\": true}"},
    {"role": "assistant", "tool_calls": [
      {"type": "function", "function": {"name": "view",
        "arguments": {"op": "reset"}}}
    ]},
    {"role": "tool", "name": "view", "content": "{\"ok\": true}"},
    {"role": "assistant", "content": "Material 3 hidden; view reset to fit."}
  ],
  "tool_calls_flat": [
    {"name": "material", "arguments": {"enable": false, "material": 3}},
    {"name": "view", "arguments": {"op": "reset"}}
  ],
  "verifier": {
    "max_tier": 3,
    "l0_in_grammar": true,
    "l1_parsed": true,
    "l2_executed": true,
    "l3_postcondition": true,
    "postcondition": {"kind": "materials_visible", "expect": {"3": false}},
    "observed": {"3": false},
    "exec_log": "…",
    "reward": 1.0
  },
  "teacher": {"model": "claude-…", "sample_idx": 2, "temperature": 0.7},
  "split": "train"
}
```

`tool_calls_flat` is a denormalized view used by the verifier and by
dedup (Stage 6) — `(intent, fixture, tool_calls_flat)` is the
deduplication key, replacing the earlier `(…, commands)` key. `tools`
is the JSON-schema list shown to the model at this rollout (typically
the full inventory, but can be ablated for tool-surface robustness
experiments — see `agent-local-llm.md` open Q on macro inventory).
A rollout that uses the `griz_raw` fallback emits one
`{"name": "griz_raw", "arguments": {"line": "..."}}` tool call whose
inner `line` is graded against the Stage-1 grammar at L0/L1.

- **SFT (Q&A) projection** — emit `{messages}` for every record with
  `max_tier >= 3` (configurable floor; L2 fallback only if L3 coverage
  is thin for an intent). This is rejection-sampling SFT.
- **Preference / RL projection** — for each `(intent, fixture,
  paraphrase)` group with both a passing and a failing sample, emit
  DPO pairs `(chosen=highest tier, rejected=lowest)`; emit the graded
  `reward` for GRPO/PPO. Both fall straight out of the same records —
  no second generation pass.

One generator, both formats, so SFT vs. RL is a *consumer* decision
made later, not a fork in data production.

## 2. Pipeline stages and exact build order

Stages 1–4 and 6–7 have **no interface dependency** and are built
first. Stage 5 is the only one gated on `GrizSession`.

### Stage 1 — Grammar + vocabulary extraction (no dependency)

Source: `reference/griz/Src/interpret.c` (11,139 lines; **318**
`strcmp(tokens[0], …)` dispatch sites) and `Src/viewer.c`
`usage_text[]` / `usage_text_batch[]`.

- Parser walks the `parse_command` `else if` chain, extracting:
  keyword set, alias groups (explicit in source:
  `quit|done|exit|end`, `select|unselect|deselect`, `clrsel|poof`,
  `cap|clrallpicks`, `help|man|grizman|?`, `rnotes|rn|notes|relnotes`,
  …), and per-command argument arity / token-type from the
  `token_count` checks and `sscanf`/`atoi`/`atof` calls in each arm.
- Output: `grammar/griz_vocab.json` (canonical command → aliases,
  arg spec) and a derived `grammar/griz.gbnf` for constrained
  decoding. Same artifact used at inference (decision 3 of
  `agent-local-llm.md`) and as the L0 oracle.
- **Honest-grammar test** (mirrors `scripting.md`'s "Layer 0 ≡ raw
  stream" discipline): a checked-in test re-derives the table from
  `interpret.c` and diffs it against `griz_vocab.json`; griz source
  bump → test fails → table regenerated deliberately. The dispatch is
  irregular (nested `parse_command`, stateful modes — open Q in
  `agent-local-llm-posttraining.md` §5); the table is therefore
  **seeded mechanically, hand-corrected, and pinned by this test**,
  not assumed fully auto-derivable.

Deliverable now; independently useful (documents the vocab the main
agent needs) → low-regret even if the tiny model is dropped.

**Role under the typed-tool surface choice
(`agent-local-llm.md` "Surface choice").** The tiny model emits JSON
tool calls into the typed `Command` oneof, not raw griz lines, so
this grammar is no longer the model's *primary* output constraint —
its primary constraint is the per-tool JSON schema. The Stage-1
artifact stays on the critical path for three other reasons:

1. it constrains the **one fallback tool** (`griz_raw(line: str)`),
   whose argument *is* a raw griz line;
2. it remains the L1-parse oracle for verifier inputs that come back
   from `griz_raw`;
3. it documents the surface that the larger reasoning-agent tier
   (`client.md` decision 4) also drives.

So Stage 1 is unchanged in scope; only its *consumer* shifts from "the
tiny model's whole grammar" to "the fallback tool's grammar".

### Stage 2 — Intent / NL grounding corpus (no dependency)

Sources: `Src/viewer.c usage_text[]` (terse NL ↔ command pairs),
`Src/Doc/griz_manual.pdf` + `griz_manual.docx`, `Src/Doc/CHANGES`.

- Extract `(command, prose description, usage form)` triples from
  `usage_text[]` and the manual (pdf→text; `.docx` as the cleaner
  fallback for tables).
- Hand-curate an **intent catalog** `intents/catalog.yaml`: each
  entry is `{intent_id, canonical_commands, prose, params,
  fixture_constraints, postcondition_template}`. Start from the
  ~30–50 highest-value commands (load/state/select/show/material/
  view/iso/contour/query) — the proto's typed set is the priority
  spine; the long tail of 318 keywords is breadth, added later.
- Each intent declares its **postcondition kind** (see Stage 4) so
  scenarios are checkable by construction, not retrofitted.

**Two sources of truth, intersected.** `reference/griz/Src/interpret.c`
documents what griz *can do* — the full ~318-keyword vocabulary plus
prose from `usage_text[]` and the manual. The model can't actually
execute any of those directly; what it can call is enumerated in
`data/posttraining/grammar/tools.json` (~16 typed tools, derived from
`crates/mili-viz-proto/proto/mili_viz.proto` via
`mili-llm-bench derive-schemas` — source is `TOOL_DESCRIPTIONS` in
`python/mili-llm-bench/src/mili_llm_bench/schemas.py`). Each tool
entry carries `{name, description, input_schema, output_schema}` and
is the **executable surface**. The v1 intent catalog is the filtered
intersection: only intents whose canonical command maps to a name in
`tools.json` ship in the v1 corpus. Griz commands without a typed-tool
counterpart wait for `griz_raw` fallback coverage or future proto
extension — they are *vocabulary* the model must understand for
breadth, not behavior we can teach it to reliably perform.

### Stage 3 — Scenario synthesis over fixtures (no dependency)

Fixtures (`reference/mili/test/xmilics/`): `bar1`, `bar5`, `basic2`,
`cylinder`, `cylinder_4hex`, `d3samp6`, `d3samp6_tfile`, `ml40`,
`shell_mat2` — 9 executable runs.

- Cross intents × fixtures. For each, instantiate concrete params
  from **real fixture facts** (actual material ids, class names,
  state count, label ranges) so post-conditions are grounded, not
  invented. Fixture facts are pulled from the existing `mili-rs`
  parity suite's known-good values (`crates/mili-rs/tests/parity_*.rs`,
  `parity_corpus*.rs`) — no new oracle.
- Paraphrase each instantiated intent N ways: (a) template
  permutations, (b) light LLM paraphrase. Tag `instruction_source`.
  Measuring whether this yields real intent diversity vs. a narrow
  synthetic style is the **first thing to evaluate** (open Q,
  `agent-local-llm-posttraining.md` §4) — Stage 8 gates on it.
- Output: `scenarios/*.jsonl` = `{scenario_id, fixture, intent_id,
  instruction, instruction_source, params, postcondition}`. Fully
  produced **before** any teacher or interface work.

### Stage 4 — The verifier (built now against `MockGrizSession`)

The single graded check reused as SFT filter, RL reward, and eval
metric. Tiers from `agent-local-llm-posttraining.md` §2, refolded onto
the typed-tool surface (`agent-local-llm.md` "Surface choice"):

| Tier | Check (typed tool call) | Check (`griz_raw` fallback) | Needs interface? |
|---|---|---|---|
| L0 | output parses as a tool-call object (`{name, arguments}`) | inner `line` ∈ Stage-1 grammar | no |
| L1 | `name` is a known tool **and** `arguments` matches its JSON schema (type + arity) | `parse_command` accepts (`valid_command`) | no |
| L2 | tool dispatch runs against the fixture session, no error | raw line runs, no error | **yes — `GrizSession::execute`** |
| L3 | post-condition reached (`snapshot`/`query` vs. expected) | post-condition reached | **yes — `snapshot`/`query`** |

**L1 thins, L2 carries more weight.** JSON-schema validity says only
"argument type and arity are valid"; it does not say "argument *exists
for this fixture*" (e.g. `material: 7` when only 1–4 exist). The
original Layer-0 grammar had the same gap, but its dispatcher returned
rich runtime errors that the verifier could lean on at L1. Under the
typed-tool surface those errors only surface at L2 (real dispatch), so
the L2 oracle must catch most argument-level mistakes. This matches
the vLLM "structured decoding guarantees parseable, not semantically
correct" caveat called out in the deep-research report.

Practical implication for L2: do **not** rely on the tool call shape
alone; always execute against a real (or `MockGrizSession`) backend
that returns the dispatcher's argument-validity errors, and treat
"executed without error" as the L2 pass condition.

Post-condition kinds (closed set, declared by the intent):
`state_index`, `selection_set`, `active_result`, `result_range`,
`materials_visible`, `camera_named_view`, `query_value`
(tolerance-compared against the mili-rs suite's pinned answers).

- L0/L1 fully unit-testable now (pure functions over the grammar
  artifact). Build and pin them with fixtures of known-good /
  known-bad command strings.
- L2/L3 coded now against `MockGrizSession` (a scripted state machine
  honoring the closed post-condition kinds); swapped to the real
  adapter at Stage 5 with **zero pipeline change** — that is the
  point of the §0 seam.
- Reward shaping: max tier passed → `0.1 / 0.3 / 0.7 / 1.0`.

### Stage 5 — Teacher rollouts (GATED on interface)

The only interface-dependent stage.

- For each scenario, teacher (Claude API, or strong local 7–14B)
  proposes K candidate command sequences at temperature, under
  grammar-constrained decoding where the teacher supports it.
- Each candidate is run through the **full L0–L3 verifier with the
  real `GrizSession`**. All candidates (pass and fail) are written as
  canonical rollout records — failures are not discarded; they are
  the negative half of the preference/RL projection.
- `reset()` between candidates keeps rollouts cheap and independent.
- **Budget gate before bulk run:** cost ≈ scenarios × paraphrases ×
  K × teacher token price. Estimate and cap *before* the full sweep;
  run a 200-scenario pilot first, inspect tier distribution, then
  decide volume (open Q, `agent-local-llm-posttraining.md` §5).

### Stage 6 — Dataset assembly, dedup, splits (no dependency)

- Deduplicate on `(normalized_instruction, fixture,
  tool_calls_flat)` (the typed-tool dedup key — see §1; the earlier
  `commands` key was the raw-DSL form); near-dup instruction filter
  (embedding or MinHash) to stop paraphrase collapse inflating counts.
- **Contamination control:** split by `intent_id × fixture`, not by
  row. Held-out eval = whole `(intent, fixture)` cells never seen in
  train, so eval measures generalization, not memorization. Reserve
  ≥1 fixture (candidate: `shell_mat2` or `bar5`) entirely for eval.
- Balance: cap records per `(intent, fixture)` so high-yield easy
  intents don't swamp the compositional tail.
- Emit `sft/{train,val}.jsonl`, `pref/{train,val}.jsonl`,
  `eval/heldout.jsonl`, plus a `dataset_card.md` (counts, tier
  distribution, fixture/intent coverage matrix, known gaps).

### Stage 6.5 — Dataset smoke test (Claude validation; no dependency once Stage 5 lands)

**Catch data bugs before they look like model bugs.** Before kicking
off SFT, run a strong teacher (Claude) over the full assembled
scenario set as a data-quality gate. The verifier from Stage 4 grades
each scenario; the *failures* are the signal — they point at *data*
problems, not model problems:

* `wrong_*` failures (`wrong_final_state`, `wrong_result`,
  `wrong_selection`, `wrong_materials`, `wrong_range`): the
  scenario's postcondition or its grounded fixture facts disagree
  with what the canonical command actually produces. Either the
  postcondition was synthesized from a stale fixture fact, or the
  prose paraphrase changed the intent in a way the postcondition no
  longer captures.
* `dispatch_error` failures with a real teacher: the canonical
  command path isn't wired through the typed-tool surface (e.g.,
  missing pygriz method, missing dispatcher arm) — fix the
  dispatcher or drop the intent from the v1 spine.
* `wrong_result` / `wrong_*` clustered by paraphrase variant:
  the paraphrase introduced ambiguity ("color the mesh" vs. "color
  the mesh by stress" — only the second has a grounded
  postcondition). Tighten the paraphrase template or drop the
  ambiguous variant.
* Persistent `step_cap_hit` against Claude: the scenario requires
  more steps than `step_cap` permits, or the intent decomposition
  is wrong. Re-author or split.

**Gate threshold.** The v0 bootstrap ceiling under Claude is 92% L3
(see §5 below). A synthesized corpus should clear ≥85% L3 under
Claude with grammar-constrained decoding before SFT consumes it.
Below that, the data is broken in a way SFT will faithfully *learn*
— the rejection-sampling SFT filter (`max_tier >= 3`) will throw out
the salvageable rollouts and over-fit to whatever bad-postcondition
artifact remains.

Output: a triage file `dataset_card.smoke.md` listing
`(scenario_id, failure_mode, observed, expected, claude_calls)` rows
for the failures. Hand-fix or drop bad scenarios; re-run until the
gate clears. The runner is the same harness as Stage 4/7 with a
different consumer — no new code beyond a `scripts/smoke-dataset.py`
wrapper.

### Stage 7 — Eval harness (no dependency; same code as Stage 4)

Held-out `(intent, fixture, postcondition)` triples; metric = **L3
success rate under grammar-constrained decoding**, with L1 parse-rate
as a cheap regression tripwire. Literally the Stage-4 verifier pointed
at the eval split → eval is nearly free once the verifier exists.

### Stage 8 — Pre-experiment gate (before committing to fine-tune)

Per `agent-local-llm-posttraining.md` §4: run a stock 0.5–1B model
with grammar-constrained decoding and **no fine-tune** against the
Stage-7 eval set. If it already clears the bar, post-training is moot
for v1 and we stop. Also the diversity check from Stage 3 lands here:
inspect whether teacher paraphrases collapse stylistically. Only on
measured need do we proceed to SFT, and only if SFT plateaus to
DPO/GRPO ("phase 2 of phase 2").

## 3. Repo layout (proposed)

```
crates/mili-viz-dataset/          # new crate, gated workspace member
  src/grammar/        # Stage 1 extractor + honest-grammar test
  src/intents/        # Stage 2 catalog loader
  src/scenarios/      # Stage 3 synthesis
  src/verifier/       # Stage 4 (L0–L3), GrizSession trait + Mock
  src/teacher/        # Stage 5 client (feature-gated, needs key)
  src/assemble/       # Stage 6 dedup/split/card
  src/eval/           # Stage 7 (re-exports verifier)
data/posttraining/
  grammar/  intents/  scenarios/  rollouts/  sft/  pref/  eval/
  dataset_card.md
```

Generated data is **not** committed (size + churn); only the
generators, the `intents/catalog.yaml`, the pinned grammar artifact,
and the `dataset_card.md` are. A `make dataset` target chains the
stages; `scripts/setup-parity.sh` already provides the fixtures and
Python oracle this depends on.

## 4. Build order summary (what to do the day the interface lands)

| When | Stages | Blocking dep |
|---|---|---|
| **Now** (no interface) | 1, 2, 3, 4 (vs. Mock), 6, 7 scaffolding | none |
| **Now** | Stage 8 pre-experiment harness wiring | none |
| **Interface lands** | swap Mock → real `GrizSession` adapter; run Stage 5 pilot (200 scenarios) | `load/execute/query/snapshot/reset` in-process |
| **After pilot + budget OK** | full Stage 5 sweep → Stage 6 assemble → **Stage 6.5 Claude data smoke test** → Stage 7 eval → Stage 8 gate | teacher API budget |

Because stages 1–4/6–7 are interface-independent and verifier-swap is
a one-line adapter change, "hit the ground running" means: the only
work remaining when the interface arrives is the adapter + the
(already-scripted) teacher sweep.

## 5. Baseline numbers (pinned, 2026-05-24)

Pre-SFT baseline against the 50-scenario bootstrap eval
(`data/posttraining/eval/bootstrap.jsonl`) under the v0 harness:
`step_cap=8`, `temperature=0.0`, `max_new_tokens=256`,
`per_turn_timeout_s=120`,
`system_prompt_sha256=9f36d0deb5e98a89`. Real fixtures (the bench
fixture resolver maps the bare name → absolute `.A` path with loud
failure on miss; runs prior to **2026-05-24** ran against an empty
M1 stub corpus and are not comparable). Auto-terminate is on (driver
breaks the loop as soon as the verifier grades L3).

| Run | Provider | Model | L3 | step intent | mean turns |
|---|---|---|---|---|---|
| **v4 floor** | llamacpp | functiongemma-270m-it-GGUF:BF16 | **32%** (16/50) | 17% (1/6) | 5.00 |
| **v4 ceiling** | anthropic | claude-sonnet-4-5 | **92%** (46/50) | 100% (6/6) | 1.18 |

Per-intent (FunctionGemma v4): load 83%, set-state 83%, colormap
75%, show-derived 25%, show-primal/step 17%,
material/select/clrsel/view-reset/compound 0%.

Per-intent (Claude v4): load/set-state/select/colormap/material/
show-primal/step/view-reset all 100%, clrsel 50%, show-derived 75%,
compound 0%. The 4 residual Claude failures are real (not stub
artifacts): 2× `clrsel` `dispatch_error` (missing
`selection.clear_all()` in pygriz), 2× `wrong_result` (Claude
doesn't know "effective stress" → `eff_stress` symbol — a
domain-vocab gap addressable in the tool description).

Artifacts:
* floor: `data/posttraining/runs/v4-llamacpp-realfixtures-fullresolve/`
* ceiling: `data/posttraining/runs/v4-anthropic-realfixtures/`

**Success criteria for v1 SFT.** Close at least half the
FunctionGemma↔Claude gap on the same bootstrap eval (32% → ≥62% L3)
before committing to a full posttraining cycle. Concretely:

* **Floor regression tripwire:** any SFT'd model that scores below
  ~30% L3 has overfit to a paraphrase artifact or broken termination
  behavior — investigate before iterating on hyperparameters.
* **Target after SFT pilot:** ≥62% L3 (half the gap), with
  per-intent rates ≥50% for the four currently-zero intents
  (material, select, clrsel, view-reset). The zero-rate intents are
  the cheapest source of lift; failing to move them means SFT is
  teaching the wrong thing.
* **Stretch (post-SFT):** ≥80% L3 before considering DPO/GRPO. If
  SFT alone closes ≥80% of the gap, DPO is incremental rather than
  necessary.

These numbers also pin what "Stage 8 pre-experiment gate" means in
practice — Stage 8's purpose is to confirm there is room for
post-training to add value. With a 60-point gap between floor and
ceiling, the answer is yes; this section is what makes that answer
falsifiable on future re-runs.

## 6. Open questions (carried, not resolved here)

- Grammar robustness: how much of `interpret.c`'s 318-site dispatch
  is mechanically extractable vs. needs hand-seeding (the §5 open Q).
  Stage 1's honest-grammar test bounds the risk but not the effort.
- L1-parse-valid ≠ semantically sensible (`state 999999` parses).
  How much must L3 carry, and is 9-fixture coverage wide enough to
  make L3 meaningful for the compositional tail?
- Teacher cost at SFT-scale volume — pin with the Stage-5 pilot
  before the full sweep.
- Intent diversity without a large natural corpus — the Stage-8
  diversity check is the go/no-go for fine-tuning at all.
- Whether the `GrizSession::execute` raw-line path is available
  early enough, given Phase 4 M1 itself is not yet scoped
  (`status.md` open Q3). This dataset plan deliberately depends on
  the *smallest* possible slice of M1 to de-risk that.
```

Do not promote any of this to a binding decision until the
`agent-local-llm*.md` research is itself promoted (`status.md`
open Q7) — this is the dataset-side build plan for that research,
not its conclusion.
