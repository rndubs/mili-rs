"""W4a — agent harness tests.

Always-on (no LLM, no GPU, no pygriz). All paths exercised via
``MockLlmProvider`` / ``ReplayLlmProvider`` + ``FakeDispatcher``.

Groups (mirroring the PR-3 spec):

1. Harness invariants (per-field): no ``state_times`` /
   ``flight_ticket`` / ``agent`` ever reach the model.
2. N-tool-calls-per-turn dispatch ordering + message shape.
3. Parse-error feedback loop (option (b)).
4. ``error_kind`` enum identity vs ``verifier.FAILURE_MODES``.
5. ``ReplayLlmProvider`` round-trip on a fabricated rollout.
6. Per-turn wall-clock timeout.
7. Schema-mismatch and unknown-tool paths.
8. W2 × W3 × W4a contract: a perfect Mock script for ``bs-001``
   grades L3 through the live verifier.
"""

from __future__ import annotations

import json
from pathlib import Path
from typing import Any

import pytest

from mili_llm_bench import harness, verifier
from mili_llm_bench.harness import (
    FakeDispatcher,
    HARNESS_ERROR_KINDS,
    PARSE_ERROR_TOOL_NAME,
    Registry,
    TurnResult,
    _project_response,
    _strip_forbidden,
    run_turn,
)
from mili_llm_bench.providers import (
    MockLlmProvider,
    ProviderOutput,
    ReplayLlmProvider,
)
from mili_llm_bench.scenarios import default_bootstrap_path, load_scenarios


# Loaded once — both the always-on registry and the W3 contract test
# need it. ``Registry.load_from_artifact`` reads the pinned W1
# ``tools.json``.
_REGISTRY = Registry.load_from_artifact()
_TOOLS_LIST = _REGISTRY.all()


# ---------------------------------------------------------------------------
# 1. Harness invariants — the three forbidden fields never reach the model.
# ---------------------------------------------------------------------------


def _fabricated_snapshot_response() -> dict[str, Any]:
    """A snapshot-shaped dict carrying all three forbidden fields at
    multiple nesting levels — exercises the recursive belt."""
    return {
        "ok": True,
        "state": 5,
        "num_states": 101,
        "state_times": [0.0, 0.01, 0.02, 0.03],  # forbidden at top
        "loaded": {
            "num_states": 101,
            "state_times": list(range(101)),  # forbidden nested
            "classes": ["brick", "node"],
        },
        "result": {
            "result": "sx",
            "geometry": {
                "flight_ticket": b"opaque-bytes",  # forbidden in nested obj
            },
        },
        "geometry_list": [
            {"flight_ticket": "second-opaque"},  # forbidden inside list element
        ],
        "agent": {  # forbidden top-level — self-echo trap
            "transcript": [{"role": "assistant", "content": "hi"}],
        },
    }


def test_no_state_times_in_response() -> None:
    """``Snapshot.loaded.state_times`` is unbounded ``repeated double``
    and stripped anywhere in the tree."""
    raw = _fabricated_snapshot_response()
    projected = _project_response(raw)
    flat = json.dumps(projected)
    assert "state_times" not in flat, (
        "state_times leaked through the harness projection belt; "
        "this is the load-bearing pin from baseline.md §W1"
    )
    # The dispatcher's payload (sans forbidden fields) is preserved.
    assert projected["ok"] is True
    assert projected["state"] == 5
    assert projected["loaded"]["num_states"] == 101
    assert projected["loaded"]["classes"] == ["brick", "node"]


def test_no_flight_ticket_in_response() -> None:
    """``GeometryRef.flight_ticket`` is opaque ``bytes`` for Arrow Flight;
    base64'd it adds ~33% bloat and is useless to the LLM."""
    raw = _fabricated_snapshot_response()
    projected = _project_response(raw)
    flat = json.dumps(projected, default=repr)
    assert "flight_ticket" not in flat, (
        "flight_ticket leaked through the harness projection belt"
    )
    # The geometry list element is kept (sans forbidden field).
    assert projected["geometry_list"] == [{}]


def test_no_agent_in_response() -> None:
    """``Snapshot.agent`` is the agent's own transcript — echoing it
    back into the agent's tool-response context is a self-echo trap
    (quadratic context growth + likely model confusion)."""
    raw = _fabricated_snapshot_response()
    projected = _project_response(raw)
    flat = json.dumps(projected)
    assert "agent" not in flat, (
        "Snapshot.agent leaked through the harness projection belt"
    )


