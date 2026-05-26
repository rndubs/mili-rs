"""Shared W1 ↔ FunctionGemma/OpenAI tool-format conversion.

Stage 6 (``mili_llm_bench.assemble``) and the llamacpp inference path
(``providers.llamacpp.LlamaCppProvider``) both need to project the
canonical W1 tool entry shape — ``{name, description, input_schema,
output_schema}`` from ``data/posttraining/grammar/tools.json`` — into
the FunctionGemma/OpenAI training-and-inference shape ``{"type":
"function", "function": {"name", "description", "parameters"}}``.

Centralizing the conversion here pins the contract: a train-time
record's ``tools`` block matches what llama-server (with ``--jinja``)
sees at inference time byte-for-byte, modulo dict-key ordering. The
``output_schema`` field is dropped intentionally — FunctionGemma's
training format has no slot for it; the dispatcher enforces output
shape server-side, not on the wire.

See ``planning/mili-viz/mili-agent/posttraining-dataset.md`` §Stage 6
"Tools-array format conversion".
"""

from __future__ import annotations

from typing import Any


def w1_to_openai_tool(tool: dict[str, Any]) -> dict[str, Any]:
    """Convert one W1 tool entry to FunctionGemma/OpenAI tool format.

    Input shape (W1, from ``tools.json``):
        ``{"name", "description", "input_schema", "output_schema"}``
    Output shape (FG/OpenAI training + inference):
        ``{"type": "function", "function": {"name", "description",
        "parameters"}}``

    ``output_schema`` is dropped — FG's training format has no slot
    for it. Missing ``description`` / ``input_schema`` default to ``""``
    / ``{}`` so a malformed registry entry still produces a wire-valid
    OpenAI tool entry.
    """
    return {
        "type": "function",
        "function": {
            "name": tool.get("name", ""),
            "description": tool.get("description", ""),
            "parameters": tool.get("input_schema", {}),
        },
    }


def w1_tools_to_openai(tools: list[dict[str, Any]]) -> list[dict[str, Any]]:
    """Vectorized ``w1_to_openai_tool`` over a list."""
    return [w1_to_openai_tool(t) for t in tools]


__all__ = ["w1_to_openai_tool", "w1_tools_to_openai"]
