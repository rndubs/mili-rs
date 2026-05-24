"""W3 — verifier (L0..L3 + failure-mode taxonomy); see
``planning/mili-viz/agent-local-llm-baseline.md`` §W3.

Reusable by v0 *and* by the future post-training pipeline
(``posttraining-dataset.md`` Stage 4) — operates entirely on a
pre-recorded ``messages`` list + a scenario ``postcondition``, no live
pygriz session. Dispatch is the W4a harness's job; W3 only grades.

Two-column L0..L3 table from baseline.md §W3:

============  ========================================  ========================================
Tier          Typed tool call                          ``griz_raw``
============  ========================================  ========================================
L0            output parses as ``{name, arguments}``    inner ``line`` parses (Stage-1 grammar)
L1            name known + arguments matches schema     ``parse_command`` accepts
L2            dispatch returns ``ok=true``              raw runs without error
L3            post-condition met                        same
============  ========================================  ========================================

``griz_raw``'s L1 grammar check is a future Stage-1 deliverable
(``planning/mili-viz/mili-agent/posttraining-dataset.md`` Stage 1); for v0 the
check degenerates to "``arguments.line`` is a string". See the
``# TODO(stage-1)`` comment on ``_grade_call``.

Failure-mode taxonomy (closed set; the W4a harness ``error_kind`` enum
imports these strings so the two stay in sync):

* parse / schema (L0/L1): ``parse_error``, ``unknown_tool``,
  ``schema_mismatch``
* dispatch + argument-level (L2): ``dispatch_error``,
  ``nonexistent_material``, ``nonexistent_class``,
  ``nonexistent_result``, ``state_out_of_range``
* post-condition (L3): ``wrong_final_state``, ``wrong_selection``,
  ``wrong_result``, ``wrong_range``, ``wrong_materials``
* driver-level: ``step_cap_hit``, ``token_cap_hit``, ``timeout``
"""

from __future__ import annotations

import json
import math
from dataclasses import dataclass
from typing import Any, Callable

import jsonschema

from .scenarios import VALID_POSTCONDITION_KINDS
from .schemas import default_artifact_path

# ---------------------------------------------------------------------------
# Closed taxonomies (the harness ``error_kind`` enum imports from here).
# ---------------------------------------------------------------------------

FAILURE_MODES: tuple[str, ...] = (
    # Parse / schema (L0/L1)
    "parse_error",
    "unknown_tool",
    "schema_mismatch",
    # Dispatch + argument-level (L2)
    "dispatch_error",
    "nonexistent_material",
    "nonexistent_class",
    "nonexistent_result",
    "state_out_of_range",
    # Post-condition (L3)
    "wrong_final_state",
    "wrong_selection",
    "wrong_result",
    "wrong_range",
    "wrong_materials",
    # Driver-level
    "step_cap_hit",
    "token_cap_hit",
    "timeout",
)

DRIVER_LEVEL_STOPS: frozenset[str] = frozenset(
    {"step_cap_hit", "token_cap_hit", "timeout"}
)

# Subset of FAILURE_MODES that map to argument-level L2 misses.
_L2_ARG_FAILS: frozenset[str] = frozenset(
    {
        "dispatch_error",
        "nonexistent_material",
        "nonexistent_class",
        "nonexistent_result",
        "state_out_of_range",
    }
)

# Per-kind L3 miss labels.
_WRONG_BY_KIND: dict[str, str] = {
    "state_index": "wrong_final_state",
    "selection_set": "wrong_selection",
    "active_result": "wrong_result",
    "result_range": "wrong_range",
    "materials_visible": "wrong_materials",
    "camera_named_view": "wrong_final_state",
    "query_value": "wrong_final_state",
}


@dataclass(frozen=True)
class VerifierResult:
    max_tier: int  # 0..3
    reward: float  # max_tier / 3.0
    failure_mode: str | None


# ---------------------------------------------------------------------------
# Tool registry (loaded from the W1 artifact for schema validation).
# ---------------------------------------------------------------------------


