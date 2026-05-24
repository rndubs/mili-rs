"""W5 — ``AnthropicProvider`` (frontier baseline + future teacher); see
``planning/mili-viz/agent-local-llm-baseline.md`` §W5.

The frontier-ceiling provider for the v0 baseline report; the same
provider doubles as the teacher for the Stage-5 teacher-rollout loop
that ``posttraining-dataset.md`` reaches once the v0 number is in hand.

The v0 frontier baseline is taken against the exact model id pinned by
``DEFAULT_MODEL_ID`` — bumping it invalidates the published number and
the new run is a deliberate rebaseline. Override via the CLI's
``--anthropic-model`` flag if comparing model variants.

Cost note (LOAD-BEARING — see baseline.md §"Anthropic prompt
caching"). The frontier baseline runs ~50 scenarios × ~3 turns ≈ 150
calls against the same constant system prompt + 18-tool inventory.
Without ``cache_control`` on the system block and the tools list, every
turn re-bills the (small but recurring) prompt+tools input as fresh
tokens — a ~10x cost multiplier on the recurring frontier-baseline
run. This module sets ``cache_control: {"type": "ephemeral"}`` on the
system block and on the final tool entry (Anthropic caches everything
up to and including a marker). The unit test
``test_providers_anthropic.py::test_prompt_caching_is_set_on_system_and_tools``
asserts the marker is on the request body so this can't regress
silently.

Heavy deps lazy-imported. Reading ``ANTHROPIC_API_KEY`` at call time
(not import time) means the bench imports cleanly without the key set.
"""

from __future__ import annotations

import json
import os
from dataclasses import dataclass, field
from typing import Any

from .base import ProviderOutput

# v0 frontier-baseline pin. The README / report.md headline records
# this id so the number is reproducible.
DEFAULT_MODEL_ID = "claude-sonnet-4-5"

# Pinned Anthropic API version; the SDK ships its own default, but
# pinning here makes the request-body shape deterministic across SDK
# upgrades. (The SDK exposes this only as an http header; the SDK
# carries the wiring — we pass the model id and trust the rest.)


def _import_anthropic() -> Any:
    try:
        import anthropic  # type: ignore[import-not-found]
    except ImportError as exc:  # pragma: no cover — exercised on the user's box.
        raise ImportError(
            "AnthropicProvider requires the 'anthropic' optional dependency. "
            "Install with `pip install mili-llm-bench[anthropic]`."
        ) from exc
    return anthropic


# ---------------------------------------------------------------------------
# Message + tool conversion.
# ---------------------------------------------------------------------------


_ANTHROPIC_UNSUPPORTED_SCHEMA_KEYS = ("oneOf", "allOf", "anyOf")


def _strip_top_level_combinators(schema: dict[str, Any]) -> dict[str, Any]:
    """Drop top-level ``oneOf``/``allOf``/``anyOf`` from a JSON schema.

    The Anthropic tool API rejects these at the schema root (the `view`
    tool uses a top-level ``oneOf`` to enforce "exactly one of rotate /
    translate / … / reset"). The constraint is still enforced bench-side
    by jsonschema during dispatch, so dropping it for the wire payload
    only loosens what Anthropic *validates*, not what we *execute*.
    """
    if not any(k in schema for k in _ANTHROPIC_UNSUPPORTED_SCHEMA_KEYS):
        return schema
    return {k: v for k, v in schema.items() if k not in _ANTHROPIC_UNSUPPORTED_SCHEMA_KEYS}


def _convert_tools(tools: list[dict[str, Any]]) -> list[dict[str, Any]]:
    """Our W1 tools.json shape ``{name, description, input_schema, ...}``
    → Anthropic's tool shape ``{name, description, input_schema}``.

    The last tool carries ``cache_control: {"type": "ephemeral"}`` so
    Anthropic caches everything up through the tools block — critical
    for the recurring frontier-baseline cost (see module docstring).
    """
    out: list[dict[str, Any]] = []
    for t in tools:
        out.append(
            {
                "name": t["name"],
                "description": t.get("description", ""),
                "input_schema": _strip_top_level_combinators(t["input_schema"]),
            }
        )
    if out:
        # Anthropic caches everything UP TO AND INCLUDING the last
        # block that carries cache_control. The tools list precedes
        # the messages in the request body, so marking the last tool
        # caches the entire tools array.
        out[-1] = {**out[-1], "cache_control": {"type": "ephemeral"}}
    return out


