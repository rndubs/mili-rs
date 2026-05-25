# M5 — SFT pipeline (live tracker)

**Status (2026-05-25):** Stages 2, 3, 5, 6, 6.5 cleared; preflight
#1, #2, **#3**, **#4**, **#5** cleared (#2 via Path A rev 8; #3
cleared on `matrix41` H100 in rev 15, retested under TRL 1.5.0 in
rev 16 — `formatting_func` is mandatory under TRL 0.12.x, optional
under TRL 1.x but kept in the recipe for drift-proofing; **#4
cleared on `matrix41` H100 in rev 17 via option (B) — custom
data collator; TRL 1.5.0's native `assistant_only_loss=True` raised
at `SFTTrainer.__init__` because FG's chat template lacks
`{% generation %}` markers and is too macro-heavy for TRL's
auto-patch. The rev-12 config-seam claim in `cluster-setup.md` §0
turned out to be genuinely vacuous (kwarg exists on TRL 1.5.0 but
trainer dies on the template); see report
`data/posttraining/sft/preflight-4-loss-mask.md`**; #5 cleared
off-GPU in rev 14 — see below). **Stage 8 deferred 2026-05-25
(rev 18)** — Stage 6.5's 97.71 % L3 already supplies the
"strong model" signal; `google/gemma-{2b,7b}-it` parked as
post-SFT fallbacks if the regression tripwire fires. Critical path
is now `trainer.train()` (cluster-setup.md §6, rev 4 recipe)
→ preflight #6 → real Stage 7 eval. **rev 19 (2026-05-25):**
training entry point landed — `scripts/sft_train.sbatch` +
`python/scripts/sft_train.py`. **rev 20 (2026-05-25):**
`trainer.train()` landed — 168 steps / 8 epochs / 111.9 s on
`matrix41`; 8 per-epoch checkpoints + `final/` at
`data/posttraining/checkpoints/v1/` (13 GB). Loss collapsed to
~0 by epoch 6 — pure memorization on 82 rows. Per-checkpoint
heldout eval pending; **GGUF conversion deferred** to post-winner
selection (`eval/heldout.jsonl` can be graded directly off HF
checkpoints via a new TransformersProvider). **rev 21
(2026-05-25):** `TransformersProvider` landed (shared FG
envelope parser at `providers/_fg_envelope.py`; deleted
`FunctionGemmaProvider` — stale parser, no functional callers);
**8 × 81 heldout sweep cleared all four gates.** Curve: epoch 1
= 48 / 81 (59.3 %), epoch 2 = 54 / 81 (66.7 %), epoch 3 =
65 / 81 (80.2 %), epoch 4 = 76 / 81 (93.8 %), epoch 5 =
75 / 81 (92.6 %), epoch 6+ plateau at **77 / 81 (95.1 %)**.
Three-way tie at the plateau (`checkpoint-126` / `-147` / `-168`
— identical per-intent profile); **winner = `checkpoint-126`**
(earliest of the plateau triplet, least over-trained; symlink
`data/posttraining/checkpoints/v1/winner → checkpoint-126`).
Surfaced a v1-corpus data shape: assistant turns store
`function.arguments` as JSON **strings**, which the FG chat
template's string-branch (jinja lines 194-197) renders as
`call:NAME{<JSON_dict>}` (double-braced) in the training tokens
— the v1 checkpoints learned and emit that exact shape, not the
FG-DSL `<escape>` form. Parser now handles both; corpus
re-render queued under v2 backlog. **rev 22 (2026-05-25):**
preflight #6 cleared, GGUF conversion landed, and llamacpp
re-eval matched the HF number byte-for-byte — **77 / 81 =
95.06 % L3** on both the HF (transformers) and GGUF (llamacpp)
paths, identical per-scenario outcomes, identical model
`tool_calls` on the four residual `select` failures. **v1 SFT
ships:** `data/posttraining/checkpoints/v1/winner →
checkpoint-126` (HF) +
`data/posttraining/checkpoints/v1/functiongemma-v1.bf16.gguf`
(GGUF for serving). Quantization deferred to v2+. **TRL pin bumped
`>=0.11,<0.13` → `>=1.0,<2` in rev 16** (the original pin's stated
justification — `assistant_only_loss` — did not exist on the pinned
versions; trl 0.20+ is the floor for that kwarg). rev-9
parser gap resolved via **option (b)** in rev 10 — a client-side
`content → tool_calls` fallback inside `LlamaCppProvider.generate`,
gated on `/props` `chat_template_caps.supports_tool_calls = false`.
v7 re-baseline on the same `--jinja` path with the fallback active
lands at **13 / 50 L3 (26.0 %)**. **Stage 5 cleared in rev 11
(pilot) + rev 12 (full sweep)** — 171 / 175 retention (97.7 %) at
$1.88 total spend, K=3, byte-for-byte the same retention rate as the
v7 stage-6.5 ceiling. **Stage 6 cleared in rev 13** — `mili-llm-bench
assemble` produced the v1 SFT corpus from the rev-12 rollouts: 82
train / 8 val / 81 heldout / 0 DPO pairs (K=3@T=0.7 produced 0
mixed-tier scenarios; pref files land empty by construction). Floor
re-pinned from ≥40 → **≥10** after dedup math falsified the rev-4
paraphrase-multiplier assumption (see rev 13 changelog). **Stage 7
loader landed in rev 14** — `scenarios.load_scenarios` auto-detects
the assembled shape so the eval harness reads `eval/heldout.jsonl`
directly (no synth.jsonl join); mock-provider smoke against the real
81-row heldout split confirms end-to-end load. **Preflight #5
cleared in rev 14** at max=3341 / gate=4096 (deliberate bump above
Google's `max_length=512` recipe pin — recorded here per
sft-preflight-gpu.md §5). Preflight #3/#6 + Stage 8 still
**unblocked** (GPU-bound). Matched-tools ceiling 97.71 % L3 (Claude
Sonnet 4.5 on synth.jsonl, rev 7) stands.

This is the **single live entry point** for "where are we in SFT?"
Other docs in this directory (`m1-…`, `m2-…`, `m3-…`, `m4-…`) are
historical milestone records — they do not move. This one does.

For *why* SFT (vs. GEPA / vs. nothing), read
[`GEPA-vs-POSTTRAINING.md`](GEPA-vs-POSTTRAINING.md) and the pipeline
design in [`posttraining-dataset.md`](posttraining-dataset.md).
This tracker is the *how* and *when*.

---

## Pinned baselines (frozen 2026-05-24)

50-scenario bootstrap eval `data/posttraining/eval/bootstrap.jsonl`,
canonical harness config (`step_cap=8`, `temperature=0.0`,
`max_new_tokens=256`, `per_turn_timeout_s=120`,
`system_prompt_sha256=9f36d0deb5e98a89`):

| Run                                                       | Provider                      | L3       | tools_sha256 | Notes                                            |
| --------------------------------------------------------- | ----------------------------- | -------- | ------------ | ------------------------------------------------ |
| **v7 floor** (`v7-llamacpp-b-fallback-20260524-215520`)   | llamacpp / FunctionGemma-270M | **26.0 %** | `27ffbd0e…` | Canonical re-baseline on `--jinja` path with the rev-10 client-side fallback active. 13 / 50 L3 (`L0=20`, `L2=17`, `L3=13`). Failure-mode shift vs v6: parser-gap cluster (37/50) fully cleared; remaining 13 parse_errors are the same v6 refusal cluster (load 5/6, colormap 3/4, scattered show/select/clrsel). Stage 5 unblocked — see rev 10. |
| v6 (`v6-llamacpp-jinja-rebaseline-20260524-205725`)       | llamacpp / FunctionGemma-270M | 0 %      | `27ffbd0e…`  | Broken-parser floor; kept for the rev-9 ↔ rev-10 lift comparison. Same `--jinja` prompt path as v7, no fallback. 50 / 50 `parse_error` because llama.cpp b9307 (`549b9d843`) has no FG response parser. |
| v4 floor (`v4-llamacpp-realfixtures-fullresolve`)         | llamacpp / FunctionGemma-270M | 32 %     | `cdda3677…`  | Pre-GEPA-promotion; historical                   |
| **v4 ceiling** (`v4-anthropic-realfixtures`)              | anthropic / claude-sonnet-4-5 | **92 %** | `cdda3677…`  | Pre-promotion tools, bootstrap eval; re-measured |
| **v7 ceiling** (`v7-stage65-anthropic-smoke-…`)           | anthropic / claude-sonnet-4-5 | **98 %** | (synth)      | Post-promotion tools on `synth.jsonl` (175 rows) |
| **v1 SFT winner — HF** (`v1-sft-sweep/checkpoint-126-20260525-152545`) | transformers / `checkpoint-126` | **95.1 %** | (heldout)    | rev 21 sweep. 77 / 81 L3 on `eval/heldout.jsonl` (81 rows). Different corpus from the bootstrap eval — not a direct successor to the v7 26.0 % floor; both numbers are pinned for parallel reference. Three-way tie at the plateau (`checkpoint-126` / `-147` / `-168`); winner = earliest. See rev 21 full curve + per-intent breakdown. |
| **v1 SFT winner — GGUF** (`v1-sft-winner-gguf-20260525-161145`) | llamacpp / `functiongemma-v1.bf16.gguf` | **95.06 %** | (heldout)    | rev 22 round-trip. 77 / 81 L3, **identical per-scenario outcomes** to the HF row above (same 4 `wrong_selection` IDs, byte-identical model `tool_calls` on each failure). Confirms GGUF conversion preserved deterministic greedy decode. b9307 `549b9d843` + `--jinja` + rev-10 client-side fallback (supports_tool_calls=false in caps).

Earlier runs (`v0…v3`) ran against the empty M1 stub corpus before the
fixture-resolver landed; their absolute numbers are not comparable —
see [`bench-fixture-stub-fallback-fixed`](../../../../.claude/projects/-Users-rwhit-Workspace-mili-rs/memory/bench-fixture-stub-fallback-fixed.md)
in memory.

