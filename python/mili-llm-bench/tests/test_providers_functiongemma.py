"""W5 — FunctionGemmaProvider gated tests.

Skip-on-absent: ``transformers`` / ``torch`` / the model weights are
all optional. Runs behind the ``test-heavy`` CI job — the default
``pytest`` invocation skips both tests cleanly.

The pure-Python parsing helper (``_parse_tool_call_block``) is tested
always-on as a harness pin — it exercises the canonical
``<start_function_call>...<end_function_call>`` shape without loading
the model.
"""

from __future__ import annotations

import json
import os

import pytest

from mili_llm_bench.providers.functiongemma import _parse_tool_call_block


# ---------------------------------------------------------------------------
# Always-on: the parse helper round-trips canonical FunctionGemma output.
# ---------------------------------------------------------------------------


def test_parse_tool_call_block_extracts_single_call() -> None:
    text = (
        "<start_function_call>"
        + json.dumps({"name": "load", "arguments": {"root": "d3samp6"}})
        + "<end_function_call>"
    )
    calls = _parse_tool_call_block(text)
    assert calls == [{"name": "load", "arguments": {"root": "d3samp6"}}]


def test_parse_tool_call_block_extracts_list_of_calls() -> None:
    text = (
        "<start_function_call>"
        + json.dumps(
            [
                {"name": "load", "arguments": {"root": "x"}},
                {"name": "show", "arguments": {"result": "eff_stress"}},
            ]
        )
        + "<end_function_call>"
    )
    calls = _parse_tool_call_block(text)
    assert calls is not None
    assert len(calls) == 2
    assert calls[0]["name"] == "load"
    assert calls[1]["name"] == "show"


def test_parse_tool_call_block_returns_none_when_no_block() -> None:
    assert _parse_tool_call_block("Just final text, no call block.") is None


def test_parse_tool_call_block_returns_empty_list_on_bad_json() -> None:
    text = "<start_function_call>{not json}<end_function_call>"
    assert _parse_tool_call_block(text) == []


def test_parse_tool_call_block_normalizes_string_arguments() -> None:
    """Some tokenizer paths emit arguments as a JSON-encoded string; the
    helper normalizes to a dict so the harness sees one canonical shape."""
    inner = {
        "name": "set_state",
        "arguments": json.dumps({"state": 5}),
    }
    text = (
        "<start_function_call>" + json.dumps(inner) + "<end_function_call>"
    )
    calls = _parse_tool_call_block(text)
    assert calls == [{"name": "set_state", "arguments": {"state": 5}}]


# ---------------------------------------------------------------------------
# Skip-on-absent: the model smoke. Requires `transformers` + `torch` +
# the model weights (~270M model). The smoke confirms the chat-template
# path round-trips through to a ``ProviderOutput`` with either tool_calls
# or final_text populated.
# ---------------------------------------------------------------------------


@pytest.mark.skipif(
    os.environ.get("MILI_LLM_BENCH_RUN_FUNCTIONGEMMA_SMOKE") != "1",
    reason=(
        "FunctionGemma model load is heavy; set "
        "MILI_LLM_BENCH_RUN_FUNCTIONGEMMA_SMOKE=1 to run."
    ),
)
def test_functiongemma_smoke_loads_and_generates() -> None:
    pytest.importorskip("transformers")
    pytest.importorskip("torch")
    from mili_llm_bench.providers.functiongemma import FunctionGemmaProvider

    provider = FunctionGemmaProvider()
    out = provider.generate(
        [
            {"role": "developer", "content": "You are an assistant. Use tools."},
            {"role": "user", "content": "load d3samp6"},
        ],
        [
            {
                "name": "load",
                "description": "Load a Mili DB.",
                "input_schema": {
                    "type": "object",
                    "properties": {"root": {"type": "string"}},
                    "required": ["root"],
                },
            }
        ],
        temperature=0.0,
        max_new_tokens=64,
        seed=0,
    )
    # Either path is acceptable; the assertion is that the provider
    # returned a structured ProviderOutput (not raised).
    assert out is not None
    assert out.tool_calls is not None or out.final_text is not None
