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

**Canonical rollout record** (`rollouts/*.jsonl`, one JSON per line):

```json
{
  "id": "rs-000123",
  "fixture": "d3samp6",
  "intent_id": "disable-mat-then-frame",
  "instruction": "hide material 3 and zoom to fit",
  "instruction_source": "template|manual-paraphrase|teacher-paraphrase",
  "messages": [
    {"role": "user", "content": "hide material 3 and zoom to fit"},
    {"role": "assistant", "content": "disable mat 3\nrfit"}
  ],
  "commands": ["disable mat 3", "rfit"],
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
metric. Tiers from `agent-local-llm-posttraining.md` §2:

| Tier | Check | Needs interface? |
|---|---|---|
| L0 | output ∈ grammar (Stage 1 artifact) | no |
| L1 | `parse_command` accepts (`valid_command`) | no — uses the Stage-1 parser model / dispatcher |
| L2 | runs against a fixture session, no error | **yes — `GrizSession::execute`** |
| L3 | post-condition reached (`snapshot`/`query` vs. expected) | **yes — `snapshot`/`query`** |

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

- Deduplicate on `(normalized_instruction, fixture, commands)`;
  near-dup instruction filter (embedding or MinHash) to stop
  paraphrase collapse inflating counts.
- **Contamination control:** split by `intent_id × fixture`, not by
  row. Held-out eval = whole `(intent, fixture)` cells never seen in
  train, so eval measures generalization, not memorization. Reserve
  ≥1 fixture (candidate: `shell_mat2` or `bar5`) entirely for eval.
- Balance: cap records per `(intent, fixture)` so high-yield easy
  intents don't swamp the compositional tail.
- Emit `sft/{train,val}.jsonl`, `pref/{train,val}.jsonl`,
  `eval/heldout.jsonl`, plus a `dataset_card.md` (counts, tier
  distribution, fixture/intent coverage matrix, known gaps).

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
| **After pilot + budget OK** | full Stage 5 sweep → Stage 6 assemble → Stage 7 eval → Stage 8 gate | teacher API budget |

Because stages 1–4/6–7 are interface-independent and verifier-swap is
a one-line adapter change, "hit the ground running" means: the only
work remaining when the interface arrives is the adapter + the
(already-scripted) teacher sweep.

## 5. Open questions (carried, not resolved here)

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
