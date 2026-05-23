"""W5 — ``LlmProvider`` Protocol and ``ProviderOutput`` dataclass.

The minimum surface every provider implements; consumed by W4a's
``run_turn``. See ``planning/mili-viz/agent-local-llm-baseline.md`` §W5.

PR-3 ships this Protocol plus the two pure-Python providers W4a's
tests need (``MockLlmProvider``, ``ReplayLlmProvider``).
FunctionGemma + Anthropic providers are PR-5; they live behind this
same seam so the v0 driver and W4a's tests do not change when they
land.
"""

from __future__ import annotations

from dataclasses import dataclass, field
from typing import Any, Protocol


@dataclass
class ProviderOutput:
    """One generate() result.

    Exactly one of ``tool_calls`` / ``final_text`` is the model's
    intended emission; if neither (or a malformed ``tool_calls`` slot)
    is present, the W4a harness records a synthetic ``parse_error``
    ``ExecutedCall`` so the model can self-correct on the next turn
    (baseline §W4a "Parse-error recovery").

    ``tool_calls`` is a list of *already-parsed* canonical entries
    ``{"name": str, "arguments": dict}``. A provider that cannot
    normalize its raw output to that shape either (a) returns the
    malformed entries unchanged so the harness can route them to
    ``parse_error``, or (b) returns ``tool_calls=None`` plus
    ``final_text`` set to the raw text.

    ``tokens_used`` is the total LLM tokens charged to this call
    (prompt + completion when the provider reports both). Drivers
    bound rollouts by a wall-clock and step-cap; tokens are reported
    for accounting only.
    """

    tool_calls: list[dict[str, Any]] | None = None
    final_text: str | None = None
    tokens_used: int = 0
    raw: Any = field(default=None, repr=False)


class LlmProvider(Protocol):
    """One generate per turn; deterministic when given the same inputs
    + ``seed``.

    The driver mutates ``messages`` between turns (one ``assistant``
    + N ``tool`` messages per dispatched call slot — see W4a). The
    provider receives the full transcript each turn; long-context
    truncation is not v0 work (baseline §W4a "Conversation truncation").
    """

    def generate(
        self,
        messages: list[dict[str, Any]],
        tools: list[dict[str, Any]],
        *,
        temperature: float,
        max_new_tokens: int,
        seed: int,
    ) -> ProviderOutput:
        ...
