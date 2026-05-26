"""Shared FunctionGemma response-envelope parser.

The FG chat template emits tool calls as
``<start_function_call>call:NAME{k:<escape>v<escape>, …}<end_function_call>``
in the model's generated tokens. Two providers consume this shape:

  * ``LlamaCppProvider`` — the rev-10 client-side fallback for
    llama-server builds without an FG response parser.
  * ``TransformersProvider`` — direct HF generate() against an SFT
    checkpoint. The FG jinja is applied via
    ``tokenizer.apply_chat_template(messages, tools=tools, …)``, so the
    trained model emits the same envelope shape.

Mirrors vLLM's ``FunctionGemmaToolParser``. Single regex set, single
coercion path — train- and inference-time can't drift on what
``call:NAME{…}`` means.

Pinned directly by ``tests/test_fg_envelope.py``; both providers also
carry a drift-prevention pin asserting they reference
``parse_fg_envelopes`` from this module rather than a local copy.
"""

from __future__ import annotations

import json
import re
from typing import Any

# Envelope regex — captures the call name and the body between
# ``call:NAME{`` and the trailing ``}<end_function_call>``. The body
# group is non-greedy so multiple envelopes in one generation each
# match independently. We anchor the trailing brace on
# ``\}\s*<end_function_call>`` so an inner JSON dict (which itself
# contains ``}`` chars) doesn't terminate the body early.
_FG_ENVELOPE_RE = re.compile(
    r"<start_function_call>\s*call:(\w+)\s*\{(.*?)\}\s*<end_function_call>",
    re.DOTALL,
)
# FG-DSL string-arg form: ``key:<escape>value<escape>``. Used by the
# stock pretrained FunctionGemma-270M (v7 baseline at 26 % L3 against
# llamacpp).
_FG_STRING_ARG_RE = re.compile(r"(\w+):<escape>(.*?)<escape>", re.DOTALL)
# FG-DSL bare-arg form: ``key:scalar``.
_FG_BARE_ARG_RE = re.compile(r"(\w+):([^,}]+)")


def coerce_fg_scalar(raw: str) -> Any:
    """Coerce a bare (unquoted) FunctionGemma arg value to bool/int/float
    or leave as string. ``true``/``false`` → bool; otherwise try int,
    then float, else strip and return the original string.
    """
    s = raw.strip()
    if s == "true":
        return True
    if s == "false":
        return False
    try:
        return int(s)
    except ValueError:
        pass
    try:
        return float(s)
    except ValueError:
        pass
    return s


def _parse_body_args(body: str) -> dict[str, Any]:
    """Parse one call envelope's body into ``{key: value}``.

    The body can take one of two shapes, both observed in production:

    1. **JSON-literal form** (the v1 SFT corpus's accidental shape).
       Stage 5 rollouts wrote ``function.arguments`` as a JSON string
       (``'{"root": "cylinder"}'``); the FG chat template's
       string-arguments branch then inserts the literal text inside
       the call braces, producing ``call:NAME{<whitespace><JSON>}``.
       The v1 checkpoints learned to emit this shape.
    2. **FG-DSL form** (stock pretrained FunctionGemma-270M and the
       v7 llamacpp baseline). Key/value pairs are bare, with strings
       wrapped in ``<escape>…<escape>``.

    We try JSON first because it's unambiguous when present. If the
    body fails to JSON-parse to a dict, we fall through to FG-DSL
    parsing. An empty body returns ``{}``.

    `TODO(v2)`: re-render the training data with
    ``function.arguments`` as a dict so the chat template's
    mapping-branch emits the canonical FG-DSL; this dual-shape
    branch can then retire. The v1 checkpoints we've trained will
    still need this branch for grading, but new training runs would
    not. See `m5-sft-pipeline.md` rev 20+ TODO(v2) for the
    accidental-shape carry-over.
    """
    stripped = body.strip()
    if not stripped:
        return {}
    try:
        parsed = json.loads(stripped)
    except json.JSONDecodeError:
        parsed = None
    if isinstance(parsed, dict):
        return parsed

    args: dict[str, Any] = {}
    # Pull string args first; record names so the bare-scalar pass
    # doesn't double-count them.
    for sm in _FG_STRING_ARG_RE.finditer(body):
        args[sm.group(1)] = sm.group(2)
    # Mask out the string-arg spans (including their <escape>...<escape>
    # values) so the bare-scalar regex doesn't capture <escape> as a
    # bogus value.
    masked = _FG_STRING_ARG_RE.sub("", body)
    for bm in _FG_BARE_ARG_RE.finditer(masked):
        key = bm.group(1)
        if key in args:
            continue
        args[key] = coerce_fg_scalar(bm.group(2))
    return args


def parse_fg_envelopes(content: str) -> list[dict[str, Any]]:
    """Parse FunctionGemma ``<start_function_call>...<end_function_call>``
    envelopes out of a generated string and return one canonical
    ``{"name", "arguments": dict}`` per envelope, in source order.

    Two body shapes are handled (see ``_parse_body_args``):

      * **JSON-literal**: ``call:NAME{ {"k": "v"} }`` (v1 SFT
        checkpoints).
      * **FG-DSL**: ``call:NAME{k:<escape>v<escape>}`` (stock
        FunctionGemma-270M / v7 llamacpp baseline).

    Returns ``[]`` when ``content`` contains no recognizable
    envelope; callers treat that as "no tool calls" and either fall
    back to ``final_text`` or route a synthetic parse_error.
    """
    out: list[dict[str, Any]] = []
    for env in _FG_ENVELOPE_RE.finditer(content):
        name = env.group(1)
        body = env.group(2)
        out.append({"name": name, "arguments": _parse_body_args(body)})
    return out


__all__ = ["coerce_fg_scalar", "parse_fg_envelopes"]
