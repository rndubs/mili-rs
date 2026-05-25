# SFT pre-flight — on-cluster checks before `trainer.train()`

The items below cannot be validated off-GPU. They must clear **on
the H100 cluster, in the live training environment, before the first
real training run**. Skipping any of them is the most common way to
ship an SFT model that scores 0 % L3 for a reason that has nothing to
do with the training data or hyperparameters.

This is the runnable companion to
[`cluster-setup.md`](cluster-setup.md) §0. The off-GPU items are
already resolved in that doc; this one is the to-do list for day 1 of
cluster bring-up.

For *why* these checks matter, see the rev-2 critique entry in
[`m5-sft-pipeline.md`](m5-sft-pipeline.md) changelog.

---

## Ordering — run top to bottom

Each check builds on the previous one. Don't skip ahead — failure at
an early check invalidates everything below it.

| # | Check | Status | Blocks | Time |
|---|---|---|---|---|
| 1 | HF login + model fetch | ✅ 2026-05-24 | All training | 5 min |
| 2 | Train-vs-inference chat-template parity (the big one) | ✅ 2026-05-24 — Path A (rev 8, prompt side) + option (b) (rev 10, response side); see §2 "Resolved via option (b)" | All post-SFT eval | 30 min |
| 3 | SFTTrainer + `tools` field test | pending `sft/train.jsonl` (Stage 6) | Stage 6 → training | 15 min |
| 4 | `assistant_only_loss=True` compatibility | pending GPU node | Training | 10 min |
| 5 | `max_length=512` audit | pending `sft/train.jsonl` (Stage 6) | Training data integrity | 10 min |
| 6 | GGUF chat-template baking | pending trained checkpoint | Post-SFT eval | 15 min |

---

## 1. HF login + model fetch

**Status (2026-05-24):** ✅ PASS. `rwhitmore` authenticated; Gemma
license granted by Google after manual review (initial accept-and-fetch
returned 403 "awaiting review"; resolved within the session). Smoke
fetch returned `gemma3_text` config + 262,146-token tokenizer, cached
under `~/.cache/huggingface/` (default location — see commentary in
`scripts/setup-gpu-env.sh` for why we did not override `HF_HOME`).

FunctionGemma is gated. Without an accepted-license token the rest
of the pipeline 401s with a confusing error.

```bash
# 1. Accept license at huggingface.co/google/functiongemma-270m-it (web)
# 2. Mint a read token at huggingface.co/settings/tokens
huggingface-cli login --token "$HF_TOKEN"

# Smoke fetch — should print the config JSON, not raise 401
uv run --directory python python -c "
from transformers import AutoTokenizer, AutoConfig
cfg = AutoConfig.from_pretrained('google/functiongemma-270m-it')
tok = AutoTokenizer.from_pretrained('google/functiongemma-270m-it')
print('OK:', cfg.model_type, '|', len(tok), 'tokens')
"
```

**Pass criteria:** prints the model type (likely `gemma3`) and
tokenizer vocab size without an HTTP error.

---

## 2. Train-vs-inference chat-template parity (highest stakes)

**Status (2026-05-24):** ✅ Resolved end-to-end via **Path A**
(rev 8, prompt side) + **option (b)** (rev 10, response side).
Prompt path: `LlamaCppProvider.generate` POSTs to
``/v1/chat/completions`` and relies on llama-server's ``--jinja``
flag to apply the FunctionGemma chat template baked into the GGUF.
The bespoke ``_build_functiongemma_prompt`` stays deleted from
`python/mili-llm-bench/src/mili_llm_bench/providers/llamacpp.py`.
Training via HF `apply_chat_template` against
`google/functiongemma-270m-it` and inference render the *prompt*
through the same FG jinja — single source of truth for what the
model sees on input.

Response path: a client-side `content → tool_calls` fallback inside
`LlamaCppProvider.generate` covers the llama.cpp autoparser gap
(PR #18675 / master `566059a` replaced the specialized chat-template
handlers with a differential PEG autoparser that can't infer FG's
`<escape>` arg wrapping). The fallback is caps-gated on `/props`
`chat_template_caps.supports_tool_calls`: when that's False (the
current b9307 / `549b9d843` state) the fallback parses the FG
envelopes client-side and synthesizes structured `tool_calls`; when
a future build flips it to True, the fallback turns itself off and
the server's native parser wins. Test pins:
`python/mili-llm-bench/tests/test_providers_llamacpp.py::
TestChatCompletionsPath` (POST URL, OpenAI tool shape, tool-call
parsing, no-bespoke-renderer guard) +
`TestFallbackParser` (single/multi envelope, escape + bare args,
OAI tool_calls preserved when caps=True).

