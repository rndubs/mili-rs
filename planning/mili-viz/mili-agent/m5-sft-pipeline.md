# M5 — SFT pipeline (live tracker)

**Status (2026-05-24):** Stages 2, 3, 6.5 cleared; preflight #2
cleared via Path A (rev 8). v6 re-baseline on the new `--jinja`
path landed at **0 / 50 L3 (rev 9)** — diagnosed as an upstream
parser gap in `llama.cpp` b9307 (`549b9d843`):
`chat_template_caps.supports_tool_calls = false` for the
FunctionGemma BF16 GGUF, so FG's `<start_function_call>…
<end_function_call>` response markers return as literal
`message.content` instead of structured `tool_calls`. **Stage 5 is
blocked** until the inference path emits structured tool_calls.
Matched-tools ceiling 97.71 % L3 (Claude Sonnet 4.5 on synth.jsonl,
rev 7) stands. Next decision: (a) upgrade `llama.cpp` to a build
with a FG response parser, (b) add a content→`tool_calls` fallback
inside `LlamaCppProvider.generate` (re-introduces parser code Path
A deleted), (c) revert Path A and re-apply the developer-message
fix to the bespoke renderer, or (d) switch the inference runtime.

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
| **v6 floor** (`v6-llamacpp-jinja-rebaseline-20260524-205725`) | llamacpp / FunctionGemma-270M | **0 %** | `27ffbd0e…` | Re-baseline on `--jinja` path; 50 / 50 `parse_error`. llama.cpp b9307 (`549b9d843`) has no FG response parser (`chat_template_caps.supports_tool_calls = false`); 37 / 50 emit FG markers in content, 13 / 50 model-refused. Stage 5 blocked — see rev 9. Supersedes the stale v5 floor (40 % L3 under the deleted bespoke renderer). |
| v4 floor (`v4-llamacpp-realfixtures-fullresolve`)         | llamacpp / FunctionGemma-270M | 32 %     | `cdda3677…`  | Pre-GEPA-promotion; historical                   |
| **v4 ceiling** (`v4-anthropic-realfixtures`)              | anthropic / claude-sonnet-4-5 | **92 %** | `cdda3677…`  | Pre-promotion tools, bootstrap eval; re-measured |
| **v7 ceiling** (`v7-stage65-anthropic-smoke-…`)           | anthropic / claude-sonnet-4-5 | **98 %** | (synth)      | Post-promotion tools on `synth.jsonl` (175 rows) |

Earlier runs (`v0…v3`) ran against the empty M1 stub corpus before the
fixture-resolver landed; their absolute numbers are not comparable —
see [`bench-fixture-stub-fallback-fixed`](../../../../.claude/projects/-Users-rwhit-Workspace-mili-rs/memory/bench-fixture-stub-fallback-fixed.md)
in memory.

**Per-intent floor (FunctionGemma v6, 0 %):** every intent at 0 / N
on the new `--jinja` path — see rev 9 for the failure-cluster
breakdown. Not a measurement of FG capability under a working
inference path; reflects only that the response parser doesn't
recognize FG's format. Historical v5 per-intent numbers (load 83 %,
set-state 83 %, colormap 75 %, show-derived 25 %, show-primal /
step 17 %, material / select / clrsel / view-reset / compound 0 %)
are kept here for orientation but were measured against the deleted
bespoke renderer that nullified the bench system prompt.

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
