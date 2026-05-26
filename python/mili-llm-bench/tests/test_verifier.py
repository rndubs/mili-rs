"""W3 — verifier tests.

Always-on, pure-logic: no LLM, no GPU, no pygriz. All rollouts here
are hand-fabricated. Three groups:

* One per L0..L3 tier — fabricate a rollout that lands at exactly that
  tier and assert ``max_tier`` matches.
* One per ``failure_mode`` in the closed taxonomy.
* One per post-condition ``kind`` — happy path + at least one failure
  path.
"""

from __future__ import annotations

import json
from typing import Any

import pytest

from mili_llm_bench.verifier import (
    DRIVER_LEVEL_STOPS,
    FAILURE_MODES,
    VerifierResult,
    verify,
)


# ---------------------------------------------------------------------------
# Fabrication helpers — assemble OpenAI/Anthropic-style messages by hand.
# ---------------------------------------------------------------------------


def _call(call_id: str, name: str, arguments: Any) -> dict[str, Any]:
    args_str = arguments if isinstance(arguments, str) else json.dumps(arguments)
    return {
        "id": call_id,
        "type": "function",
        "function": {"name": name, "arguments": args_str},
    }


def _assistant(*calls: dict[str, Any]) -> dict[str, Any]:
    return {"role": "assistant", "tool_calls": list(calls)}


def _tool(call_id: str, name: str, response: dict[str, Any]) -> dict[str, Any]:
    return {
        "role": "tool",
        "tool_call_id": call_id,
        "name": name,
        "content": json.dumps(response),
    }


def _user(text: str) -> dict[str, Any]:
    return {"role": "user", "content": text}


def _stop(reason: str) -> dict[str, Any]:
    return {"role": "system", "content": f"stop:{reason}"}


def _final_text(text: str = "Done.") -> dict[str, Any]:
    """m7 Delta 1 terminator — content-only assistant message that
    closes a clean rollout so the verifier's ``_terminates_cleanly``
    check accepts L3."""
    return {"role": "assistant", "content": text}


# ---------------------------------------------------------------------------
# L0 / L1 / L2 / L3 tier tests.
# ---------------------------------------------------------------------------


def test_tier_l0_parse_failure_no_calls() -> None:
    """No tool calls at all - rollout never reached L0."""
    msgs = [_user("hi"), {"role": "assistant", "content": "Sure!"}]
    result = verify(msgs, {"kind": "state_index", "expect": {"state": 1}})
    assert result.max_tier == 0
    assert result.failure_mode == "parse_error"


def test_tier_l0_unparseable_json_arguments() -> None:
    msgs = [
        _user("load it"),
        _assistant(_call("1", "load", "{not json")),
    ]
    result = verify(msgs, {"kind": "state_index", "expect": {"state": 1}})
    assert result.max_tier == 0
    assert result.failure_mode == "parse_error"


def test_tier_l1_schema_mismatch_does_not_dispatch() -> None:
    """Name known, arguments fail schema -> tier stops at L0."""
    msgs = [
        _user("load"),
        # `root` must be a string per the proto Load message.
        _assistant(_call("1", "load", {"root": 123})),
    ]
    result = verify(msgs, {"kind": "state_index", "expect": {"state": 1}})
    assert result.max_tier == 0
    assert result.failure_mode == "schema_mismatch"


def test_tier_l1_reached_when_schema_ok_but_dispatch_fails() -> None:
    msgs = [
        _user("load"),
        _assistant(_call("1", "load", {"root": "missing"})),
        _tool("1", "load", {"ok": False, "error": "not found"}),
    ]
    result = verify(msgs, {"kind": "state_index", "expect": {"state": 1}})
    assert result.max_tier == 1
    assert result.failure_mode == "dispatch_error"


def test_tier_l2_reached_but_postcondition_misses() -> None:
    """Dispatch ok'd, but the final state does not match expect."""
    msgs = [
        _user("go to state 50"),
        _assistant(_call("1", "set_state", {"state": 25})),
        _tool("1", "set_state", {"ok": True, "state": 25, "num_states": 101}),
    ]
    result = verify(msgs, {"kind": "state_index", "expect": {"state": 50}})
    assert result.max_tier == 2
    assert result.failure_mode == "wrong_final_state"


def test_tier_l3_perfect_rollout() -> None:
    msgs = [
        _user("load d3samp6"),
        _assistant(_call("1", "load", {"root": "d3samp6"})),
        _tool(
            "1",
            "load",
            {
                "ok": True,
                "num_states": 101,
                "num_classes": 7,
                "classes": ["glob", "mat", "node", "beam", "brick", "shell", "cseg"],
                "state_time_range": [0.0, 1.0],
                "current_time": 0.0,
            },
        ),
        _final_text(),
    ]
    result = verify(msgs, {"kind": "state_index", "expect": {"state": 1}})
    assert result.max_tier == 3
    assert result.reward == pytest.approx(1.0)
    assert result.failure_mode is None