_TOOL_REGISTRY_CACHE: dict[str, dict[str, Any]] | None = None


def _load_tool_registry() -> dict[str, dict[str, Any]]:
    """Load the W1 ``tools.json`` artifact, keyed by tool name."""
    global _TOOL_REGISTRY_CACHE
    if _TOOL_REGISTRY_CACHE is None:
        path = default_artifact_path()
        raw = json.loads(path.read_text())
        _TOOL_REGISTRY_CACHE = {t["name"]: t for t in raw}
    return _TOOL_REGISTRY_CACHE


# ---------------------------------------------------------------------------
# Message walking — extract tool-call attempts and their tool responses.
# ---------------------------------------------------------------------------


@dataclass
class ExtractedCall:
    name: str
    arguments_str: Any
    arguments: dict[str, Any] | None  # None ⇔ parse failed
    response: dict[str, Any] | None  # None ⇔ no tool-message reply
    tool_call_id: str | None


def _extract_calls(messages: list[dict[str, Any]]) -> list[ExtractedCall]:
    tool_msgs_by_id: dict[str | None, dict[str, Any]] = {}
    for msg in messages:
        if msg.get("role") == "tool":
            tool_msgs_by_id[msg.get("tool_call_id")] = msg

    calls: list[ExtractedCall] = []
    for msg in messages:
        if msg.get("role") != "assistant":
            continue
        for tc in msg.get("tool_calls") or []:
            fn = tc.get("function") or {}
            name = fn.get("name", "")
            args_raw = fn.get("arguments", "")
            tc_id = tc.get("id")
            args: dict[str, Any] | None
            try:
                parsed = (
                    json.loads(args_raw) if isinstance(args_raw, str) else args_raw
                )
                args = parsed if isinstance(parsed, dict) else None
            except (json.JSONDecodeError, TypeError):
                args = None
            tool_msg = tool_msgs_by_id.get(tc_id)
            response: dict[str, Any] | None = None
            if tool_msg is not None:
                content = tool_msg.get("content", "")
                try:
                    parsed_resp = (
                        json.loads(content) if isinstance(content, str) else content
                    )
                    if isinstance(parsed_resp, dict):
                        response = parsed_resp
                except json.JSONDecodeError:
                    response = None
            calls.append(
                ExtractedCall(
                    name=name,
                    arguments_str=args_raw,
                    arguments=args,
                    response=response,
                    tool_call_id=tc_id,
                )
            )
    return calls


def _detect_driver_stop(messages: list[dict[str, Any]]) -> str | None:
    """Driver-level stop reasons ride on a synthetic ``system`` message
    of the form ``"stop:<reason>"`` — the convention W4a will adopt.
    """
    for msg in messages:
        if msg.get("role") != "system":
            continue
        content = msg.get("content", "")
        if not isinstance(content, str) or not content.startswith("stop:"):
            continue
        reason = content[len("stop:") :].strip()
        if reason in DRIVER_LEVEL_STOPS:
            return reason
    return None


# ---------------------------------------------------------------------------
# Per-call grading (L0..L2).
# ---------------------------------------------------------------------------


def _coerce_arguments(
    arguments: dict[str, Any], schema: dict[str, Any]
) -> dict[str, Any]:
    """Attempt to coerce argument types to match the schema (e.g., string to int).

    Returns a new dict with coerced values. If coercion fails, returns original.
    """
    coerced = dict(arguments)
    properties = schema.get("properties", {})

    for key, value in arguments.items():
        if key not in properties:
            continue
        prop_schema = properties[key]
        expected_type = prop_schema.get("type")

        # String to int coercion
        if expected_type == "integer" and isinstance(value, str):
            try:
                coerced[key] = int(value)
            except (ValueError, TypeError):
                pass  # Keep original if coercion fails
        # String to number coercion
        elif expected_type == "number" and isinstance(value, str):
            try:
                coerced[key] = float(value)
            except (ValueError, TypeError):
                pass
        # String to boolean coercion
        elif expected_type == "boolean" and isinstance(value, str):
            if value.lower() in ("true", "1", "yes"):
                coerced[key] = True
            elif value.lower() in ("false", "0", "no"):
                coerced[key] = False

    return coerced


