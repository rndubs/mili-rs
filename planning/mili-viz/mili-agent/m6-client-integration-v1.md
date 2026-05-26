# M6 — wire the v1 SFT model into the griz client

**Status (2026-05-25):** Plan only. M5 rev 22 shipped a winner GGUF
at 95.06 % L3 (`data/posttraining/checkpoints/v1/functiongemma-v1.bf16.gguf`);
this milestone replaces the stock FunctionGemma-270M that M4 wired in
with the v1 SFT model and reconciles the Rust agent's prompt /
parser path with the rev-21 canonical paths the bench measures
against.

This is **not a from-scratch integration** — M4 already built the
plumbing
([`crates/mili-viz-server/src/llamacpp_agent.rs`](../../crates/mili-viz-server/src/llamacpp_agent.rs),
~700 LOC + 23 unit tests). The deltas below are an incremental
upgrade. The big-rock structural items from
[`m4-client-integration-status.md`](m4-client-integration-status.md)
"Next moves" carry forward unchanged; M6 does not depend on any of
them.

---

## What this milestone delivers

The griz client AI panel, when launched with `--agent llamacpp`,
talks to a `llama-server` serving **the v1 SFT BF16 GGUF**, not the
stock FunctionGemma-270M GGUF. Tool calls emitted by the v1 model
reach the dispatcher unchanged; the L3 number that holds against
`eval/heldout.jsonl` (95.06 %) is the same number that holds in the
live client when the user types the same instruction.

