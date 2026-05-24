"""W4a — agent harness (the factored core).

One module that owns the "tool-call JSON → real session mutation →
projected response JSON" translation. Provider-agnostic and
session-agnostic by construction; consumed by three downstream
loops (baseline.md §W4a "Harness contract"):

* W4b's eval driver (next PR),
* the future ``posttraining-dataset.md`` Stage 5 teacher rollouts,
* the eventual server-side ``AgentChat`` handler in
  ``crates/mili-viz-server/src/agent.rs``.

The harness imports **no** pygriz / transformers / anthropic / GPU
stack — those plug in behind two seams:

* ``Dispatcher`` Protocol — typed Commands → live state. Lives here;
  the pygriz adapter (the only file in the package that imports
  ``pygriz``) lives in ``dispatchers/pygriz.py``.
* ``LlmProvider`` Protocol — model generate. Lives in
  ``providers/base.py``; v0 ships ``MockLlmProvider`` and
  ``ReplayLlmProvider`` only — FunctionGemma / Anthropic land in PR-5
  behind the same Protocol.

Three harness invariants (baseline.md §W1) are pinned at the
projection seam: ``Snapshot.loaded.state_times``,
``GeometryRef.flight_ticket``, and ``Snapshot.agent`` are stripped
*anywhere* in the response tree before it reaches the model. The
output schemas in W1's ``tools.json`` already ban these fields; the
defensive belt in ``_project_response`` is the load-bearing belt that
catches a misbehaving dispatcher.

``error_kind`` enum identity is pinned by the W3 verifier import:
``HARNESS_ERROR_KINDS is verifier.FAILURE_MODES`` (the same tuple).
Drift between the W3 grading taxonomy and the W4a emit taxonomy is
exactly the bug the enum-identity test catches.
"""

from __future__ import annotations

import json
import time
from dataclasses import dataclass, field
from pathlib import Path
from typing import Any, Callable, Protocol

import jsonschema

from .providers.base import LlmProvider
from .schemas import default_artifact_path
from .verifier import FAILURE_MODES, _coerce_arguments

# ---------------------------------------------------------------------------
# Closed taxonomy — one source of truth (the W3 verifier).
# ---------------------------------------------------------------------------

# Re-export the FAILURE_MODES tuple under a harness-side alias. The
# enum-identity test asserts the two are the same object.
HARNESS_ERROR_KINDS: tuple[str, ...] = FAILURE_MODES

# Synthetic name used for the parse-error recovery slot (option (b),
# baseline.md §W4a). Distinct from any real tool name so the verifier
# can identify the recovery turn unambiguously.
PARSE_ERROR_TOOL_NAME = "<parse_error>"

# Three forbidden field names — pre-enforced anywhere in the projected
# response tree (baseline.md §W1 "Harness invariants").
_FORBIDDEN_FIELDS: frozenset[str] = frozenset(
    {"state_times", "flight_ticket", "agent"}
)

# ---------------------------------------------------------------------------
# Protocols + dataclasses.
# ---------------------------------------------------------------------------


class Dispatcher(Protocol):
    """Lowers one tool call to a side-effecting session mutation and
    returns the **already-projected, harness-invariant-safe** response
    dict the model will see.

    Implementations live outside the harness; ``dispatchers/pygriz.py``
    is the production lowering (lazy-imports ``pygriz`` so this base
    module stays GPU-free / server-free / pygriz-free).
    """

    def dispatch(self, name: str, arguments: dict[str, Any]) -> dict[str, Any]:
        ...


@dataclass
class Registry:
    """Tool registry loaded from W1's ``tools.json`` artifact.

    Schema-only (no dispatch table here — dispatch lives behind the
    ``Dispatcher`` Protocol). The artifact is the serialized form of
    everything the model is shown.
    """

    tools: dict[str, dict[str, Any]] = field(default_factory=dict)

    @classmethod
    def load_from_artifact(cls, path: Path | None = None) -> "Registry":
        p = path or default_artifact_path()
        entries = json.loads(Path(p).read_text())
        return cls(tools={t["name"]: t for t in entries})

    def has(self, name: str) -> bool:
        return name in self.tools

    def input_schema(self, name: str) -> dict[str, Any]:
        return self.tools[name]["input_schema"]

    def output_schema(self, name: str) -> dict[str, Any]:
        return self.tools[name]["output_schema"]

    def all(self) -> list[dict[str, Any]]:
        return list(self.tools.values())


