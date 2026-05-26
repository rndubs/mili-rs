"""SFT v1 training entry point — FunctionGemma-270M-IT on the rev-13 corpus.

Implements the recipe pinned in
``planning/mili-viz/mili-agent/cluster-setup.md`` §6 (rev 4): Google's
reference hyperparameters (LR 5e-5 / 8 epochs / bs=4 / constant LR /
``adamw_torch_fused``) with two deliberate deviations recorded in
``planning/mili-viz/mili-agent/m5-sft-pipeline.md``:

  * ``max_length=4096`` (vs Google's 512) — preflight #5 observed
    ``max=3341`` tokens on the rev-13 corpus's 82 rows (`m5` rev 14).
  * ``assistant_only_loss=False`` at the TRL level — TRL 1.5.0's native
    path raises at ``SFTTrainer.__init__`` on FG's macro-heavy chat
    template (preflight #4, `m5` rev 17). Loss masking is supplied by
    ``MaskAssistantOnlyCollator`` instead; the training *intent* (loss
    only on assistant turns) is unchanged.

Designed to be launched via ``scripts/sft_train.sbatch`` from a login
node (sbatch picks the GPU); also runnable directly inside an existing
GPU shell as a fallback.
"""

from __future__ import annotations

import argparse
import sys
from pathlib import Path

import torch
from datasets import load_dataset
from transformers import (
    AutoModelForCausalLM,
    AutoTokenizer,
    DataCollatorForLanguageModeling,
)
from trl import SFTConfig, SFTTrainer

from mili_llm_bench.assistant_only_collator import MaskAssistantOnlyCollator


# Absolute paths — login-node and compute-node safe; the bench's
# uv-cwd gotcha (memory: bench-cli-uv-cwd) applies to relative paths
# through `uv run --directory python`.
_REPO_ROOT = Path("/p/vast1/whitmore/cadsat/mili-rs")
_DEFAULT_TRAIN = _REPO_ROOT / "data/posttraining/sft/sft/train.jsonl"
_DEFAULT_VAL = _REPO_ROOT / "data/posttraining/sft/sft/val.jsonl"
_DEFAULT_OUT = _REPO_ROOT / "data/posttraining/checkpoints/v1"