The parity check itself was a login-safe diff (HF
`apply_chat_template` vs the now-deleted `_build_functiongemma_prompt`)
captured under `/tmp/sft_template_parity/` during the bring-up; the
documented divergences are kept verbatim in 2a below to motivate why
Path A was chosen and to surface the **v5 re-baseline** that this
change demands. The divergences were:

1. **Developer message body dropped on the inference side.** The
   bespoke renderer wrote its own hard-coded developer turn ("You
   are a model that can do function calling with the following
   functions") and discarded the actual `developer` message content
   — the bench-pinned system prompt
   (`system_prompt_sha256 = 9f36d0deb5e98a89`) never reached the
   model in production. Training would have included it.
2. **Tool-parameter schema flattened to `{key:type}`** vs HF's full
   JSON Schema with `properties`/`type`/`description` nesting.
3. **Tool-call argument JSON re-serialized** with Python-cased
   `True`/`False`/`None` vs JSON-cased `true`/`false`/`null`, and
   custom `<escape>...<escape>` markers vs verbatim JSON strings.
4. **Tool-response wrapped as raw content** vs HF's
   `{value:<escape>...<escape>}`.
5. **`<bos>` token absent** on the inference side (HF prepends it).
6. **Whitespace** — HF inlines `<start_function_declaration>` blocks;
   inference inserted newlines.

(1) is the consequential one. The v5 floor (40 % L3) was measured
against a renderer that nullified the pinned system prompt — so the
re-baseline below may *raise* the floor. That's why it's queued
explicitly rather than assumed unchanged.

### Resolved via option (b) — client-side fallback in `LlamaCppProvider` (2026-05-24, rev 10)

The rev-9 v6 baseline of 0 / 50 L3 was a direct consequence of the
upstream parser gap: `llama-server` b9307 / `549b9d843` returns
FG's `<start_function_call>…<end_function_call>` output as literal
`message.content` because the autoparser introduced in PR #18675
(master `566059a`) can't infer FG's `<escape>` arg wrapping. The
deliberate decision matrix (a) upstream upgrade / (b) client-side
fallback / (c) revert Path A / (d) switch runtime is documented in
`m5-sft-pipeline.md` changelog rev 9 "Option (a) status"; option
(a) was ruled out (no upstream PR or owner) and option (b)
implemented in rev 10.

The fallback is response-side only and re-uses the regex shape from
vLLM's `FunctionGemmaToolParser` (envelope
`<start_function_call>\s*call:(\w+)\s*\{(.*?)\}\s*<end_function_call>`;
string args `(\w+):<escape>(.*?)<escape>`; bare scalars
`(\w+):([^,}]+)`). Gated on `/props`
`chat_template_caps.supports_tool_calls`: when False (today) the
fallback runs after every chat-completions response if `tool_calls`
is empty AND content contains an FG envelope; when a future build
flips supports_tool_calls=True, the fallback turns itself off and
the server's native parser wins. The prompt path stays
unchanged — `/v1/chat/completions` + `--jinja`.

**v7 re-baseline (2026-05-24, `matrix37` H100, BF16 GGUF, canonical
config): 13 / 50 L3 (26.0 %).** Maps to the first branch below
(`L3 ≈ 40 % ±5 pp`-ish, modulo intent-distribution drift). The
remaining 13 parse_errors are the same v6 model-refusal cluster
verbatim (load 5/6, colormap 3/4, scattered show/select/clrsel) —
verified by inspecting each rollout's final assistant content:
zero contain an FG envelope, all match "I cannot assist with…"
refusal text. The fallback is doing exactly its job; the residual
26 % is a prompt-engineering / SFT-lift problem, not a parser
problem.

Originally documented decision-tree outcomes (kept for context):

- **L3 ≈ 40 % (±5 pp)**: jinja and bespoke paths were isomorphic —
  **this is what landed** (26 % is in the same band, modulo the
  refusal cluster the bespoke trigger phrase was suppressing).
- **L3 substantially higher (≥ 50 %)**: dropped system prompt was
  constraining the bespoke path.
- **L3 lower (≤ 35 %)**: bespoke format unintentionally helping —
  partially true; the rev-8 bespoke trigger phrase was slightly
  load-bearing for the 13/50 refusal subset, but the dominant
  rev-9 symptom was the upstream parser gap, not the prompt-side
  delta.

