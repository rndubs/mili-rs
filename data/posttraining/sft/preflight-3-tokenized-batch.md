# Preflight #3 — SFTTrainer + `tools`-field dump

**Status:** ✅ PASS — with `formatting_func`. The `tools` array
reaches the tokenized training batch through
`apply_chat_template(messages, tools=tools, …)`. **Without
`formatting_func`, TRL 0.12.1 raises `KeyError: 'text'`** — so
`formatting_func` is **mandatory** for the v1 `trainer.train()`
recipe, matching `cluster-setup.md` §6.

Run date: 2026-05-25. Host: `matrix41` (NVIDIA H100 80GB).
Branch: `m5-sft-cluster-bringup`, head `b7cee2e`.

---

## Environment

| Component       | Version              |
| --------------- | -------------------- |
| python          | 3.13                 |
| torch           | 2.12.0+cu130         |
| transformers    | 4.57.6               |
| trl             | 0.12.1               |
| datasets        | 4.8.5                |
| accelerate      | 1.13.0               |
| HF model        | `google/functiongemma-270m-it` (BF16) |
| tokenizer       | same                 |

Recipe sources:

- `sft-preflight-gpu.md` §3 (the dump procedure)
- `cluster-setup.md` §6 (recipe knobs `formatting_func`,
  `max_length`, `packing=False`, `assistant_only_loss=True`)
- `python/scripts/sft_dump_one_batch.py` (the runnable script)

---

## Commands

```bash
source scripts/setup-gpu-env.sh
uv run --directory python python \
  /p/vast1/whitmore/cadsat/mili-rs/python/scripts/sft_dump_one_batch.py \
  --with-formatting-func

uv run --directory python python \
  /p/vast1/whitmore/cadsat/mili-rs/python/scripts/sft_dump_one_batch.py \
  --without-formatting-func
```

Defaults: `--train data/posttraining/sft/sft/train.jsonl`,
`--tokenizer google/functiongemma-270m-it`,
`--max-seq-length 4096`.

---

## Result — with `formatting_func`

```
loaded 82 rows; columns = ['scenario_id', 'intent_id', 'fixture',
  'instruction', 'instruction_source', 'messages', 'tools',
  'tool_calls_flat', 'postcondition']

batch keys:                              ['attention_mask', 'input_ids', 'labels']
input_ids shape:                         (1, 3311)
'start_function_declaration' in decoded[0]: True
'<start_function_call>' in decoded[0]:      True

PASS: tool declarations present in tokenized training batch
```

Decoded `batch[0]` head opens with `<bos><bos><start_of_turn>developer`,
followed by the canonical bench system prompt
(`system_prompt_sha256 = 9f36d0deb5e98a89`, unchanged from rev 8),
followed by 18 `<start_function_declaration> … <end_function_declaration>`
blocks (one per tool — `close`, `clrsel`, `colormap`, … — using FG's
`<escape>…<escape>` arg syntax exactly as the inference path produces
it). The assistant-turn `<start_function_call>` envelope appears
later in the row, confirming the rollout's tool calls also survive.

The `(1, 3311)` token count is consistent with the preflight #5
audit's range (`min=3234, p50=3263, p95=3337, max=3341` across 82
rows; this happened to be a mid-band row). No truncation at the
`max_seq_length=4096` ceiling.

## Result — without `formatting_func`

```
File "trl/trainer/sft_trainer.py", line 513, in tokenize
    element[dataset_text_field] if formatting_func is None else formatting_func(element),
File "datasets/formatting/formatting.py", line 283, in __getitem__
    value = self.data[key]
KeyError: 'text'
```

TRL 0.12.1's non-packed dataloader path defaults
`dataset_text_field="text"` and indexes the row by that key when no
`formatting_func` is supplied. Our assembled rows carry `messages` /
`tools` / `postcondition` instead — `KeyError` is the correct failure
shape. (Stronger than the §3 recipe's predicted silent-drop failure
mode; the result is the same: `formatting_func` is required.)

---

## Decision

- **`formatting_func` is mandatory** for the v1 `trainer.train()`
  call against this corpus. Pin it explicitly in the recipe — do not
  rely on `dataset_text_field` auto-detect.
- The `formatting_func` shape used here
  (`tokenizer.apply_chat_template(messages, tools=tools, tokenize=False)`)
  is the same call SFTTrainer makes internally with
  `add_generation_prompt=False`. Reuse this signature in the
  cluster-setup.md §6 recipe.
- The chat-template renders the developer / user / assistant /
  tool roles into the FG envelope syntax cleanly. No template
  surgery needed.

---

## TRL-0.12.1-vs-recipe API drift (surfaced, not fixed here)

Two pinned knobs in `cluster-setup.md` §6 don't exist on TRL 0.12.1:

| Recipe knob               | trl 0.12.1 status                                |
| ------------------------- | ------------------------------------------------ |
| `SFTConfig(max_length=…)` | Renamed `max_seq_length` (trl 0.13+ uses `max_length`) |
| `SFTConfig(assistant_only_loss=True)` | Added in trl 0.20+ (`AssistantOnlyLoss` callback in 0.12.x) |

This script substitutes `max_seq_length` for the dump-only check
(no functional difference for tokenization). `assistant_only_loss`
is preflight #4's surface and is not exercised here.

**Action:** before `trainer.train()` we either (a) lift the trl pin
to ≥ 0.20 in `python/mili-llm-bench/pyproject.toml`'s `train` extra,
or (b) rewrite the loss-masking path in cluster-setup.md §6 to use
a custom data collator that drops non-assistant-turn labels to
`-100`. (Option b is what the §6 entry calls "the config seam"
already landed pre-rev 12 — to be verified during preflight #4.)

---

## What this clears

- **#3 row in `sft-preflight-gpu.md`**: pending → ✅ 2026-05-25 PASS.
- Per §3's pass criteria: the assertion holds *with*
  `formatting_func`. The fallback diagnosis ("if it fails *without*
  `formatting_func`, the `formatting_func` is mandatory") is the
  observed branch — `formatting_func` is mandatory.
- Unblocks preflight #4 (`assistant_only_loss=True` mask check),
  which sits on top of the same `formatting_func` + data-loader
  plumbing.

## What this does NOT clear

- The runtime forward/backward path. We only walked the data side.
  Preflight #4 is the first check that actually runs a step.
- The TRL pin drift above. Surfaced; the cluster-setup.md §6
  decision is queued for preflight #4.