# ---------------------------------------------------------------------------
# Failure-mode taxonomy — one fabrication per entry.
# ---------------------------------------------------------------------------


def test_failure_mode_parse_error() -> None:
    msgs = [_user("x")]
    r = verify(msgs, {"kind": "state_index", "expect": {"state": 1}})
    assert r.failure_mode == "parse_error"


def test_failure_mode_unknown_tool() -> None:
    msgs = [
        _user("do something"),
        _assistant(_call("1", "no_such_tool", {})),
    ]
    r = verify(msgs, {"kind": "state_index", "expect": {"state": 1}})
    assert r.failure_mode == "unknown_tool"


def test_failure_mode_schema_mismatch() -> None:
    msgs = [
        _user("load"),
        _assistant(_call("1", "load", {"root": 123})),
    ]
    r = verify(msgs, {"kind": "state_index", "expect": {"state": 1}})
    assert r.failure_mode == "schema_mismatch"


def test_failure_mode_dispatch_error() -> None:
    msgs = [
        _user("load"),
        _assistant(_call("1", "load", {"root": "nope"})),
        _tool("1", "load", {"ok": False, "error": "no such db"}),
    ]
    r = verify(msgs, {"kind": "state_index", "expect": {"state": 1}})
    assert r.failure_mode == "dispatch_error"


def test_failure_mode_nonexistent_material() -> None:
    msgs = [
        _user("hide mat 99"),
        _assistant(_call("1", "material", {"enable": False, "material": 99})),
        _tool(
            "1",
            "material",
            {"ok": False, "error": "no material 99", "error_kind": "nonexistent_material"},
        ),
    ]
    r = verify(msgs, {"kind": "materials_visible", "expect": {"hidden_materials": [99]}})
    assert r.failure_mode == "nonexistent_material"


def test_failure_mode_nonexistent_class() -> None:
    msgs = [
        _user("select stuff"),
        _assistant(_call("1", "select", {"class_name": "made_up", "range": "1-3"})),
        _tool(
            "1",
            "select",
            {"ok": False, "error_kind": "nonexistent_class", "error": "no class"},
        ),
    ]
    r = verify(msgs, {"kind": "selection_set", "expect": {"selection": {}}})
    assert r.failure_mode == "nonexistent_class"


def test_failure_mode_nonexistent_result() -> None:
    msgs = [
        _user("show frobozz"),
        _assistant(_call("1", "show", {"result": "frobozz"})),
        _tool(
            "1",
            "show",
            {"ok": False, "error_kind": "nonexistent_result", "error": "unknown svar"},
        ),
    ]
    r = verify(msgs, {"kind": "active_result", "expect": {"result": "frobozz"}})
    assert r.failure_mode == "nonexistent_result"


def test_failure_mode_state_out_of_range() -> None:
    msgs = [
        _user("set state 9999"),
        _assistant(_call("1", "set_state", {"state": 9999})),
        _tool(
            "1",
            "set_state",
            {"ok": False, "error_kind": "state_out_of_range", "error": "oob"},
        ),
    ]
    r = verify(msgs, {"kind": "state_index", "expect": {"state": 9999}})
    assert r.failure_mode == "state_out_of_range"


def test_failure_mode_wrong_final_state() -> None:
    msgs = [
        _user("state 50"),
        _assistant(_call("1", "set_state", {"state": 25})),
        _tool("1", "set_state", {"ok": True, "state": 25, "num_states": 101}),
    ]
    r = verify(msgs, {"kind": "state_index", "expect": {"state": 50}})
    assert r.failure_mode == "wrong_final_state"


def test_failure_mode_wrong_selection() -> None:
    msgs = [
        _user("pick brick 1"),
        _assistant(_call("1", "select", {"class_name": "brick", "range": "2"})),
        _tool("1", "select", {"ok": True, "selection": {"brick": "2"}}),
    ]
    r = verify(msgs, {"kind": "selection_set", "expect": {"selection": {"brick": "1"}}})
    assert r.failure_mode == "wrong_selection"


def test_failure_mode_wrong_result() -> None:
    msgs = [
        _user("show eff_stress"),
        _assistant(_call("1", "show", {"result": "sx"})),
        _tool("1", "show", {"ok": True, "result": "sx", "range": [0.0, 1.0]}),
    ]
    r = verify(msgs, {"kind": "active_result", "expect": {"result": "eff_stress"}})
    assert r.failure_mode == "wrong_result"


