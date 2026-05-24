# `mili-viz` — local LLM post-training, deep dive (exploratory)

**Status: exploration / not a priority / revisit before building.**
Companion to `agent-local-llm.md` (read that first for scope: this is
the *command-writer* tier, not the autonomous agent). This doc goes
deeper on the one question that decides feasibility: **where does the
training signal come from, and how do we close the loop without human
labeling.**

This deep dive is written against the now-checked-out
`reference/griz` submodule (`git submodule update --init
reference/griz`). It supersedes one claim in `agent-local-llm.md`:
that there is "no natural corpus." There is — see §1.

## 1. What the griz submodule actually gives us

The earlier doc was written before `reference/griz` was populated and
assumed only binary fixtures. The real source changes the data story:

- **`Src/interpret.c` (~11k lines) — `parse_command()`.** The command
  vocabulary is a literal `strcmp(tokens[0], "...")` dispatch chain.
  This is *mechanically extractable*: the set of first-token keywords,
  their aliases (explicit in source — `quit|done|exit|end`,
  `select|unselect|deselect`, `clrsel|poof`, `cap|clrallpicks`,
  `help|man|grizman|?`), and per-command argument arity. It is both
  the **grammar** (constrained-decoding front end) and a **free
  validity oracle** (the function sets `valid_command` and emits
  `[grizinit - Line:N]`-style diagnostics).
- **`Src/viewer.c` `usage_text[]` / `usage_text_batch[]`.** Built-in
  usage strings — terse NL ↔ command-form pairs straight from source.
- **`Src/Doc/griz_manual.pdf` (+ `.docx`).** A real natural-language
  command manual: prose descriptions of what each command does. This
  is the seed for *paraphrase grounding* — the thing a translation
  model needs and which we previously thought we lacked.
- **`grizinit` mechanism (`Src/init_io.c`).** griz auto-runs a
  `grizinit` / `grizinit.<plotfile>` file of commands at startup.
  Any real `grizinit` files (sites have them) are gold: authentic
  command *sequences* in deployment order. We don't ship a large
  collection, but the format and the manual's worked examples seed it.

Net: we have a grammar oracle, a validity oracle, an NL description
corpus, and executable fixtures. That is the full closed loop without
a single hand-labeled example.

## 2. The verifier is the whole game

Everything downstream (SFT data filter, RL reward, eval metric) is one
graded check. Build it once:

| Tier | Check | Cost | Source |
|------|-------|------|--------|
| L0 lexical | output is in-grammar | ~free | grammar from `interpret.c` |
| L1 parse | `parse_command` accepts it (`valid_command`) | cheap | `interpret.c` / the mili-viz dispatcher |
| L2 execute | runs against a fixture session, no error | medium | `reference/mili/test/xmilics/*` via server dispatch |
| L3 post-condition | reaches the intended state / `Query` value | medium | mili-rs test suite's known-good values |

L0/L1 come from a single grammar artifact generated from
`interpret.c` (and kept honest by a test that re-derives it and
diffs — same discipline as `scripting.md`'s "Layer 0 ≡ raw stream"
assertion). L2/L3 reuse the server command dispatch and the existing
fixture/parity infrastructure (`scripts/setup-parity.sh`,
`crates/mili-rs/tests/*_fixtures.rs`). No new oracle is invented.

**Reward shaping** is then just the max tier passed: in-grammar (0.1)
→ parses (0.3) → executes (0.7) → post-condition met (1.0). Dense
enough for RL, strict enough for SFT rejection sampling.

> **Build plan:** the concrete, ordered dataset-construction plan
> (interface seam, record schema, stage-by-stage build order, what is
> buildable now vs. gated on the Griz-python interface) lives in
> `posttraining-dataset.md`. §3 below is the sketch it operationalizes.

## 3. Minimal post-training pipeline (sketch — revisit)

