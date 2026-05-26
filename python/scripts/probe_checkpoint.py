"""One-shot probe: load a checkpoint, render one heldout row's prompt,
generate, and dump the raw model output. Use to diagnose why
TransformersProvider sees a tool call with empty arguments.
"""

from __future__ import annotations

import argparse
import json
import sys
from pathlib import Path

import torch
from transformers import AutoModelForCausalLM, AutoTokenizer


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--model-path", required=True)
    ap.add_argument(
        "--scenarios",
        default="/p/vast1/whitmore/cadsat/mili-rs/data/posttraining/sft/eval/heldout.jsonl",
    )
    ap.add_argument("--row", type=int, default=0, help="Which row to probe.")
    ap.add_argument("--max-new-tokens", type=int, default=256)
    args = ap.parse_args()

    with Path(args.scenarios).open() as f:
        rows = [json.loads(line) for line in f if line.strip()]
    row = rows[args.row]

    print(f"=== Row {args.row} ===")
    print(f"scenario_id: {row.get('scenario_id')}")
    print(f"intent_id:   {row.get('intent_id')}")
    print(f"instruction: {row.get('instruction')}")
    print(f"expected tool_calls_flat: {row.get('tool_calls_flat')}")
    print()

    tok = AutoTokenizer.from_pretrained(args.model_path)
    model = AutoModelForCausalLM.from_pretrained(
        args.model_path,
        dtype=torch.bfloat16,
        attn_implementation="eager",
        device_map="cuda",
    )
    model.eval()

    # The heldout row already carries `messages` and `tools` in the
    # exact shape SFTTrainer rendered against. Drop the trailing
    # assistant turn (the "target") so we generate it.
    messages = row["messages"]
    target_turn = None
    while messages and messages[-1]["role"] == "assistant":
        target_turn = messages.pop()
    tools = row["tools"]

    prompt = tok.apply_chat_template(
        messages,
        tools=tools,
        add_generation_prompt=True,
        tokenize=False,
    )
    print("=== Rendered prompt (first 800 chars) ===")
    print(prompt[:800])
    print("...")
    print("=== Rendered prompt (last 400 chars) ===")
    print(prompt[-400:])
    print()
    print(f"=== Target assistant turn ===")
    print(json.dumps(target_turn, indent=2))
    print()

    inputs = tok(prompt, return_tensors="pt").to(model.device)
    with torch.no_grad():
        output = model.generate(
            **inputs,
            max_new_tokens=args.max_new_tokens,
            do_sample=False,
            pad_token_id=tok.eos_token_id,
        )

    prompt_len = inputs["input_ids"].shape[-1]
    completion = output[0, prompt_len:]
    text_with_specials = tok.decode(completion, skip_special_tokens=False)
    text_no_specials = tok.decode(completion, skip_special_tokens=True)

    print("=== Raw model output (with specials) ===")
    print(repr(text_with_specials))
    print()
    print("=== Raw model output (no specials) ===")
    print(repr(text_no_specials))
    return 0


if __name__ == "__main__":
    sys.exit(main())