def _grade_call(
    call: ExtractedCall, tools: dict[str, dict[str, Any]]
) -> tuple[int, str | None]:
    """Returns (tier_reached, failure_mode_if_didnt_advance)."""
    if call.arguments is None:
        return 0, "parse_error"
    if call.name not in tools:
        return 0, "unknown_tool"
    # griz_raw — bespoke L1 check (Stage-1 grammar deferred).
    if call.name == "griz_raw":
        # TODO(stage-1): grade arguments.line against the Stage-1
        # griz GBNF artifact (planning/mili-viz/mili-agent/posttraining-dataset.md
        # Stage 1). v0 degenerates to "is a non-empty string".
        line = call.arguments.get("line")
        if not isinstance(line, str):
            return 0, "schema_mismatch"
    else:
        schema = tools[call.name]["input_schema"]
        coerced = _coerce_arguments(call.arguments, schema)
        try:
            jsonschema.validate(instance=coerced, schema=schema)
        except jsonschema.ValidationError:
            return 0, "schema_mismatch"
    # L1 reached.
    if call.response is None:
        return 1, "dispatch_error"
    if not call.response.get("ok", False):
        return 1, _classify_dispatch_fail(call.response)
    # L2 reached.
    return 2, None


def _classify_dispatch_fail(response: dict[str, Any]) -> str:
    """The harness tags structured dispatch failures with
    ``response["error_kind"]``; fall back to ``dispatch_error``."""
    ek = response.get("error_kind")
    if isinstance(ek, str) and ek in _L2_ARG_FAILS:
        return ek
    return "dispatch_error"


# ---------------------------------------------------------------------------
# Post-condition handlers (one per closed kind).
# ---------------------------------------------------------------------------


def _final_state(calls: list[ExtractedCall]) -> int | None:
    """Walk successful state-affecting calls; return the final 1-based
    state index, or None if no call established one."""
    state: int | None = None
    for c in calls:
        if c.response is None or not c.response.get("ok", False):
            continue
        if c.name == "load":
            state = 1  # post-load convention; the response itself omits state.
        elif c.name in ("set_state", "step"):
            s = c.response.get("state")
            if isinstance(s, int):
                state = s
    return state


def _final_selection(calls: list[ExtractedCall]) -> dict[str, str]:
    sel: dict[str, str] = {}
    for c in calls:
        if c.response is None or not c.response.get("ok", False):
            continue
        if c.name in ("select", "clrsel"):
            resp_sel = c.response.get("selection", {})
            if isinstance(resp_sel, dict):
                # The projected response carries the *new* selection
                # in full; replace, don't merge.
                sel = {k: v for k, v in resp_sel.items() if v}
    return sel


def _final_show(calls: list[ExtractedCall]) -> dict[str, Any] | None:
    last: dict[str, Any] | None = None
    for c in calls:
        if c.name != "show":
            continue
        if c.response is None or not c.response.get("ok", False):
            continue
        last = c.response
    return last


def _final_hidden_materials(calls: list[ExtractedCall]) -> set[int]:
    hidden: set[int] = set()
    for c in calls:
        if c.name != "material":
            continue
        if c.response is None or not c.response.get("ok", False):
            continue
        raw = c.response.get("hidden_materials", [])
        if isinstance(raw, list):
            hidden = {int(x) for x in raw}
    return hidden


def _pc_state_index(
    expect: dict[str, Any], calls: list[ExtractedCall]
) -> tuple[bool, str | None]:
    want = expect.get("state")
    got = _final_state(calls)
    if got is not None and got == want:
        return True, None
    return False, "wrong_final_state"