@dataclass
class ExecutedCall:
    """One dispatched (or attempted) tool slot in a turn."""

    name: str
    arguments: dict[str, Any]
    response: dict[str, Any]
    error_kind: str | None  # None on L2+ success
    dispatch_ms: int


@dataclass
class TurnResult:
    """One turn's outcome.

    ``kind == "tool_calls"``: the model emitted at least one tool call;
    ``tool_calls`` is the list of dispatched (and possibly
    parse_error / unknown_tool / schema_mismatch / dispatch_error)
    slots, in declared order. ``messages`` was mutated in place with
    one ``assistant`` message + N ``tool`` messages (canonical
    OpenAI/Anthropic shape — matches W3's verifier reader).

    ``kind == "final_text"``: the model emitted a non-tool-call text
    completion. ``final_text`` carries it; ``messages`` was mutated
    with one ``assistant`` ``content`` message.

    ``kind == "error"``: the harness itself bailed (currently only
    ``timeout``). ``error_kind`` carries the closed-taxonomy reason.
    Driver-level stops (``step_cap_hit`` / ``token_cap_hit``) are
    *not* emitted by the harness — the driver appends those itself
    via the synthetic ``"stop:<reason>"`` system message convention
    W3 reads (see ``verifier._detect_driver_stop``).
    """

    kind: str
    tool_calls: list[ExecutedCall] = field(default_factory=list)
    final_text: str | None = None
    error_kind: str | None = None
    tokens_used: int = 0
    wall_ms: int = 0


# ---------------------------------------------------------------------------
# Response projection — the defensive belt for the W1 harness invariants.
# ---------------------------------------------------------------------------


def _strip_forbidden(node: Any) -> Any:
    """Recursively remove ``state_times`` / ``flight_ticket`` / ``agent``
    *anywhere* in the response tree.

    The dispatcher is responsible for the live projection (it has the
    session state); this belt is the load-bearing pin that catches a
    regressed dispatcher before forbidden bytes ever reach the model.
    """
    if isinstance(node, dict):
        return {
            k: _strip_forbidden(v)
            for k, v in node.items()
            if k not in _FORBIDDEN_FIELDS
        }
    if isinstance(node, list):
        return [_strip_forbidden(v) for v in node]
    if isinstance(node, tuple):
        return tuple(_strip_forbidden(v) for v in node)
    return node


def _project_response(response: Any) -> dict[str, Any]:
    """Run the dispatcher's response through the harness invariant belt.

    Always returns a dict; if the dispatcher misbehaves and returns a
    non-dict, we wrap it into a ``dispatch_error``.
    """
    if not isinstance(response, dict):
        return {
            "ok": False,
            "error": f"dispatcher returned non-dict {type(response).__name__}",
            "error_kind": "dispatch_error",
        }
    return _strip_forbidden(response)  # type: ignore[no-any-return]


# ---------------------------------------------------------------------------
# Call validation + per-slot dispatch.
# ---------------------------------------------------------------------------


def _is_canonical_call(call: Any) -> bool:
    if not isinstance(call, dict):
        return False
    return isinstance(call.get("name"), str) and isinstance(
        call.get("arguments"), dict
    )


def _classify_dispatch_error_kind(response: dict[str, Any]) -> str:
    """A dispatcher tagging its own structured failure wins; otherwise
    fall back to the generic ``dispatch_error``.

    The accepted tag set is the L2 argument-level slice of the closed
    taxonomy (the W3 verifier's ``_L2_ARG_FAILS``). Anything else is
    treated as ``dispatch_error`` — keeps the harness emit taxonomy
    closed.
    """
    ek = response.get("error_kind")
    if not isinstance(ek, str):
        return "dispatch_error"
    if ek in HARNESS_ERROR_KINDS:
        return ek
    return "dispatch_error"


def _make_assistant_tool_call(
    call_id: str, name: str, arguments: dict[str, Any]
) -> dict[str, Any]:
    """The OpenAI/Anthropic-shape tool_call dict W3's verifier reads."""
    return {
        "id": call_id,
        "type": "function",
        "function": {"name": name, "arguments": json.dumps(arguments)},
    }


def _make_tool_message(
    call_id: str, name: str, response: dict[str, Any]
) -> dict[str, Any]:
    return {
        "role": "tool",
        "tool_call_id": call_id,
        "name": name,
        "content": json.dumps(response),
    }