def _build_formatting_func(tokenizer):
    """Render messages + tools through apply_chat_template.

    Mirrors `sft_dump_one_batch.py::_build_formatting_func` so the
    train-time render is byte-identical to the preflight-#3 render
    the rev-15 dump locked in. `formatting_func` is optional on
    TRL 1.x (auto-detect handles `messages`+`tools` rows) but the §6
    recipe retains it for drift-proofing against a hypothetical TRL
    2.x auto-detect change.
    """

    def formatting_func(row):
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
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument(
        "--model-id",
        default="google/functiongemma-270m-it",
        help="HF base model id (gated; needs huggingface-cli login)",
    )
    ap.add_argument(
        "--train",
        type=Path,
        default=_DEFAULT_TRAIN,
        help="Path to sft/train.jsonl (rev-13 layout: sft/sft/train.jsonl)",
    )
    ap.add_argument(
        "--val",
        type=Path,
        default=_DEFAULT_VAL,
        help="Path to sft/val.jsonl",
    )
    ap.add_argument(
        "--output-dir",
        type=Path,
        default=_DEFAULT_OUT,
        help="Checkpoint dir; trainer writes per-epoch ckpts here",
    )
    # Hyperparameters — defaults match cluster-setup.md §6 rev 4.
    # Override only with a one-line justification in the run's
    # dataset_card.md so silent drift is impossible.
    ap.add_argument("--learning-rate", type=float, default=5e-5)
    ap.add_argument("--num-train-epochs", type=int, default=8)
    ap.add_argument("--per-device-train-batch-size", type=int, default=4)
    ap.add_argument(
        "--per-device-eval-batch-size",
        type=int,
        default=1,
        help=(
            "Default 1 — HF's 8 OOMs on FG's 262K-vocab × 4096-seq logits "
            "tensor (~17 GB per bs at eval, doubled by the contiguous-slice "
            "copy in TRL's compute_loss). bs=1 keeps eval-time logits ≈ 2 GB. "
            "Eval set is small (8 rows) so wall-clock cost is trivial."
        ),
    )
    ap.add_argument("--gradient-accumulation-steps", type=int, default=1)
    ap.add_argument("--lr-scheduler-type", default="constant")
    ap.add_argument("--max-length", type=int, default=4096)
    ap.add_argument("--optim", default="adamw_torch_fused")
    ap.add_argument("--seed", type=int, default=42)
    ap.add_argument(
        "--attn-implementation",
        default="eager",
        choices=["eager", "sdpa", "flash_attention_2"],
        help="`eager` matches Google's validated recipe; switch only after v1 clears tripwire",
    )
    ap.add_argument(
        "--report-to",
        default="none",
        help='"none" by default; pass "wandb" for a run dashboard',
    )
    args = ap.parse_args()

    if not args.train.exists():
        print(f"FAIL: train file missing: {args.train}", file=sys.stderr)
        return 1
    if not args.val.exists():
        print(f"FAIL: val file missing: {args.val}", file=sys.stderr)
        return 1

    print("# SFT v1 — FunctionGemma-270M-IT")
    print(f"model_id:          {args.model_id}")
    print(f"train:             {args.train}")
    print(f"val:               {args.val}")
    print(f"output_dir:        {args.output_dir}")
    print(f"learning_rate:     {args.learning_rate}")
    print(f"num_train_epochs:  {args.num_train_epochs}")
    print(f"train_batch_size:  {args.per_device_train_batch_size}")
    print(f"eval_batch_size:   {args.per_device_eval_batch_size}")
    print(f"grad_accum:        {args.gradient_accumulation_steps}")
    print(f"lr_scheduler:      {args.lr_scheduler_type}")
    print(f"max_length:        {args.max_length}")
    print(f"optim:             {args.optim}")
    print(f"attn_impl:         {args.attn_implementation}")
    print(f"seed:              {args.seed}")
    print()

    print("Loading tokenizer + model...")
    tokenizer = AutoTokenizer.from_pretrained(args.model_id)
    # Preflight #3 surfaced a TRL warning when padding_side was not
    # "right"; harmless for the dump pass, cosmetic-but-fixable for the
    # real training run.
    tokenizer.padding_side = "right"

    model = AutoModelForCausalLM.from_pretrained(
        args.model_id,
        dtype=torch.bfloat16,
        attn_implementation=args.attn_implementation,
        device_map="cuda",
    )
    print(f"model loaded: {sum(p.numel() for p in model.parameters()) / 1e6:.1f}M params")
    print()

    print("Loading datasets...")
    train_ds = load_dataset("json", data_files=str(args.train))["train"]
    val_ds = load_dataset("json", data_files=str(args.val))["train"]
    print(f"train rows: {len(train_ds)} | val rows: {len(val_ds)}")
    print()

    cfg = SFTConfig(
        output_dir=str(args.output_dir),
        # Google reference recipe (cluster-setup.md §6 hyperparam table)
        num_train_epochs=args.num_train_epochs,
        per_device_train_batch_size=args.per_device_train_batch_size,
        per_device_eval_batch_size=args.per_device_eval_batch_size,
        gradient_accumulation_steps=args.gradient_accumulation_steps,
        learning_rate=args.learning_rate,
        lr_scheduler_type=args.lr_scheduler_type,
        max_length=args.max_length,
        packing=False,
        optim=args.optim,
        bf16=True,
        # TRL's native assistant_only_loss path raises on FG's template
        # (preflight #4, m5 rev 17). Mask via the custom collator below.
        assistant_only_loss=False,
        # Reporting / checkpoints
        eval_strategy="epoch",
        save_strategy="epoch",
        logging_steps=1,
        report_to=args.report_to,
        seed=args.seed,
        data_seed=args.seed,
    )

    trainer = SFTTrainer(
        model=model,
        args=cfg,
        train_dataset=train_ds,
        eval_dataset=val_ds,
        processing_class=tokenizer,
        formatting_func=_build_formatting_func(tokenizer),
        data_collator=MaskAssistantOnlyCollator(
            DataCollatorForLanguageModeling(tokenizer=tokenizer, mlm=False),
        ),
    )

    print("Starting trainer.train()...")
    train_result = trainer.train()
    print()
    print(f"train_runtime:        {train_result.metrics.get('train_runtime'):.1f} s")
    print(f"train_loss:           {train_result.metrics.get('train_loss'):.4f}")
    print(f"train_samples_per_s:  {train_result.metrics.get('train_samples_per_second'):.2f}")
    print()

    final_dir = args.output_dir / "final"
    print(f"Saving final checkpoint to {final_dir}...")
    trainer.save_model(str(final_dir))
    # Belt + suspenders per cluster-setup.md §6 — ensure the GGUF
    # converter finds the tokenizer next to the weights.
    tokenizer.save_pretrained(str(final_dir))
    print("Done.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
