"""Custom data collator that masks loss on non-assistant tokens for FG SFT.

Why this exists: TRL 1.5.0's native `assistant_only_loss=True` requires
the chat template to expose `{% generation %}...{% endgeneration %}`
markers (or a structure TRL can auto-patch). FunctionGemma's chat
template is macro-heavy and has neither, so `SFTTrainer.__init__` raises
``ValueError: The chat template is not training-compatible``. See
``data/posttraining/sft/preflight-4-loss-mask.md`` for the bring-up
report and the m5-sft-pipeline.md rev-17 changelog entry.

What this collator does: after the base causal-LM collator assembles the
batch, walk each row's ``input_ids`` and rewrite ``labels`` so the loss
is computed only on tokens the *model* emits — content + tool calls +
the bare ``<start_function_response>`` cue. Everything else (developer
preamble, user instructions, tool-response payloads) is masked to -100.

Two-pass algorithm:

  1. Find ``[<start_of_turn>=105, model=4368, \\n=107] ... <end_of_turn>=106``
     spans (start exclusive of header; EOT inclusive — model should
     learn to emit it).
  2. Inside each model-turn span, *remask* tool-response payloads:
     positions inside ``<start_function_response>response: ...
     <end_function_response>``. The bare ``<start_function_response>``
     that the assistant emits at the end of its tool calls (no following
     ``response:``) stays unmasked.

Token IDs are pinned constants below — verified once at runtime against
``tokenizer.encode`` in tests. If the FG tokenizer ever changes them
upstream, the unit test in ``tests/test_assistant_only_collator.py``
fires.
"""

from __future__ import annotations

import torch


# FunctionGemma role-marker token IDs (verified via tokenizer.encode):
_MODEL_HEADER_IDS = (105, 4368, 107)
"""``<start_of_turn>`` + ``model`` + ``\\n``."""

_EOT_ID = 106
"""``<end_of_turn>``."""

_SFR_ID = 50
"""``<start_function_response>`` (single special token)."""

_EFR_ID = 51
"""``<end_function_response>``."""

_RESPONSE_ID = 6275
"""Regular text token ``response``."""

_COLON_ID = 236787
"""Regular text token ``:``."""


class MaskAssistantOnlyCollator:
    """Wrap a base causal-LM collator; mask labels outside assistant content.

    Usage::

        from transformers import DataCollatorForLanguageModeling
        from mili_llm_bench.assistant_only_collator import MaskAssistantOnlyCollator

        base = DataCollatorForLanguageModeling(tokenizer=tok, mlm=False)
        trainer = SFTTrainer(
            ...,
            args=SFTConfig(..., assistant_only_loss=False),
            data_collator=MaskAssistantOnlyCollator(base),
        )
    """

    def __init__(self, base_collator):
        self.base_collator = base_collator

    def __call__(self, features):
        batch = self.base_collator(features)
        input_ids = batch["input_ids"]
        labels = batch["labels"]

        new_labels = torch.full_like(labels, -100)
        b_count, n = input_ids.shape
        h0, h1, h2 = _MODEL_HEADER_IDS

        for b in range(b_count):
            ids = input_ids[b].tolist()

            # Pass 1: model-turn spans.
            i = 0
            model_spans = []
            while i <= n - 3:
                if ids[i] == h0 and ids[i + 1] == h1 and ids[i + 2] == h2:
                    start = i + 3
                    j = start
                    while j < n and ids[j] != _EOT_ID:
                        j += 1
                    end = min(j + 1, n)
                    model_spans.append((start, end))
                    new_labels[b, start:end] = labels[b, start:end]
                    i = end
                else:
                    i += 1

            # Pass 2: subtract tool-response payloads from each model span.
            for s, e in model_spans:
                k = s
                while k <= e - 3:
                    if (
                        ids[k] == _SFR_ID
                        and ids[k + 1] == _RESPONSE_ID
                        and ids[k + 2] == _COLON_ID
                    ):
                        m = k + 1
                        while m < e and ids[m] != _EFR_ID:
                            m += 1
                        remask_end = min(m + 1, e)
                        new_labels[b, k + 1:remask_end] = -100
                        k = remask_end
                    else:
                        k += 1

        batch["labels"] = new_labels
        return batch


__all__ = [
    "MaskAssistantOnlyCollator",
    "_MODEL_HEADER_IDS",
    "_EOT_ID",
    "_SFR_ID",
    "_EFR_ID",
    "_RESPONSE_ID",
    "_COLON_ID",
]
