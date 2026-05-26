"""Preflight #3 — SFTTrainer + tools-field dump.

Recipe: sft-preflight-gpu.md §3. Build SFTTrainer with model=None,
load sft/train.jsonl, optionally pass a formatting_func that renders
messages + tools through apply_chat_template, then decode batch[0]
and assert that the `<start_function_declaration>` token block reached
the tokenized batch.

Uses TRL 1.x's `SFTConfig.max_length` (the current upstream spelling).
"""

from __future__ import annotations

import argparse
import json
import os
import sys
from pathlib import Path

import torch
from datasets import load_dataset
from transformers import AutoModelForCausalLM, AutoTokenizer
from trl import SFTConfig, SFTTrainer


MARKER = "start_function_declaration"


def _build_formatting_func(tokenizer):
    def formatting_func(row):
        # SFTTrainer hands `row` to formatting_func in either a
        # single-record shape ({"messages": [...], ...}) or a batched
        # shape ({"messages": [[...], [...]], ...}) depending on the
        # collator path. Normalize.
        messages = row["messages"]
        tools = row.get("tools")
        if messages and isinstance(messages[0], list):
            return [
                tokenizer.apply_chat_template(
                    m,
                    tools=t,
                    add_generation_prompt=False,
                    tokenize=False,
                )
                for m, t in zip(messages, tools or [None] * len(messages))
            ]
        return tokenizer.apply_chat_template(
            messages,
            tools=tools,
            add_generation_prompt=False,
            tokenize=False,
        )

    return formatting_func


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument(
        "--train",
        default="/p/vast1/whitmore/cadsat/mili-rs/data/posttraining/sft/sft/train.jsonl",
        help="Path to sft/train.jsonl",
    )
    ap.add_argument(
        "--tokenizer",
        default="google/functiongemma-270m-it",
        help="HF tokenizer id",
    )
    ap.add_argument(
        "--max-length",
        type=int,
        default=4096,
        help="Match preflight #5 bumped value, not the 512 default",
    )
    ap.add_argument(
        "--with-formatting-func",
        dest="with_formatting_func",
        action="store_true",
        default=True,
    )
    ap.add_argument(
        "--without-formatting-func",
        dest="with_formatting_func",
        action="store_false",
    )
    ap.add_argument(
        "--output-dir",
        default="/tmp/sft_dump",
    )
    ap.add_argument(
        "--head-chars",
        type=int,
        default=4096,
        help="How many chars of decoded batch[0] to print",
    )
    args = ap.parse_args()

    print(f"# preflight #3 — SFTTrainer + tools-field dump")
    print(f"train:                {args.train}")
    print(f"tokenizer:            {args.tokenizer}")
    print(f"max_length:           {args.max_length}")
    print(f"with_formatting_func: {args.with_formatting_func}")
    print()

    tokenizer = AutoTokenizer.from_pretrained(args.tokenizer)
    # Trainer in transformers 4.57 rejects model=None even for dump-only
    # paths (§3 recipe is stale on this point). Load the real model on
    # CPU — we never call forward/backward, so it stays cold.
    model = AutoModelForCausalLM.from_pretrained(
        args.tokenizer,
        dtype=torch.bfloat16,
    )
    ds = load_dataset("json", data_files=args.train)["train"]
    print(f"loaded {len(ds)} rows; columns = {ds.column_names}")

    cfg_kwargs = dict(
        output_dir=args.output_dir,
        max_length=args.max_length,
        packing=False,
        per_device_train_batch_size=1,
        report_to="none",
    )
    if args.with_formatting_func:
        formatting_func = _build_formatting_func(tokenizer)
    else:
        formatting_func = None

    os.makedirs(args.output_dir, exist_ok=True)
    cfg = SFTConfig(**cfg_kwargs)

    trainer = SFTTrainer(
        model=model,
        args=cfg,
        train_dataset=ds,
        processing_class=tokenizer,
        formatting_func=formatting_func,
    )

    loader = trainer.get_train_dataloader()
    batch = next(iter(loader))
    print()
    print(f"batch keys:           {sorted(batch.keys())}")
    print(f"input_ids shape:      {tuple(batch['input_ids'].shape)}")

    decoded = tokenizer.decode(batch["input_ids"][0])
    marker_present = MARKER in decoded
    has_envelope = "<start_function_call>" in decoded

    print()
    print(f"'{MARKER}' in decoded[0]: {marker_present}")
    print(f"'<start_function_call>' in decoded[0]: {has_envelope}")
    print()
    print(f"--- decoded[0] head ({args.head_chars} chars) ---")
    print(decoded[: args.head_chars])
    print(f"--- /head ---")

    if not marker_present:
        print()
        print(
            "FAIL: tool declarations did NOT reach the tokenized batch. "
            "Either formatting_func is mandatory (re-run without "
            "--without-formatting-func) or the tokenizer's chat template "
            "ignores the `tools` argument — file a bug, do not train."
        )
        return 1

    print()
    print("PASS: tool declarations present in tokenized training batch")
    return 0


if __name__ == "__main__":
    sys.exit(main())