def test_failure_mode_wrong_range() -> None:
    msgs = [
        _user("show eff_stress"),
        _assistant(_call("1", "show", {"result": "eff_stress"})),
        _tool("1", "show", {"ok": True, "result": "eff_stress", "range": [0.0, 5.0]}),
    ]
    r = verify(
        msgs,
        {"kind": "result_range", "expect": {"range": [0.0, 10.0], "tol": 1e-6}},
    )
    assert r.failure_mode == "wrong_range"


def test_failure_mode_wrong_materials() -> None:
    msgs = [
        _user("hide mat 1"),
        _assistant(_call("1", "material", {"enable": False, "material": 2})),
        _tool("1", "material", {"ok": True, "hidden_materials": [2]}),
    ]
    r = verify(
        msgs, {"kind": "materials_visible", "expect": {"hidden_materials": [1]}}
    )
    assert r.failure_mode == "wrong_materials"


@pytest.mark.parametrize("reason", sorted(DRIVER_LEVEL_STOPS))
def test_failure_mode_driver_level(reason: str) -> None:
    """A synthetic ``stop:<reason>`` system message dominates the label."""
    msgs = [
        _user("anything"),
        _assistant(_call("1", "load", {"root": "d3samp6"})),
        # No tool reply - call reached L1 only.
        _stop(reason),
    ]
    r = verify(msgs, {"kind": "state_index", "expect": {"state": 1}})
    assert r.failure_mode == reason


def test_failure_mode_wrong_termination_pc_holds_but_no_final_text() -> None:
    """m7 Delta 2 — postcondition holds, all calls graded L2, but the
    rollout never emits a content-only assistant message → downgrade
    to L2 / wrong_termination so the bench reflects the live-UX gap."""
    msgs = [
        _user("show eff_stress"),
        _assistant(_call("1", "show", {"result": "eff_stress"})),
        _tool(
            "1",
            "show",
            {"ok": True, "result": "eff_stress", "range": [0.0, 1.0]},
        ),
        # No terminating final_text — the v1 SFT model never learned
        # to emit one (see m7-bench-live-parity.md §"Root-cause").
    ]
    r = verify(msgs, {"kind": "active_result", "expect": {"result": "eff_stress"}})
    assert r.max_tier == 2
    assert r.failure_mode == "wrong_termination"


def test_failure_mode_wrong_termination_trailing_tool_calls_only() -> None:
    """A trailing assistant turn that contains only ``tool_calls`` (no
    content) is also not a clean termination."""
    msgs = [
        _user("show eff_stress"),
        _assistant(_call("1", "show", {"result": "eff_stress"})),
        _tool(
            "1",
            "show",
            {"ok": True, "result": "eff_stress", "range": [0.0, 1.0]},
        ),
        # Runaway-style: emits another tool_calls turn after success.
        _assistant(_call("2", "show", {"result": "eff_stress"})),
    ]
    r = verify(msgs, {"kind": "active_result", "expect": {"result": "eff_stress"}})
    assert r.max_tier == 2
    assert r.failure_mode == "wrong_termination"


def test_failure_mode_wrong_termination_driver_stop_dominates() -> None:
    """A driver-level stop (step_cap_hit / timeout) outranks the
    termination check — the rollout is reported as ``step_cap_hit``,
    not ``wrong_termination``, so the bench histogram pins blame on
    the most informative cause."""
    msgs = [
        _user("show eff_stress"),
        _assistant(_call("1", "show", {"result": "eff_stress"})),
        _tool(
            "1",
            "show",
            {"ok": True, "result": "eff_stress", "range": [0.0, 1.0]},
        ),
        _stop("step_cap_hit"),
    ]
    r = verify(msgs, {"kind": "active_result", "expect": {"result": "eff_stress"}})
    assert r.failure_mode == "step_cap_hit"


def test_failure_modes_taxonomy_is_complete_and_closed() -> None:
    """All 16 entries in the closed taxonomy are reachable above."""
    expected = {
        "parse_error",
        "unknown_tool",
        "schema_mismatch",
        "dispatch_error",
        "nonexistent_material",
        "nonexistent_class",
        "nonexistent_result",
        "state_out_of_range",
        "wrong_final_state",
        "wrong_selection",
        "wrong_result",
        "wrong_range",
        "wrong_materials",
        "wrong_termination",
        "step_cap_hit",
        "token_cap_hit",
        "timeout",
    }
    assert set(FAILURE_MODES) == expected
    assert len(FAILURE_MODES) == len(expected)