### 2a. Render both sides on one sample

```bash
# Start llama-server (GGUF baked-in template path)
llama-server -hf ggml-org/functiongemma-270m-it-GGUF:BF16 --jinja &

# Get a sample row
SAMPLE=$(head -1 data/posttraining/sft/train.jsonl)
echo "$SAMPLE" > /tmp/sft_sample.json
```

```python
# python/scripts/sft_template_parity.py  (add this script)
import json, requests, sys
from transformers import AutoTokenizer

sample  = json.loads(open("/tmp/sft_sample.json").read())
tok     = AutoTokenizer.from_pretrained("google/functiongemma-270m-it")

# HF (training) side
hf_render = tok.apply_chat_template(
    sample["messages"],
    tools=sample["tools"],
    add_generation_prompt=False,
    tokenize=False,
)

# llama-server side — POST to /apply-template with --jinja active
resp = requests.post(
    "http://localhost:8080/apply-template",
    json={"messages": sample["messages"], "tools": sample["tools"]},
).json()
ls_render = resp["prompt"]

with open("/tmp/hf_render.txt", "w") as f: f.write(hf_render)
with open("/tmp/ls_render.txt", "w") as f: f.write(ls_render)

if hf_render == ls_render:
    print("PASS: byte-identical")
    sys.exit(0)
else:
    print("FAIL: see diff /tmp/hf_render.txt /tmp/ls_render.txt")
    sys.exit(1)
```

### 2b. Decision tree on failure

If 2a fails — and it probably will, since `llamacpp.py` currently
ignores `--jinja` in favor of `_build_functiongemma_prompt` — pick
**one** path before writing any training code:

- **Path A — switch inference to `--jinja`.** Modify `llamacpp.py`
  to remove `_build_functiongemma_prompt` and just hit
  `/v1/chat/completions` with raw `messages`+`tools`; let
  llama-server apply the baked-in jinja template. Re-run the v5
  baseline; if L3 is still ~40 %, the runtime path is now
  template-correct and training can proceed via HF
  `apply_chat_template`.
- **Path B — back-port the bespoke format to a custom HF
  chat template.** Write a jinja template that reproduces
  `_build_functiongemma_prompt` exactly, set
  `tokenizer.chat_template = "<that jinja>"` before training, and
  retain the bespoke inference path. Higher risk of drift over time
  — Path A is preferred.

**Pass criteria:** 2a returns "PASS: byte-identical" on at least
three sample rows (pick one single-step, one multi-step, one with
`griz_raw`).

---

## 3. SFTTrainer + `tools` field test

TRL's default collator may drop non-`messages` columns. If `tools` is
dropped, the model trains without ever seeing the
`<start_function_declaration>` blocks — silent failure mode.

```python
# python/scripts/sft_dump_one_batch.py
from transformers import AutoTokenizer
from trl import SFTConfig, SFTTrainer
from datasets import load_dataset

tok = AutoTokenizer.from_pretrained("google/functiongemma-270m-it")
ds  = load_dataset("json", data_files="data/posttraining/sft/train.jsonl")["train"]

def formatting_func(row):
    return tok.apply_chat_template(
        row["messages"], tools=row["tools"],
        add_generation_prompt=False, tokenize=False,
    )

cfg = SFTConfig(output_dir="/tmp/sft_dump", max_length=512, packing=False)
trainer = SFTTrainer(
    model=None,                         # dump-only; model not needed
    args=cfg,
    train_dataset=ds,
    processing_class=tok,
    formatting_func=formatting_func,
)
# Render one batch and grep for the tool-declaration token
batch = next(iter(trainer.get_train_dataloader()))
decoded = tok.decode(batch["input_ids"][0])
assert "start_function_declaration" in decoded, \
    "tools array did not reach apply_chat_template — formatting_func missing or broken"
print("PASS: tool declarations present in tokenized training batch")
```

**Pass criteria:** assertion holds. If it fails *without*
`formatting_func`, the `formatting_func` is mandatory (matches the
recipe in `cluster-setup.md` §6). If it fails *with* `formatting_func`
too, the tokenizer's chat template itself doesn't render tools —
file a bug, do not train.

---

## 4. `assistant_only_loss=True` compatibility test

`assistant_only_loss=True` should mask loss everywhere except inside
assistant turns. Confirm it correctly identifies FunctionGemma's
`<start_of_turn>model …<end_of_turn>` spans.