def test_strip_forbidden_walks_through_dispatch() -> None:
    """End-to-end: a dispatcher that ignores the projection contract
    cannot smuggle forbidden fields through to the model — they are
    stripped from the harness-projected ``response`` on the
    ``ExecutedCall`` AND from the appended ``tool`` message body."""
    bad_dispatcher = FakeDispatcher(handler=lambda n, a: _fabricated_snapshot_response())
    messages: list[dict[str, Any]] = []
    result = run_turn(
        MockLlmProvider([
            ProviderOutput(tool_calls=[{"name": "snapshot", "arguments": {}}])
        ]),
        bad_dispatcher,
        messages,
        _TOOLS_LIST,
        step_index=0,
        registry=_REGISTRY,
    )
    assert result.kind == "tool_calls"
    ec = result.tool_calls[0]
    for forbidden in ("state_times", "flight_ticket", "agent"):
        assert forbidden not in json.dumps(ec.response), (
            f"forbidden field {forbidden!r} reached ExecutedCall.response"
        )
        for msg in messages:
            assert forbidden not in json.dumps(msg, default=repr), (
                f"forbidden field {forbidden!r} reached appended messages"
            )


# ---------------------------------------------------------------------------
# 2. N tool calls per turn — dispatched in declared order; message shape
#    matches what W3's verifier already reads.
# ---------------------------------------------------------------------------


def test_n_tool_calls_per_turn_dispatched_in_order() -> None:
    seen: list[tuple[str, dict[str, Any]]] = []

    def handler(name: str, arguments: dict[str, Any]) -> dict[str, Any]:
        seen.append((name, dict(arguments)))
        if name == "material":
            return {"ok": True, "hidden_materials": [arguments["material"]]}
        if name == "show":
            return {"ok": True, "result": arguments["result"], "range": [0.0, 1.0]}
        return {"ok": True}

    dispatcher = FakeDispatcher(handler=handler)
    messages: list[dict[str, Any]] = []
    result = run_turn(
        MockLlmProvider([
            ProviderOutput(
                tool_calls=[
                    {"name": "material", "arguments": {"enable": False, "material": 3}},
                    {"name": "show", "arguments": {"result": "eff_stress"}},
                ]
            )
        ]),
        dispatcher,
        messages,
        _TOOLS_LIST,
        step_index=2,
        registry=_REGISTRY,
    )

    assert result.kind == "tool_calls"
    assert [ec.name for ec in result.tool_calls] == ["material", "show"]
    assert [n for n, _ in seen] == ["material", "show"]
    assert result.tool_calls[0].response["hidden_materials"] == [3]
    assert result.tool_calls[1].response["result"] == "eff_stress"

    # Exactly one assistant message carrying both tool_calls, then N
    # tool messages in order — the OpenAI/Anthropic / W3-verifier shape.
    assert len(messages) == 1 + 2
    assist = messages[0]
    assert assist["role"] == "assistant"
    assert len(assist["tool_calls"]) == 2
    assert [tc["function"]["name"] for tc in assist["tool_calls"]] == [
        "material",
        "show",
    ]
    assert [m["role"] for m in messages[1:]] == ["tool", "tool"]
    assert [m["name"] for m in messages[1:]] == ["material", "show"]
    # tool_call_id linkage so the verifier matches calls to responses.
    ids_asst = [tc["id"] for tc in assist["tool_calls"]]
    ids_resp = [m["tool_call_id"] for m in messages[1:]]
    assert ids_asst == ids_resp
    # The step_index makes ids deterministic per turn.
    assert ids_asst == ["call_2_0", "call_2_1"]


# ---------------------------------------------------------------------------
# 3. Parse-error feedback (option (b)).
# ---------------------------------------------------------------------------


def test_parse_error_feedback_creates_synthetic_recovery_slot() -> None:
    """A provider that emits text the harness cannot normalize to a
    canonical call still produces one synthetic ExecutedCall + the
    matching tool message so the next turn the model can self-correct
    (option (b) recovery loop, baseline.md §W4a)."""
    messages: list[dict[str, Any]] = []
    result = run_turn(
        MockLlmProvider([
            # No tool_calls AND no final_text — the malformed-emission arm.
            ProviderOutput()
        ]),
        FakeDispatcher(),
        messages,
        _TOOLS_LIST,
        step_index=0,
        registry=_REGISTRY,
    )
    assert result.kind == "tool_calls"
    assert len(result.tool_calls) == 1
    ec = result.tool_calls[0]
    assert ec.name == PARSE_ERROR_TOOL_NAME
    assert ec.error_kind == "parse_error"
    assert ec.response["ok"] is False
    assert ec.response["error_kind"] == "parse_error"

    assert len(messages) == 2  # assistant + tool
    assert messages[0]["role"] == "assistant"
    assert messages[0]["tool_calls"][0]["function"]["name"] == PARSE_ERROR_TOOL_NAME
    assert messages[1]["role"] == "tool"
    assert messages[1]["name"] == PARSE_ERROR_TOOL_NAME
    parsed_resp = json.loads(messages[1]["content"])
    assert parsed_resp["error_kind"] == "parse_error"


