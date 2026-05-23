"""W5 — ``ReplayLlmProvider``: yield pre-recorded provider outputs from
a stored ``rollouts.jsonl`` instead of calling a real LLM.

Two uses (baseline §W4a "Replay mode"):

1. **Deterministic verifier regression** — re-grade a stored run
   under a new ``verifier.py`` or a new post-condition without
   re-running the LLM.
2. **Dataset validation** — round-trip a training corpus through the
   harness to confirm every recorded ``tool_calls`` slot still parses,
   dispatches under the live dispatcher, and produces the recorded
   response. Catches schema drift or fixture-fact drift before it
   pollutes a fine-tune.

The canonical rollout record shape lives in
``planning/mili-viz/posttraining-dataset.md`` §1. For v0 only the
``messages`` array is read; the rest is forward-compatible (we don't
parse fields we don't need).
"""

from __future__ import annotations

import json
from dataclasses import dataclass, field
from pathlib import Path
from typing import Any

from .base import ProviderOutput


def _parse_arguments(raw: Any) -> dict[str, Any]:
    """Normalize a stored ``tool_calls[*].function.arguments`` payload to
    a dict. JSONL records ship arguments as JSON-encoded strings (the
    OpenAI / FunctionGemma wire shape); we accept either form so a
    hand-fabricated rollout dict in a unit test still round-trips.
    """
    if isinstance(raw, dict):
        return raw
    if isinstance(raw, str):
        try:
            parsed = json.loads(raw)
        except json.JSONDecodeError:
            return {}
        return parsed if isinstance(parsed, dict) else {}
    return {}


def _extract_assistant_turns(
    messages: list[dict[str, Any]],
) -> list[ProviderOutput]:
    """Project a stored ``messages`` array onto the per-turn
    ``ProviderOutput`` stream the harness would have seen.

    Each ``assistant`` message becomes one ``ProviderOutput`` — either
    ``tool_calls=[...]`` (when the assistant emitted tool calls) or
    ``final_text=<content>`` (when it emitted plain text). Tool
    response messages (``role == "tool"``) are interleaved by the
    harness on replay; we do not surface them through the provider.
    """
    out: list[ProviderOutput] = []
    for msg in messages:
        if msg.get("role") != "assistant":
            continue
        raw_calls = msg.get("tool_calls") or []
        if raw_calls:
            tool_calls: list[dict[str, Any]] = []
            for tc in raw_calls:
                fn = tc.get("function") or {}
                tool_calls.append(
                    {
                        "name": fn.get("name", ""),
                        "arguments": _parse_arguments(fn.get("arguments", {})),
                    }
                )
            out.append(ProviderOutput(tool_calls=tool_calls))
        else:
            content = msg.get("content")
            text = content if isinstance(content, str) else ""
            out.append(ProviderOutput(final_text=text))
    return out


@dataclass
class ReplayLlmProvider:
    """Reads ``rollouts.jsonl``, finds the record with ``id ==
    scenario_id``, and replays its ``assistant`` turns on successive
    ``generate`` calls.

    Mirrors the ``MockLlmProvider`` exhaustion behaviour: asking for
    one more turn than the stored rollout contains raises
    ``IndexError`` — a clear signal that the live verifier or
    dispatcher is driving the loop further than the recording.
    """

    rollouts_path: Path
    scenario_id: str
    _stream: list[ProviderOutput] = field(default_factory=list, init=False)
    _calls: int = field(default=0, init=False)

    def __post_init__(self) -> None:
        record = self._find_record()
        if record is None:
            raise KeyError(
                f"scenario_id {self.scenario_id!r} not found in {self.rollouts_path}"
            )
        messages = record.get("messages")
        if not isinstance(messages, list):
            raise ValueError(
                f"rollout {self.scenario_id!r} in {self.rollouts_path}: "
                "'messages' missing or not a list"
            )
        self._stream = _extract_assistant_turns(messages)

    def _find_record(self) -> dict[str, Any] | None:
        with Path(self.rollouts_path).open() as f:
            for lineno, raw in enumerate(f, start=1):
                raw = raw.strip()
                if not raw:
                    continue
                try:
                    obj = json.loads(raw)
                except json.JSONDecodeError as exc:
                    raise ValueError(
                        f"{self.rollouts_path}:{lineno}: invalid JSON ({exc})"
                    ) from exc
                if obj.get("id") == self.scenario_id:
                    return obj
        return None

    def generate(
        self,
        messages: list[dict[str, Any]],
        tools: list[dict[str, Any]],
        *,
        temperature: float,
        max_new_tokens: int,
        seed: int,
    ) -> ProviderOutput:
        if self._calls >= len(self._stream):
            raise IndexError(
                f"ReplayLlmProvider exhausted after {self._calls} turns for "
                f"scenario {self.scenario_id!r} "
                f"(stored rollout has {len(self._stream)} assistant turns)"
            )
        out = self._stream[self._calls]
        self._calls += 1
        return out

    @property
    def turns(self) -> int:
        """Number of assistant turns in the stored rollout."""
        return len(self._stream)