def _dispatch_one(
    call: Any,
    call_id: str,
    dispatcher: Dispatcher,
    registry: Registry,
) -> tuple[ExecutedCall, dict[str, Any], dict[str, Any]]:
    """Process one tool-call slot end-to-end.

    Returns ``(ExecutedCall, assistant_tool_call_dict, tool_message_dict)``.
    All three are *always* produced — even on parse / unknown / schema
    / dispatch failure — so the model sees a faithful echo of its own
    attempt plus a structured error response on the next turn (the
    option (b) recovery loop pinned in baseline.md §W4a).
    """
    if not _is_canonical_call(call):
        err = {
            "ok": False,
            "error": "tool call is not canonical {name: str, arguments: dict}",
            "error_kind": "parse_error",
        }
        return (
            ExecutedCall(
                name=PARSE_ERROR_TOOL_NAME,
                arguments={},
                response=err,
                error_kind="parse_error",
                dispatch_ms=0,
            ),
            _make_assistant_tool_call(call_id, PARSE_ERROR_TOOL_NAME, {}),
            _make_tool_message(call_id, PARSE_ERROR_TOOL_NAME, err),
        )

    name: str = call["name"]
    arguments: dict[str, Any] = call["arguments"]
    asst = _make_assistant_tool_call(call_id, name, arguments)

    if not registry.has(name):
        err = {
            "ok": False,
            "error": f"unknown tool {name!r}",
            "error_kind": "unknown_tool",
        }
        return (
            ExecutedCall(
                name=name,
                arguments=arguments,
                response=err,
                error_kind="unknown_tool",
                dispatch_ms=0,
            ),
            asst,
            _make_tool_message(call_id, name, err),
        )

    coerced_args = _coerce_arguments(arguments, registry.input_schema(name))
    try:
        jsonschema.validate(instance=coerced_args, schema=registry.input_schema(name))
    except jsonschema.ValidationError as exc:
        err = {
            "ok": False,
            "error": f"schema mismatch: {exc.message}",
            "error_kind": "schema_mismatch",
        }
        return (
            ExecutedCall(
                name=name,
                arguments=arguments,
                response=err,
                error_kind="schema_mismatch",
                dispatch_ms=0,
            ),
            asst,
            _make_tool_message(call_id, name, err),
        )

    dispatch_start = time.monotonic()
    try:
        raw = dispatcher.dispatch(name, coerced_args)
    except Exception as exc:  # adapter exceptions also tagged.
        raw = {
            "ok": False,
            "error": str(exc),
            "error_kind": "dispatch_error",
        }
    dispatch_ms = int((time.monotonic() - dispatch_start) * 1000)

    projected = _project_response(raw)

    error_kind: str | None = None
    if not projected.get("ok", False):
        error_kind = _classify_dispatch_error_kind(projected)

    return (
        ExecutedCall(
            name=name,
            arguments=arguments,
            response=projected,
            error_kind=error_kind,
            dispatch_ms=dispatch_ms,
        ),
        asst,
        _make_tool_message(call_id, name, projected),
    )


# ---------------------------------------------------------------------------
# Public surface — exactly baseline.md §W4a.
# ---------------------------------------------------------------------------