def _pc_selection_set(
    expect: dict[str, Any], calls: list[ExtractedCall]
) -> tuple[bool, str | None]:
    want = expect.get("selection", {})
    got = _final_selection(calls)
    if got == want:
        return True, None
    return False, "wrong_selection"


def _pc_active_result(
    expect: dict[str, Any], calls: list[ExtractedCall]
) -> tuple[bool, str | None]:
    want = expect.get("result")
    last = _final_show(calls)
    if last is not None and last.get("result") == want:
        return True, None
    return False, "wrong_result"


def _pc_result_range(
    expect: dict[str, Any], calls: list[ExtractedCall]
) -> tuple[bool, str | None]:
    want = expect.get("range")
    last = _final_show(calls)
    if last is None or not isinstance(want, list) or len(want) != 2:
        return False, "wrong_range"
    got = last.get("range")
    if not isinstance(got, list) or len(got) != 2:
        return False, "wrong_range"
    tol = float(expect.get("tol", 1e-6))
    if math.isclose(got[0], want[0], abs_tol=tol) and math.isclose(
        got[1], want[1], abs_tol=tol
    ):
        return True, None
    return False, "wrong_range"


def _pc_materials_visible(
    expect: dict[str, Any], calls: list[ExtractedCall]
) -> tuple[bool, str | None]:
    want = set(int(x) for x in expect.get("hidden_materials", []))
    got = _final_hidden_materials(calls)
    if got == want:
        return True, None
    return False, "wrong_materials"


def _pc_camera_named_view(
    expect: dict[str, Any], calls: list[ExtractedCall]
) -> tuple[bool, str | None]:
    """Verify the right camera/colormap action was applied.

    ``expect.action`` is one of:

    * ``"reset"`` — ``view`` tool called with ``reset=True``;
    * ``"save" | "restore" | "list"`` — ``named_view`` tool called
      with the matching op (and ``name`` when applicable);
    * ``"colormap"`` — ``colormap`` tool called with the named ramp.

    The ``camera_named_view`` kind is the catch-all for visual-setting
    grading; the closed set has no per-tool kind, and the W3 verifier
    is meant to stay closed.
    """
    action = expect.get("action")
    if action == "reset":
        for c in calls:
            if c.name != "view":
                continue
            if c.response is None or not c.response.get("ok", False):
                continue
            args = c.arguments or {}
            if args.get("reset") is True:
                return True, None
        return False, "wrong_final_state"
    if action == "colormap":
        want_name = expect.get("name")
        for c in calls:
            if c.name != "colormap":
                continue
            if c.response is None or not c.response.get("ok", False):
                continue
            args = c.arguments or {}
            if want_name is None or args.get("name") == want_name:
                return True, None
        return False, "wrong_final_state"
    if action in ("save", "restore", "list"):
        want_op = action.upper()
        want_name = expect.get("name")
        for c in calls:
            if c.name != "named_view":
                continue
            if c.response is None or not c.response.get("ok", False):
                continue
            args = c.arguments or {}
            if args.get("op") != want_op:
                continue
            if want_name is not None and args.get("name") != want_name:
                continue
            return True, None
        return False, "wrong_final_state"
    return False, "wrong_final_state"


def _pc_query_value(
    expect: dict[str, Any], calls: list[ExtractedCall]
) -> tuple[bool, str | None]:
    """Verify a ``query`` tool call returned ``expect.table``
    (compared via JSON equality of the projected ``table`` field)."""
    want_table = expect.get("table")
    want_args = expect.get("arguments")  # optional filter
    for c in calls:
        if c.name != "query":
            continue
        if c.response is None or not c.response.get("ok", False):
            continue
        if want_args is not None and c.arguments != want_args:
            continue
        if want_table is None or c.response.get("table") == want_table:
            return True, None
    return False, "wrong_final_state"


_PC_HANDLERS: dict[
    str,
    Callable[[dict[str, Any], list[ExtractedCall]], tuple[bool, str | None]],
] = {
    "state_index": _pc_state_index,
    "selection_set": _pc_selection_set,
    "active_result": _pc_active_result,
    "result_range": _pc_result_range,
    "materials_visible": _pc_materials_visible,
    "camera_named_view": _pc_camera_named_view,
    "query_value": _pc_query_value,
}