def _split_system_and_messages(
    messages: list[dict[str, Any]],
) -> tuple[list[dict[str, Any]], list[dict[str, Any]]]:
    """Pull the leading ``developer`` / ``system`` messages off the top
    of the transcript into Anthropic's ``system`` parameter (a list of
    text blocks); the rest of the transcript stays in ``messages``.

    The driver always sends ``[{"role": "developer", ...}, {"role":
    "user", ...}, ...]``. Anthropic doesn't speak ``developer`` — we
    coerce it to a top-level ``system`` text block.
    """
    system_blocks: list[dict[str, Any]] = []
    rest: list[dict[str, Any]] = []
    in_prefix = True
    for msg in messages:
        role = msg.get("role")
        content = msg.get("content", "")
        if in_prefix and role in ("developer", "system"):
            # Driver-stop synthetic ``system`` messages ("stop:...") are
            # an internal verifier convention; we leave them in the
            # transcript on the message side too so the API sees the
            # full state. But the leading developer prompt is the only
            # one we promote to the top-level system parameter.
            if role == "system" and isinstance(content, str) and content.startswith(
                "stop:"
            ):
                in_prefix = False
                rest.append(msg)
                continue
            system_blocks.append({"type": "text", "text": content if isinstance(content, str) else json.dumps(content)})
            continue
        in_prefix = False
        rest.append(msg)
    # Mark the last system block with cache_control so Anthropic
    # caches the system prompt for every subsequent turn in this
    # rollout AND across rollouts (cache key includes the block).
    if system_blocks:
        system_blocks[-1] = {**system_blocks[-1], "cache_control": {"type": "ephemeral"}}
    return system_blocks, rest


def _convert_messages(messages: list[dict[str, Any]]) -> list[dict[str, Any]]:
    """Our canonical transcript → Anthropic's ``messages`` array.

    Roles map:

    * ``user`` → ``user`` with a single text block.
    * ``assistant`` text → ``assistant`` with a single text block.
    * ``assistant`` ``tool_calls`` → ``assistant`` with one ``tool_use``
      block per slot.
    * ``tool`` → ``user`` with one ``tool_result`` block (Anthropic
      routes tool results through a user message, not a tool message).
    * ``system`` (driver stops) → drop. They are an internal verifier
      convention; surfacing them to the model would be noise.
    """
    out: list[dict[str, Any]] = []

    # We need to fold consecutive ``tool`` messages into one user
    # message carrying a list of tool_result blocks (Anthropic's
    # one-message-per-batch convention). Iterate and buffer.
    pending_tool_results: list[dict[str, Any]] = []

    def flush_tool_results() -> None:
        if pending_tool_results:
            out.append({"role": "user", "content": list(pending_tool_results)})
            pending_tool_results.clear()

    for msg in messages:
        role = msg.get("role")
        if role == "system":
            # Internal driver-stop marker (`stop:...`); skip.
            continue
        if role == "tool":
            content = msg.get("content", "")
            pending_tool_results.append(
                {
                    "type": "tool_result",
                    "tool_use_id": msg.get("tool_call_id", ""),
                    "content": content if isinstance(content, str) else json.dumps(content),
                }
            )
            continue
        flush_tool_results()
        if role == "user":
            text = msg.get("content", "")
            out.append({"role": "user", "content": text if isinstance(text, str) else json.dumps(text)})
            continue
        if role == "assistant":
            tcs = msg.get("tool_calls") or []
            if tcs:
                blocks: list[dict[str, Any]] = []
                for tc in tcs:
                    fn = tc.get("function") or {}
                    raw_args = fn.get("arguments", "{}")
                    if isinstance(raw_args, str):
                        try:
                            parsed = json.loads(raw_args)
                        except json.JSONDecodeError:
                            parsed = {}
                    else:
                        parsed = raw_args if isinstance(raw_args, dict) else {}
                    blocks.append(
                        {
                            "type": "tool_use",
                            "id": tc.get("id", ""),
                            "name": fn.get("name", ""),
                            "input": parsed,
                        }
                    )
                out.append({"role": "assistant", "content": blocks})
            else:
                text = msg.get("content", "")
                out.append(
                    {
                        "role": "assistant",
                        "content": text if isinstance(text, str) else json.dumps(text),
                    }
                )
            continue
        # Unknown role — drop silently rather than 400-ing the API.
    flush_tool_results()
    return out


