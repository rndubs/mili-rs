# Preflight #4 — `assistant_only_loss` mask check + BOS-doubling probe

**Status:** ✅ PASS (2026-05-25, `matrix41` H100, TRL 1.5.0).

**Runnable:** `python/scripts/sft_loss_mask_check.py`.
**Module:** `python/mili-llm-bench/src/mili_llm_bench/assistant_only_collator.py`.
**JSON report:** `data/posttraining/sft/preflight-4-loss-mask.json`.

---

## Headline finding

**TRL 1.5.0's `assistant_only_loss=True` is not usable as-is on
FunctionGemma's chat template.** `SFTTrainer.__init__` raises
``ValueError: The chat template is not training-compatible (missing
prefix-preservation or `{% generation %}` markers) and patching is not
supported for this template.``

The FG chat template is macro-heavy
(`format_parameters`, `format_function_declaration`, `format_argument`)
and TRL's auto-patch can't infer assistant boundaries from the
substituted-`role` pattern. The rev-12 "config seam" claim in
`cluster-setup.md` §0 (line 53-55) was therefore genuinely vacuous —
the kwarg accepts `True` but the trainer dies before any batch is
produced. Cleared by **option B** (custom data collator) instead of
**option A** (patching the FG template to add `{% generation %}`
markers, which would require structurally rewriting the for-loop body
because Jinja requires balanced block tags that can't span
`{% endif %}` boundaries).

## Path taken: option B — custom collator

`MaskAssistantOnlyCollator` (in
`python/mili-llm-bench/src/mili_llm_bench/assistant_only_collator.py`)
wraps any base causal-LM collator. After the base assembles a batch,
the wrapper walks each row's `input_ids` and rewrites `labels` so the
loss is computed only on tokens the *model* emits. Two passes:

1. **Find model-turn spans** — every `[<start_of_turn>=105, model=4368,
   \n=107] ... <end_of_turn>=106` region. Start exclusive of the
   3-token header (model doesn't predict its own role marker); EOT
   inclusive (model learns to stop).

2. **Subtract tool-response payloads** inside each span — positions
   inside `<start_function_response>response: ... <end_function_response>`
   (token IDs 50, 6275, 236787, ..., 51). The bare
   `<start_function_response>` (token 50) that the assistant emits
   *without* a following `response:` stays unmasked, because that's
   the assistant's own cue to the tool.

TRL config: `assistant_only_loss=False`; pass the collator via
`data_collator=` on `SFTTrainer`.

## Gate results

Single-row probe (row 0 of `data/posttraining/sft/sft/train.jsonl`,
3310 non-pad tokens, no padding):

| Check | Result | Verdict |
| --- | --- | --- |
| `mask=off`: visible / non-pad deviation | 0.0000 | PASS (< 0.02) |
| `mask=on, formatting_func=on`: visible / non-pad fraction | 0.0051 | PASS (0.001..0.5) |
| `mask=on, formatting_func=on`: decoded visible contains assistant content | yes (`<start_function_call>...<end_function_call><start_function_response>`) | PASS |
| `mask=on, formatting_func=off`: visible / non-pad fraction | 0.0051 | PASS |
| Tool-response payload masked | yes (verified by decode) | PASS |

Full-corpus scan (all 82 train rows):

| Stat | Visible tokens | Visible / non-pad |
| --- | --- | --- |
| min | 13 | 0.0040 |
| p50 | 17 | 0.0052 |
| p95 | 39 | 0.0117 |
| max | 39 | 0.0117 |

**0 / 82 rows have zero visible tokens** — the mask never collapses to
all `-100`. Single-tool scenarios cluster at 13–17 visible tokens
(one `<start_function_call>...<end_function_call><start_function_response>`
envelope); compound rows reach 39 (three envelopes).

## BOS-doubling probe — resolved

The rev-16 changelog flagged that the `formatting_func` path produced
doubled `<bos>` while the TRL 1.x auto-detect path produced single. On
the option-B path measured here, **both paths produce single `<bos>`** —
TRL 1.5.0's tokenize step (under `assistant_only_loss=False` + custom
data_collator) honors the BOS already in the formatted string and does
not prepend another. No per-row BOS tax under option B.

## Side observation worth noting

Per-row visible-token counts are small (0.4 %–1.2 % of each row). This
is the expected consequence of the ~2700-token tool-declaration
overhead per row (preflight #5 finding). The model still receives
strong gradient signal on tool-call syntax (every row produces tool
call envelopes), but free-text and natural-language gradient signal
is sparse. **`TODO(v2)`:** if SFT plateaus below the regression
tripwire, oversample rows that include a final free-text assistant
message (compound scenarios with explicit summary turns) so the model
also learns to emit short post-tool acknowledgments.

## Tests

`python/mili-llm-bench/tests/test_assistant_only_collator.py` — 8
pins (always-on except the live token-ID check, which gates on the FG
tokenizer being available). Covers: single assistant turn, tool
response remasking, bare `<start_function_response>` (no
following `response:`), multiple tool calls in one model turn, padded
batches, multiple model turns per row, no-model-turn row, and a runtime
pin that the 6 hard-coded token IDs still match what
`tokenizer.encode(...)` produces against
`google/functiongemma-270m-it`. Full suite: 229 / 229 pass, 1 skip
(unchanged baseline from rev 16's 221, +8 new).

## Implications for the §6 training recipe

The `cluster-setup.md` §6 recipe code block currently has
`assistant_only_loss=True` in `SFTConfig`. That kwarg must flip to
`False`, and the `SFTTrainer(...)` call must pass
`data_collator=MaskAssistantOnlyCollator(
DataCollatorForLanguageModeling(tokenizer, mlm=False))`. The
parameters table entry's *intent* ("compute loss only on assistant
turns") is unchanged; the *implementation* moves from a TRL kwarg to a
local wrapper. This change is gated on user confirmation before
`trainer.train()`.
