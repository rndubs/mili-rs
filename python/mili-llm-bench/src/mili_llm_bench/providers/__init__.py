"""LLM provider seam — see ``planning/mili-viz/agent-local-llm-baseline.md`` §W5.

Always-on imports are limited to the pure-Python providers
(``MockLlmProvider``, ``ReplayLlmProvider``) so importing this package
does **not** drag in ``transformers`` / ``torch`` / ``anthropic``. The
heavy providers (``FunctionGemmaProvider``, ``AnthropicProvider``)
live in their own modules and are imported on demand by the CLI
factory; tests pin this with a fresh-reload ``sys.modules`` check.
"""

from .base import LlmProvider, ProviderOutput
from .mock import MockExhausted, MockLlmProvider
from .replay import ReplayLlmProvider

__all__ = [
    "LlmProvider",
    "ProviderOutput",
    "MockExhausted",
    "MockLlmProvider",
    "ReplayLlmProvider",
]