**Per-intent floor (FunctionGemma v7, 26.0 %):** step 4/6 (66.7%),
view-reset 2/3 (66.7%), clrsel 2/4 (50.0%), material 2/6 (33.3%),
set-state 2/6 (33.3%), load 1/6 (16.7%), colormap 0/4, compound
0/1, select 0/4, show-derived 0/4, show-primal 0/6. Compound is
1/50 in the bootstrap eval (acknowledged sparsity — see "Multi-step
tool calls" below), so its 0/1 is unmeasured, not measured. The
zero-rate intents (colormap, select, show-primal, show-derived) are
the SFT lift targets; their per-intent ≥50 % gate sits in the gates
table below. Historical v5 per-intent numbers (load 83 %, set-state
83 %, colormap 75 %, show-derived 25 %, show-primal / step 17 %,
material / select / clrsel / view-reset / compound 0 %) are kept
here for orientation but were measured against the deleted bespoke
renderer that nullified the bench system prompt.

---

## v1 scope — intentionally narrow

The first SFT cycle is a **pilot**, not the final corpus. We are
deliberately constraining everything we can to get one honest signal
end-to-end before scaling. Every constraint here is a known debt with
a planned sequel.

| Knob                       | v1 pilot                                           | Sequel (v2+)                                                  |
| -------------------------- | -------------------------------------------------- | ------------------------------------------------------------- |
| Intent inventory           | Intersect `tools.json` ∩ `interpret.c` (~10–15)    | Add long-tail griz keywords via `griz_raw` fallback           |
| Scenarios                  | ~200 (intents × ~3 fixtures × ~5 paraphrases)      | Scale to ~1k after smoke-test signal is honest                |
| Fixtures used in synthesis | 2–3 of 9 (the ones `_FIXTURE_PATHS` already maps)  | Add the remaining 6 once resolver entries + serial `.A` exist |
| Held-out fixture           | 1 reserved (candidate `shell_mat2`, **TBD**)       | Multi-fixture held-out grid by `(intent, fixture)` cell       |
| Paraphrase source          | Template + light Claude paraphrase                 | Diversity audit per Stage-8 open Q before scaling             |
| Multi-step coverage        | **First-class category** — see below               | Same shape, more depth & longer chains                        |
| Teacher                    | Claude Sonnet 4.5 only                             | Possibly add a 7–14B local teacher for cost                   |
| Training method            | SFT (rejection-sampling) only                      | DPO/GRPO if SFT plateaus < 80 % L3                            |
| Base model                 | FunctionGemma-270M                                 | Larger base if 270M plateaus < target after SFT               |

**The discipline:** every "v1 only" decision goes in a `TODO(v2)` row
in [Stage 6 §dataset_card.md](#stage-6) so the next cycle has a
ready-made backlog. We do not let the narrow pilot turn into the
permanent corpus by accident.

---

## Multi-step tool calls — first-class category

The bootstrap eval has **1 compound scenario out of 50** (bs-050:
"disable material 3 and then color the mesh by effective stress").
That is not coverage; that is one example. The current 0 % L3 on
compound is therefore unmeasured, not measured.

For v1 SFT we treat multi-step as a top-level intent shape with its
own synthesis recipe, not a residual category sprinkled in at the end:

1. **Distinct intent_id family.** `compound-{select-show, material-view,
   step-query, …}` — each compound is a *named* two-or-three-step
   pattern with its own postcondition. Not a free-form "do two things"
   bucket.
2. **Postcondition shape.** A compound's postcondition checks the
   final state only (the verifier doesn't grade intermediate steps).
   If you need to assert ordering, use a `state_sequence` kind — but
   the v1 verifier kinds stay closed; new kinds go in
   `verifier.py` deliberately, not implicitly.
3. **Synthesis ratio.** ≥20 % of v1 scenarios are compound. The
   bootstrap's 2 % ratio is the failure mode this fixes.
4. **Teacher rollouts honor step structure.** No grammar-constrained
   compression of multiple calls into one — the rollout record must
   show the actual sequence the verifier saw.
5. **Held-out compounds.** The held-out fixture reserves at least
   one compound per intent family so generalization is measurable,
   not interpolated.

---

## Stage status

Stage numbering matches [`posttraining-dataset.md`](posttraining-dataset.md) §2.

- [x] **Stage 0** — `GrizSession` seam exists (`mili-viz-server`'s
      pygriz dispatcher; the M1 stub-fallback gap is closed, fixture
      resolver is loud-on-miss).
- [ ] **Stage 1** — Grammar / vocabulary extraction from `interpret.c`.
      Deferred: only gates `griz_raw` fallback grading, off v1 critical
      path. Counted-but-not-blocking.
- [x] **Stage 2** — Intent catalog `data/posttraining/intents/catalog.yaml`
      (11 atomic + 3 compound; closed-7 postcondition kinds; Risk #2
      resolved by keeping the set closed — see changelog rev 4).
- [x] **Stage 3** — Scenario synthesis. `data/posttraining/scenarios/synth.jsonl`
      lands 163 scenarios (41 compound, 25.15% ratio — clears the ≥20%
      gate) under `mili_llm_bench.synth` (see changelog rev 5). Round-trip
      pinned by `test_synth_round_trip.py` (8 tests). Two dispatcher gaps
      surfaced under live pygriz and parked in catalog `todo_v2`:
      `selection.clear_all()` (clrsel) and `Session.query` (query) — both
      missing on `crates/mili-viz-server` / `pygriz` today.
- [ ] **Stage 4** — Verifier (already exists at
      `python/mili-llm-bench/src/mili_llm_bench/verifier.py`; L0–L3,
      closed failure-mode taxonomy). Reuse, do not rebuild.
- [x] **Stage 6.5** — Claude data smoke test. Cleared the ≥85 % gate
      at **97.71 % L3 (171 / 175)** on synth.jsonl against Claude
      Sonnet 4.5 with native tool-use (no GBNF qualifier — Claude
      doesn't support grammar-constrained decoding). First pass
      (`v6-stage65-anthropic-smoke-20260524-191418`) failed the
      per-intent gate on a synth bug (`select` at 0/16 because
      `<param:class>` keys were not resolved); fix landed in
      `slots.py`, corpus regenerated deterministically, second pass
      cleared. See changelog rev 7.
- **Stage 5** — Teacher rollouts. Burns Anthropic API on every
      scenario; deliberately last among data stages. Split into pilot
      (first 50 of 175) and full sweep so the $50/$200 budget gates
      have a checkpoint.
  - [x] **Stage 5 pilot** — `stage5-pilot-anthropic-20260524-222350`:
        50 / 50 retention, $0.43, K=3, ≈ 6 min wall-clock. Cleared
        the ≤ $50 budget and ≥ 85 % retention gates; full sweep
        authorized. See changelog rev 11.
  - [x] **Stage 5 full sweep** —
        `stage5-fullsweep-anthropic-20260524-223426`: **171 / 175
        retention (97.7 %)**, **$1.88** (vs $200 gate — 106×
        headroom), K=3, ≈ 27 min wall-clock. Matches v7 stage-6.5
        ceiling exactly; same 4 `query` failures persist
        (parked under `query.todo_v2` per rev 7). See changelog rev
        12. **Stage 6 unblocked.**
- [x] **Stage 6** — Assembly, dedup, splits, `dataset_card.md`.
      Cleared in rev 13. `mili-llm-bench assemble` reads
      Stage 5 rollouts, dedups on
      `(normalized_instruction, fixture, tool_calls_flat)` (the §6
      key), splits by per-intent held-out cell (smaller of each
      `(intent, fixture)` pair, alphabetical-fixture tiebreak),
      enforces ≥20 % compound ratio in BOTH train and heldout,
      writes `sft/{train,val}.jsonl` (82 + 8 rows) +
      `eval/heldout.jsonl` (81 rows) + `pref/{train,val}.jsonl`
      (empty — K=3@T=0.7 produced 0 mixed-tier scenarios) +
      `dataset_card.md`. Tools-array conversion shared with the
      llamacpp inference path via `mili_llm_bench.tool_format`
      (train↔inference drift-proof). Contamination clean: heldout
      scenario IDs ∩ train+val = ∅; heldout cells ∩ train+val cells
      = ∅. See rev 13 changelog.
- [x] **Stage 7** — Eval harness. Loader landed in rev 14;
      **8 × 81 sweep ran in rev 21** via the new
      `--provider transformers --model-path` path. Curve: 59.3 % →
      66.7 % → 80.2 % → 93.8 % → 92.6 % → 95.1 % (plateau, epochs
      6–8); winner `checkpoint-126` at **77 / 81 = 95.1 %** L3 on
      the 81-row heldout split (`data/posttraining/runs/v1-sft-sweep/`).
      Symlink `data/posttraining/checkpoints/v1/winner →
      checkpoint-126`. All four gates cleared (regression tripwire,
      v1 target, stretch, per-intent floor); see rev 21 for the
      detailed curve, per-intent profile, and the four residual
      `select` semantic-disambiguation misses queued as a v2
      lever.
- [~] **Stage 8** — Pre-experiment gate. **Deferred 2026-05-25 (rev 18).**
      Original framing was "stock 0.5–1B + GBNF, does it clear the
      ceiling?" — a self-hosted-small-model gate. We already have the
      "what does a strong model do?" signal from Stage 6.5 at **97.71 %
      L3** (Claude Sonnet 4.5 on the same corpus distribution); no need
      to re-establish a ceiling via a self-hosted run. `google/gemma-2b-it`
      and `google/gemma-7b-it` are parked as **post-SFT fallbacks** —
      revisit only if `trainer.train()` lands below the regression
      tripwire (≥ 40 % L3) and the v2 levers from rev 17 (oversample
      free-text-bearing compounds; K=1 or T≥1.0 resampling) don't
      recover. No new GBNF authoring or TransformersProvider work
      on the v1 critical path.

The interface-independent stages (1, 2, 3, 4, 6, 6.5, 7, 8) can all
land before any teacher cost; Stage 5 is the only one that burns API.

---

## Training environment — NVIDIA H100 cluster

SFT training itself runs on an NVIDIA H100 cluster (single GPU is
plenty for 270M full BF16 fine-tune). The toolchain bring-up
(CUDA-enabled `llama.cpp`, `transformers` + `trl` + `flash-attn`
training stack, HF → GGUF conversion, and re-serving the trained
checkpoint through the existing bench harness) is documented in
[`cluster-setup.md`](cluster-setup.md). Data synthesis (Stages 2–4
/ 6 / 6.5) and Claude-API rollouts (Stage 5) can run anywhere — the
cluster is only on the critical path for **training + post-SFT eval**.

---

## Gates (falsifiable, no graceful slides)

These are not aspirational targets; they decide whether the next
stage runs.

| Gate                          | Threshold                          | Action on miss                                                   |
| ----------------------------- | ---------------------------------- | ---------------------------------------------------------------- |
| Stage 5 — pilot K & budget    | K=3, ≤ $50 for 50-scenario pilot   | Re-plan: smaller K, cheaper teacher, or fewer paraphrases        |
| Stage 5 — full-sweep budget   | ≤ $200 for ~200-scenario sweep     | Same — re-plan before scaling                                    |
| Stage 6 — per-intent SFT rows | ≥10 rows/intent in `sft/train.jsonl` (rebooted in rev 13; **TODO(v2):** restore to ≥40 once synth gains a paraphrase multiplier) | Flag the deficient intent in `dataset_card.md`; SFT pipeline decides whether to oversample or accept |
| Stage 6.5 — data quality      | ≥85 % L3 under Claude (native tool-use; no GBNF qualifier — Claude doesn't support it) | Hand-fix or drop failing scenarios; re-run before SFT |
| Stage 8 — pre-experiment gate | Stock 0.5–1B + GBNF < ceiling      | Confirms SFT room exists. If it *clears* ceiling: stop, ship that |
| SFT regression tripwire       | ≥40 % L3 post-SFT                  | Below the GEPA-only ceiling = SFT is harming. Stop and diagnose  |
| SFT v1 target                 | ≥62 % L3 post-SFT                  | Half the gap. Below: investigate before retraining               |
| Per-intent L3 floor (post-SFT) | ≥50 % L3 on material/select/clrsel/view-reset | These are the 0 % intents. Failing to move them = SFT taught the wrong thing |
| SFT v1 stretch                | ≥80 % L3                           | At/above: DPO/GRPO is incremental, not necessary                 |

---

## Risks and open questions

Carried, not resolved here. Most are pinned in
[`posttraining-dataset.md`](posttraining-dataset.md) §6 — listing
them here too so the live tracker shows the live unknowns.

1. **Held-out fixture choice.** `shell_mat2` vs. `bar5`. Needs decision
   before Stage 3 starts grounding params in fixture facts.
2. **Postcondition kinds for compound intents.** Closed set today is 7;
   may need `state_sequence` or a thin `composite` kind. Decide in
   Stage 2 with the catalog, not later.
3. **Re-measure Claude ceiling on promoted tools.** ~~Current 92 % was
   on pre-promotion `tools.json`. Re-run is cheap; queue it before
   Stage 5 so the gap measurement is matched-tools.~~ **Resolved
   2026-05-24 (rev 7).** Stage 6.5 on `synth.jsonl` (post-promotion
   `tools.json`, system_prompt_sha256 `9f36d0deb5e98a89`) measured
   Claude at **97.71 % L3 (171 / 175)**. The matched-tools ceiling
   for the SFT corpus is therefore ~98 %, not 92 %. The 92 % number
   on the 50-row bootstrap eval stands as a separate pinned baseline;
   they are different corpora, not directly comparable.
4. **Paraphrase diversity.** Stage-8 has a diversity check — if v1
   paraphrases collapse stylistically, that invalidates the corpus
   regardless of L3 numbers.
5. **`griz_raw` fallback grammar (Stage 1).** Deferred from v1, but it
   gates whether long-tail griz commands can ever participate. Track
   in v2 backlog.
6. **v1 corpus `function.arguments` as JSON string.** Discovered in
   rev 21. The Stage 5 driver writes each assistant rollout's
   `function.arguments` as a JSON **string** instead of a dict. The FG
   chat template's string-arguments branch (`chat_template.jinja`
   lines 194-197) inserts that literal between the call's curly
   braces, producing double-braced training tokens
   (`call:NAME{<whitespace>{<JSON>}}`) instead of the canonical FG-DSL
   (`call:NAME{key:<escape>value<escape>}`). The v1 checkpoints
   learned the accidental shape; `parse_fg_envelopes` was extended to
   accept both. v2 fix path: normalize string → dict in
   `assemble.project_sft_record` on the way out of dedup (path b in
   rev 21 (4)) so the next training run renders canonical FG-DSL and
   the parser's JSON-literal branch can retire. The current v1 corpus
   stays as the pinned input for this generation; not refactored
   in place.
7. **`select` per-intent floor at exactly 50 %.** Winner
   (`checkpoint-126`) clears the per-intent floor on the four 0-rate
   intents from v7 (colormap, show-primal, show-derived all at 100 %;
   select at 4/8 = exactly 50 %). The four residual misses are
   semantic disambiguation (range vs singular, brick vs node) — not
   parse-shape bugs; the model emits well-formed FG envelopes that
   just resolve to the wrong arguments. v2 lever: paraphrase
   multiplier on the `select` intent with disambiguating phrasings.
   Tracked under `query.todo_v2` analogue (`select.todo_v2`).

---

## How to operate this doc

- **One file moves: this one.** When a stage flips, update the
  checkbox and add a one-line entry in the changelog below.
- **Numbers in tables get re-pinned, not edited in place.** If the
  v5 floor moves, add a v6 row; don't mutate v5.
- **`TODO(v2)` is a real label.** Anything we punt for the pilot
  lands there with one sentence on *what* and *why deferred*.
- **External pointers stay external.** This doc references but does
  not duplicate `posttraining-dataset.md`, the verifier source, or
  the memory entries — they are the source of truth.

---

## Changelog

- **2026-05-25 (rev 22)** — **GGUF round-trip clean; v1 SFT ships.**
  Preflight #6 (`sft-preflight-gpu.md` §6) cleared on `winner →
  checkpoint-126`; GGUF conversion produced
  `data/posttraining/checkpoints/v1/functiongemma-v1.bf16.gguf` (236
  tensors, 542 MB); llamacpp re-eval on the same 81-row heldout split
  matched the HF number exactly — **77 / 81 = 95.06 % L3**, identical
  per-scenario outcomes, identical model tool_calls on the four
  residual `select` failures.

  **Preflight #6 — pre-conversion diff (HF source
  `google/functiongemma-270m-it` snapshot `39eccb09…` vs
  `data/posttraining/checkpoints/v1/winner/`).** Read-only.

  | File | Result |
  | --- | --- |
  | `chat_template.jinja` (rendering authority) | byte-identical (sha256 `db61fb01…`, 13792 B) |
  | `special_tokens_map.json` | byte-identical |
  | `added_tokens.json` | byte-identical |
  | `tokenizer.json` | 2 of 6416 added-token entries flipped `special: False → True` |
  | `tokenizer_config.json` | same 2 entries flipped (consistent with `tokenizer.json`) |
  | `chat_template` field *inside* `tokenizer_config.json` | absent on both sides — both rely on the sibling `chat_template.jinja` |

  The two flips (cosmetic; all other 6414 added tokens unchanged):
  `id=50 <start_function_response>` and `id=255999 <start_of_image>`,
  both `special: False → True`. Critically unchanged: `id=48
  <start_function_call>` and `id=49 <end_function_call>` — the FG
  envelope tokens the model *emits*. The flip is cosmetic for the
  L3 emission path: AddedTokens always tokenize as one piece
  regardless of the `special` flag, so the encoded token stream the
  model sees is identical; the flag only affects
  `decode(skip_special_tokens=True)`, and the emission path is the
  unchanged `<start_function_call>`/`<end_function_call>` pair
  (id 48/49). Likely cause: TRL/transformers `save_pretrained()`
  normalizing the flag during checkpointing; not investigated in
  detail because the L3 round-trip below is byte-identical.

  **GGUF conversion** via
  `convert_hf_to_gguf.py /p/vast1/whitmore/cadsat/mili-rs/data/posttraining/checkpoints/v1/winner --outtype bf16`
  — 236 tensors, 536.3 M total, BF16. Used llama.cpp's in-tree
  `gguf-py` on `PYTHONPATH` with the existing mili-rs uv `train`
  extra (`torch 2.12.0+cu130`, `transformers 4.57.6`,
  `safetensors 0.7.0`, `numpy 2.4.6`); no new dependency installs.
  Pure-Python operation; **did not** source
  `scripts/setup-gpu-env.sh` (same rule as training /
  TransformersProvider — converter uses torch's bundled CUDA
  runtime, not the module-loaded one).

  **GGUF post-conversion template inspection.** Read the
  `tokenizer.chat_template` field out of the GGUF via
  `gguf.GGUFReader`; **byte-identical to both HF source and winner
  `chat_template.jinja`** (sha256 `db61fb01…`, 13792 B). The GGUF
  converter round-tripped the template cleanly — the failure mode
  preflight #6 exists to catch (jinja whitespace normalization,
  specials reordering, etc.) did not fire.

  **llamacpp re-eval** on `matrix41` against the new GGUF, b9307
  `549b9d843` with `--jinja`. `/props` reports
  `supports_tool_calls=False` (same b9307 caps as the v7 baseline)
  → rev-10 client-side fallback in `LlamaCppProvider.generate`
  engages. Run artifacts at
  `data/posttraining/runs/v1-sft-winner-gguf-20260525-161145/`.

  **Result vs HF baseline (per-scenario diff against
  `v1-sft-sweep/checkpoint-126-20260525-152545`):**

  | Check | HF (transformers) | GGUF (llamacpp) |
  | --- | --- | --- |
  | L3 pass rate | 77 / 81 = 95.06 % | **77 / 81 = 95.06 %** |
  | Wall-clock | 44.2 s | 49.8 s |
  | Failure modes | 4× `wrong_selection` | 4× `wrong_selection` |
  | Failing scenario IDs | `synth-00042,43,46,47` | **identical set** |
  | Per-scenario tier/failure_mode mismatches | — | **0 of 81** |
  | Model `tool_calls` on each failing scenario | (reference) | **byte-identical to HF** |

  The model emitted the same exact wrong answer on each of the four
  `select` semantic-disambiguation misses through both decode
  paths — strongest possible evidence the BF16 GGUF conversion
  preserved deterministic greedy decode without precision loss
  material to L3 grading.

  **Ship state.** `v1 SFT winner` =
  `data/posttraining/checkpoints/v1/winner → checkpoint-126` (HF) +
  `data/posttraining/checkpoints/v1/functiongemma-v1.bf16.gguf` (GGUF
  for serving). Both bear the same 95.06 % L3 number on the 81-row
  heldout. Quantization (Q4_K_M) **not pursued** for v1 — BF16 GGUF
  already serves at the target number on H100; quantization is a
  v2+ lever (edge / CPU serving) and would only introduce noise to
  rule out, not a problem to solve.

  **v2 backlog: unchanged from rev 21.** Two items still pinned
  in "Risks and open questions" — (#6) v1-corpus
  JSON-literal `function.arguments` shape (path: normalize in
  `assemble.project_sft_record`), (#7) `select` per-intent floor
  at exactly 50 % (paraphrase-multiplier lever). No new v2 items
  added by this rev; the preflight #6 `special`-flag flip is the
  rare delta that's both detected *and* provably benign on
  inspection.

  **Test deltas.** No code changes this rev; 250 / 250 + 1 skip
  stand.

- **2026-05-25 (rev 21)** — **Eval path landed; v1 corpus data-shape
  discovery flagged for v2.** Three changes plus one finding.

  **(1) `TransformersProvider` landed** at
  `python/mili-llm-bench/src/mili_llm_bench/providers/transformers.py`.
  Loads an HF checkpoint in-process
  (`AutoTokenizer.from_pretrained` + `AutoModelForCausalLM.from_pretrained(
  dtype=torch.bfloat16, attn_implementation="eager", device_map="cuda")`);
  re-uses `tokenizer.apply_chat_template(messages, tools=tools,
  add_generation_prompt=True)` — the same call SFTTrainer's
  `formatting_func` rendered against — so prompt distribution at eval
  matches training byte-for-byte; greedy decode at `temperature=0` with
  `do_sample=False`; converts the bench harness's W1-shape tools to
  OpenAI shape via the existing `tool_format.w1_to_openai_tool` helper
  (same call site as `LlamaCppProvider`). CLI: `--provider transformers
  --model-path <ckpt-dir>` (required; raises `ValueError` if missing).

  **(2) Shared FG envelope parser** at
  `python/mili-llm-bench/src/mili_llm_bench/providers/_fg_envelope.py`.
  Lifted the `_FG_ENVELOPE_RE` / `_FG_STRING_ARG_RE` / `_FG_BARE_ARG_RE`
  regexes out of `providers/llamacpp.py` so the train- and
  inference-time paths can't drift on what `<start_function_call>call:NAME{
  …}<end_function_call>` means. Both `LlamaCppProvider` and
  `TransformersProvider` import `parse_fg_envelopes`; a
  drift-prevention test pins both providers' references to the same
  function object (mirrors the `TestToolFormatHelper` pattern from
  rev 13).

  **(3) `FunctionGemmaProvider` deleted** (along with its tests and the
  `[functiongemma]` pyproject extra). Its parser expected
  `<start_function_call>[{"name": "…", "arguments": {…}}]<end_function_call>`
  — a JSON-list inner payload that no FG checkpoint ever emits. The
  helper was authored alongside the v0 baseline before the FG-DSL was
  understood; the v0–v3 baselines that "used" it ran against the empty
  M1 stub corpus and graded ~0 % L3 for unrelated reasons (no fixtures
  resolved), so the parser's brokenness was masked. No tracked baseline
  was ever produced by it correctly. `TransformersProvider` replaces it
  wholesale; the `train` extra (`transformers` + `torch` + `accelerate`)
  was already in place from the SFT training stack, so no dep churn.

  **(4) Finding — v1 corpus data shape: `function.arguments` as a
  string instead of a dict.** The Stage 5 driver wrote each assistant
  rollout's `function.arguments` as a JSON **string** (e.g.
  `'{"root": "d3samp6"}'`) rather than a dict (`{"root": "d3samp6"}`).
  The FG chat template's `arguments` branch checks `is mapping` first,
  but with strings it falls into the secondary `is string` branch
  (`chat_template.jinja` lines 194-197) and renders the literal JSON
  text between the call's curly braces — producing
  `<start_function_call>call:load{                    {"root": "d3samp6"}}<end_function_call>`
  in the training tokens. The v1 checkpoints learned this exact
  double-braced shape; the probe against `checkpoint-21` confirms
  greedy decode emits the same.

  *Implication on what training actually learned.* The model still
  learned a correct mapping from prompt → structured tool call — the
  tool name, the argument keys, and the argument values are all in the
  output and the JSON inside the braces is well-formed in the 3 / 3
  smoke. The training *target* shape is just different from the FG
  chat template's documented dict-rendering. Not a correctness bug in
  the trained weights; a data-rendering bug in the corpus that ships
  to v1 as an accidental encoding choice.

  *Parser handles both shapes.* `parse_fg_envelopes` now JSON-parses
  the envelope body first; on success returns those args, on failure
  falls through to the existing `<escape>` / bare-scalar FG-DSL
  pass. Stock pretrained FG-270M (which emits the FG-DSL — the
  v7 26 % L3 llamacpp baseline) still parses correctly via the
  fallthrough; the v1 SFT checkpoints (which emit the JSON-literal
  shape) parse via the JSON branch. New test pins in
  `tests/test_fg_envelope.py::TestParseEnvelopes::
  test_json_literal_body_shape` (and `_with_compound_calls`); existing
  FG-DSL pins unchanged.

  *`TODO(v2)` — re-render the training corpus with dict-shaped
  arguments.* Two paths: (a) fix the Stage 5 driver to write
  `arguments` as a dict on the rollout's wire shape, then re-assemble;
  (b) fix `assemble.project_sft_record` to normalize string → dict
  on the way out of dedup, leaving Stage 5 untouched. Path (b) is
  smaller and lets the existing rev-12 rollouts feed forward without
  re-running teacher API calls. The current v1 corpus stays as the
  pinned input for the in-flight sweep; v2 corpus is the place to
  fix.

  **(5) Sweep results — all four gates clear.** Sweep artifacts at
  `data/posttraining/runs/v1-sft-sweep/checkpoint-*-20260525-152545/`;
  full log at `sweep-20260525-152545.log`. Wall-clock ~10 min total
  across 8 checkpoints on `matrix41` (≈ 70 s per checkpoint /
  81 scenarios). The per-epoch L3 curve on the 81-row heldout:

  | Epoch | Checkpoint        | L3        | Notes                          |
  |-------|-------------------|-----------|--------------------------------|
  | 1     | `checkpoint-21`   | 48 / 81 = **59.3 %** | Above regression tripwire on epoch 1; epoch with non-trivial gradient signal. Falls short of per-intent floor on colormap (0/9) and show-derived (0/3). |
  | 2     | `checkpoint-42`   | 54 / 81 = **66.7 %** | Crosses v1 target; per-intent floor cleared on all four 0-rate intents (colormap 6/9, select 5/8, show-primal 9/9, show-derived 3/3). But compound-material 0/6 and compound-state 0/7 regress vs epoch 1. |
  | 3     | `checkpoint-63`   | 65 / 81 = **80.2 %** | Crosses stretch gate. Compound-material partial (2/6), compound-state partial (3/7). |
  | 4     | `checkpoint-84`   | 76 / 81 = **93.8 %** | Saturates most intents; per-intent floor cleared. |
  | 5     | `checkpoint-105`  | 75 / 81 = **92.6 %** | Slight regression on query (2/4 vs prior 4/4); not material. |
  | 6     | `checkpoint-126`  | 77 / 81 = **95.1 %** | **Plateau begins.** All non-`select` intents at 100 %. ✅ winner. |
  | 7     | `checkpoint-147`  | 77 / 81 = **95.1 %** | Identical per-intent profile to `-126`. |
  | 8     | `checkpoint-168`  | 77 / 81 = **95.1 %** | Identical per-intent profile to `-126`. (= `final/`) |

  **Winner: `checkpoint-126`.** Three-way tie at the plateau; same
  per-intent numbers across the three. The tiebreak is *earliest of
  the plateau triplet* — less over-trained, identical eval signal.
  Symlink `data/posttraining/checkpoints/v1/winner → checkpoint-126`
  is the single canonical pointer for post-winner work.

  **Per-intent profile at the plateau (`checkpoint-126`).** 13 / 14
  intents at 100 % L3. Single residual cell: `select` at **4 / 8**
  (exactly the ≥ 50 % per-intent floor; not above). The four
  persistent `select` failures are all semantic, not parse-shape:

  - `synth-00042` "select bricks 7" → model emits `range: '1-7'`
    instead of `range: '7'` (confused singular vs range).
  - `synth-00043` "pick brick 7" → same pattern.
  - `synth-00046` "select nodes 1" → model emits
    `class_name: 'brick', range: '1'` (class confusion: node →
    brick).
  - `synth-00047` "pick node 1" → emits `class_name: 'node'`
    (correct class) but no `range` (omitted required arg).

  These are not training-pipeline bugs; they're genuine
  paraphrase-disambiguation failures. The model knows the FG-DSL
  shape and the `select` tool exists; it doesn't disambiguate
  *which element class* / *singular vs range* from natural language.
  This is the v2 paraphrase-multiplier lever — more training rows
  per intent with disambiguating phrasings would lift this cell.

  **Gates (post-sweep status):**

  - [x] Regression tripwire (≥ 40 % L3): cleared on epoch 1
        (`checkpoint-21` at 59.3 %).
  - [x] v1 target (≥ 62 % L3): cleared on epoch 2 (`checkpoint-42`
        at 66.7 %).
  - [x] Stretch (≥ 80 % L3): cleared on epoch 3 (`checkpoint-63`
        at 80.2 %).
  - [x] Per-intent floor (≥ 50 % on colormap / select /
        show-primal / show-derived): cleared starting epoch 4
        (`checkpoint-84` — colormap 100 %, select 50 %,
        show-primal 100 %, show-derived 100 %). At the winner the
        floor is still **exactly 50 %** on select; v2 lever
        flagged.
  - [x] Winner choice defensible: earliest of the plateau triplet,
        identical per-intent profile to later plateau checkpoints,
        less over-trained.
  - [x] Per-checkpoint L3 curve recorded above.
  - [x] `data/posttraining/checkpoints/v1/winner → checkpoint-126`
        symlink in place.

  **Two side-observations from the curve.**

  - **Loss=0 by epoch 6 (rev 20 observation) does not mean
    over-memorization.** The heldout split is in-distribution
    (same Stage-5 rollout pool, different `(intent, fixture)`
    cells), so 95 % L3 is *plausibly* memorization of patterns
    rather than generalization. But the *late-epoch plateau is
    flat, not regressing* — there is no observed overfitting cost
    to training the full 8 epochs vs. early-stopping at epoch 6.
    Picking `-126` over `-168` is paranoia-driven (zero
    measurable lift), not signal-driven.
  - **Epoch-by-epoch curve has a non-monotone dip.**
    `checkpoint-105` at 92.6 % is below `checkpoint-84` at 93.8 %
    — a 1-scenario regression on `query` (2/4 → was 4/4 at -84;
    back to 4/4 at -126). Within the noise floor of a 4-scenario
    cell. No action needed.

  **Path forward (winner-only).** Preflight #6 (GGUF chat-template
  baking diff, `sft-preflight-gpu.md` §6) on `winner →
  checkpoint-126` → GGUF conversion (`cluster-setup.md` §7) →
  `--provider llamacpp` re-eval on the same heldout split
  (`cluster-setup.md` §8b) to confirm the GGUF round-trips the same
  95.1 % L3 as the HF path. If the llamacpp number matches, ship.

  **Test deltas.** 229 / 229 + 1 skip before → 250 / 250 + 1 skip
  after (added 22 new pins across `test_fg_envelope.py` (13) +
  `test_providers_transformers.py` (12), deleted 5 `test_providers_functiongemma.py`
  always-on pins + 1 skip-gated; net +17 always-on, ‑1 skip-gated).
  All `mili-llm-bench` tests green.

- **2026-05-25 (rev 20)** — **`trainer.train()` landed; v1 checkpoint
  pool ready for heldout eval.** First end-to-end SFT run on
  `matrix41` H100: 168 steps over 8 epochs, **111.9 s wall-clock**
  (≈ 14 s/epoch). Output at
  `data/posttraining/checkpoints/v1/`: 8 per-epoch checkpoints
  (`checkpoint-21` through `checkpoint-168` — TRL's
  `save_strategy="epoch"`) plus an explicit `final/` copy of
  epoch 8. Total disk: 13 GB.

  **Loss curve — strong memorization signal.** Epoch 1 trained
  from `loss=4.13` to `loss=0.07` (the 1-epoch smoke window
  captured this); epoch 2 drifted between 0.1 and 0.0; epochs 3–8
  ran at essentially `loss=0.0` with `mean_token_accuracy=1.0`,
  `entropy≈0`, and `grad_norm` in the 1e-3 to 1e-2 range. Final
  reported metrics: `train_loss=0.095` (averaged), `eval_loss=2.5e-5`,
  `eval_mean_token_accuracy=1.0`. **The eval signal is not
  generalization** — `val.jsonl` (8 rows) is split from the same
  Stage-5 rollout pool as `train.jsonl`. The actual generalization
  test is `eval/heldout.jsonl` (81 rows, separate
  `(intent, fixture)` cells). The strong-memorization signal across
  the late epochs is the reason `save_strategy="epoch"` was wired
  into the recipe up front: the per-checkpoint heldout sweep picks
  the right stopping point empirically rather than us guessing now.
  Plausible candidates: `checkpoint-21` (epoch 1, the only epoch
  with non-trivial gradient signal) or one of the early
  checkpoints (2–4) if memorization of tool-call syntax helps L3
  without hurting semantic correctness.

  **One smoke-driven fix landed in the recipe.** First smoke run
  OOM'd at end-of-epoch eval — FG's 262K-token vocab × `max_length=4096`
  makes the per-batch logits tensor enormous, and HF's default
  `per_device_eval_batch_size=8` (not equal to train batch size as
  one might assume) compounds it. `sft_train.py` now defaults
  `per_device_eval_batch_size=1`. Training batch size unchanged.
  Second smoke + full run both cleared without retry.

  **Eval path decision.** GGUF conversion + preflight #6 are
  **deferred** to post-winner selection. The eval harness can grade
  HF checkpoints directly via a new TransformersProvider —
  `tokenizer.apply_chat_template(messages, tools=tools, …)` is the
  source of truth that training rendered against, and re-using it
  for eval eliminates the GGUF chat-template-mutation risk that
  preflight #6 exists to catch (we only have to clear that gate
  for the one shipping checkpoint, not all 8). vLLM's native FG
  tool parser is the upstream alternative but isn't in the env;
  TransformersProvider is the lighter lift. Pending implementation.

  **Path forward.** TransformersProvider → per-checkpoint heldout
  eval → pick best L3 by gate (regression tripwire ≥ 40 %, v1
  target ≥ 62 %, stretch ≥ 80 %, per-intent floor ≥ 50 % on the
  four 0-rate intents) → preflight #6 + GGUF conversion only for
  the winner → ship.

- **2026-05-25 (rev 19)** — **Training entry point landed.**
  `python/scripts/sft_train.py` realizes the `cluster-setup.md` §6
  recipe (rev 4) as a runnable argparse-driven script. Defaults match
  the pinned hyperparameter table verbatim; overrides require a
  one-line justification in the run's `dataset_card.md` so silent
  drift is impossible. `scripts/sft_train.sbatch` is the slurm
  submission wrapper — works via `sbatch` from a login node, `srun`
  interactively, or direct execution from an existing GPU shell
  (`#SBATCH` lines are bash comments outside slurm; the script
  `exec`s into `uv run --directory python python scripts/sft_train.py
  "$@"` at the end). Forwards arbitrary CLI overrides to the python
  script. Short-circuits with the exact sbatch / srun recovery
  commands when launched from a non-GPU host (caught the
  `matrix2`-login-node "ran it from the wrong shell" failure mode).
  Inlines the training-safe subset of `setup-gpu-env.sh`'s exports
  (`HF_HUB_DISABLE_TELEMETRY=1`, `UV_LINK_MODE=copy`) but does not
  source it — the `cuda/12.9.1` module-load would put system CUDA on
  `LD_LIBRARY_PATH` and risk shadowing torch's bundled CUDA 13
  runtime (same reasoning as `gpu-sanity.sh`'s top-of-file comment).
  Launch instructions live in `cluster-setup.md` §6 "Launching the
  run". Drive-by drift fix in §6's constraints bullet (TRL native
  `assistant_only_loss=True` → `MaskAssistantOnlyCollator` per rev 4)
  rolled in.

  **Smoke run in flight** on `matrix41` H100 via `srun --partition=pbatch
  --time=00:15:00 --gres=gpu:1 ./scripts/sft_train.sbatch
  --num-train-epochs 1 --output-dir /tmp/sft-smoke`. Pending result.

- **2026-05-25 (rev 18)** — **Stage 8 deferred to post-SFT fallback.**
  Original framing was a self-hosted-small-model gate ("stock 0.5–1B
  + GBNF, does it clear the ceiling?"). Reframed in conversation:
  Stage 6.5 already measured Claude Sonnet 4.5 at **97.71 % L3
  (171 / 175)** on `synth.jsonl` — that *is* the "what could we do
  with a stronger model" signal, and the only reason we're still on
  the FunctionGemma path is the self-hosted-small-deployment story,
  not raw quality. A self-hosted Stage 8 run with grammar-constrained
  gemma-7b-it would (a) require GBNF authoring + a TransformersProvider
  (or GBNF + GGUF conversion + llama-server), (b) test "with max
  format help, can a larger general model match a strong API model?"
  — a question whose answer doesn't change the v1 SFT decision either
  way. `google/gemma-2b-it` and `google/gemma-7b-it` are parked as
  **post-SFT fallbacks**: revisit only if `trainer.train()` lands
  below the regression tripwire (≥ 40 % L3) and the rev-17 v2 levers
  (oversample free-text-bearing compounds; K=1 or T≥1.0 resampling
  for DPO pairs) don't recover.

  **Path forward.** Critical path collapses to `trainer.train()`
  (cluster-setup.md §6, rev 4 recipe) → preflight #6 (GGUF
  chat-template baking, gated on a trained checkpoint) → real
  Stage 7 eval pass against `eval/heldout.jsonl`. No tracker artifacts
  modified; the stage entry above flips to `[~]` (deferred) with
  the fallback policy pinned inline.

- **2026-05-25 (rev 17)** — **Preflight #4 cleared on `matrix41` H100
  via option (B) — custom data collator.** New module
  `python/mili-llm-bench/src/mili_llm_bench/assistant_only_collator.py`
  (`MaskAssistantOnlyCollator`) replaces TRL 1.5.0's native
  `assistant_only_loss=True` path, which **fails at
  `SFTTrainer.__init__`** on FunctionGemma's chat template with
  `ValueError: The chat template is not training-compatible (missing
  prefix-preservation or {% generation %} markers) and patching is not
  supported for this template.` The FG template is macro-heavy
  (`format_parameters`, `format_function_declaration`,
  `format_argument`) and TRL's auto-patch can't infer assistant
  boundaries from the substituted-`role` rendering pattern. The
  rev-12 config-seam claim in `cluster-setup.md` §0 (line 53-55) was
  therefore genuinely vacuous — the kwarg exists on TRL 1.5.0
  (rev-16's bump made it accept) but the trainer dies before any
  batch is produced. Option A (patching the FG template to add
  `{% generation %}` markers) would require structurally rewriting
  the for-loop body — Jinja requires balanced block tags that cannot
  span `{% endif %}` boundaries — so the smaller and more
  inspection-friendly path was option B.

  **Collator algorithm (two passes per row).** Pass 1: find
  `[<start_of_turn>=105, model=4368, \n=107] ... <end_of_turn>=106`
  spans; unmask labels from header-end (exclusive) through EOT
  (inclusive — model learns to stop). Pass 2: subtract tool-response
  payloads inside each span — positions inside
  `<start_function_response>response: ... <end_function_response>`
  (token IDs `[50, 6275, 236787, ..., 51]`). The bare
  `<start_function_response>` (token 50) that the assistant emits at
  the end of its last tool_call (with no following `response:`)
  stays unmasked, because that signal is the model's own output.

  **Verification.** Single-row probe (3310 non-pad tokens): 17
  visible labels (0.51 %) — the decoded visible content is exactly
  `<start_function_call>call:load{...}<end_function_call><start_function_response>`,
  with the entire tool-response payload correctly masked. Full-corpus
  scan over all 82 train rows: visible / non-pad min=0.40 %, p50=0.52
  %, p95=1.17 %, max=1.17 %; visible-tokens min=13, p50=17, p95=39,
  max=39 (single-tool rows ≈ 13–17, compound multi-step rows up to
  39); **0 / 82 rows collapsed to all -100**. Cross-check with
  `mask=off`: visible / non-pad deviation = 0.0000 (matches
  HF default pad-only masking).

  **BOS-doubling side observation — resolved.** Rev 16 flagged that
  the `formatting_func` path produced doubled `<bos>` and the TRL
  1.x auto-detect path produced single. On the option-B path
  (`assistant_only_loss=False` + custom data collator), **both
  formatting_func=on and =off produce single `<bos>`** — TRL 1.5.0's
  tokenize step under this config honors the BOS already in the
  formatted string and does not prepend another. No per-row BOS tax.
  formatting_func is retained in the recipe for drift-proofing per
  rev 16.

  **Side observation worth flagging (not blocking).** Per-row visible
  fraction is 0.40 %–1.17 % — small because each row carries
  ~2700 tokens of tool declarations (preflight #5 finding). The
  model still gets strong gradient signal on tool-call syntax (every
  row produces tool-call envelopes), but free-text gradient signal is
  sparse. **`TODO(v2)`:** if SFT plateaus below the regression
  tripwire, oversample compound scenarios that include an explicit
  final assistant text turn.

  **Tests.**
  `python/mili-llm-bench/tests/test_assistant_only_collator.py` — 8
  pins (always-on except a runtime FG-tokenizer ID check). 229 / 229
  pass + 1 skip on the full `mili-llm-bench` suite (+8 from rev 16's
  221).

  **§6 recipe landed in `cluster-setup.md` rev 4.**
  `SFTConfig(assistant_only_loss=False)` at the TRL level;
  `SFTTrainer(...)` constructor now passes
  `data_collator=MaskAssistantOnlyCollator(
  DataCollatorForLanguageModeling(tokenizer=tok, mlm=False))`. The
  training *intent* (compute loss only on assistant turns) is
  unchanged; only the implementation moves from a TRL kwarg to a local
  wrapper. Hyperparam table row reworded accordingly. No other
  hyperparameters changed.

  **Path forward.** Preflight #4 ✅. Preflight #6 (GGUF chat-template
  baking) still gated on a trained checkpoint. Stage 8
  (pre-experiment gate) remains runnable in parallel and should land
  before `trainer.train()` so the SFT lift is measurable.

- **2026-05-25 (rev 16)** — **TRL pin bumped from `>=0.11,<0.13`
  to `>=1.0,<2`.** The rev-2 pin (in `cluster-setup.md` line 211)
  was self-contradicting: its stated justification was
  `assistant_only_loss`, but that kwarg was added in trl 0.20+ —
  neither 0.11 nor 0.12 supported it. Preflight #3 exposed the gap
  (rev 15), at which point Google's own FunctionGemma fine-tuning
  guide (no version pin; uses `max_length=512` which is the
  trl 0.13+ spelling) and the trl 1.5.0 release made the path
  forward obvious.

  **What changed.**
  - `python/mili-llm-bench/pyproject.toml`: `train` extra
    `trl>=0.11,<0.13` → `trl>=1.0,<2`.
  - `python/scripts/sft_dump_one_batch.py`: `max_seq_length`
    (0.12.x spelling) → `max_length` (trl 1.x spelling); dropped
    the drift apology in the docstring.
  - `cluster-setup.md` §0 pin paragraph + §6 recipe knob table +
    §6 recipe block: pin updated; `max_length=4096` mirrored from
    rev-14 preflight #5 (was stale at `512` here); `formatting_func`
    comment annotated as "mandatory on 0.12.x, optional on 1.x".
    New rev-3 changelog entry over there.

  **What was tested.** Full `uv sync --directory python --all-extras`
  → trl 1.5.0 + transformers 4.57.6 + torch 2.12.0+cu130 +
  datasets 4.8.5 + accelerate 1.13.0 (transformers/torch unchanged
  from rev 14). Full `pytest -q` from
  `python/mili-llm-bench/` → **221 passed, 1 skipped** (same as
  rev 14 baseline). Preflight #3 retested on `matrix41` H100:
  - `--with-formatting-func`: PASS, `input_ids.shape = (1, 3311)`
    (matches rev 15's TRL 0.12.1 number to within 1 token).
  - `--without-formatting-func`: **also PASS** on TRL 1.5.0
    (vs. `KeyError: 'text'` on TRL 0.12.1) — TRL 1.x has chat-dataset
    auto-detection that dispatches `apply_chat_template` when it
    sees `messages` + `tools` columns. `formatting_func` is no
    longer mandatory; we keep it in the §6 recipe for drift-proofing
    against a future TRL 2.x auto-detect change.

  **One side observation worth surfacing.** The `formatting_func`
  path produces a **doubled BOS** in `decoded[0]`
  (`<bos><bos><start_of_turn>developer`), while the TRL 1.x
  auto-detect path produces a single `<bos>`. Likely cause: the
  explicit `formatting_func` returns a string from
  `apply_chat_template(..., tokenize=False)` that already includes
  BOS, and TRL's tokenize pass prepends BOS again. For training,
  a single leading `<bos>` is correct — so either (a) drop the
  `formatting_func` and rely on auto-detect, or (b) pass
  `add_special_tokens=False` somewhere downstream of the formatter.
  **Decision queued for preflight #4**: I'll grep whether the
  TRL 1.x SFTTrainer strips a leading BOS before re-tokenizing or
  not, and pick the path that produces a single BOS. The chat
  template's correctness for FG is already pinned (preflight #2 —
  the GGUF baked template + HF tokenizer template are identical),
  so this is a per-row token-budget concern, not a semantic one.

  **Stop-loss.** If a future bump (transformers 5.x, torch 3.x,
  trl 2.x) breaks this stack: revert to `trl>=0.20,<0.21` — that's
  the oldest release that still carries `assistant_only_loss`
  natively, and was the conservative-bump option I would have picked
  if Google's guide had been more cautious.

  **Path forward.** Preflight #4 (`assistant_only_loss=True` mask
  check) is now genuinely runnable — TRL 1.5.0 actually has the
  kwarg, so we can run the on-GPU compat check the §4 recipe in
  `sft-preflight-gpu.md` describes. Stage 8 (pre-experiment gate)
  is also still runnable in parallel.

- **2026-05-25 (rev 15)** — **Preflight #3 cleared on `matrix41`
  H100.** New runbook script `python/scripts/sft_dump_one_batch.py`
  builds an `SFTTrainer` over `data/posttraining/sft/sft/train.jsonl`,
  pulls the first batch via `get_train_dataloader()`, and asserts
  the `<start_function_declaration>` token block reaches the
  tokenized batch. Result with `formatting_func`: **PASS** — 18 tool
  declarations + the assistant `<start_function_call>` envelope
  land verbatim in `decoded[0]`; `input_ids.shape = (1, 3311)`
  (mid-band per the preflight #5 audit, `max=3341` overall).
  Result without `formatting_func`: hard failure — TRL 0.12.1's
  `_prepare_non_packed_dataloader` defaults
  `dataset_text_field="text"` and raises `KeyError: 'text'`.
  **Decision: `formatting_func` is mandatory** in the v1
  `trainer.train()` recipe; pin the
  `tokenizer.apply_chat_template(messages, tools=tools, …)` form
  used by the script. Report:
  `data/posttraining/sft/preflight-3-tokenized-batch.md`.

  **Two API drifts in the `cluster-setup.md` §6 recipe surfaced
  and recorded (not fixed here):** (a) `SFTConfig(max_length=…)`
  doesn't exist on TRL 0.12.1 — the 0.12.x spelling is
  `max_seq_length` (renamed `max_length` in trl 0.13+). (b)
  `SFTConfig(assistant_only_loss=True)` doesn't exist on TRL
  0.12.1 either; the kwarg was added in trl 0.20+. The "config
  seam" that already landed pre-rev 12 is presumably a custom
  data-collator path that drops non-assistant labels to `-100`
  server-side — to be verified during preflight #4. **Path
  forward:** before `trainer.train()` we either (a) lift the
  trl pin to ≥ 0.20 in the `train` extra and use the native
  `assistant_only_loss=True`, or (b) keep the custom collator
  and ship trl 0.12.1. Preflight #4 picks the path.

  Two other small findings in the script:
  - The §3 recipe's `model=None` shortcut is stale —
    transformers 4.57's `Trainer.__init__` rejects it with
    "requires either a `model` or `model_init`". The script
    loads `google/functiongemma-270m-it` (already cached from
    preflight #1) on CPU in BF16; ~540MB, no forward/backward
    run.
  - Padding-side warning: TRL warns
    `processing_class.padding_side` is not `"right"`. Cosmetic
    for a dump-only pass; recipe-side fix is to set
    `tokenizer.padding_side = "right"` before constructing the
    trainer.

  **Path forward.** Preflight #4
  (`assistant_only_loss=True` mask check / TRL version
  decision) is unblocked. #6 (GGUF chat-template baking) still
  gates on a trained checkpoint. Stage 8 (pre-experiment gate)
  is also runnable and should land before `trainer.train()`.

- **2026-05-25 (rev 14)** — **Stage 7 loader + preflight #5
  cleared.** Three landing changes:

  **(1) Assembled corpus is now self-contained.**
  `assemble.project_sft_record` lifts
  `record["verifier"]["postcondition"]` to a top-level
  `postcondition` field on each emitted SFT/heldout row (option (a)
  of the "where does Stage 7 read the postcondition from?" decision
  — option (b) was a synth.jsonl join, rejected because a future
  synth regeneration would silently rewrite heldout postconditions
  and invalidate the eval set with no detectable trace). One new
  pin: `tests/test_assemble.py::TestProjectRecord::
  test_postcondition_preserved`. Existing pins in `TestEndToEnd`
  (`test_contamination_clean`, the rev-12 rollouts smoke) still
  pass — 20 / 20 assemble tests.

  **(2) Stage 7 loader auto-detects the assembled shape.**
  `mili_llm_bench.scenarios` gains `_parse_assembled_record`,
  `_is_assembled_record` (keys on `scenario_id` AND `messages` — an
  assembled row could theoretically grow an `id` field, so both
  discriminators are checked), `load_scenarios_from_assembled`
  (strict — rejects synth-shape rows for explicit eval-harness use)
  and a per-row auto-detect inside `load_scenarios`. The eval reads
  `eval/heldout.jsonl` standalone — no synth.jsonl path required at
  eval time. Stage 7 mock smoke against the real 81-row heldout
  split (`runs/stage7-smoke-mock-20260525-000258/`) graded
  end-to-end: 81 / 81 `parse_error` from the mock provider is the
  expected behavior; the success signal is that all 81 rows loaded,
  resolved to `Scenario` objects, and reached the verifier without
  any synth-join lookup. **Loader-side unit pins are a follow-up**
  (the heldout smoke is end-to-end coverage but does not lock the
  parser's error paths) — tracked in `TODO(v2)`.

  **(3) Preflight #5 cleared off-GPU.** New module
  `python/mili-llm-bench/src/mili_llm_bench/audit_token_budget.py`
  + `mili-llm-bench audit-token-budget` subcommand renders every
  row of `sft/train.jsonl` through
  `tokenizer.apply_chat_template(messages, tools=tools,
  tokenize=True)` — the same call SFTTrainer makes — and emits a
  pass/fail report under `data/posttraining/sft/
  preflight-5-token-budget.md`. Result on the rev-13 corpus
  (`google/functiongemma-270m-it`, 82 rows): **max = 3341 tokens**,
  p95 = 3337, p50 = 3263, min = 3234, 0 / 82 over budget. The cost
  driver is the ~18-tool inventory (~2700 tokens/row); messages
  contribute a few hundred more. **Verdict: PASS at gate = 4096
  (deliberate bump from Google's recipe pin of 512)**. The bump is
  recorded here per `sft-preflight-gpu.md` §5's instruction (the
  trained checkpoint's context window must be traceable to a
  decision in this tracker). VRAM cost on H100 is the headroom-side
  of "small" — the linear bump from 512 → 4096 is 8× the per-row
  token count, and the recipe's batch_size=4 means peak activation
  memory stays well under H100's 80GB. The alternative —
  per-scenario tool pruning — narrows the training distribution vs
  inference, so we pin the inventory-wide bump for v1 and revisit
  in v2 only if VRAM forces it.

  **(4) Preflight #5's runtime label was wrong.** The `sft-preflight-gpu.md`
  §5 entry was queued as "pending GPU node + sft/train.jsonl"; in
  practice the audit is tokenizer-only (login-node safe — needs only
  the HF tokenizer cache populated by preflight #1) and gates the
  data, not training. Marking off-GPU-runnable in §5.

  **Path forward.** Preflights #3 (SFTTrainer + tools dump),
  #4 (`assistant_only_loss=True` mask check), and #6 (GGUF
  chat-template baking — gated on a trained checkpoint) remain
  GPU-bound. Stage 8 (pre-experiment gate: stock 0.5–1B + GBNF
  against `eval/heldout.jsonl`) is also runnable and should land
  before `trainer.train()` so we know SFT room exists. 221 / 221
  tests + 1 skip pass on the full `mili-llm-bench` suite (+1 from
  rev 13's 220, the postcondition pin).

- **2026-05-24 (rev 13)** — **Stage 6 cleared.** `mili-llm-bench
  assemble` ran against
  `data/posttraining/runs/stage5-fullsweep-anthropic-20260524-223426/rollouts.jsonl`
  with the three preamble decisions pinned in the new session:
  `heldout-policy=per-intent`, `query-policy=accept`,
  `compound-ratio-min=0.20` (enforced in both train and heldout),
  `seed=42`. Output:
  `data/posttraining/sft/{sft,eval,pref,dataset_card.md}` — 82 train
  + 8 val + 81 heldout + 0 DPO pairs.

  **Per-intent ≥40 floor rebooted to ≥10.** First-turn discovery:
  the rev-4 ≥40 floor was set on a wrong premise. The rev-12
  changelog claimed "13/14 intents will satisfy the ≥40 floor,
  query at 8 will not" — but `retention_by_intent.count` is the
  *retained scenario count*, not the dedup-collapsed row count, and
  the actual synth corpus (`data/posttraining/scenarios/synth.jsonl`,
  175 scenarios) emits ONE canonical instruction per scenario. K=3
  rollouts at T=0.7 collapsed to one trajectory per scenario
  (rev-12 finding), so after `(normalized_instruction, fixture,
  tool_calls_flat)` dedup the retained corpus is 171 unique
  trajectories distributed 4–20 across 14 intents — no intent can
  clear ≥40 without re-running synth with a paraphrase multiplier.
  Re-pinned to ≥10 as the realistic v1 floor; under-floor intents
  flagged as v1 holes in `dataset_card.md` with explicit `TODO(v2)`
  to re-run synth with `paraphrase_count > 1`. The ≥40 number is
  carried in the gates table as a v2 target.

  **13 of 14 intents are under the ≥10 train floor.** Only
  `material` (train=11) clears. This is the floor measured on
  `sft/train.jsonl` (the trainer's consumed corpus), not on the
  pre-split retained pool. Pre-split, the picture is friendlier
  (material 20, set-state 18, show-primal 18, colormap 18,
  compound-* ≈ 13–14, select 16) but the per-intent held-out split
  consumes ~50 % of the corpus, so train-side counts halve.
  Under-floor cells are documented in `dataset_card.md` for the SFT
  pipeline to decide whether to oversample or accept.

  **Held-out split deterministic.** For each intent, the smaller of
  `(d3samp6, cylinder)` is held out in full; ties broken
  alphabetically by fixture name (`cylinder` < `d3samp6`). Every
  intent has a `cylinder` heldout cell in this corpus because the
  rev-12 retention is balanced or `cylinder`-light per (intent,
  fixture). Train cells are uniformly `d3samp6`. The
  whole-fixture alternative (a third fixture bound through Stage 3
  + Stage 5 against `shell_mat2` or `bar5`) is logged as
  `TODO(v2)` — pursue once the v1 baseline is measured.

  **Compound ratio in both splits.** train = 22.0 %, heldout = 24.7 %
  — both above the ≥20 % gate (Stage 2 / Stage 3 carrying constraint
  into Stage 6). Heldout carries one row per compound family + one
  spare; train carries 6 of each compound family. The compound-ratio
  gate is enforced by the assembler (`compound_ratio_min=0.20`), so
  any future rerun that violates fails loud.

  **Contamination clean.** Scenario IDs in `eval/heldout.jsonl` ∩
  `sft/train.jsonl` ∪ `sft/val.jsonl` = ∅. `(intent_id, fixture)`
  cells in heldout ∩ train+val = ∅. Both pinned as runtime
  assertions in the assembler + as test pins in
  `tests/test_assemble.py::TestEndToEnd::test_contamination_clean`.

  **DPO data is empty (expected).** `pref/{train,val}.jsonl` land
  empty: K=3@T=0.7 produced 0 / 175 scenarios with a mixed-tier
  rollout set (every scenario's 3 rollouts share the same
  pass/fail outcome — rev-12 finding). v2 path: rerun a subset at
  T ≥ 1.0 specifically to harvest `(chosen, rejected)` pairs.

  **Tools-array drift-proofing.** Lifted the W1 → FG/OpenAI
  conversion out of `providers/llamacpp.py::_convert_to_openai_tool`
  into a new shared module `mili_llm_bench.tool_format`
  (`w1_to_openai_tool` + `w1_tools_to_openai`). Both
  `LlamaCppProvider._convert_to_openai_tool` (inference) and
  `assemble.project_sft_record` (train) call the shared helper.
  `output_schema` is intentionally dropped — FG's training format
  has no slot for it; the dispatcher enforces output shape server-
  side. New pin in `tests/test_assemble.py::TestToolFormatHelper::
  test_llamacpp_provider_uses_shared_helper` asserts the two paths
  remain equal byte-for-byte. 19 new pins in `tests/test_assemble.py`
  cover the dedup key, the heldout partition, the compound-ratio
  gate, the contamination check, and an end-to-end smoke against
  the actual rev-12 rollouts file (skipped if the artifact is
  absent). 220 / 220 tests pass (+19 from rev 12's 201).

  **Anthropic → FG message conversion is a no-op at Stage 6.** The
  Stage 5 driver writes each rollout's `messages` array in the
  FG/OpenAI canonical shape (developer / user /
  assistant.tool_calls / tool.content) — Anthropic's `tool_use` /
  `tool_result` shape never lands on disk. The §Stage 5 spec's
  "byte-for-byte FG-prompt round-trip" pin lives on the Stage-5
  side (`tests/test_providers_anthropic.py::TestRoundTrip`). Stage 6
  re-emits the messages verbatim minus the driver's synthetic
  `stop:...` markers (see `assemble._strip_driver_stop_markers`).

  **Path forward.** Preflight #3 (data-loader sanity), #5
  (train-1-step-generate smoke), #6 (empty-class regression) are
  now unblocked — they consume `sft/train.jsonl` which exists at
  `data/posttraining/sft/sft/train.jsonl`. Stage 7 (eval harness
  on `eval/heldout.jsonl`) and Stage 8 (pre-experiment gate) are
  the remaining data-pipeline stages before `trainer.train()`. The
  4 query `wrong_final_state` misses, the v2 paraphrase
  multiplier, the K=1 retune, and the third-fixture binding are
  all carried under `TODO(v2)` in `dataset_card.md`.

- **2026-05-24 (rev 12)** — Stage 5 full sweep cleared. K=3 teacher
  rollouts (Claude Sonnet 4.5, `--temperature 0.7`,
  `system_prompt_sha256 = 9f36d0deb5e98a89`,
  `tools_sha256 = 27ffbd0e…`) over all 175 scenarios of
  `data/posttraining/scenarios/synth.jsonl` produced **513 / 525 L3
  (97.71 %)** with **171 / 175 scenarios retained (97.71 %,
  retain="passing")**. Cost: **$1.88** (vs $200 gate — 106×
  headroom). Wall-clock 1613 s (≈ 27 min). Artifacts:
  `data/posttraining/runs/stage5-fullsweep-anthropic-20260524-223426/`.

  **Retention by intent (Stage 6's per-intent floor input).** 13 / 14
  intents at 100 % retention (clrsel 6/6, colormap 18/18,
  compound-material-then-show 13/13, compound-select-then-show 14/14,
  compound-state-then-show 14/14, load 6/6, material 20/20, select
  16/16, set-state 18/18, show-derived 8/8, show-primal 18/18, step
  8/8, view-reset 4/4). `query` at **8 / 12 (66.7 %)** — same 4
  `wrong_final_state` misses v6/v7 stage-6.5 flagged, parked under
  catalog `query.todo_v2` per rev 7 and explicitly out of scope for
  v1 (model retries `query(...)` without `states=[1]`,
  postcondition exact-matches). Every other intent satisfies the
  Stage 6 ≥ 40-rows-per-intent floor when the assembler is run with
  the typical paraphrase multiplier — the four `query` misses leave
  that intent at 8 retained rollouts, which is below the ≥ 40 floor.
  Stage 6 either oversamples `query` deliberately, drops the intent
  to `v2_backlog`, or accepts the smaller-than-floor cell with a
  documented `dataset_card.md` note.

  **Pilot-era diversity hypothesis falsified.** Rev 11 conjectured
  that K=3 collapse on the easy first-50 subset (clrsel / load /
  select / set-state / step) was an artifact of those single-answer
  intents, and that compound / query / material / show-* would
  exhibit real K-pass diversity. Post-hoc inspection of the full
  sweep (group by `scenario_id`, count distinct
  JSON-serialized `tool_calls_flat`) **falsifies that hypothesis
  unambiguously: 0 / 175 scenarios produced any diverse trajectory
  under K=3 at temperature=0.7**. Mean distinct rollouts per
  scenario = 1.00 across every intent including the compound and
  query families. Even the 4 `query` failures fail identically
  three times — K=3 did not recover any failing scenario. **K=3 at
  temperature=0.7 against Claude Sonnet 4.5 on this corpus is
  wasted spend.** The cost is small enough ($1.88 vs $200) that
  the lesson is "v2 should be K=1 or temperature ≥ 1.0", not "rerun
  v1". For Stage 6, dedup on `(intent_id, fixture, tool_calls_flat)`
  collapses the 525 rollouts to 175 unique trajectories (one per
  scenario) by construction; the 3× K redundancy contributes zero
  net training signal beyond what K=1 would have produced. **v2
  knob:** drop K to 1 for the next sweep against a frontier teacher;
  reserve K > 1 for genuinely stochastic teachers (local 7B with
  temperature 1.0+ and top_p sampling).

  **Stage 6 unblocked.** The corpus is graded, retained rollouts
  are flagged in-place, cost telemetry is pinned. Next stage:
  `mili-llm-bench` assembler reads `rollouts.jsonl`, filters
  `retained == True`, dedups on the §1 key, splits by
  `(intent_id, fixture)` cell, writes `sft/{train,val}.jsonl` +
  `pref/{train,val}.jsonl` + `eval/heldout.jsonl` +
  `dataset_card.md`. Per the working brief, Stage 6 is **out of
  scope for this session** — separate work, gated on this row's
  landing (which just happened).

- **2026-05-24 (rev 11)** — Stage 5 pilot cleared. K=3 teacher
  rollouts (Claude Sonnet 4.5, `--temperature 0.7`) over the first
  50 scenarios of `data/posttraining/scenarios/synth.jsonl` produced
  **150 / 150 L3 (100 %)** with **50 / 50 scenarios retained**
  (≥ 1 passing rollout per scenario; `retain="passing"`). Cost:
  **$0.43** (vs $50 gate — 116× headroom). Wall-clock 378 s. Per-K
  seed plumbing forwards `config.seed + k_idx` to the provider
  (tested via `_SeedRecordingProvider` in `tests/test_driver_stage5.py`);
  the Anthropic API does not honor seed, so per-pass diversity is
  driven by `temperature=0.7`. Artifacts:
  `data/posttraining/runs/stage5-pilot-anthropic-20260524-222350/`.

  **Surface delivered (option (a) — extends `mili-llm-bench run`).**
  Five new flags on the `run` subcommand:
  `--limit N` (cap scenarios), `--k K` (rollouts per scenario,
  default 1), `--retain {passing,all}` (Stage 6 SFT-filter key,
  default `all`), `--temperature` (default 0.0). New driver helpers:
  `estimate_cost_usd` against pinned Sonnet 4.5 pricing
  (input $3 / output $15 / cache_read $0.30 / cache_creation
  $3.75 per Mtok); `RETAIN_MODES`; per-category usage aggregation
  through `ProviderOutput.usage → TurnResult.usage →
  ScenarioRunResult.usage_sum → summary.usage_totals`. K=1
  preserves the bench-as-eval record shape byte-for-byte (no
  `k_idx`/`retained`/`usage` keys land on those rollouts —
  pinned by `test_run_eval_k_eq_1_preserves_pre_rev11_record_shape`).
  Summary adds `scenarios_total / scenarios_retained /
  retention_rate / retention_by_intent / usage_totals /
  cost_estimate_usd` under K > 1. Five new pins land in
  `tests/test_driver_stage5.py`; 201 / 201 tests pass.

  **Two caveats worth surfacing.**
  - **Intent coverage.** The first-50-row pilot subset only
    exercises 5 of the 14 intents in `synth.jsonl` (clrsel 2,
    load 6, select 16, set-state 18, step 8 = 50). The other 9
    intents (colormap, compound-{material,select,state}-then-show,
    material, query, show-derived, show-primal, view-reset) sit at
    rows 51..174 and are unmeasured by the pilot. The full sweep
    is what tells us those intents are healthy under Claude; the
    pilot only tells us the K-pass plumbing is honest and the
    budget gate is non-issue.
  - **Per-K trajectory collapse on easy intents.** Despite
    `temperature=0.7`, all K=3 rollouts produced byte-identical
    `tool_calls_flat` for every one of the 50 pilot scenarios.
    Confirmed by post-hoc inspection (group-by `(scenario_id)`,
    count distinct JSON-serialized `tool_calls_flat`): 50 / 50
    scenarios have K=3 collapse, 0 / 50 have any diversity. Cause
    is the intent mix, not the plumbing — `load("d3samp6")`,
    `set_state(state=N)`, `select(...)`, `step(direction=...)` and
    `clrsel(...)` are single-answer tool calls; once Claude picks
    the canonical call, `temperature=0.7` doesn't change the result
    because there is no alternative correct call. The harder intents
    (compound, query, material, show-*) are expected to exhibit
    real K-pass diversity in the full sweep. For Stage 6 dedup
    (`(intent_id, fixture, tool_calls_flat)` as the dedup key) this
    means K=3 on easy intents yields ~50 unique trajectories from
    150 rollouts — a 3× redundancy that costs $0 extra (we already
    paid) but produces no extra training signal. Worth re-tuning
    the K policy in Stage 6 (or v2): K=1 on easy intents,
    K=3 only on the hard tail. For v1, the redundancy is harmless;
    the dedup pass collapses it.

  **Verdict: full sweep AUTHORIZED.** Both gates cleared. Launch
  the 175-scenario sweep at the same config (K=3, temperature=0.7,
  retain=passing, step_cap=8, per_turn_timeout_s=120,
  max_new_tokens=256). Projected cost ≲ $5 (linear extrapolation
  from the pilot is $1.50; the harder-intent token mix biases it
  upward, but still ≪ the $200 full-sweep gate).

- **2026-05-24 (rev 10)** — Rev-9 parser gap resolved via **option
  (b)**: client-side `content → tool_calls` fallback added to
  `LlamaCppProvider.generate` in
  `python/mili-llm-bench/src/mili_llm_bench/providers/llamacpp.py`.
  Gate: on the first request, GET `/props` once and cache
  `chat_template_caps.supports_tool_calls`; the fallback activates
  iff (caps != True) AND (`tool_calls` is empty) AND (content
  contains `<start_function_call>`). Defensive disjunction covers
  the case where `/props` lies on a future build. Regexes mirror
  vLLM's `FunctionGemmaToolParser`: envelope
  `<start_function_call>\s*call:(\w+)\s*\{(.*?)\}\s*<end_function_call>`,
  string args `(\w+):<escape>(.*?)<escape>`, bare scalars
  `(\w+):([^,}]+)` with `true`/`false`/int/float coercion. Prompt
  path untouched — still `/v1/chat/completions` + `--jinja`; the
  rev-8 bespoke renderer stays deleted. Five new pins in
  `tests/test_providers_llamacpp.py::TestFallbackParser`
  (`test_fallback_parses_single_call`, `_parses_multiple_calls`,
  `_preserves_oai_tool_calls`, `_disabled_when_supports_tool_calls_true`,
  `_handles_escape_and_bare_args`); 20 / 20 provider tests pass.

  **v7 re-baseline (canonical bootstrap eval, step_cap=8,
  temperature=0.0, system_prompt_sha256 `9f36d0deb5e98a89`,
  tools_sha256 `27ffbd0e…`, on `matrix37` H100 with
  `llama-server -m functiongemma-270m-it-bf16.gguf --jinja`):
  **13 / 50 L3 (26.0 %)**.** By tier: L0=20, L1=0, L2=17, L3=13.
  By failure mode: 13 parse_error, 6 wrong_final_state, 5
  unknown_tool, 4 wrong_materials, 4 wrong_result, 3 wrong_selection,
  2 schema_mismatch. Wall-clock 23 s. Artifacts:
  `data/posttraining/runs/v7-llamacpp-b-fallback-20260524-215520/`.

  **Failure-cluster shift vs v6.** The rev-9 parser-gap cluster
  (37/50) is fully cleared — every scenario that previously
  short-circuited on `parse_error` now grades the actual rollout.
  The 13 remaining parse_errors are the *same* model-refusal
  cluster v6 already identified ("I cannot assist with…",
  concentrated on load 5/6 and colormap 3/4 plus scattered
  show-primal / show-derived / select / clrsel). Verified by
  inspecting all 13 v7 parse_error rollouts: zero contain an FG
  envelope in content, all match the v6 refusal-text pattern. The
  fallback is doing exactly its job — the residual 13/50 is a
  separate prompt-engineering problem, not a parser problem, and
  is the SFT lift target.

  **Per-intent floor pinned (v7):** step 4/6 (66.7%), view-reset
  2/3 (66.7%), clrsel 2/4 (50.0%), material 2/6 (33.3%), set-state
  2/6 (33.3%), load 1/6 (16.7%), colormap 0/4, compound 0/1
  (unmeasured — sparsity), select 0/4, show-derived 0/4,
  show-primal 0/6. The 0% intents (colormap, select, show-primal,
  show-derived) plus low-rate load/material are the SFT lift
  surface; the per-intent ≥50 % gate in the gates table is what
  v1 SFT needs to clear.

  **Stage 5 is unblocked.** The "inference path emits structured
  tool_calls" precondition is now met for the post-SFT eval cycle,
  whether on this `llama.cpp` build or on a future upstream build
  with a real FG parser (caps gate flips the fallback off
  automatically when supports_tool_calls returns true).

- **2026-05-24 (rev 9)** — v5 re-baseline (the required follow-on to
  rev 8) ran on the new `--jinja` inference path against the
  canonical bootstrap eval (50 scenarios, step_cap=8,
  temperature=0.0, system_prompt_sha256 `9f36d0deb5e98a89`,
  tools_sha256 `27ffbd0e…`). Result: **0 / 50 L3 (0.0 %)**, all 50
  graded `parse_error`. Pinned as **v6 floor**; supersedes the
  stale v5 row. Artifacts:
  `data/posttraining/runs/v6-llamacpp-jinja-rebaseline-20260524-205725/`.
  Wall-clock 12 s — `parse_error` short-circuits the harness's
  retry path so each scenario costs a single turn.

  **Root cause: upstream parser gap in llama.cpp.** `llama-server`
  build `b9307` / `549b9d843` reports
  `chat_template_caps.supports_tool_calls = false` for the
  FunctionGemma BF16 GGUF (via `/props`). The GGUF's baked-in jinja
  template renders the tool inventory correctly into the prompt —
  `/apply-template` confirms `<start_function_declaration>` blocks
  appear in the rendered string — but llama.cpp's chat-handler in
  `common/chat.cpp` does not have a parser for FunctionGemma's
  response format (`<start_function_call>call:NAME{…}
  <end_function_call>`). The server returns the literal markers
  inside `message.content`; the OpenAI-shape `tool_calls` field
  stays empty. Sample (bs-001 / load):
  `'<start_function_call>call:load{root:<escape>d3samp6<escape>}
  <end_function_call>…'`. `LlamaCppProvider.generate` reads
  `tool_calls=[]`, emits an empty `ProviderOutput.tool_calls`, and
  the verifier grades the rollout as `parse_error` — correctly,
  given what was returned.

  **Two failure clusters** across the 50 scenarios:
  - **37 / 50 parser gap.** FG emits valid `<start_function_call>`
    markers and llama.cpp leaves them in `content`. Affects every
    intent the model actually attempts: material 6/6, set-state 6/6,
    step 6/6, view-reset 3/3, show-primal 4/6, show-derived 3/4,
    clrsel 3/4, select 3/4, compound 1/1, colormap 1/4, load 1/6.
  - **13 / 50 model refusal.** Even when FG decides to call a
    function, the bench's canonical developer prompt sometimes
    leaves it chatting instead: `"I cannot assist with…"`, `"My
    current capabilities are limited to…"`. Concentrated on `load`
    (5/6) and `colormap` (3/4) — the intents where the deleted
    bespoke renderer's trigger phrase ("You are a model that can
    do function calling…") had the most lift over the
    bench-pinned system prompt.

  **Stage 5 is blocked.** Maps to the §2 third gate-branch
  (`L3 ≤ 35 % — investigate before SFT`), with a sharper
  diagnosis than §2 anticipated: the bespoke trigger phrase *was*
  slightly load-bearing for FG's emission rate (the 13/50 refusal
  subset), but the dominant blocker is the response-parser side of
  the runtime contract. Path A's claim of "single source of truth
  via the FG jinja baked into the GGUF" holds for the prompt path
  but not the response path on this `llama.cpp` build. No code
  change in this rev; surfacing to the user for the path-forward
  decision (see Status header — options a/b/c/d).

  **Option (a) status (2026-05-25): no upstream fix.** In-tree
  check: `ggml-org/llama.cpp origin/master` is at `549b9d843` (the
  same commit as the existing build under test — zero commits
  ahead). `git grep -iE 'functiongemma|function_gemma|
  start_function_call|end_function_call|start_function_declaration'`
  returns **zero** matches across `common/`, `tools/`, `src/`,
  `include/`. The jinja-mode dispatcher in
  `common/chat.cpp::common_chat_try_specialized_template`
  enumerates specialized handlers for Ministral / GPT-OSS /
  Functionary v3.2 / Kimi K2 / LFM2 / LFM2.5 / GigaChat V3 /
  DeepSeek V3.2 / Gemma4; the closest is Gemma4 (`PEG_GEMMA4`),
  but it keys on the substring `'<|tool_call>call:'` — a different
  marker family from FG's `<start_function_call>call:`. No remote
  branch on `origin/` references FunctionGemma either. **Confirmed
  externally (2026-05-25):** zero PRs, zero issues, zero discussions
  on `ggml-org/llama.cpp` mention FunctionGemma; no public owner /
  RFC / draft. Why the autoparser fails on FG: PR #18675 (merged
  2026-03-06, master `566059a`) replaced the specialized-template
  handlers with a differential PEG autoparser that infers a grammar
  from the template. FG's `<escape>…<escape>` argument wrapping and
  bare-key dict syntax are exactly the "odd constructs" the autoparser
  cannot infer; the same failure mode is documented for LFM2.5 in
  upstream issue #20245 (`tool_mode: NONE` → `supports_tool_calls =
  false` on `/props`). vLLM and Ollama both ship FG tool-call parsers
  natively (vLLM: `--tool-call-parser functiongemma`, Apache-2.0
  reference at `vllm/tool_parsers/functiongemma_tool_parser.py`;
  Ollama: `ollama pull functiongemma`). llama.cpp's own in-tree
  workaround is `tools/agent` (the `llama-agent` binary), which does
  FG-format parsing in-process but bypasses the OpenAI HTTP layer
  entirely. Recommendation: pursue **option (b)** (client-side
  `content → tool_calls` fallback inside `LlamaCppProvider.generate`,
  gated on `caps.supports_tool_calls == false`, with a new pin in
  `test_providers_llamacpp.py`). The vLLM parser file is a drop-in
  regex reference:
  `<start_function_call>call:(\w+)\{(.*?)\}<end_function_call>`
  for the envelope, `(\w+):<escape>(.*?)<escape>` for string args,
  `(\w+):([^,}]+)` for bare bool / numeric args. Not started here
  — explicit decision requested before any code change.

- **2026-05-24 (rev 8)** — Preflight check #2 (train-vs-inference
  chat-template parity) resolved via **Path A**. Login-safe diff of
  HF `apply_chat_template` against the bespoke
  `_build_functiongemma_prompt` returned FAIL on all 3 sample shapes
  (atomic / compound / griz_raw) with six structural divergences;
  the consequential one was that the bespoke renderer **discarded
  the developer message** and substituted a hard-coded one-liner,
  silently nullifying the bench-pinned system prompt
  (`system_prompt_sha256 = 9f36d0deb5e98a89`) on every llamacpp run
  since GEPA promotion. Rewrote
  `python/mili-llm-bench/src/mili_llm_bench/providers/llamacpp.py`:
  `generate()` now POSTs to `/v1/chat/completions` (the server must
  be started with `--jinja`), parses OpenAI-shape `tool_calls`,
  removes the bespoke prompt + parser path entirely (file shrank
  ~460 → 265 lines). Added `TestChatCompletionsPath` (4 tests) in
  `tests/test_providers_llamacpp.py` pinning the URL, OpenAI tool
  conversion, tool-call normalization, and the no-bespoke-renderer
  guard — 15 / 15 pass. The deletion makes Path B (custom HF jinja)
  unrecoverable without a re-add discussion. **v5 floor re-baseline
  is required** — see `sft-preflight-gpu.md` §2 "Required follow-on";
  GPU-blocked because llama-server is not on the matrix login
  `$PATH`. Until that lands, the 40 % v5 floor in the baselines
  table is stale (measured against the wrong system prompt). The
  ~98 % matched-tools ceiling from rev 7 is unaffected (Anthropic
  path, not llamacpp).

- **2026-05-24 (rev 7)** — Stage 6.5 cleared. Claude Sonnet 4.5
  smoke test on `synth.jsonl` (175 scenarios, post-promotion
  `tools.json`, system_prompt_sha256 `9f36d0deb5e98a89`, step_cap=8,
  temperature=0.0) measured **97.71 % L3 (171 / 175)** — above the
  ≥85 % gate, no intent at 0 %. Per-intent: 13 / 14 intents at
  100 %; `query` at 8 / 12 (66.7 %, all 4 misses `wrong_final_state`,
  same scenario IDs in both v6 and v7 — model retries without
  `states=[1]`, postcondition exact-matches). Artifacts:
  `data/posttraining/runs/v7-stage65-anthropic-smoke-20260524-193008/`.
  Risk #3 (re-measure Claude ceiling on promoted tools) resolved —
  the matched-tools ceiling for the SFT corpus is ~98 %, not the 92 %
  pre-promotion number on the 50-row bootstrap eval.

  **First-pass blocker found and fixed.** v6 run
  (`v6-stage65-anthropic-smoke-20260524-191418`) hit 88.6 % overall
  but failed the per-intent gate at **`select` = 0 / 16**: every
  atomic `select` postcondition carried a literal `<param:class>`
  dict key because the synth slot resolver
  (`python/mili-llm-bench/src/mili_llm_bench/synth/slots.py`) walked
  dict *values* but not dict *keys*. Catalog templates encode the
  selection bucket as `{"selection": {"<param:class>": "<param:range>"}}`,
  so the value got resolved (`"1-10"`) while the key stayed literal,
  and the verifier compared `{"<param:class>": "1-10"}` against the
  live `{"brick": "1-10"}` — never matched. Fix factored the token
  resolver out and applied it to keys in both `substitute()` and
  `resolve_expect.walk()`. Two new pins in
  `python/mili-llm-bench/tests/test_synth_round_trip.py`:
  `test_substitute_resolves_dict_keys` (unit) and
  `test_no_unsubstituted_param_tokens_anywhere` (broader invariant —
  catches any future template token leaking into synth.jsonl, not
  just `<param:>` in keys). 11 / 11 synth tests pass.
  `data/posttraining/scenarios/synth.jsonl` regenerated
  deterministically at `seed=42`: still 175 scenarios, 41 compound
  (23.43 %), zero `<param:…>` tokens. `compound-select-then-show`
  passed 14 / 14 in *both* v6 and v7 because the compound
  postcondition checks the final `show`, not selection, so the bug
  was invisible in the compound rows — exactly the failure mode
  Stage 6.5 is designed to catch.

  `query` weakness parked, **not** patched in this rev. Top-3 of 4
  failures (synth-00124, 00127, 00130, 00133) all have the model
  emit `query(...)` once with `states="null"` (string), then retry
  without `states`; postcondition expects `states=[1]`. Three
  possible fixes (verifier leniency on default state, instruction
  pinning `state 1`, tool-schema default) — none decided. Tracked
  in catalog `query.todo_v2` for the next pass (do not gate Stage 5
  on this; 66.7 % is below 0 % only by definition, and the corpus
  clears 85 % overall + has no zero-rate intents).

- **2026-05-24 (rev 6)** — Stage 3 query read-path wired end-to-end.
  `mili-viz-server`'s `Query` RPC, frozen as an M1 shape-only stub
  (`crates/mili-viz-server/src/lib.rs` `async fn query`), now calls
  `mili_rs::Database::query_with_labels` against the loaded run and
  projects `StateValues` → `InlineTable.values` (f64). `pygriz` adds
  `Session.query(**kwargs) -> dict` so the Stage 3 live oracle's
  `s.query(...)` resolves through a typed `QueryRequest` instead of
  raising `AttributeError`. Re-run of
  `mili-llm-bench synth`: 175 scenarios (up from 163), 41 compound
  (23.43% — still ≥20% gate), 0 skipped rows; `query` cell counts
  6/d3samp6 + 6/cylinder. Catalog `query.todo_v2` line about the
  pygriz gap removed. New gates:
  `crates/mili-viz-server/tests/query_rpc.rs` round-trips the RPC
  against a direct `mili-rs` call on d3samp6, plus a no-DB-loaded
  error-path test; `tests/test_synth_round_trip.py` adds
  `test_every_catalog_intent_has_at_least_one_row` so a whole intent
  silently dropping fails the round-trip immediately rather than
  hiding inside the report-only `skipped` list. The other Stage 3
  dispatcher gap (`selection.clear_all()` for empty `clrsel`) is
  unchanged — separate parked fix.
- **2026-05-24 (rev 5)** — Stage 3 landed.
  `data/posttraining/scenarios/synth.jsonl` (163 scenarios, 41 compound,
  25.15% ratio, deterministic at `seed=42`) plus its
  `synth.report.md` audit. Implementation in
  `python/mili-llm-bench/src/mili_llm_bench/synth/` — `catalog.py` parses
  + validates `data/posttraining/intents/catalog.yaml`, `slots.py`
  resolves `<param:>` / `<derived:>` tokens, `sample.py` holds the
  per-intent tuple generators + per-cell quotas, `run.py` orchestrates
  the pass and writes the report; surfaced via `mili-llm-bench synth`
  (login-node safe, no GPU). `Scenario` extended with optional
  `instruction_source` field (template / manual-paraphrase) so the W4b
  rollout writer stamps the tag through verbatim. Round-trip and
  compound-ratio pins live in
  `python/mili-llm-bench/tests/test_synth_round_trip.py` (8 tests,
  passes against the full always-on suite). Two pre-existing
  dispatcher gaps surfaced during the live oracle pass and are parked
  in catalog `todo_v2`: `selection.clear_all()` (the same gap as
  bootstrap's 2× clrsel `dispatch_error`) and `Session.query` —
  Stage 3 emits class-only `clrsel` and drops every `query` row.
  Risk #1 (held-out fixture) still pending; Stage 3 binds only against
  `d3samp6` + `cylinder`, which is the cell-pair Stage 6 will split.
- **2026-05-24 (rev 4)** — Stage 2 landed.
  `data/posttraining/intents/catalog.yaml` written with 11 atomic intents
  (`load, set-state, step, select, clrsel, show-primal, show-derived,
  material, view-reset, colormap, query` — 10 mirror `bootstrap.jsonl`
  plus `query` for read-path coverage) and 3 compound families
  (`compound-material-then-show`, `compound-select-then-show`,
  `compound-state-then-show`). **Risk #2 resolved:** verifier
  postcondition kinds stay closed at 7; compounds grade the final state
  only via the existing `active_result` kind. `state_sequence` and
  `composite` are parked under `todo_v2.verifier_kinds`. Fixture facts
  for `d3samp6` and `cylinder` filled from `bootstrap.jsonl` +
  `interpret.c` as placeholders; Stage 3 confirms them via real
  load+snapshot before grounding params. Risk #1 (held-out fixture
  choice) still pending; the catalog only registers the two fixtures in
  `_FIXTURE_PATHS`, no held-out binding yet. Punted intents
  (`snapshot/legend/iso/contour/cutplane/named_view/close`) and the 7
  unmapped fixtures live under `todo_v2:` so the v2 backlog is a diff,
  not a re-derivation. Catalog passes 5 sanity checks against
  `scenarios.VALID_POSTCONDITION_KINDS`, `tools.json`, and the
  shape↔steps invariants.
- **2026-05-24 (rev 3)** — Cluster bring-up on the H100 login node
  (`matrix2`). Workspace `train` extra added via `uv add` (transformers,
  torch+cu130, accelerate, trl 0.12.1, datasets, sentencepiece); the
  workspace now resolves through the LLNL Nexus PyPI mirror with
  `native-tls = true`. Two new scripts: `scripts/setup-gpu-env.sh`
  (sourceable session env matching `cadsat/build.sh`'s toolchain)
  and `scripts/gpu-sanity.sh` (srun-able smoke check). `torch+cu130`
  wheel verified end-to-end on H100 via `pdebug` allocation: sm_90,
  BF16 supported, real `bf16` matmul through PyTorch's bundled cu130
  runtime — confirms PyTorch's bundled CUDA coexists with llama.cpp
  built against `cuda/12.9.1`. Preflight check #1 PASS (Gemma license
  granted after Google's manual review; config + tokenizer cached).
  Checks #2–#6 remain deferred — they require either a GPU compute
  node + `llama-server` running (#2, #4) or the assembled
  `sft/train.jsonl` (#3, #5) which Stage 6 produces. Work landed on
  branch `m5-sft-cluster-bringup` (unpushed pending git auth).
- **2026-05-24 (rev 2)** — Critique pass against Google's
  FunctionGemma fine-tuning guide. Resolved off-GPU:
  hyperparameters re-pinned to Google's reference recipe
  (LR 5e-5 / 8 epochs / bs=4 / constant LR / `max_length=512` /
  `packing=False`); TRL API drift fixed (`processing_class=`,
  `assistant_only_loss=True`); HF model id confirmed
  (`google/functiongemma-270m-it`, gated); tools-array
  format-conversion step pinned in Stage 6; Claude→FG record
  conversion specced in Stage 5; K=3 pinned with $50/$200 budget
  caps; ≥40-row/intent floor added to Stage 6 gates; Stage 6.5 gate
  reworded (dropped infeasible GBNF qualifier for Claude). GPU-blocked
  items split into a new pre-flight doc
  ([`sft-preflight-gpu.md`](sft-preflight-gpu.md)) and `cluster-setup.md` §0.
- **2026-05-24 (rev 1)** — Doc created. v5 floor (40 % L3) reproduced
  and pinned. Stage 2 marked active. Cluster bring-up doc
  ([`cluster-setup.md`](cluster-setup.md)) added for the H100
  training environment.

---

## Pointers

- Build plan: [`posttraining-dataset.md`](posttraining-dataset.md)
- Cluster bring-up (H100 + llama.cpp + training stack):
  [`cluster-setup.md`](cluster-setup.md)
- Training entry point (script + slurm launcher; instructions in
  `cluster-setup.md` §6 "Launching the run"):
  `python/scripts/sft_train.py`, `scripts/sft_train.sbatch`
- GPU-blocked pre-flight checklist (must clear before `trainer.train()`):
  [`sft-preflight-gpu.md`](sft-preflight-gpu.md)
- Why SFT vs. GEPA: [`GEPA-vs-POSTTRAINING.md`](GEPA-vs-POSTTRAINING.md)
- Original strategy (superseded as a tracker, kept as design rationale):
  [`m3-posttraining-strategy.md`](m3-posttraining-strategy.md)
- Verifier (reuse, do not rebuild):
  `python/mili-llm-bench/src/mili_llm_bench/verifier.py`
- Scenario / postcondition shape:
  `python/mili-llm-bench/src/mili_llm_bench/scenarios.py`
- Executable tool surface:
  `python/mili-llm-bench/src/mili_llm_bench/schemas.py:TOOL_DESCRIPTIONS`
- Fixture resolver:
  `python/mili-llm-bench/src/mili_llm_bench/dispatchers/pygriz.py` —
  `_FIXTURE_PATHS`, `_resolve_fixture`
- Bootstrap eval (50 scenarios, do not edit without re-pinning baselines):
  `data/posttraining/eval/bootstrap.jsonl`
- Current run artifacts (do not delete):
  `data/posttraining/runs/v5-llamacpp-promoted-tools/`,
  `data/posttraining/runs/v4-anthropic-realfixtures/`,
  `data/posttraining/gepa-runs/gepa-run-20260524-135543/`