def test_parse_error_slot_inside_otherwise_valid_tool_call_list() -> None:
    """A malformed single slot does not poison the rest of the batch:
    the harness emits ``parse_error`` for the bad slot and dispatches
    the others normally. Mirrors what an OpenAI provider doing N tool
    calls does when one of them has malformed arguments."""
    messages: list[dict[str, Any]] = []
    result = run_turn(
        MockLlmProvider([
            ProviderOutput(
                tool_calls=[
                    {"name": "load", "arguments": {"root": "d3samp6"}},
                    # arguments is a string, not a dict — fails canonical shape.
                    {"name": "show", "arguments": "result=sx"},  # type: ignore[dict-item]
                    {"name": "snapshot", "arguments": {}},
                ]
            )
        ]),
        FakeDispatcher(handler=lambda n, a: {"ok": True}),
        messages,
        _TOOLS_LIST,
        step_index=0,
        registry=_REGISTRY,
    )
    assert result.kind == "tool_calls"
    kinds = [ec.error_kind for ec in result.tool_calls]
    names = [ec.name for ec in result.tool_calls]
    assert kinds == [None, "parse_error", None]
    assert names == ["load", PARSE_ERROR_TOOL_NAME, "snapshot"]


# ---------------------------------------------------------------------------
# 4. error_kind enum identity vs the W3 verifier — load-bearing pin.
# ---------------------------------------------------------------------------


def test_error_kind_enum_identity_with_verifier_failure_modes() -> None:
    """``HARNESS_ERROR_KINDS`` is *the same tuple* as
    ``verifier.FAILURE_MODES``. The two-source-of-truth anti-pattern is
    exactly what W3 was designed to catch loudly — keep these aliased,
    not duplicated."""
    assert HARNESS_ERROR_KINDS is verifier.FAILURE_MODES
    assert tuple(HARNESS_ERROR_KINDS) == verifier.FAILURE_MODES


# ---------------------------------------------------------------------------
# 5. Replay round-trip — re-grade a stored rollout deterministically.
# ---------------------------------------------------------------------------


