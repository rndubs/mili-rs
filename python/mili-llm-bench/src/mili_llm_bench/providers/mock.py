"""W5 — ``MockLlmProvider`` for deterministic harness/driver tests.

Yields its scripted outputs in order on successive ``generate`` calls;
raises ``MockExhausted`` if the harness asks for one more than
scripted. The strict shape forces tests to declare exactly the
rollout they are exercising (no silent-pad of a longer-than-expected
loop).

Used by:

* every W4a harness test in ``tests/test_harness.py``,
* the W4b driver tests when that PR lands,
* anywhere the consumer wants a pure-Python provider with no LLM /
  GPU / network requirement.
"""

from __future__ import annotations

import time
from dataclasses import dataclass, field
from typing import Any

from .base import ProviderOutput


class MockExhausted(IndexError):
    """``MockLlmProvider`` was asked for more outputs than scripted.

    A bare ``IndexError`` would also work, but the named subclass
    makes the test failure trace point at the test's own ``script``
    length rather than at an interior list index.
    """


@dataclass
class MockLlmProvider:
    """Scripted provider; the i-th ``generate`` call yields ``script[i]``.

    ``sleep_s`` is per-call (the same delay on every call) — used to
    exercise the W4a timeout path in
    ``test_harness.test_per_turn_timeout``.
    """

    script: list[ProviderOutput]
    sleep_s: float = 0.0
    _calls: int = field(default=0, init=False)

    def __post_init__(self) -> None:
        for i, item in enumerate(self.script):
            if not isinstance(item, ProviderOutput):
                raise TypeError(
                    f"MockLlmProvider script[{i}] must be a ProviderOutput, "
                    f"got {type(item).__name__}"
                )

    def generate(
        self,
        messages: list[dict[str, Any]],
        tools: list[dict[str, Any]],
        *,
        temperature: float,
        max_new_tokens: int,
        seed: int,
    ) -> ProviderOutput:
        if self._calls >= len(self.script):
            raise MockExhausted(
                f"MockLlmProvider exhausted after {self._calls} calls "
                f"(script length {len(self.script)})"
            )
        out = self.script[self._calls]
        self._calls += 1
        if self.sleep_s:
            time.sleep(self.sleep_s)
        return out

    @property
    def calls_made(self) -> int:
        return self._calls