# ---------------------------------------------------------------------------
# Public entry point.
# ---------------------------------------------------------------------------


def verify(
    messages: list[dict[str, Any]],
    postcondition: dict[str, Any],
) -> VerifierResult:
    """Grade a recorded rollout against a scenario post-condition.

    Pure-logic; no live session, no LLM. Returns the highest tier
    reached (0..3) plus a closed failure-mode label disambiguating
    where the rollout stopped advancing. ``reward`` is the linear
    ``max_tier / 3``; weighting is left to the consumer.
    """
    kind = postcondition.get("kind")
    if kind not in VALID_POSTCONDITION_KINDS:
        raise ValueError(
            f"unknown postcondition kind {kind!r}; "
            f"expected one of {sorted(VALID_POSTCONDITION_KINDS)}"
        )
    expect = postcondition.get("expect", {})
    if not isinstance(expect, dict):
        raise ValueError("postcondition.expect must be a dict")

    driver_stop = _detect_driver_stop(messages)
    calls = _extract_calls(messages)
    tools = _load_tool_registry()

    if not calls:
        # No tool-call attempt at all. Driver-level stop wins if set;
        # otherwise this is a parse_error (model emitted text, never a
        # callable tool slot) — the L0 default.
        failure_mode = driver_stop or "parse_error"
        return VerifierResult(max_tier=0, reward=0.0, failure_mode=failure_mode)

    per_call: list[tuple[int, str | None]] = [_grade_call(c, tools) for c in calls]
    tiers = [t for t, _ in per_call]
    max_call_tier = max(tiers)

    pc_ok = False
    pc_fail: str | None = None
    if max_call_tier >= 2:
        handler = _PC_HANDLERS[kind]
        pc_ok, pc_fail = handler(expect, calls)

    if pc_ok and driver_stop is None:
        return VerifierResult(max_tier=3, reward=1.0, failure_mode=None)

    # Did not reach L3.
    final_tier = min(max_call_tier, 2)
    if driver_stop is not None:
        # Driver-level stop dominates the failure label; the tier
        # itself still reports what was achieved before the cap.
        failure_mode = driver_stop
    elif final_tier == 2:
        failure_mode = pc_fail or _WRONG_BY_KIND.get(kind, "wrong_final_state")
    elif final_tier == 1:
        failure_mode = _first_label(per_call, target_tier=1)
    else:
        failure_mode = _first_label(per_call, target_tier=0)
    return VerifierResult(
        max_tier=final_tier, reward=final_tier / 3.0, failure_mode=failure_mode
    )


def _first_label(
    per_call: list[tuple[int, str | None]], *, target_tier: int
) -> str:
    """Pick the most informative failure label across calls.

    Prefers a non-``dispatch_error`` argument-level label (``nonexistent_*``,
    ``state_out_of_range``) over the generic ``dispatch_error`` when at
    L1, and ``unknown_tool`` / ``schema_mismatch`` over ``parse_error``
    at L0 — so a row that *parsed* one slot but tripped over an unknown
    tool reports ``unknown_tool``, not ``parse_error``.
    """
    candidates = [
        label for tier, label in per_call if label is not None and tier == target_tier
    ]
    if not candidates:
        candidates = [label for _, label in per_call if label is not None]
    # Preference order for L1 / L2.
    pref_l1 = (
        "nonexistent_material",
        "nonexistent_class",
        "nonexistent_result",
        "state_out_of_range",
        "dispatch_error",
    )
    pref_l0 = ("unknown_tool", "schema_mismatch", "parse_error")
    if target_tier == 1:
        for p in pref_l1:
            if p in candidates:
                return p
    else:
        for p in pref_l0:
            if p in candidates:
                return p
    return candidates[0] if candidates else "parse_error"
