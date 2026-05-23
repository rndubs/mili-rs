"""W5 — AnthropicProvider gated tests.

Skip-on-absent: the ``anthropic`` package is optional. Run behind the
``test-heavy`` CI job, not the default ``pytest`` invocation.

The prompt-caching pin (``test_prompt_caching_is_set_on_system_and_tools``)
is the load-bearing test for the recurring frontier-baseline cost — it
asserts the request body carries ``cache_control`` on the system block
and on the final tool entry. Without that pin, the 10x-cost regression
called out in baseline.md goes silent.

The smoke test
(``test_anthropic_generate_smoke_against_live_api``) requires
``ANTHROPIC_API_KEY``; it is the only test that hits the network.
"""

from __future__ import annotations

import json
import os
from typing import Any

import pytest

# Pure-Python conversion helpers do NOT require the anthropic SDK —
# they live in our own module. Test them always-on as the harness pin.
from mili_llm_bench.providers.anthropic import (
    AnthropicProvider,
    DEFAULT_MODEL_ID,
    _convert_messages,
    _convert_tools,
    _split_system_and_messages,
    _parse_response,
)


def _require_anthropic_sdk() -> None:
    """Skip the calling test if the optional ``anthropic`` SDK isn't
    installed. The pure-Python conversion helpers above this point do
    NOT need the SDK and stay always-on."""
    pytest.importorskip("anthropic")


# ---------------------------------------------------------------------------
# CRITICAL: prompt-caching pin. Without cache_control on the system
# block and the tools list, the frontier-baseline run is 10x more
# expensive than it needs to be (baseline.md §"Anthropic prompt
# caching"). The test mocks the SDK transport so it doesn't hit the
# network.
# ---------------------------------------------------------------------------


class _FakeMessagesClient:
    def __init__(self) -> None:
        self.last_request: dict[str, Any] | None = None

    def create(self, **kwargs: Any) -> Any:
        self.last_request = kwargs

        class _Block:
            type = "text"
            text = "(ok)"

        class _Usage:
            input_tokens = 10
            output_tokens = 5
            cache_read_input_tokens = 0
            cache_creation_input_tokens = 0

        class _Response:
            content = [_Block()]
            usage = _Usage()

        return _Response()


class _FakeClient:
    def __init__(self) -> None:
        self.messages = _FakeMessagesClient()


def test_prompt_caching_is_set_on_system_and_tools() -> None:
    """Pin baseline.md §"Anthropic prompt caching".

    Without this pin, the recurring frontier-baseline run is ~10x more
    expensive (system+tools re-tokenized every turn). The unit test
    mocks the SDK transport so it doesn't hit the network."""
    fake = _FakeClient()
    provider = AnthropicProvider(model="claude-test", client=fake)
    messages = [
        {"role": "developer", "content": "you are the assistant"},
        {"role": "user", "content": "do the thing"},
    ]
    tools = [
        {"name": "load", "description": "load db", "input_schema": {"type": "object"}},
        {"name": "show", "description": "show", "input_schema": {"type": "object"}},
    ]
    provider.generate(messages, tools, temperature=0.0, max_new_tokens=128, seed=0)

    body = fake.messages.last_request
    assert body is not None

    # System prompt: a list of blocks, last block carries cache_control.
    assert isinstance(body["system"], list)
    assert body["system"][-1].get("cache_control") == {"type": "ephemeral"}

    # Tools list: last tool carries cache_control.
    assert isinstance(body["tools"], list)
    assert body["tools"][-1].get("cache_control") == {"type": "ephemeral"}
    # Non-final tools do NOT carry cache_control (caching is cumulative
    # up to the marker; tagging earlier tools wastes a cache breakpoint).
    assert "cache_control" not in body["tools"][0]


def test_anthropic_message_conversion_round_trips_tool_use() -> None:
    """A canonical ``assistant`` tool_calls turn → Anthropic tool_use
    blocks; the matching ``tool`` reply → a user message carrying a
    ``tool_result`` block."""
    messages = [
        {"role": "user", "content": "load it"},
        {
            "role": "assistant",
            "tool_calls": [
                {
                    "id": "c_0",
                    "type": "function",
                    "function": {
                        "name": "load",
                        "arguments": json.dumps({"root": "d3samp6"}),
                    },
                }
            ],
        },
        {
            "role": "tool",
            "tool_call_id": "c_0",
            "name": "load",
            "content": json.dumps({"ok": True}),
        },
    ]
    converted = _convert_messages(messages)
    assert converted[0] == {"role": "user", "content": "load it"}
    assert converted[1]["role"] == "assistant"
    blocks = converted[1]["content"]
    assert blocks[0]["type"] == "tool_use"
    assert blocks[0]["id"] == "c_0"
    assert blocks[0]["name"] == "load"
    assert blocks[0]["input"] == {"root": "d3samp6"}
    assert converted[2]["role"] == "user"
    tr = converted[2]["content"][0]
    assert tr["type"] == "tool_result"
    assert tr["tool_use_id"] == "c_0"


def test_anthropic_system_split_handles_developer_prefix() -> None:
    messages = [
        {"role": "developer", "content": "you are the assistant"},
        {"role": "user", "content": "go"},
    ]
    system, rest = _split_system_and_messages(messages)
    assert len(system) == 1
    assert system[0]["text"] == "you are the assistant"
    # System block carries cache_control marker.
    assert system[0]["cache_control"] == {"type": "ephemeral"}
    assert rest == [{"role": "user", "content": "go"}]


def test_anthropic_parse_response_routes_tool_use_vs_text() -> None:
    class _Tool:
        type = "tool_use"
        id = "x"
        name = "load"
        input = {"root": "d3samp6"}

    class _Usage:
        input_tokens = 100
        output_tokens = 50
        cache_read_input_tokens = 25
        cache_creation_input_tokens = 0

    class _Resp:
        content = [_Tool()]
        usage = _Usage()

    out = _parse_response(_Resp())
    assert out.tool_calls is not None
    assert out.tool_calls[0]["name"] == "load"
    assert out.tool_calls[0]["arguments"] == {"root": "d3samp6"}
    # tokens_used includes cache_read_input_tokens (we record the full
    # ledger for accounting honesty).
    assert out.tokens_used == 175


@pytest.mark.skipif(
    not os.environ.get("ANTHROPIC_API_KEY"),
    reason="ANTHROPIC_API_KEY not set; skipping live API smoke",
)
def test_anthropic_generate_smoke_against_live_api() -> None:
    """One real round-trip against the API. Asserts the provider
    produces a ``ProviderOutput`` with ``tokens_used > 0`` and either
    a tool call or final text. Hits the network — skipped if the env
    var is not set."""
    _require_anthropic_sdk()
    provider = AnthropicProvider(model=DEFAULT_MODEL_ID)
    out = provider.generate(
        [
            {"role": "developer", "content": "Reply with a single word."},
            {"role": "user", "content": "Say 'hello'."},
        ],
        [],
        temperature=0.0,
        max_new_tokens=32,
        seed=0,
    )
    assert out.tokens_used > 0
    assert out.tool_calls is not None or out.final_text is not None
