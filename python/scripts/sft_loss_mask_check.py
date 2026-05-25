"""Preflight #4 — assistant_only_loss mask + BOS-doubling probe.

Recipe: sft-preflight-gpu.md §4. Build SFTTrainer with the rev-16 TRL 1.5.0
pin, pull the first batch via get_train_dataloader(), and inspect the
labels tensor:

  - assistant_only_loss=True: count non--100 label positions; decode them;
    assert they look like assistant content (FG `<start_function_call>`
    envelopes, not user / developer / tool turns).
  - assistant_only_loss=False (cross-check): non--100 count should match
    the non-pad token count to within a couple of percent (HF default
    causal-LM label policy: pad -> -100, everything else trains).

Side: count the leading <bos> token doubling that the rev-16 changelog
flagged on the formatting_func path, and compare against the TRL 1.x
auto-detect path (no formatting_func).

Dump-only — no forward/backward; model is loaded on GPU in BF16 because
SFTTrainer's __init__ rejects model=None on transformers 4.57+, but
no compute touches it.
"""

from __future__ import annotations

import argparse
import json
import os
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


def _build_formatting_func(tokenizer):
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


def _build_trainer(
    tokenizer,
    model,
    ds,
    output_dir: str,
    max_length: int,
    mask_assistant_only: bool,
    use_formatting_func: bool,
) -> SFTTrainer:
    os.makedirs(output_dir, exist_ok=True)
    # Always assistant_only_loss=False at the TRL level — option B routes the
    # masking through our custom collator instead of TRL's native path.
    cfg = SFTConfig(
        output_dir=output_dir,
        max_length=max_length,
        packing=False,
        per_device_train_batch_size=1,
        report_to="none",
        assistant_only_loss=False,
    )
    formatting_func = _build_formatting_func(tokenizer) if use_formatting_func else None
    base_collator = DataCollatorForLanguageModeling(
        tokenizer=tokenizer, mlm=False,
    )
    data_collator = (
        MaskAssistantOnlyCollator(base_collator) if mask_assistant_only else base_collator
    )
    return SFTTrainer(
        model=model,
        args=cfg,
        train_dataset=ds,
        processing_class=tokenizer,
        formatting_func=formatting_func,
        data_collator=data_collator,
    )


def _count_leading_bos(decoded: str, bos_token: str) -> int:
    n = 0
    while decoded.startswith(bos_token):
        n += 1
        decoded = decoded[len(bos_token) :]
    return n


