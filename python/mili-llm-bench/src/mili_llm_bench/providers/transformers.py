"""``TransformersProvider`` — direct HF generate() against an SFT
checkpoint.

The companion to `LlamaCppProvider` for the M5 post-training pipeline.
Where `LlamaCppProvider` drives a llama-server over HTTP and consumes a
GGUF artifact, this provider loads a Hugging Face checkpoint in-process
and runs `model.generate(...)` directly. The two providers share the
same FG response-envelope parser (``providers._fg_envelope``) so the
prompt → generation → parsed-tool_calls round-trip is byte-identical to
the GGUF path — no chat-template mutation risk between HF and GGUF for
the bulk of the per-checkpoint heldout sweep.

The training recipe (cluster-setup.md §6, rev 4) renders prompts via
``tokenizer.apply_chat_template(messages, tools=tools, …)`` and runs
SFTTrainer over the result. This provider re-uses that same call at
inference time, so the model sees the exact prompt distribution it
trained on. Deterministic by default: ``temperature=0`` → greedy decode
with ``do_sample=False``; ``torch.manual_seed(seed)`` plumbs through
for reproducibility.

**Single template source.** The FG jinja baked into the trained
tokenizer is the only template; both training and inference call
``apply_chat_template`` against it. No bespoke renderer, no GBNF, no
secondary parser. The response is decoded as new-tokens-only and
handed to ``parse_fg_envelopes`` — the same helper the llama.cpp
fallback uses.

Heavy deps (``torch`` + ``transformers``) are lazy-imported on first
``generate``; importing this module on a torch-less machine succeeds.
Behind the ``[train]`` extra in ``pyproject.toml``.
"""

from __future__ import annotations

from dataclasses import dataclass, field
from typing import Any

from .base import ProviderOutput
from ._fg_envelope import parse_fg_envelopes
from ..tool_format import w1_to_openai_tool


def _import_runtime() -> tuple[Any, Any]:
    """Lazy-load ``torch`` and ``transformers``; raise a friendly
    ``ImportError`` if the ``train`` extra is not installed.
    """
    try:
        import torch  # type: ignore[import-not-found]
        import transformers  # type: ignore[import-not-found]
    except ImportError as exc:  # pragma: no cover — exercised on the user's box.
        raise ImportError(
            "TransformersProvider requires the 'train' optional "
            "dependency. Install with `pip install mili-llm-bench[train]`."
        ) from exc
    return torch, transformers


@dataclass
class TransformersProvider:
    """Run a local HF checkpoint via ``model.generate(...)``.

    Lazy-loads the tokenizer + model on the first ``generate`` call so
    importing the module is cheap. Subsequent calls reuse the loaded
    weights.

    Args:
        model_path: Filesystem path to a checkpoint directory (one of
            ``data/posttraining/checkpoints/v1/checkpoint-*`` for the v1
            sweep). No HF-hub default — the trained checkpoint is the
            only sane target.
        device: ``"cuda"`` (default), ``"cpu"``, ``"mps"``, etc.
            Forwarded to ``device_map=`` on ``from_pretrained``.
        attn_implementation: ``"eager"`` (default) to match the §6
            training recipe; switch to ``"flash_attention_2"`` post-v1
            once the FA2 wheel is in the env.

    The dtype is pinned to ``bfloat16`` to match the training recipe
    (`dtype=torch.bfloat16` in the §6 `from_pretrained` call). The
    270M model fits trivially in fp16/bf16 on H100; no quantization.
    """

    model_path: str
    device: str = "cuda"
    attn_implementation: str = "eager"
    _tokenizer: Any = field(default=None, init=False, repr=False)
    _model: Any = field(default=None, init=False, repr=False)
    _torch: Any = field(default=None, init=False, repr=False)
    _last_tool_call_parsing_approach: str | None = field(
        default=None, init=False, repr=False
    )

    def _ensure_loaded(self) -> None:
        if self._model is not None and self._tokenizer is not None:
            return
        torch, transformers = _import_runtime()
        self._torch = torch
        self._tokenizer = transformers.AutoTokenizer.from_pretrained(self.model_path)
        self._model = transformers.AutoModelForCausalLM.from_pretrained(
            self.model_path,
            dtype=torch.bfloat16,
            attn_implementation=self.attn_implementation,
            device_map=self.device,
        )
        self._model.eval()

    def generate(
        self,
        messages: list[dict[str, Any]],
        tools: list[dict[str, Any]],
        *,
        temperature: float,
        max_new_tokens: int,
        seed: int,
    ) -> ProviderOutput:
        """Render prompt via apply_chat_template, generate greedily,
        decode new-tokens-only, parse FG envelopes.

        ``temperature == 0`` → ``do_sample=False`` (greedy); any
        ``temperature > 0`` enables sampling at the requested
        temperature. The SFT eval path pins ``temperature=0``;
        sampling is a v2 lever (T≥1.0 to harvest DPO pairs) wired
        for completeness, not used in v1 grading.
        """
        self._ensure_loaded()
        torch = self._torch
        torch.manual_seed(seed)

        # The bench harness passes tools in W1 shape
        # (``{name, description, input_schema, output_schema}``); the
        # FG chat template expects the OpenAI shape
        # (``{type:"function", function:{name, description, parameters}}``).
        # Same conversion the llama.cpp provider applies before POST.
        # ``assemble.project_sft_record`` ran the same projection at
        # train time, so prompt-time and train-time agree byte-for-byte.
        openai_tools = (
            [w1_to_openai_tool(t) for t in tools] if tools else []
        )

        prompt = self._tokenizer.apply_chat_template(
            messages,
            tools=openai_tools,
            add_generation_prompt=True,
            tokenize=False,
        )
        inputs = self._tokenizer(prompt, return_tensors="pt").to(self._model.device)
        prompt_len = int(inputs["input_ids"].shape[-1])

        do_sample = temperature > 0.0
        with torch.no_grad():
            output_ids = self._model.generate(
                **inputs,
                max_new_tokens=max_new_tokens,
                do_sample=do_sample,
                temperature=temperature if do_sample else 1.0,
                pad_token_id=self._tokenizer.eos_token_id,
            )

        completion_ids = output_ids[0, prompt_len:]
        # skip_special_tokens=False keeps the <start_function_call>…
        # markers intact for parse_fg_envelopes; we re-decode without
        # specials for the final_text fallback path below.
        text = self._tokenizer.decode(completion_ids, skip_special_tokens=False)
        tokens_used = int(prompt_len + completion_ids.shape[-1])

        parsed = parse_fg_envelopes(text)
        if parsed:
            self._last_tool_call_parsing_approach = "fg-envelope"
            return ProviderOutput(
                tool_calls=parsed,
                tokens_used=tokens_used,
                raw=text,
            )

        self._last_tool_call_parsing_approach = "final-text"
        final = self._tokenizer.decode(
            completion_ids, skip_special_tokens=True
        ).strip()
        return ProviderOutput(
            final_text=final,
            tokens_used=tokens_used,
            raw=text,
        )


__all__ = ["TransformersProvider"]