Out of scope for M6: dispatch-feedback gap (M4 next-move #1, the
load-bearing M4 limitation), tool-coverage gaps (M4 #6), vision
(M4 #7), streaming (M4 unwired), cancellation (M4 unwired).
M6 is *narrowly* "swap the served model end-to-end without losing
the 95.06 % number on the live path." Everything else stays as it is.

---

## Reading order (one-time orientation)

1. [`m5-sft-pipeline.md`](m5-sft-pipeline.md) rev 22 changelog — the
   trained model and its measured properties.
2. [`m4-client-integration-status.md`](m4-client-integration-status.md)
   — the existing integration; what works, what's broken, what's
   limited. The two "broken / limited" categories *survive* M6 unchanged.
3. [`crates/mili-viz-server/src/llamacpp_agent.rs`](../../../../crates/mili-viz-server/src/llamacpp_agent.rs)
   — the file that mostly changes.
4. [`python/mili-llm-bench/src/mili_llm_bench/providers/llamacpp.py`](../../../../python/mili-llm-bench/src/mili_llm_bench/providers/llamacpp.py)
   — the rev-8 / rev-10 / rev-21 reference; the Rust agent's prompt
   and parser paths should match what this Python file does.
5. [`python/mili-llm-bench/src/mili_llm_bench/providers/_fg_envelope.py`](../../../../python/mili-llm-bench/src/mili_llm_bench/providers/_fg_envelope.py)
   — the rev-21 shared FG envelope parser (handles JSON-literal +
   `<escape>` shapes).

---

## Deltas vs M4 — five items in order of impact

### 1. Model swap (the entire point)

**Today:** M4 docs show `llama-server -hf ggml-org/functiongemma-270m-it-GGUF:BF16 --jinja`
serving the stock pretrained model.

**Change:** point llama-server at the new local GGUF.

```bash
source scripts/setup-gpu-env.sh
llama-server \
  -m /p/vast1/whitmore/cadsat/mili-rs/data/posttraining/checkpoints/v1/functiongemma-v1.bf16.gguf \
  --port 8080 \
  --jinja
```

No Rust change. The agent already talks to `http://localhost:8080`.
The GGUF carries the same chat-template that training rendered
against (m5-sft-pipeline.md rev 22 preflight #6 — byte-identical sha
`db61fb01…`), so `--jinja` produces the same prompt distribution at
serve time as the bench measured at eval time.

**Verification:** `curl localhost:8080/props | jq .model_path` returns
the absolute path to `functiongemma-v1.bf16.gguf`. `chat_template_caps.supports_tool_calls`
remains `false` on b9307; the Rust agent's response-side parser must
still synthesize tool_calls from content (item 3 below).

### 2. Prompt path — `/completion` → `/v1/chat/completions` + `--jinja`

**Today:** `llamacpp_agent.rs:118` calls
`build_functiongemma_prompt(&messages, &tools)` (bespoke Rust
renderer that hand-builds the FG prompt string) and POSTs to
`/completion`.

**Why this is broken in the v1 world:** the Python side discovered
in M5 rev 8 that the bespoke FG renderer (`_build_functiongemma_prompt`
in Python) diverged from HF `apply_chat_template` on at least six
axes — dropped developer message, flattened tool schema, etc.
(See [`_sft-preflight-gpu.md` §2](_sft-preflight-gpu.md) for the
verbatim list.) The training distribution used `apply_chat_template`;
serving with a bespoke renderer means the v1 model sees a different
prompt than it trained on. The 95.06 % number measured on the bench
will *not* survive the serving path unless the Rust agent also goes
through the GGUF's baked-in jinja.

**Change:** delete `build_functiongemma_prompt` from
`llamacpp_agent.rs`; switch the POST target from `/completion` to
`/v1/chat/completions` and send `messages` + `tools` as raw OpenAI
shape. llama-server's `--jinja` flag then applies the GGUF's baked
template — the same FG jinja the training side used.

**Tool-shape conversion.** The Rust agent already keeps a typed
tools registry (`pb::tool` etc.); needs a small helper to project to
the OpenAI tool shape (`{type: "function", function: {name, description,
parameters: <full JSON Schema>}}`). The Python equivalent is
`tool_format.w1_to_openai_tool` — port the relevant subset (10–15
lines of straightforward dict construction).

### 3. Parser — align with `_fg_envelope.py`

**Today:** `llamacpp_agent.rs` has its own JSON + FG-text tool-call
parser. M4 doc says it handles JSON (`{"name":…,"arguments":…}`) and
FG text (`<start_function_call>call:name{...}<end>`) with type
coercion.

**Why this needs review in the v1 world:** the v1 SFT model emits
the **JSON-literal** envelope shape, not the FG-DSL `<escape>` shape
the stock FG-270M emits (m5-sft-pipeline.md rev 21 (4)). The Python
side handled this by extending `parse_fg_envelopes` to JSON-parse
the envelope body first, fall through to the existing
`<escape>` / bare-scalar pass on failure. The Rust parser must do
the same — try `serde_json::from_str` on the envelope body first;
fall through to the FG-DSL path on failure.

**Change:** port the rev-21 parser shape from
`providers/_fg_envelope.py` to Rust. The relevant regex set:

| regex | shape |
| --- | --- |
| `_FG_ENVELOPE_RE` | `<start_function_call>\s*call:(\w+)\s*\{(.*?)\}\s*<end_function_call>` |
| (JSON-literal branch) | `serde_json::from_str::<Map<String,Value>>(envelope_body)` |
| `_FG_STRING_ARG_RE` (fallback) | `(\w+):<escape>(.*?)<escape>` |
| `_FG_BARE_ARG_RE` (fallback) | `(\w+):([^,}]+)` |

The order matters: JSON-literal first, escape-form second, bare-scalar
last. v1 SFT emissions hit the JSON branch; stock FG-270M emissions
(any fallback to the prior model) hit the escape branch.

The existing M4 JSON-object parser (`{"name": …, "arguments": …}`)
is *separate* — that's the OpenAI tool-call shape, which
`/v1/chat/completions` *should* return natively when
`supports_tool_calls=true`. On b9307 with FG GGUFs the cap is
`false`, so the model's output lands in `message.content` as raw
text containing FG envelopes; the client-side fallback (this parser)
does the work. Same caps-gating logic as the Python rev-10 fallback.

### 4. System prompt — reconcile with bench-pinned

**Today:** `llamacpp_agent.rs:25-53` carries a custom system prompt
that talks about tool-response semantics, key tool mappings, JSON
format requirements, and task completion. ~60 lines.

**Why this is a train-vs-serve drift risk:** the bench config carries
`system_prompt_sha256 = 9f36d0deb5e98a89` — pinned in the rev-22
GGUF run. The v1 SFT model was trained against rollouts that were
*generated* under that exact prompt hash (Stage 5 / 6.5 in M5).
A different system prompt at serve time means the model sees a
different prefix than training rendered against. The 95.06 % number
does not survive prompt drift.

**Change:** replace the Rust agent's system prompt with the
bench-pinned one. Source:
`python/mili-llm-bench/src/mili_llm_bench/driver.py` `_DEFAULT_SYSTEM_PROMPT`.
Either:
- (a) `include_str!` it from a shared file under `data/posttraining/grammar/`,
  same way `tools.json` is loaded today (lines 23–24 of `llamacpp_agent.rs`),
- (b) duplicate the literal into the Rust source with a unit test
  asserting the sha256 matches `9f36d0deb5e98a89`.

(a) is the better path — single source of truth, no drift possible.
Adds one file (`data/posttraining/grammar/system_prompt.txt` or
similar) but no compile-time coupling. The Python driver should also
read from this file so the bench and the serving agent literally
share bytes.

### 5. Heuristic guards — relax with the SFT model

**Today:** `MAX_STEPS=4` (line 21) + name-based signature window
(lines 104–109). These exist because the stock FG-270M sprays tool
calls past the point the task is done; without the guards, multi-step
intents oscillate A-B-A-B.

**Why these may now hurt:** on the 81-row heldout the v1 SFT model
averages 1.23 turns to completion (m5-sft-pipeline rev 22 summary —
`mean_turns_to_completion`). The over-call failure mode the guards
were built to suppress has trained away. `MAX_STEPS=4` is still
plenty of headroom for legitimate compound intents (the v1 corpus
caps compound chains at 2–3 steps); the signature window is the
more aggressive guard and likely the one that now blocks legitimate
"disable material 2 and material 3" patterns.

**Change:** start by *measuring* whether the guards fire on real
v1 traffic before relaxing. Cheap diagnostic: log a counter for
"signature window fired" / "MAX_STEPS hit" and watch it on the
manual smoke (validation §B below). If it's near-zero, keep them as
safety nets at no cost. If it's firing on multi-step requests,
loosen to "same `(name, args)` repeated twice" rather than "same
name in last 4."

Not a code change in the first pass; a metric and a decision.

---

## Validation plan

The release gate for M6 is: same L3 number on the live serving path
as on the bench heldout path.

### A — bench parity sanity check (cheap, mechanical)

Re-run the rev-22 llamacpp eval against
`functiongemma-v1.bf16.gguf` *with the new system prompt* (delta #4),
confirm 77 / 81 = 95.06 % holds. If the number moves, the system
prompt unification surfaced a drift the rev-22 run didn't catch.

### B — manual smoke against the live client

Launch the griz client + `mili-viz-server --agent llamacpp` against
the v1 GGUF, type one query per intent family from
`data/posttraining/sft/eval/heldout.jsonl` (sample 1 per intent =
~11 queries), observe whether:
- the tool call dispatches with the same arguments the heldout
  scenario expected,
- the M4 dispatch-feedback gap manifests (it will — independent of
  M6),
- the heuristic guards fire on multi-step compound queries.

### C — regression check on the Rust unit suite

`cargo test -p mili-viz-server` — the 23 LlamaCppAgent unit tests
must still pass. The parser-shape change (delta #3) is the most
likely to introduce a regression; expect to update those tests to
cover the JSON-literal envelope shape.

---

## Gates

- **Gate 1 (parity):** validation §A returns 77 / 81. Different
  number means the system prompt / prompt-path delta is more
  consequential than expected — investigate before B.
- **Gate 2 (serve path):** validation §B shows ≥ 8 / 11 dispatches
  emit the correct tool with the correct arguments (≈ the per-intent
  L3 floor from rev 22). Below that, the rev-22 → live drift is
  large enough that the M5 number is misleading and we re-debug.
- **Gate 3 (no regression):** `cargo test -p mili-viz-server` green.

---

## v2 deferrals (do not pursue in M6)

| Item | Source | Why deferred |
| --- | --- | --- |
| Dispatch-feedback gap | m4 #1 | Independent of model; load-bearing across *all* agents, not v1-specific |
| Streaming responses | m4 (unwired) | Not blocking the 95.06 % number |
| Vision / image input | m4 #7 | FG-270M family doesn't support multimodal anyway |
| Mid-flight cancellation | m4 (unwired) | Independent; complex enough to be its own milestone |
| Tool coverage (7 unmapped tools) | m4 #6 | Independent; griz tool surface, not model surface |
| Q4_K_M quantization | m5 rev 22 | BF16 already at target; quant is edge/CPU lever |
| `select` per-intent floor (rev 21 #7) | m5 rev 21 | Needs v2 corpus paraphrase multiplier; not a serving fix |
| Re-render v1 corpus with dict-shaped args | m5 risks #6 | ✅ **Landed in m5 rev 23** (`assemble._normalize_tool_call_arguments`); takes effect on the v2 SFT cycle, not on the in-flight v1 ship. |

---

## Path forward

1. M5 risks #6 fix in `assemble.project_sft_record` (in-flight as a
   parallel workstream — see the rev 22+ commit on
   `m5-sft-cluster-bringup`).
2. M6 delta #1 — model swap. No code; documentation + a
   `llama-server` invocation change.
3. M6 delta #4 — share the system prompt file. Smallest code change,
   highest train-vs-serve risk; do this before deltas #2/#3.
4. M6 delta #2 — switch to `/v1/chat/completions` + `--jinja`.
5. M6 delta #3 — port the rev-21 parser to Rust; update unit tests.
6. M6 delta #5 — measurement only; decide after smoke.
7. Run validation §A → §B → §C; iterate if any gate fails.

Each step is independently revertable; #2/#3 are the biggest code
deltas and should land together (the new prompt path returns
content-shape responses that the new parser is built for).