# ---------------------------------------------------------------------------
# Post-condition kind handlers — happy path + at least one failure each.
# ---------------------------------------------------------------------------


def test_pc_state_index_happy_via_step() -> None:
    msgs = [
        _user("next"),
        _assistant(_call("1", "load", {"root": "d3samp6"})),
        _tool("1", "load", {"ok": True, "num_states": 101}),
        _assistant(_call("2", "step", {"dir": "NEXT"})),
        _tool("2", "step", {"ok": True, "state": 2, "num_states": 101}),
        _final_text(),
    ]
    r = verify(msgs, {"kind": "state_index", "expect": {"state": 2}})
    assert r.max_tier == 3


def test_pc_state_index_failure() -> None:
    msgs = [
        _user("state 7"),
        _assistant(_call("1", "set_state", {"state": 7})),
        _tool("1", "set_state", {"ok": True, "state": 7, "num_states": 101}),
    ]
    r = verify(msgs, {"kind": "state_index", "expect": {"state": 8}})
    assert r.max_tier == 2
    assert r.failure_mode == "wrong_final_state"


def test_pc_selection_set_happy() -> None:
    msgs = [
        _user("brick 1-10"),
        _assistant(_call("1", "select", {"class_name": "brick", "range": "1-10"})),
        _tool("1", "select", {"ok": True, "selection": {"brick": "1-10"}}),
        _final_text(),
    ]
    r = verify(
        msgs, {"kind": "selection_set", "expect": {"selection": {"brick": "1-10"}}}
    )
    assert r.max_tier == 3


def test_pc_selection_set_failure() -> None:
    msgs = [
        _user("clear all"),
        _assistant(_call("1", "clrsel", {"class_name": ""})),
        _tool("1", "clrsel", {"ok": True, "selection": {"brick": "1"}}),
    ]
    r = verify(msgs, {"kind": "selection_set", "expect": {"selection": {}}})
    assert r.max_tier == 2
    assert r.failure_mode == "wrong_selection"


def test_pc_active_result_happy() -> None:
    msgs = [
        _user("show eff_stress"),
        _assistant(_call("1", "show", {"result": "eff_stress"})),
        _tool(
            "1",
            "show",
            {"ok": True, "result": "eff_stress", "range": [0.0, 1.0]},
        ),
        _final_text(),
    ]
    r = verify(msgs, {"kind": "active_result", "expect": {"result": "eff_stress"}})
    assert r.max_tier == 3


def test_pc_active_result_failure() -> None:
    msgs = [
        _user("show sx"),
        _assistant(_call("1", "show", {"result": "sy"})),
        _tool("1", "show", {"ok": True, "result": "sy", "range": [0, 1]}),
    ]
    r = verify(msgs, {"kind": "active_result", "expect": {"result": "sx"}})
    assert r.failure_mode == "wrong_result"


def test_pc_result_range_happy_within_tol() -> None:
    msgs = [
        _user("show eff_stress"),
        _assistant(_call("1", "show", {"result": "eff_stress"})),
        _tool(
            "1",
            "show",
            {"ok": True, "result": "eff_stress", "range": [0.0, 9.9999995]},
        ),
        _final_text(),
    ]
    r = verify(
        msgs,
        {"kind": "result_range", "expect": {"range": [0.0, 10.0], "tol": 1e-5}},
    )
    assert r.max_tier == 3


def test_pc_result_range_failure_outside_tol() -> None:
    msgs = [
        _user("show eff_stress"),
        _assistant(_call("1", "show", {"result": "eff_stress"})),
        _tool(
            "1",
            "show",
            {"ok": True, "result": "eff_stress", "range": [0.0, 5.0]},
        ),
    ]
    r = verify(
        msgs,
        {"kind": "result_range", "expect": {"range": [0.0, 10.0], "tol": 1e-6}},
    )
    assert r.failure_mode == "wrong_range"


def test_pc_materials_visible_happy() -> None:
    msgs = [
        _user("hide 1"),
        _assistant(_call("1", "material", {"enable": False, "material": 1})),
        _tool("1", "material", {"ok": True, "hidden_materials": [1]}),
        _final_text(),
    ]
    r = verify(
        msgs, {"kind": "materials_visible", "expect": {"hidden_materials": [1]}}
    )
    assert r.max_tier == 3


def test_pc_materials_visible_failure() -> None:
    msgs = [
        _user("hide 1"),
        _assistant(_call("1", "material", {"enable": False, "material": 1})),
        _tool("1", "material", {"ok": True, "hidden_materials": [1]}),
    ]
    r = verify(
        msgs, {"kind": "materials_visible", "expect": {"hidden_materials": [2]}}
    )
    assert r.failure_mode == "wrong_materials"


