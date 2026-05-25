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
| 2 | Train-vs-inference chat-template parity (the big one) | pending GPU node | All post-SFT eval | 30 min |
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

The runtime serving path
(`python/mili-llm-bench/src/mili_llm_bench/providers/llamacpp.py::_build_functiongemma_prompt`)
hand-rolls the FunctionGemma prompt rather than using the GGUF's
baked-in jinja template. Training via HF `apply_chat_template` will
likely **not** produce a byte-identical prompt to what llama-server
feeds the trained model.

If you train against one format and serve through another, the SFT
optimization target is divergent from the inference distribution —
post-SFT L3 numbers are then uninterpretable.

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
- Runtime serving path that hand-rolls the FG prompt:
  `python/mili-llm-bench/src/mili_llm_bench/providers/llamacpp.py:198`
  (`_build_functiongemma_prompt`)
- Tool-format conversion helper (lift to shared module per
  `posttraining-dataset.md` Stage 6):
  `python/mili-llm-bench/src/mili_llm_bench/providers/llamacpp.py:330`
  (`_convert_to_openai_tool`)
- Google FunctionGemma fine-tuning guide:
  <https://ai.google.dev/gemma/docs/functiongemma/finetuning-with-functiongemma>

---

**Last updated:** 2026-05-24.
