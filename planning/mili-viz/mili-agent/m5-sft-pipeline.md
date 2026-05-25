# M5 — SFT pipeline (live tracker)

**Status (2026-05-24):** Stage 2 about to start. Floor 40% L3
(FunctionGemma-270M + GEPA-promoted tools), ceiling 92% L3 (Claude
Sonnet 4.5). SFT must close the 52-point gap.

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
| **v5 floor** (`v5-llamacpp-promoted-tools`)               | llamacpp / FunctionGemma-270M | **40 %** | `27ffbd0e…`  | Current default; reproduces GEPA-promoted tools  |
| v4 floor (`v4-llamacpp-realfixtures-fullresolve`)         | llamacpp / FunctionGemma-270M | 32 %     | `cdda3677…`  | Pre-GEPA-promotion; historical                   |
| **v4 ceiling** (`v4-anthropic-realfixtures`)              | anthropic / claude-sonnet-4-5 | **92 %** | `cdda3677…`  | Pre-promotion tools — re-measure planned         |

Earlier runs (`v0…v3`) ran against the empty M1 stub corpus before the
fixture-resolver landed; their absolute numbers are not comparable —
see [`bench-fixture-stub-fallback-fixed`](../../../../.claude/projects/-Users-rwhit-Workspace-mili-rs/memory/bench-fixture-stub-fallback-fixed.md)
in memory.

**Per-intent floor (FunctionGemma v5, 40 %):** load 83 %, set-state
83 %, colormap 75 %, show-derived 25 %, show-primal/step 17 %,
material / select / clrsel / view-reset / compound **0 %**. The
zero-rate intents are SFT's primary lift target.

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
- [ ] **Stage 3** — Scenario synthesis (~200 records, multi-step
      ratio ≥ 20 %). **Active next.** Blocked on nothing.
- [ ] **Stage 4** — Verifier (already exists at
      `python/mili-llm-bench/src/mili_llm_bench/verifier.py`; L0–L3,
      closed failure-mode taxonomy). Reuse, do not rebuild.
- [ ] **Stage 6.5** — Claude data smoke test. **Runs immediately
      after Stage 3 produces the first batch**, not after Stage 5.
      Catches data bugs before they look like model bugs. Gate: ≥85 %
      L3 under Claude with native tool-use (no GBNF qualifier — Claude
      doesn't support grammar-constrained decoding).
- [ ] **Stage 5** — Teacher rollouts. Burns Anthropic API on every
      scenario; deliberately last among data stages. Pilot the first
      ~50 scenarios before committing to the full sweep.
- [ ] **Stage 6** — Assembly, dedup, splits, `dataset_card.md`.
      Contamination control: split by `(intent_id, fixture)` cell,
      not by row.
- [ ] **Stage 7** — Eval harness (same code as Stage 4, pointed at the
      held-out split).
- [ ] **Stage 8** — Pre-experiment gate. Run a stock 0.5–1B model with
      grammar-constrained decoding and **no fine-tune** against
      Stage-7 eval. If it clears the bar, post-training is moot for v1
      and we stop.

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
| Stage 6 — per-intent SFT rows | ≥40 rows/intent in `sft/train.jsonl` | Oversample the deficient intent before training                |
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
3. **Re-measure Claude ceiling on promoted tools.** Current 92 % was
   on pre-promotion `tools.json`. Re-run is cheap; queue it before
   Stage 5 so the gap measurement is matched-tools.
4. **Paraphrase diversity.** Stage-8 has a diversity check — if v1
   paraphrases collapse stylistically, that invalidates the corpus
   regardless of L3 numbers.
5. **`griz_raw` fallback grammar (Stage 1).** Deferred from v1, but it
   gates whether long-tail griz commands can ever participate. Track
   in v2 backlog.

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