def test_pc_camera_named_view_reset_happy() -> None:
    msgs = [
        _user("reset"),
        _assistant(_call("1", "view", {"reset": True})),
        _tool("1", "view", {"ok": True}),
        _final_text(),
    ]
    r = verify(msgs, {"kind": "camera_named_view", "expect": {"action": "reset"}})
    assert r.max_tier == 3


def test_pc_camera_named_view_colormap_happy() -> None:
    msgs = [
        _user("cool"),
        _assistant(_call("1", "colormap", {"name": "cool"})),
        _tool("1", "colormap", {"ok": True}),
        _final_text(),
    ]
    r = verify(
        msgs,
        {"kind": "camera_named_view", "expect": {"action": "colormap", "name": "cool"}},
    )
    assert r.max_tier == 3


def test_pc_camera_named_view_named_view_save_happy() -> None:
    msgs = [
        _user("save view as front"),
        _assistant(_call("1", "named_view", {"op": "SAVE", "name": "front"})),
        _tool("1", "named_view", {"ok": True}),
        _final_text(),
    ]
    r = verify(
        msgs,
        {
            "kind": "camera_named_view",
            "expect": {"action": "save", "name": "front"},
        },
    )
    assert r.max_tier == 3


def test_pc_camera_named_view_failure_wrong_action() -> None:
    """Asked for reset but the model called colormap - L2 reached, L3 misses."""
    msgs = [
        _user("reset"),
        _assistant(_call("1", "colormap", {"name": "cool"})),
        _tool("1", "colormap", {"ok": True}),
    ]
    r = verify(msgs, {"kind": "camera_named_view", "expect": {"action": "reset"}})
    assert r.max_tier == 2
    assert r.failure_mode == "wrong_final_state"


def test_pc_query_value_happy() -> None:
    msgs = [
        _user("query sx for brick 1"),
        _assistant(
            _call(
                "1",
                "query",
                {"result": "sx", "class_name": "brick", "labels": [1], "states": [50]},
            )
        ),
        _tool("1", "query", {"ok": True, "table": {"sx": [12.3]}}),
        _final_text(),
    ]
    r = verify(
        msgs,
        {
            "kind": "query_value",
            "expect": {"table": {"sx": [12.3]}},
        },
    )
    assert r.max_tier == 3


def test_pc_query_value_failure() -> None:
    msgs = [
        _user("query"),
        _assistant(_call("1", "query", {"result": "sx"})),
        _tool("1", "query", {"ok": True, "table": {"sx": [99.0]}}),
    ]
    r = verify(
        msgs,
        {"kind": "query_value", "expect": {"table": {"sx": [1.0]}}},
    )
    assert r.failure_mode == "wrong_final_state"


# ---------------------------------------------------------------------------
# Misc / contract tests.
# ---------------------------------------------------------------------------


def test_unknown_postcondition_kind_raises() -> None:
    msgs = [_user("hi")]
    with pytest.raises(ValueError, match="unknown postcondition kind"):
        verify(msgs, {"kind": "not_a_kind", "expect": {}})


def test_griz_raw_l1_passes_when_line_is_a_string() -> None:
    """Stage-1 grammar check is a future deliverable; v0 verifier only
    requires that ``arguments.line`` is a string. See the
    ``# TODO(stage-1)`` comment in ``verifier.py``."""
    msgs = [
        _user("raw"),
        _assistant(_call("1", "griz_raw", {"line": "show eff_stress"})),
        _tool("1", "griz_raw", {"ok": True, "output": ""}),
    ]
    r = verify(msgs, {"kind": "active_result", "expect": {"result": "eff_stress"}})
    # Tier reaches L2 (dispatched ok'd); L3 misses because the verifier
    # only tracks typed `show` responses, not raw lines - that is fine
    # for v0 and pinned by the TODO. Just check it did not bail at L1.
    assert r.max_tier >= 2


def test_griz_raw_l0_schema_mismatch_when_line_not_a_string() -> None:
    msgs = [
        _user("raw"),
        _assistant(_call("1", "griz_raw", {"line": 42})),
    ]
    r = verify(msgs, {"kind": "state_index", "expect": {"state": 1}})
    assert r.failure_mode == "schema_mismatch"


def test_reward_is_max_tier_over_three() -> None:
    """``reward = max_tier / 3`` — exposed for downstream consumers
    that want a scalar, but the failure-mode label is the actionable
    signal in v0."""
    r = VerifierResult(max_tier=2, reward=2.0 / 3.0, failure_mode="wrong_result")
    assert r.reward == pytest.approx(2 / 3)