```
griz_manual.pdf + usage_text[] ──► intent templates ──┐
interpret.c ──► grammar + arg arity ──► constrained decoding
                                                       │
xmilics fixtures ──────────────────► executable scenarios
                                                       ▼
              teacher model (Claude API / 7–14B local)
                  proposes command sequences per (intent, fixture)
                                                       ▼
              VERIFIER (§2, tiers L0..L3) ── filters / scores
                          │                         │
                    verified pairs              graded prefs
                          ▼                         ▼
                    SFT (QLoRA, 0.5–1.5B)   ── DPO / GRPO (only if SFT plateaus)
                          ▼
              eval = L3 success-rate on held-out (intent, fixture, post-cond)
```

Step detail:

1. **Grammar + intent extraction.** Parse `interpret.c` dispatch into
   a keyword/alias/arity table → GBNF. Pull command descriptions from
   `griz_manual` + `usage_text[]` into an (intent prose ↔ canonical
   command) table. This artifact is independently useful (it documents
   the vocabulary the main agent also needs) so it is low-regret even
   if the tiny model is dropped.
2. **Scenario synthesis.** For each fixture (`d3samp6`, `cylinder`,
   `ml40`, `bar1`, …) and each command/intent, generate paraphrased
   user requests (template + light LLM paraphrase). Fixtures supply
   *grounded* targets (real materials, real state counts) so
   post-conditions are checkable, not invented.
3. **Teacher rollouts.** A larger model produces N candidate command
   sequences per (paraphrase, fixture). This is the "use existing test
   data to generate rollouts" step: fixtures define the executable
   world, the teacher proposes solutions, the verifier judges.
4. **Rejection-sampling SFT.** Keep L2+ (ideally L3) rollouts as
   (request → command-sequence) pairs. QLoRA fine-tune the 0.5–1.5B
   base. Single consumer GPU, hours. Likely captures most of the
   value on its own.
5. **RL only on measured need.** The verifier already labels
   pass/fail, so DPO from L3-pass vs. L1/L2-fail pairs is nearly free;
   GRPO/PPO with the §2 graded reward if preference-only underfits the
   compositional tail. Explicitly phase-2-of-phase-2.
6. **Eval harness.** Held-out (intent, fixture, post-condition)
   triples; metric = L3 success under grammar-constrained decoding,
   plus L1 parse-rate as a cheap regression tripwire. Same code as the
   verifier.

## 4. Why this is now low-risk to explore

- **Zero human labeling.** Grammar, validity, execution, and
  post-conditions are all machine oracles from artifacts already in
  the tree.
- **One thing to build.** Teacher filter, RL reward, and eval are the
  same verifier. Most engineering is the grammar extractor + the
  fixture-execution harness — both reusable by the main agent's tests
  regardless of the tiny-model outcome.
- **Graceful failure.** If the small model underperforms, the router
  in `agent-local-llm.md` just routes more to the full agent;
  `LlmProvider`'s no-local-model path already exists.
- **Honest unknown:** the `griz_manual` is descriptive prose, not a
  large set of (NL request → command) pairs. Whether template +
  teacher paraphrase yields enough *intent diversity* — vs. collapsing
  to a narrow synthetic style — is the first thing to measure before
  committing to fine-tuning. A pre-experiment: try stock 0.5–1B +
  grammar-constrained decoding with *no* fine-tune and measure L3 on
  the eval set; if it already clears the bar, post-training is moot
  for v1.

## 5. Open questions (do not resolve until exploration)

- Can `interpret.c`'s dispatch be parsed into a grammar robustly, or
  is it irregular enough (nested `parse_command` recursion, stateful
  modes) to need a hand-written grammar seeded from it?
- Argument-level correctness: L1 parse-valid ≠ semantically sensible
  (`state 999999` parses). How much does L3 need to carry, and is
  fixture coverage wide enough to make L3 meaningful?
- Teacher cost at the rollout volume SFT needs — budget before
  running.
- Does grammar-constrained decoding alone (§4 pre-experiment) make the
  fine-tune unnecessary for v1?
- Sourcing real `grizinit` files (LLNL/site users) for authentic
  command-sequence data — worth asking, out of band.