def run_turn(
    provider: LlmProvider,
    dispatcher: Dispatcher,
    messages: list[dict[str, Any]],
    tools: list[dict[str, Any]],
    *,
    step_index: int,
    max_new_tokens: int = 256,
    temperature: float = 0.0,
    seed: int = 0,
    timeout_s: float = 60.0,
    registry: Registry | None = None,
) -> TurnResult:
    """Execute one turn: call the provider, dispatch its tool calls
    (or surface its final text), append the resulting messages in
    place.

    Mutates ``messages``: on a tool-call turn, appends one
    ``assistant`` message carrying *all* N ``tool_calls`` followed by
    N ``tool`` messages, one per call slot in declared order — the
    shape the W3 verifier already reads.

    Per-turn budget: wall-clock ``timeout_s`` is checked after the
    provider call and after each dispatch slot. On hit, returns
    ``TurnResult(kind="error", error_kind="timeout")`` and does
    **not** mutate ``messages`` further (the driver may record the
    cap via a synthetic ``"stop:timeout"`` system message; the
    harness stays neutral on driver bookkeeping). Token / step caps
    are driver-level — ``token_cap_hit`` / ``step_cap_hit`` are
    reserved in the closed enum but never emitted from here.
    """
    reg = registry if registry is not None else Registry.load_from_artifact()

    start = time.monotonic()

    try:
        output = provider.generate(
            messages,
            tools,
            temperature=temperature,
            max_new_tokens=max_new_tokens,
            seed=seed,
        )
    except Exception:
        # A clean LlmProvider should not raise; this is the safety
        # belt. Fold into the closed taxonomy so the verifier still
        # sees a closed-set label.
        return TurnResult(
            kind="error",
            error_kind="dispatch_error",
            wall_ms=int((time.monotonic() - start) * 1000),
        )

    elapsed = time.monotonic() - start
    if elapsed > timeout_s:
        return TurnResult(
            kind="error",
            error_kind="timeout",
            tokens_used=getattr(output, "tokens_used", 0),
            wall_ms=int(elapsed * 1000),
        )

    tool_calls = output.tool_calls if output.tool_calls else []
    has_text = isinstance(output.final_text, str)

    # No tool calls and a clean text completion → "final_text" arm.
    if not tool_calls and has_text:
        messages.append({"role": "assistant", "content": output.final_text})
        return TurnResult(
            kind="final_text",
            final_text=output.final_text,
            tokens_used=output.tokens_used,
            wall_ms=int((time.monotonic() - start) * 1000),
        )

    # No tool calls and no text → a malformed emission. Recover via
    # the same parse_error path a malformed call slot would take, so
    # the driver sees a uniform shape and the model can self-correct.
    if not tool_calls and not has_text:
        call_id = f"call_{step_index}_0"
        err = {
            "ok": False,
            "error": "provider returned neither tool_calls nor final_text",
            "error_kind": "parse_error",
        }
        executed = ExecutedCall(
            name=PARSE_ERROR_TOOL_NAME,
            arguments={},
            response=err,
            error_kind="parse_error",
            dispatch_ms=0,
        )
        messages.append(
            {
                "role": "assistant",
                "tool_calls": [
                    _make_assistant_tool_call(call_id, PARSE_ERROR_TOOL_NAME, {})
                ],
            }
        )
        messages.append(_make_tool_message(call_id, PARSE_ERROR_TOOL_NAME, err))
        return TurnResult(
            kind="tool_calls",
            tool_calls=[executed],
            tokens_used=output.tokens_used,
            wall_ms=int((time.monotonic() - start) * 1000),
        )

    executed_calls: list[ExecutedCall] = []
    assistant_tool_calls: list[dict[str, Any]] = []
    tool_messages: list[dict[str, Any]] = []

    for idx, call in enumerate(tool_calls):
        call_id = f"call_{step_index}_{idx}"
        ec, asst, tm = _dispatch_one(call, call_id, dispatcher, reg)
        executed_calls.append(ec)
        assistant_tool_calls.append(asst)
        tool_messages.append(tm)

        # Re-check wall clock between dispatches so a slow dispatcher
        # cannot run past the budget unbounded.
        if (time.monotonic() - start) > timeout_s:
            return TurnResult(
                kind="error",
                error_kind="timeout",
                tool_calls=executed_calls,
                tokens_used=output.tokens_used,
                wall_ms=int((time.monotonic() - start) * 1000),
            )

    messages.append({"role": "assistant", "tool_calls": assistant_tool_calls})
    messages.extend(tool_messages)

    return TurnResult(
        kind="tool_calls",
        tool_calls=executed_calls,
        tokens_used=output.tokens_used,
        wall_ms=int((time.monotonic() - start) * 1000),
    )


# ---------------------------------------------------------------------------
# A test-friendly dispatcher (no pygriz dep). Real production lowering
# lives in ``dispatchers/pygriz.py``.
# ---------------------------------------------------------------------------


@dataclass
class FakeDispatcher:
    """Test dispatcher: a per-tool ``handlers`` table and/or a fallback
    callable ``handler``.

    A handler that returns ``{"ok": false, "error_kind": <one of
    HARNESS_ERROR_KINDS>, ...}`` lets the test exercise the harness's
    failure-classification path without standing up a real pygriz
    adapter (which classifies via best-effort string match on the
    pygriz error message).
    """

    handler: Callable[[str, dict[str, Any]], dict[str, Any]] | None = None
    handlers: dict[str, Callable[[str, dict[str, Any]], dict[str, Any]]] = field(
        default_factory=dict
    )
    default_response: dict[str, Any] = field(default_factory=lambda: {"ok": True})

    def dispatch(self, name: str, arguments: dict[str, Any]) -> dict[str, Any]:
        if name in self.handlers:
            return self.handlers[name](name, arguments)
        if self.handler is not None:
            return self.handler(name, arguments)
        return dict(self.default_response)