def _report_mode(label: str, batch, tokenizer, pad_id: int) -> dict:
    input_ids = batch["input_ids"][0]
    labels = batch["labels"][0]
    attn = batch.get("attention_mask")
    if attn is not None:
        attn = attn[0]

    total = int(input_ids.numel())
    pad_mask = input_ids == pad_id
    n_pad = int(pad_mask.sum().item())
    n_non_pad = total - n_pad
    n_attn = int(attn.sum().item()) if attn is not None else n_non_pad
    n_visible = int((labels != -100).sum().item())

    mask = labels != -100
    visible_ids = input_ids[mask].tolist()
    decoded_visible = tokenizer.decode(visible_ids)
    decoded_all = tokenizer.decode(input_ids.tolist())

    bos_token = tokenizer.bos_token or "<bos>"
    leading_bos = _count_leading_bos(decoded_all, bos_token)

    print()
    print(f"=== {label} ===")
    print(f"  total tokens:              {total}")
    print(f"  pad tokens:                {n_pad}")
    print(f"  non-pad tokens:            {n_non_pad}")
    print(f"  attention_mask sum:        {n_attn}")
    print(f"  non--100 labels:           {n_visible}")
    if n_non_pad:
        print(f"  visible / non-pad:         {n_visible / n_non_pad:.4f}")
    print(f"  leading {bos_token!r}:     {leading_bos}")
    print(f"  visible-token decode (first 512 chars):")
    print(f"    {decoded_visible[:512]!r}")

    return {
        "label": label,
        "total": total,
        "pad": n_pad,
        "non_pad": n_non_pad,
        "attention_mask_sum": n_attn,
        "visible": n_visible,
        "leading_bos": leading_bos,
        "decoded_visible_head": decoded_visible[:512],
        "decoded_all_head": decoded_all[:256],
    }


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument(
        "--train",
        default="/p/vast1/whitmore/cadsat/mili-rs/data/posttraining/sft/sft/train.jsonl",
    )
    ap.add_argument("--tokenizer", default="google/functiongemma-270m-it")
    ap.add_argument("--max-length", type=int, default=4096)
    ap.add_argument("--output-dir", default="/tmp/sft_loss_mask")
    ap.add_argument(
        "--report-json",
        default=None,
        help="Optional path to dump the per-mode metrics as JSON.",
    )
    args = ap.parse_args()

    print(f"# preflight #4 — assistant_only_loss mask + BOS-doubling probe")
    print(f"train:        {args.train}")
    print(f"tokenizer:    {args.tokenizer}")
    print(f"max_length:   {args.max_length}")

    tokenizer = AutoTokenizer.from_pretrained(args.tokenizer)
    tokenizer.padding_side = "right"

    # Model loaded on GPU in BF16; never forward/backward.
    model = AutoModelForCausalLM.from_pretrained(
        args.tokenizer,
        dtype=torch.bfloat16,
        device_map="cuda" if torch.cuda.is_available() else "cpu",
    )
    ds = load_dataset("json", data_files=args.train)["train"]
    pad_id = tokenizer.pad_token_id

    print(f"rows: {len(ds)};  columns: {ds.column_names}")
    print(f"pad_token_id: {pad_id};  bos_token: {tokenizer.bos_token!r}")
    print(f"chat_template head: {tokenizer.chat_template[:120]!r}")

    modes = [
        ("mask=on,  formatting_func=on",  True,  True),
        ("mask=off, formatting_func=on",  False, True),
        ("mask=on,  formatting_func=off", True,  False),
    ]
    reports = {}
    for label, mask, ff in modes:
        out_sub = os.path.join(
            args.output_dir,
            label.replace(" ", "").replace(",", "_").replace("=", ""),
        )
        trainer = _build_trainer(
            tokenizer, model, ds,
            output_dir=out_sub,
            max_length=args.max_length,
            mask_assistant_only=mask,
            use_formatting_func=ff,
        )
        batch = next(iter(trainer.get_train_dataloader()))
        reports[label] = _report_mode(label, batch, tokenizer, pad_id)

    print()
    print("=== Gate checks ===")

    r_off = reports["mask=off, formatting_func=on"]
    deviation_off = abs(r_off["visible"] - r_off["non_pad"]) / max(r_off["non_pad"], 1)
    pass_off = deviation_off < 0.02
    print(
        f"  [off]  visible / non-pad deviation: {deviation_off:.4f} "
        f"({'PASS' if pass_off else 'FAIL'}; expect < 0.02)"
    )

    r_on = reports["mask=on,  formatting_func=on"]
    frac_on = r_on["visible"] / max(r_on["non_pad"], 1)
    pass_on_size = 0.001 < frac_on < 0.5
    print(
        f"  [on,ff] visible / non-pad fraction: {frac_on:.4f} "
        f"({'PASS' if pass_on_size else 'FAIL'}; "
        f"expect 0.001..0.5; near 0 = mask broke, near 1 = kwarg ignored)"
    )
    head_on = r_on["decoded_visible_head"]
    has_assistant_marker = (
        "<start_function_call>" in head_on or "function_call" in head_on
    )
    print(
        f"  [on,ff] decoded visible contains assistant content: "
        f"{'PASS' if has_assistant_marker else 'WARN'}"
    )

    r_on_noff = reports["mask=on,  formatting_func=off"]
    frac_on_noff = r_on_noff["visible"] / max(r_on_noff["non_pad"], 1)
    pass_on_noff = 0.001 < frac_on_noff < 0.5
    print(
        f"  [on,-ff] visible / non-pad fraction: {frac_on_noff:.4f} "
        f"({'PASS' if pass_on_noff else 'FAIL'}; same band as [on,ff])"
    )

    print()
    print("=== BOS doubling ===")
    print(f"  formatting_func=on  -> leading BOS = {r_on['leading_bos']}")
    print(f"  formatting_func=off -> leading BOS = {r_on_noff['leading_bos']}")
    if r_on["leading_bos"] == 1 and r_on_noff["leading_bos"] == 1:
        bos_verdict = "BOTH SINGLE — no BOS tax."
    elif r_on["leading_bos"] > 1 and r_on_noff["leading_bos"] == 1:
        bos_verdict = (
            "formatting_func path DOUBLES BOS; auto-detect is clean. "
            "Recommended fix: drop formatting_func and rely on TRL 1.x "
            "auto-detect (preferred — already validated by rev-16 preflight #3)."
        )
    elif r_on["leading_bos"] == 1 and r_on_noff["leading_bos"] > 1:
        bos_verdict = (
            "Auto-detect doubles BOS; formatting_func is clean. "
            "Unexpected — investigate TRL apply_chat_template behavior."
        )
    else:
        bos_verdict = "BOTH PATHS DOUBLE BOS — investigate tokenizer/template."
    print(f"  -> {bos_verdict}")

    # Full-corpus scan: apply the collator to every row, report distribution.
    print()
    print("=== Full-corpus mask scan (all rows) ===")
    full_trainer = _build_trainer(
        tokenizer, model, ds,
        output_dir=os.path.join(args.output_dir, "fullscan"),
        max_length=args.max_length,
        mask_assistant_only=True,
        use_formatting_func=True,
    )
    full_trainer.args.per_device_train_batch_size = 1
    loader = full_trainer.get_train_dataloader()
    visible_counts = []
    non_pad_counts = []
    for row_batch in loader:
        labels = row_batch["labels"][0]
        input_ids = row_batch["input_ids"][0]
        v = int((labels != -100).sum().item())
        np_ = int((input_ids != pad_id).sum().item())
        visible_counts.append(v)
        non_pad_counts.append(np_)
    n_rows = len(visible_counts)
    fractions = [v / max(np_, 1) for v, np_ in zip(visible_counts, non_pad_counts)]
    fractions_sorted = sorted(fractions)
    visible_sorted = sorted(visible_counts)
    def pct(arr, q):
        i = max(0, min(len(arr) - 1, int(round((q / 100) * (len(arr) - 1)))))
        return arr[i]
    full_min_v = min(visible_counts)
    full_p50_v = pct(visible_sorted, 50)
    full_p95_v = pct(visible_sorted, 95)
    full_max_v = max(visible_counts)
    full_min_f = min(fractions)
    full_p50_f = pct(fractions_sorted, 50)
    full_p95_f = pct(fractions_sorted, 95)
    full_max_f = max(fractions)
    print(f"  rows scanned: {n_rows}")
    print(f"  visible tokens (min/p50/p95/max): "
          f"{full_min_v}/{full_p50_v}/{full_p95_v}/{full_max_v}")
    print(f"  visible / non-pad (min/p50/p95/max): "
          f"{full_min_f:.4f}/{full_p50_f:.4f}/{full_p95_f:.4f}/{full_max_f:.4f}")
    # Sanity: every row should have at least SOME visible content;
    # the per-row floor catches any row where the mask collapses to all -100.
    rows_zero_visible = sum(1 for v in visible_counts if v == 0)
    pass_full_floor = rows_zero_visible == 0
    print(f"  rows with 0 visible tokens: {rows_zero_visible} "
          f"({'PASS' if pass_full_floor else 'FAIL — collator broke on these rows'})")

    print()
    overall = pass_off and pass_on_size and has_assistant_marker and pass_full_floor
    print("OVERALL:", "PASS" if overall else "FAIL")

    if args.report_json:
        Path(args.report_json).parent.mkdir(parents=True, exist_ok=True)
        with open(args.report_json, "w") as f:
            json.dump(
                {
                    "modes": reports,
                    "gates": {
                        "off_deviation": deviation_off,
                        "off_pass": pass_off,
                        "on_ff_fraction": frac_on,
                        "on_ff_pass_size": pass_on_size,
                        "on_ff_has_assistant_marker": has_assistant_marker,
                        "on_noff_fraction": frac_on_noff,
                        "on_noff_pass": pass_on_noff,
                        "bos_verdict": bos_verdict,
                        "full_scan_rows": n_rows,
                        "full_scan_visible_min": full_min_v,
                        "full_scan_visible_p50": full_p50_v,
                        "full_scan_visible_p95": full_p95_v,
                        "full_scan_visible_max": full_max_v,
                        "full_scan_fraction_min": full_min_f,
                        "full_scan_fraction_p50": full_p50_f,
                        "full_scan_fraction_p95": full_p95_f,
                        "full_scan_fraction_max": full_max_f,
                        "full_scan_rows_zero_visible": rows_zero_visible,
                        "full_scan_floor_pass": pass_full_floor,
                    },
                    "overall_pass": overall,
                },
                f,
                indent=2,
            )
        print(f"wrote {args.report_json}")

    return 0 if overall else 1


if __name__ == "__main__":
    sys.exit(main())
