"""W2 x W3 contract test.

For every scenario in the checked-in ``bootstrap.jsonl``, fabricate the
canonical "perfect rollout" (the tool-call sequence that should
satisfy ``postcondition``) and assert
``verifier.verify(perfect_rollout, scenario.postcondition).max_tier == 3``.

Drift in either direction fails CI:

* scenario file adds a kind the verifier does not handle -> contract test
  fails;
* verifier changes per-kind grading semantics -> contract test fails on
  the scenarios whose perfect rollout was matching the old logic.

Fabricated rollouts only - no LLM, no pygriz, no live session.
"""

from __future__ import annotations

import json
from typing import Any

from mili_llm_bench.scenarios import Scenario, default_bootstrap_path, load_scenarios
from mili_llm_bench.verifier import verify

# Fixture-fact table, pulled from the parity suite (see
# reference/mili-python/tests/test_miliinternal.py and the live probe in
# crates/mili-rs's parity_corpus_coverage.rs).  Used only to ground the
# fabricated load-response shape; verifier does not require these
# values to grade, but keeping them honest doubles as a sanity pin.
_FIXTURE_FACTS: dict[str, dict[str, Any]] = {
    "d3samp6": {
        "num_states": 101,
        "classes": ["glob", "mat", "node", "beam", "brick", "shell", "cseg"],
    },
    "cylinder": {
        "num_states": 11,
        "classes": ["glob", "mat", "lcurve", "node", "brick"],
    },
}


def _call(call_id: str, name: str, arguments: dict[str, Any]) -> dict[str, Any]:
    return {
        "id": call_id,
        "type": "function",
        "function": {"name": name, "arguments": json.dumps(arguments)},
    }


def _tool(call_id: str, name: str, response: dict[str, Any]) -> dict[str, Any]:
    return {
        "role": "tool",
        "tool_call_id": call_id,
        "name": name,
        "content": json.dumps(response),
    }


def _load_msgs(fixture: str, call_id: str) -> list[dict[str, Any]]:
    facts = _FIXTURE_FACTS[fixture]
    return [
        {"role": "assistant", "tool_calls": [_call(call_id, "load", {"root": fixture})]},
        _tool(
            call_id,
            "load",
            {
                "ok": True,
                "num_states": facts["num_states"],
                "num_classes": len(facts["classes"]),
                "classes": facts["classes"],
                "state_time_range": [0.0, 1.0],
                "current_time": 0.0,
            },
        ),
    ]