def _build_fabricated_rollout(scenario_id: str) -> dict[str, Any]:
    """Stored rollouts file shape (``posttraining-dataset.md`` §1).
    Only ``id`` and ``messages`` are needed for replay."""
    return {
        "id": scenario_id,
        "fixture": "d3samp6",
        "messages": [
            {"role": "user", "content": "load d3samp6"},
            {
                "role": "assistant",
                "tool_calls": [
                    {
                        "id": "call_orig_0",
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
                "tool_call_id": "call_orig_0",
                "name": "load",
                "content": json.dumps(
                    {
                        "ok": True,
                        "num_states": 101,
                        "num_classes": 7,
                        "classes": ["glob", "mat", "node", "beam", "brick", "shell", "cseg"],
                        "state_time_range": [0.0, 1.0],
                        "current_time": 0.0,
                    }
                ),
            },
            {
                "role": "assistant",
                "tool_calls": [
                    {
                        "id": "call_orig_1",
                        "type": "function",
                        "function": {
                            "name": "set_state",
                            "arguments": json.dumps({"state": 5}),
                        },
                    }
                ],
            },
            {
                "role": "tool",
                "tool_call_id": "call_orig_1",
                "name": "set_state",
                "content": json.dumps(
                    {"ok": True, "state": 5, "num_states": 101, "current_time": 0.04}
                ),
            },
        ],
    }


def test_replay_round_trip_matches_recorded_tool_responses(tmp_path: Path) -> None:
    """Drive ``ReplayLlmProvider`` through ``run_turn`` with a
    ``FakeDispatcher`` that echoes the *same* recorded responses; the
    resulting ``ExecutedCall.response`` payloads are byte-identical to
    the ``content`` JSON the original rollout carried. Catches schema
    drift between stored rollouts and the live verifier (the
    dataset-validation use case baseline.md §W4a "Replay mode" pins)."""
    rollouts = tmp_path / "rollouts.jsonl"
    record = _build_fabricated_rollout("rs-001")
    rollouts.write_text(json.dumps(record) + "\n")

    # Pre-index the recorded tool responses by call name so the
    # dispatcher can replay them.
    recorded: dict[str, dict[str, Any]] = {}
    for msg in record["messages"]:
        if msg.get("role") == "tool":
            recorded[msg["name"]] = json.loads(msg["content"])

    dispatcher = FakeDispatcher(handler=lambda n, a: dict(recorded[n]))
    provider = ReplayLlmProvider(rollouts_path=rollouts, scenario_id="rs-001")
    assert provider.turns == 2

    messages: list[dict[str, Any]] = []
    turn0 = run_turn(
        provider, dispatcher, messages, _TOOLS_LIST,
        step_index=0, registry=_REGISTRY,
    )
    turn1 = run_turn(
        provider, dispatcher, messages, _TOOLS_LIST,
        step_index=1, registry=_REGISTRY,
    )

    assert turn0.kind == turn1.kind == "tool_calls"
    assert [ec.name for ec in turn0.tool_calls] == ["load"]
    assert [ec.name for ec in turn1.tool_calls] == ["set_state"]
    assert turn0.tool_calls[0].response == recorded["load"]
    assert turn1.tool_calls[0].response == recorded["set_state"]

    # Exhaustion is a clear signal that the live loop diverged from
    # the stored recording (more turns demanded than stored). The
    # provider itself raises ``IndexError``; the harness folds that
    # into a closed-taxonomy error TurnResult so the driver still
    # sees a closed-set label.
    with pytest.raises(IndexError):
        provider.generate(
            messages, _TOOLS_LIST,
            temperature=0.0, max_new_tokens=256, seed=0,
        )


# ---------------------------------------------------------------------------
# 6. Per-turn wall-clock timeout.
# ---------------------------------------------------------------------------


def test_per_turn_timeout_returns_error_turn() -> None:
    """A provider that exceeds ``timeout_s`` returns
    ``TurnResult(kind="error", error_kind="timeout")``; ``messages``
    is left untouched so the driver can choose whether to record a
    ``stop:timeout`` system message or retry."""
    messages: list[dict[str, Any]] = []
    before = len(messages)
    result = run_turn(
        MockLlmProvider(
            [ProviderOutput(tool_calls=[{"name": "snapshot", "arguments": {}}])],
            sleep_s=0.15,
        ),
        FakeDispatcher(),
        messages,
        _TOOLS_LIST,
        step_index=0,
        timeout_s=0.05,
        registry=_REGISTRY,
    )
    assert isinstance(result, TurnResult)
    assert result.kind == "error"
    assert result.error_kind == "timeout"
    assert result.wall_ms >= 50
    assert len(messages) == before


# ---------------------------------------------------------------------------
# 7. Schema mismatch + unknown tool.
# ---------------------------------------------------------------------------


def test_schema_mismatch_yields_structured_response_without_raising() -> None:
    """Bad arguments → an ``ExecutedCall`` with ``error_kind ==
    'schema_mismatch'`` and an ``ok=False`` response; the harness does
    not raise through to the driver."""
    messages: list[dict[str, Any]] = []
    result = run_turn(
        MockLlmProvider([
            # `load` requires `root: str`; we ship an int.
            ProviderOutput(tool_calls=[{"name": "load", "arguments": {"root": 42}}])
        ]),
        FakeDispatcher(),
        messages,
        _TOOLS_LIST,
        step_index=0,
        registry=_REGISTRY,
    )
    assert result.kind == "tool_calls"
    ec = result.tool_calls[0]
    assert ec.name == "load"
    assert ec.error_kind == "schema_mismatch"
    assert ec.response["ok"] is False
    assert ec.response["error_kind"] == "schema_mismatch"
    # Tool message body carries the structured error too.
    tool_msg = json.loads(messages[1]["content"])
    assert tool_msg["error_kind"] == "schema_mismatch"


def test_unknown_tool_yields_structured_response() -> None:
    messages: list[dict[str, Any]] = []
    result = run_turn(
        MockLlmProvider([
            ProviderOutput(
                tool_calls=[{"name": "warp_drive", "arguments": {"factor": 9}}]
            )
        ]),
        FakeDispatcher(),
        messages,
        _TOOLS_LIST,
        step_index=0,
        registry=_REGISTRY,
    )
    ec = result.tool_calls[0]
    assert ec.error_kind == "unknown_tool"
    assert ec.response["error_kind"] == "unknown_tool"


def test_dispatcher_classifies_argument_level_failures() -> None:
    """A dispatcher tagging its own structured failure (e.g.
    ``nonexistent_material``) wins; an unknown tag falls back to the
    generic ``dispatch_error`` — keeps the closed taxonomy closed."""
    messages: list[dict[str, Any]] = []

    def handler(name: str, arguments: dict[str, Any]) -> dict[str, Any]:
        if arguments.get("material") == 999:
            return {
                "ok": False,
                "error": "unknown material 999",
                "error_kind": "nonexistent_material",
            }
        return {
            "ok": False,
            "error": "totally novel failure",
            "error_kind": "novel_tag_outside_taxonomy",
        }

    result = run_turn(
        MockLlmProvider([
            ProviderOutput(
                tool_calls=[
                    {"name": "material", "arguments": {"enable": False, "material": 999}},
                    {"name": "material", "arguments": {"enable": False, "material": 1}},
                ]
            )
        ]),
        FakeDispatcher(handler=handler),
        messages,
        _TOOLS_LIST,
        step_index=0,
        registry=_REGISTRY,
    )
    kinds = [ec.error_kind for ec in result.tool_calls]
    assert kinds == ["nonexistent_material", "dispatch_error"]


# ---------------------------------------------------------------------------
# 8. W2 × W3 × W4a contract — harness output drives the verifier to L3.
# ---------------------------------------------------------------------------


def _scenario_by_id(sid: str) -> Any:
    for s in load_scenarios(default_bootstrap_path()):
        if s.id == sid:
            return s
    raise AssertionError(f"scenario {sid!r} not found in bootstrap.jsonl")


def test_w3_contract_perfect_mock_grades_l3_on_bs_001() -> None:
    """A perfect Mock script for ``bs-001`` (the canonical ``load``
    scenario) — when driven through ``run_turn`` and then graded by
    ``verifier.verify`` — reaches ``max_tier == 3`` with no failure
    mode. Pins **W2 + W3 + W4a together**: the harness's appended
    message shape is exactly what the verifier reads."""
    scenario = _scenario_by_id("bs-001")
    dispatcher = FakeDispatcher(
        handler=lambda n, a: {
            "ok": True,
            "num_states": 101,
            "num_classes": 7,
            "classes": ["glob", "mat", "node", "beam", "brick", "shell", "cseg"],
            "state_time_range": [0.0, 1.0],
            "current_time": 0.0,
        }
    )
    messages: list[dict[str, Any]] = [
        {"role": "user", "content": scenario.instruction}
    ]
    provider = MockLlmProvider(
        [
            ProviderOutput(
                tool_calls=[{"name": "load", "arguments": {"root": scenario.fixture}}]
            ),
            ProviderOutput(final_text="loaded."),
        ]
    )
    turn0 = run_turn(
        provider, dispatcher, messages, _TOOLS_LIST,
        step_index=0, registry=_REGISTRY,
    )
    assert turn0.kind == "tool_calls"
    turn1 = run_turn(
        provider, dispatcher, messages, _TOOLS_LIST,
        step_index=1, registry=_REGISTRY,
    )
    assert turn1.kind == "final_text"
    assert turn1.final_text == "loaded."

    result = verifier.verify(messages, scenario.postcondition.to_json())
    assert result.max_tier == 3
    assert result.failure_mode is None
    assert result.reward == 1.0


# ---------------------------------------------------------------------------
# Misc — sanity for the registry + final_text arm.
# ---------------------------------------------------------------------------


def test_registry_loads_from_artifact() -> None:
    assert _REGISTRY.has("load")
    assert _REGISTRY.has("snapshot")
    assert _REGISTRY.has("griz_raw")
    # 18 tools per W1 (15 typed Command variants + query + snapshot + griz_raw).
    assert len(_REGISTRY.all()) == 18
    assert "type" in _REGISTRY.input_schema("load")


def test_final_text_arm_appends_assistant_content_only() -> None:
    messages: list[dict[str, Any]] = []
    result = run_turn(
        MockLlmProvider([ProviderOutput(final_text="all done")]),
        FakeDispatcher(),
        messages,
        _TOOLS_LIST,
        step_index=0,
        registry=_REGISTRY,
    )
    assert result.kind == "final_text"
    assert result.final_text == "all done"
    assert len(messages) == 1
    assert messages[0] == {"role": "assistant", "content": "all done"}


def test_strip_forbidden_no_op_on_already_clean_tree() -> None:
    """The belt is idempotent and does not damage a clean tree."""
    clean = {"ok": True, "state": 5, "nested": {"value": [1, 2, 3]}}
    assert _strip_forbidden(clean) == clean