def _parse_response(response: Any) -> ProviderOutput:
    """Anthropic ``Message`` → ``ProviderOutput``.

    Walks ``response.content`` (a list of typed blocks): every
    ``tool_use`` block becomes one canonical tool-call entry; every
    ``text`` block contributes to ``final_text`` (joined with spaces).
    A message that carries both is normalized to ``tool_calls`` (the
    typical Anthropic shape when the model chains a thought + a call).
    """
    tool_calls: list[dict[str, Any]] = []
    texts: list[str] = []
    for block in getattr(response, "content", []) or []:
        btype = getattr(block, "type", None) or (
            block.get("type") if isinstance(block, dict) else None
        )
        if btype == "tool_use":
            name = getattr(block, "name", None) or (
                block.get("name") if isinstance(block, dict) else ""
            )
            inp = getattr(block, "input", None)
            if inp is None and isinstance(block, dict):
                inp = block.get("input")
            if not isinstance(inp, dict):
                inp = {}
            tool_calls.append({"name": str(name or ""), "arguments": inp})
        elif btype == "text":
            text = getattr(block, "text", None) or (
                block.get("text") if isinstance(block, dict) else ""
            )
            if isinstance(text, str):
                texts.append(text)

    usage = getattr(response, "usage", None)
    tokens_used = 0
    if usage is not None:
        tokens_used = int(
            getattr(usage, "input_tokens", 0)
            + getattr(usage, "output_tokens", 0)
            + getattr(usage, "cache_read_input_tokens", 0)
            + getattr(usage, "cache_creation_input_tokens", 0)
        )

    if tool_calls:
        return ProviderOutput(
            tool_calls=tool_calls,
            tokens_used=tokens_used,
            raw=response,
        )
    return ProviderOutput(
        final_text=" ".join(texts).strip(),
        tokens_used=tokens_used,
        raw=response,
    )


# ---------------------------------------------------------------------------
# Provider.
# ---------------------------------------------------------------------------


@dataclass
class AnthropicProvider:
    """Frontier-baseline provider for the v0 report.

    ``ANTHROPIC_API_KEY`` is read at *call* time, not import time —
    the bench imports cleanly on a box without the key set, and tests
    that don't exercise the network path don't need the env var.

    ``client`` is an explicit override for unit tests; production
    callers leave it ``None`` and the provider builds its own
    ``anthropic.Anthropic()`` on the first ``generate``.
    """

    model: str = DEFAULT_MODEL_ID
    max_new_tokens_default: int = 1024
    client: Any = field(default=None, repr=False)

    def _ensure_client(self) -> Any:
        if self.client is not None:
            return self.client
        anthropic = _import_anthropic()
        api_key = os.environ.get("ANTHROPIC_API_KEY")
        if not api_key:
            raise RuntimeError(
                "ANTHROPIC_API_KEY environment variable is not set; "
                "AnthropicProvider cannot make API calls without it."
            )
        self.client = anthropic.Anthropic(api_key=api_key)
        return self.client

    def generate(
        self,
        messages: list[dict[str, Any]],
        tools: list[dict[str, Any]],
        *,
        temperature: float,
        max_new_tokens: int,
        seed: int,
    ) -> ProviderOutput:
        client = self._ensure_client()
        system_blocks, anth_messages = _split_system_and_messages(messages)
        anth_tools = _convert_tools(tools)

        request: dict[str, Any] = {
            "model": self.model,
            "max_tokens": max_new_tokens or self.max_new_tokens_default,
            "temperature": temperature,
            "messages": _convert_messages(anth_messages),
        }
        if system_blocks:
            request["system"] = system_blocks
        if anth_tools:
            request["tools"] = anth_tools
        # ``seed`` is intentionally not forwarded — the Anthropic API
        # does not expose a seed parameter; ``temperature=0`` is the
        # determinism lever the eval driver pins by default.

        response = client.messages.create(**request)
        return _parse_response(response)


__all__ = [
    "DEFAULT_MODEL_ID",
    "AnthropicProvider",
    "_convert_messages",
    "_convert_tools",
    "_parse_response",
    "_split_system_and_messages",
]