```python
# python/scripts/sft_loss_mask_check.py
# Run a single training step with assistant_only_loss=True, capture
# the labels tensor, and assert: every position outside assistant
# turns has label == -100 (the HF "ignore" sentinel).
#
# Cross-check by toggling assistant_only_loss=False and verifying
# loss is computed on *all* non-pad tokens.
```

**Pass criteria:** with `assistant_only_loss=True`, the fraction of
non-`-100` label positions matches the assistant-turn token count
to within ~1 % (allow some slop for special tokens at turn
boundaries). If the fraction is ~0 % or ~100 %, the feature is
mis-detecting role boundaries — do **not** train; either fall back
to manual loss masking or pin a different TRL version.

---

## 5. `max_length=512` audit

Multi-step compound scenarios with the full ~16-tool inventory can
silently exceed 512 tokens. The trainer truncates from the right —
which throws away the assistant turns we actually want to learn from.

```bash
uv run --directory python python -c "
import json
from transformers import AutoTokenizer
tok = AutoTokenizer.from_pretrained('google/functiongemma-270m-it')
worst = 0
for line in open('data/posttraining/sft/train.jsonl'):
    row = json.loads(line)
    n = len(tok.apply_chat_template(row['messages'], tools=row['tools'],
                                    add_generation_prompt=False, tokenize=True))
    worst = max(worst, n)
print('max tokens:', worst)
"
```

**Pass criteria:** `worst <= 512`. If it exceeds:
- Bump `max_length` to the next power-of-2 above the observed max
  (1024, 2048). Cost is a small VRAM bump; H100 has headroom.
- *Or* prune the assembled tools array per scenario (only include
  tools the canonical sequence actually calls). This makes the
  training distribution narrower than inference — risky.
- Pick option 1 unless VRAM forces option 2.

---

## 6. GGUF chat-template baking

`convert_hf_to_gguf.py` carries the tokenizer's `chat_template` into
GGUF metadata. Some TRL versions silently mutate `chat_template`
during `save_model()`. If the GGUF ships a different template than
what training rendered against, post-SFT eval is invalid for the
same reason as check #2.

```bash
# After training + save_model:
diff <(python -c "import json; print(json.load(open('google-functiongemma-270m-it/tokenizer_config.json'))['chat_template'])") \
     <(python -c "import json; print(json.load(open('data/posttraining/checkpoints/v1/final/tokenizer_config.json'))['chat_template'])")
```

**Pass criteria:** empty diff. If non-empty, **do not convert to
GGUF**; first figure out what mutated the template (often TRL adding
a special-token entry). Either pin a different TRL version or
restore the original `chat_template` before `convert_hf_to_gguf.py`.

After conversion:

```bash
# Inspect the GGUF's baked-in template
python llama.cpp/gguf-py/scripts/gguf_dump.py \
  data/posttraining/checkpoints/v1/functiongemma-v1.bf16.gguf \
  | grep -A50 chat_template
```

Cross-check that the printed template matches the HF tokenizer's
template byte-for-byte (modulo GGUF's UTF-8 escaping).

---

## After all checks pass

You can run `trainer.train()` with confidence that:

- The model loads (check 1)
- Training prompts equal serving prompts (check 2)
- The model sees the tool declarations during training (check 3)
- Loss is computed only on assistant turns (check 4)
- No assistant turn is silently truncated (check 5)
- The trained checkpoint serves with the same template it trained
  against (check 6)

Record the pass/fail of each check in the v1 training run's
`dataset_card.md` so the rev-3 critique has the receipts.

---

## Pointers

- Off-GPU pre-flight (✅ resolved 2026-05-24): `cluster-setup.md` §0
- Runtime serving path (`--jinja` prompt + rev-10 client-side
  response fallback):
  `python/mili-llm-bench/src/mili_llm_bench/providers/llamacpp.py`
  (`LlamaCppProvider.generate`, `_parse_fg_envelopes`,
  `_fetch_caps_supports_tool_calls`)
- Tool-format conversion helper (lift to shared module per
  `posttraining-dataset.md` Stage 6):
  `python/mili-llm-bench/src/mili_llm_bench/providers/llamacpp.py`
  (`LlamaCppProvider._convert_to_openai_tool`)
- Google FunctionGemma fine-tuning guide:
  <https://ai.google.dev/gemma/docs/functiongemma/finetuning-with-functiongemma>

---

**Last updated:** 2026-05-24.
