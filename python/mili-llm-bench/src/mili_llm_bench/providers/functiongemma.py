"""W5 — ``FunctionGemmaProvider`` (HF transformers + the FunctionGemma
chat-template path); see ``planning/mili-viz/agent-local-llm-baseline.md`` §W5.

The v0 local-LLM baseline. Uses the documented function-calling chat
template on the model card: build a tool-decorated prompt via
``processor.apply_chat_template(messages, tools=tools, ...)``, then
``model.generate(...)``, then parse the
``<start_function_call>...<end_function_call>`` block to a canonical
``[{"name": str, "arguments": dict}, ...]`` list. Everything past that
is plain final text.

Deterministic: ``temperature == 0`` → greedy decode; we also seed
``torch.manual_seed(seed)`` before each call. The v0 baseline number is
only defensible against a pinned model revision — pass ``revision`` (or
let the default ``DEFAULT_REVISION`` carry it) so the artifact-store hash
stays fixed.

Heavy deps are lazy-imported inside ``generate`` (and inside the
helper that builds the pipeline). Importing this module on a machine
without ``transformers`` / ``torch`` installed succeeds; only
``FunctionGemmaProvider.generate`` raises ``ImportError`` (or
``RuntimeError`` if the optional extra is missing).
"""

from __future__ import annotations

import json
import re
from dataclasses import dataclass, field
from typing import Any

from .base import ProviderOutput

# The v0 baseline pin. The model card's function-calling chat-template
# revision is what the baseline number is taken against; bumping this
# invalidates the published number and the new run is a deliberate
# rebaseline.
DEFAULT_MODEL_ID = "google/functiongemma-270m-it"
DEFAULT_REVISION: str | None = None  # let HF resolve "main"; pin in CLI when reporting.

# The chat-template emit pattern for FunctionGemma. The model wraps its
# tool calls in ``<start_function_call>``...``<end_function_call>``
# tokens around a JSON-list payload (the chat template's documented
# shape). One block per turn (matches FunctionGemma's one-call-per-turn
# convention; the harness fans out to N elements if the JSON inside is
# a list of calls).
_CALL_BLOCK_RE = re.compile(
    r"<start_function_call>(.*?)<end_function_call>",
    re.DOTALL,
)


def _parse_tool_call_block(text: str) -> list[dict[str, Any]] | None:
    """Extract the inner JSON of the ``<start_function_call>...`` block
    and normalize it to canonical ``[{"name", "arguments": dict}, ...]``.

    Returns ``None`` when the text contains no recognizable block — the
    caller then treats the raw text as ``final_text``. Returns ``[]``
    on a recognized block whose JSON is malformed; the W4a harness then
    routes one synthetic ``parse_error`` slot back to the model.
    """
    match = _CALL_BLOCK_RE.search(text)
    if match is None:
        return None
    payload = match.group(1).strip()
    if not payload:
        return []
    try:
        parsed = json.loads(payload)
    except json.JSONDecodeError:
        return []

    raw_calls: list[Any]
    if isinstance(parsed, list):
        raw_calls = parsed
    elif isinstance(parsed, dict):
        raw_calls = [parsed]
    else:
        return []

    out: list[dict[str, Any]] = []
    for entry in raw_calls:
        if not isinstance(entry, dict):
            # A non-dict slot is a parse miss; preserve it so the
            # harness sees one malformed slot and recovers.
            out.append({"name": "", "arguments": {}})
            continue
        name = entry.get("name") or entry.get("tool_name") or ""
        args = entry.get("arguments") or entry.get("parameters") or {}
        if isinstance(args, str):
            try:
                args = json.loads(args)
            except json.JSONDecodeError:
                args = {}
        if not isinstance(args, dict):
            args = {}
        out.append({"name": str(name), "arguments": args})
    return out


def _import_runtime() -> tuple[Any, Any]:
    """Lazy-load ``transformers`` and ``torch``; raise a friendly
    ``ImportError`` if the ``functiongemma`` extra is not installed.
    """
    try:
        import torch  # type: ignore[import-not-found]
        import transformers  # type: ignore[import-not-found]
    except ImportError as exc:  # pragma: no cover — exercised on the user's box.
        raise ImportError(
            "FunctionGemmaProvider requires the 'functiongemma' optional "
            "dependency. Install with `pip install mili-llm-bench[functiongemma]`."
        ) from exc
    return torch, transformers


@dataclass
class FunctionGemmaProvider:
    """Stock FunctionGemma-270M-it driven through the HF chat template.

    Lazy-loads the model on the first ``generate`` call (so importing
    the module is cheap). Subsequent calls reuse the cached
    processor/model.

    The v0 baseline run sets ``temperature=0`` (greedy); the
    ``seed`` is plumbed into ``torch.manual_seed`` before generation
    so the run is reproducible across boxes.
    """

    model_id: str = DEFAULT_MODEL_ID
    revision: str | None = DEFAULT_REVISION
    device: str | None = None  # "cuda" / "cpu" / "mps"; None → auto.
    _processor: Any = field(default=None, init=False, repr=False)
    _model: Any = field(default=None, init=False, repr=False)
    _torch: Any = field(default=None, init=False, repr=False)

    def _ensure_loaded(self) -> None:
        if self._model is not None and self._processor is not None:
            return
        torch, transformers = _import_runtime()
        self._torch = torch
        # ``AutoProcessor`` covers the chat-template path for the
        # function-calling Gemma variants per the model card.
        from_pretrained_kw: dict[str, Any] = {}
        if self.revision is not None:
            from_pretrained_kw["revision"] = self.revision
        self._processor = transformers.AutoProcessor.from_pretrained(
            self.model_id, **from_pretrained_kw
        )
        device_kw: dict[str, Any] = {}
        if self.device is not None:
            device_kw["device_map"] = self.device
        else:
            device_kw["device_map"] = "auto"
        self._model = transformers.AutoModelForCausalLM.from_pretrained(
            self.model_id,
            torch_dtype="auto",
            **from_pretrained_kw,
            **device_kw,
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
        self._ensure_loaded()
        torch = self._torch
        torch.manual_seed(seed)

        inputs = self._processor.apply_chat_template(
            messages,
            tools=tools,
            add_generation_prompt=True,
            return_tensors="pt",
            return_dict=True,
        )
        # Move tensors to the model's device. ``device_map="auto"``
        # may have sharded; sampling the first parameter's device is
        # the documented HF idiom.
        device = next(self._model.parameters()).device
        inputs = {k: v.to(device) for k, v in inputs.items() if hasattr(v, "to")}

        prompt_len = int(inputs["input_ids"].shape[-1])

        with torch.no_grad():
            output_ids = self._model.generate(
                **inputs,
                max_new_tokens=max_new_tokens,
                do_sample=(temperature > 0.0),
                temperature=temperature if temperature > 0.0 else 1.0,
            )

        completion_ids = output_ids[0, prompt_len:]
        text = self._processor.decode(completion_ids, skip_special_tokens=False)

        tool_calls = _parse_tool_call_block(text)
        tokens_used = int(prompt_len + completion_ids.shape[-1])

        if tool_calls is None:
            # No function-call block → final text. Strip any special
            # tokens for the model-facing transcript.
            final = self._processor.decode(
                completion_ids, skip_special_tokens=True
            ).strip()
            return ProviderOutput(
                final_text=final,
                tokens_used=tokens_used,
                raw=text,
            )
        return ProviderOutput(
            tool_calls=tool_calls,
            tokens_used=tokens_used,
            raw=text,
        )


__all__ = ["DEFAULT_MODEL_ID", "DEFAULT_REVISION", "FunctionGemmaProvider"]