def _perfect_rollout(scenario: Scenario) -> list[dict[str, Any]]:
    """Build the canonical tool-call sequence that should hit L3 for this
    scenario. One arm per intent_id; postcondition.expect carries the
    target values the fabrication echoes back as tool responses.
    """
    msgs: list[dict[str, Any]] = [{"role": "user", "content": scenario.instruction}]
    msgs.extend(_load_msgs(scenario.fixture, "load-1"))

    intent = scenario.intent_id
    expect = scenario.postcondition.expect
    facts = _FIXTURE_FACTS[scenario.fixture]

    if intent == "load":
        # The load above already satisfies state_index=1.
        return msgs

    if intent == "set-state":
        target = expect["state"]
        msgs += [
            {
                "role": "assistant",
                "tool_calls": [_call("set-1", "set_state", {"state": target})],
            },
            _tool(
                "set-1",
                "set_state",
                {"ok": True, "state": target, "num_states": facts["num_states"]},
            ),
        ]
        return msgs

    if intent == "step":
        target = expect["state"]
        if target == facts["num_states"]:
            direction = "LAST"
        elif target == 1:
            direction = "FIRST"
        elif target == 2:
            direction = "NEXT"
        else:
            direction = "NEXT"
        msgs += [
            {
                "role": "assistant",
                "tool_calls": [_call("step-1", "step", {"dir": direction})],
            },
            _tool(
                "step-1",
                "step",
                {"ok": True, "state": target, "num_states": facts["num_states"]},
            ),
        ]
        return msgs

    if intent == "select":
        sel = expect["selection"]
        ((class_name, range_str),) = sel.items()
        msgs += [
            {
                "role": "assistant",
                "tool_calls": [
                    _call(
                        "sel-1",
                        "select",
                        {"class_name": class_name, "range": range_str},
                    )
                ],
            },
            _tool(
                "sel-1",
                "select",
                {"ok": True, "selection": {class_name: range_str}},
            ),
        ]
        return msgs

    if intent == "clrsel":
        # Pre-select something so clrsel has a meaningful before/after.
        msgs += [
            {
                "role": "assistant",
                "tool_calls": [
                    _call(
                        "pre-sel",
                        "select",
                        {"class_name": "brick", "range": "1"},
                    )
                ],
            },
            _tool(
                "pre-sel",
                "select",
                {"ok": True, "selection": {"brick": "1"}},
            ),
            {
                "role": "assistant",
                "tool_calls": [_call("clr-1", "clrsel", {"class_name": ""})],
            },
            _tool("clr-1", "clrsel", {"ok": True, "selection": {}}),
        ]
        return msgs

    if intent in ("show-primal", "show-derived"):
        result = expect["result"]
        msgs += [
            {
                "role": "assistant",
                "tool_calls": [_call("show-1", "show", {"result": result})],
            },
            _tool(
                "show-1",
                "show",
                {"ok": True, "result": result, "range": [0.0, 1.0]},
            ),
        ]
        return msgs

    if intent == "material":
        hidden = expect["hidden_materials"]
        if hidden:
            mid = hidden[0]
            msgs += [
                {
                    "role": "assistant",
                    "tool_calls": [
                        _call(
                            "mat-1",
                            "material",
                            {"enable": False, "material": mid},
                        )
                    ],
                },
                _tool(
                    "mat-1",
                    "material",
                    {"ok": True, "hidden_materials": list(hidden)},
                ),
            ]
        else:
            # Re-enable scenario: hide something first, then enable it.
            msgs += [
                {
                    "role": "assistant",
                    "tool_calls": [
                        _call(
                            "mat-hide",
                            "material",
                            {"enable": False, "material": 1},
                        )
                    ],
                },
                _tool(
                    "mat-hide",
                    "material",
                    {"ok": True, "hidden_materials": [1]},
                ),
                {
                    "role": "assistant",
                    "tool_calls": [
                        _call(
                            "mat-show",
                            "material",
                            {"enable": True, "material": 1},
                        )
                    ],
                },
                _tool(
                    "mat-show",
                    "material",
                    {"ok": True, "hidden_materials": []},
                ),
            ]
        return msgs

    if intent == "view-reset":
        msgs += [
            {
                "role": "assistant",
                "tool_calls": [_call("view-1", "view", {"reset": True})],
            },
            _tool("view-1", "view", {"ok": True}),
        ]
        return msgs

    if intent == "colormap":
        name = expect["name"]
        msgs += [
            {
                "role": "assistant",
                "tool_calls": [_call("cm-1", "colormap", {"name": name})],
            },
            _tool("cm-1", "colormap", {"ok": True}),
        ]
        return msgs

    if intent == "compound":
        # "disable material 3 and color the mesh by effective stress"
        # — two typed calls; the verifier only checks the active_result
        # post-condition but we exercise both legs to mirror what the
        # real driver will see.
        msgs += [
            {
                "role": "assistant",
                "tool_calls": [
                    _call(
                        "cmp-mat",
                        "material",
                        {"enable": False, "material": 3},
                    )
                ],
            },
            _tool(
                "cmp-mat",
                "material",
                {"ok": True, "hidden_materials": [3]},
            ),
            {
                "role": "assistant",
                "tool_calls": [
                    _call("cmp-show", "show", {"result": expect["result"]})
                ],
            },
            _tool(
                "cmp-show",
                "show",
                {"ok": True, "result": expect["result"], "range": [0.0, 1.0]},
            ),
        ]
        return msgs

    raise AssertionError(f"unknown intent_id {intent!r} in scenario {scenario.id}")


def test_every_scenario_has_a_perfect_rollout_at_L3() -> None:
    """The W2 x W3 contract pin: per scenario, the fabricated canonical
    sequence grades L3 (max_tier == 3, no failure_mode).  Drift in
    either scenarios.jsonl or verifier.py breaks this test loudly."""
    scenarios = load_scenarios(default_bootstrap_path())
    failures: list[tuple[str, str, int, str | None]] = []
    for s in scenarios:
        msgs = _perfect_rollout(s)
        result = verify(msgs, s.postcondition.to_json())
        if result.max_tier != 3 or result.failure_mode is not None:
            failures.append(
                (s.id, s.intent_id, result.max_tier, result.failure_mode)
            )
    assert not failures, (
        "scenarios whose canonical perfect rollout did not grade L3:\n  "
        + "\n  ".join(f"{sid} ({intent}): tier={t} mode={m!r}" for sid, intent, t, m in failures)
    )
