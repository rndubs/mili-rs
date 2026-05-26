"""W4b — eval driver tests.

Always-on (no LLM, no GPU, no pygriz). Every path is exercised via
``MockLlmProvider`` + ``FakeDispatcher``.

Groups (mirroring the PR-4 spec):

1. ``run_one_scenario`` happy path — drive ``bs-001`` (load) end-to-end.
2. ``run_one_scenario`` step-cap exhaustion → ``"stop:step_cap_hit"``.
3. ``run_one_scenario`` per-turn timeout → ``"stop:timeout"``.
4. Rollout record shape pin — every ``posttraining-dataset.md`` §1 key.
5. ``extract_tool_calls_flat`` ordering across N-call turns.
6. Summary writer completeness — every ``FAILURE_MODES`` key present.
7. ``run_eval`` end-to-end smoke — 3 scenarios round-trip via JSONL.
8. System-prompt content-hash stability pin.
"""

from __future__ import annotations

import hashlib
import json
from pathlib import Path
from typing import Any

import pytest

from mili_llm_bench import verifier
from mili_llm_bench.driver import (
    INSTRUCTION_SOURCE_V0,
    EvalConfig,
    ScenarioRunResult,
    build_summary,
    compute_system_prompt_hash,
    extract_tool_calls_flat,
    run_eval,
    run_one_scenario,
    write_rollout_record,
    write_summary,
)
from mili_llm_bench.harness import FakeDispatcher, Registry
from mili_llm_bench.providers import MockLlmProvider, ProviderOutput
from mili_llm_bench.scenarios import (
    Postcondition,
    Scenario,
    default_bootstrap_path,
    load_scenarios,
)


_REGISTRY = Registry.load_from_artifact()
_TOOLS_LIST = _REGISTRY.all()


# ---------------------------------------------------------------------------
# Test helpers.
# ---------------------------------------------------------------------------


def _scenario_by_id(sid: str) -> Scenario:
    for s in load_scenarios(default_bootstrap_path()):
        if s.id == sid:
            return s
    raise AssertionError(f"scenario {sid!r} not found in bootstrap.jsonl")


def _load_response_for(fixture: str) -> dict[str, Any]:
    """A valid ``load`` response shape — matches what the verifier's
    ``_pc_state_index`` expects for the post-load convention (state=1)."""
    return {
        "ok": True,
        "num_states": 101,
        "num_classes": 7,
        "classes": ["glob", "mat", "node", "beam", "brick", "shell", "cseg"],
        "state_time_range": [0.0, 1.0],
        "current_time": 0.0,
    }


def _loader_dispatcher(_scenario: Scenario) -> FakeDispatcher:
    """A dispatcher that returns a successful ``load`` response for any
    tool, and a generic ok for anything else."""

    def handler(name: str, arguments: dict[str, Any]) -> dict[str, Any]:
        if name == "load":
            return _load_response_for(arguments.get("root", ""))
        if name == "snapshot":
            return {
                "ok": True,
                "state": 1,
                "num_states": 101,
                "state_time_range": [0.0, 1.0],
                "current_time": 0.0,
            }
        return {"ok": True}

    return FakeDispatcher(handler=handler)


# ---------------------------------------------------------------------------
# 1. Happy path — bs-001 (load) end-to-end via run_one_scenario.
# ---------------------------------------------------------------------------


def test_run_one_scenario_happy_path_bs_001() -> None:
    scenario = _scenario_by_id("bs-001")
    provider = MockLlmProvider(
        [
            ProviderOutput(
                tool_calls=[{"name": "load", "arguments": {"root": scenario.fixture}}]
            ),
            # m7 Delta 3 — the loop no longer auto-terminates on the
            # postcondition oracle, so the model must emit final_text
            # to close the rollout. The bench mirrors live-UX
            # behavior: one productive tool call, then a short
            # acknowledgment.
            ProviderOutput(final_text="loaded."),
        ]
    )
    config = EvalConfig()
    result = run_one_scenario(
        provider=provider,
        dispatcher_factory=_loader_dispatcher,
        scenario=scenario,
        tools=_TOOLS_LIST,
        config=config,
        registry=_REGISTRY,
    )
    assert isinstance(result, ScenarioRunResult)
    assert result.verifier_result.max_tier == 3
    assert result.verifier_result.failure_mode is None
    assert result.verifier_result.reward == 1.0
    kinds = [t.kind for t in result.turns]
    assert kinds == ["tool_calls", "final_text"]
    assert provider.calls_made == 2
    assert result.wall_ms_total >= 0
    # No driver-level stop on a clean run.
    assert not any(
        m.get("role") == "system" and str(m.get("content", "")).startswith("stop:")
        for m in result.messages
    )
    # The pinned developer prompt is the first message.
    assert result.messages[0] == {
        "role": "developer",
        "content": config.system_prompt,
    }
    assert result.messages[1] == {
        "role": "user",
        "content": scenario.instruction,
    }
    # m7 Delta 1/2 — clean termination is now a load-bearing
    # signal: the last message is the model's content-only ack.
    assert result.messages[-1] == {"role": "assistant", "content": "loaded."}


# ---------------------------------------------------------------------------
# 1b. Auto-terminate — a "looping" model that never emits final_text
#     still completes cleanly once the postcondition is satisfied.
# ---------------------------------------------------------------------------


def test_run_one_scenario_auto_terminates_when_postcondition_met() -> None:
    """Legacy oracle-early-exit behavior preserved behind
    ``EvalConfig.allow_oracle_early_exit=True`` (m7 Delta 3).

    Weak open-weight models often emit repeat-the-call trajectories
    and never produce final_text; opting into the oracle keeps the
    harness from converting a correct first call into a
    ``step_cap_hit`` and is useful for reproducing pre-M7 bench
    numbers. The honest default (off) is exercised by the
    happy-path test above."""
    scenario = _scenario_by_id("bs-001")
    provider = MockLlmProvider(
        [
            ProviderOutput(
                tool_calls=[{"name": "load", "arguments": {"root": scenario.fixture}}]
            )
            for _ in range(8)
        ]
    )
    config = EvalConfig(step_cap=8, allow_oracle_early_exit=True)
    result = run_one_scenario(
        provider=provider,
        dispatcher_factory=_loader_dispatcher,
        scenario=scenario,
        tools=_TOOLS_LIST,
        config=config,
        registry=_REGISTRY,
    )
    # m7 Delta 2 — the rollout still scores L3 under the oracle
    # because the wrong_termination check only fires when the
    # postcondition itself is the L3 gate; the oracle-driven break
    # leaves the rollout terminating on a tool message, which the
    # honest grader would now flag.
    assert result.verifier_result.max_tier == 2
    assert result.verifier_result.failure_mode == "wrong_termination"
    # Exactly one tool_calls turn — the first turn satisfies the
    # postcondition and the oracle breaks the loop.
    assert len(result.turns) == 1
    # No driver-level stop on the clean auto-terminate path.
    assert not any(
        m.get("role") == "system" and str(m.get("content", "")).startswith("stop:")
        for m in result.messages
    )
    assert provider.calls_made == 1


def test_run_one_scenario_runs_to_step_cap_without_final_text() -> None:
    """m7 Delta 3 default — without ``allow_oracle_early_exit``, a
    looping model that satisfies the postcondition but never emits
    final_text now reports ``step_cap_hit``, matching live-UX
    behavior. This is the honest measurement the M7 plan asks for;
    pre-M7 bench numbers that auto-terminated on the oracle no
    longer apply."""
    scenario = _scenario_by_id("bs-001")
    provider = MockLlmProvider(
        [
            ProviderOutput(
                tool_calls=[{"name": "load", "arguments": {"root": scenario.fixture}}]
            )
            for _ in range(8)
        ]
    )
    config = EvalConfig(step_cap=3)
    result = run_one_scenario(
        provider=provider,
        dispatcher_factory=_loader_dispatcher,
        scenario=scenario,
        tools=_TOOLS_LIST,
        config=config,
        registry=_REGISTRY,
    )
    assert result.verifier_result.failure_mode == "step_cap_hit"
    # The provider was invoked once per turn until step_cap.
    assert provider.calls_made == 3


# ---------------------------------------------------------------------------
# 2. Step-cap exhaustion → "stop:step_cap_hit" → verifier reports
#    failure_mode == "step_cap_hit".
# ---------------------------------------------------------------------------


def test_run_one_scenario_step_cap_exhaustion_emits_stop_system_message() -> None:
    scenario = _scenario_by_id("bs-001")
    # Provider scripts step_cap+1 benign snapshot calls; final_text
    # never arrives. step_cap=3 → loop emits 3 tool_calls turns then
    # bails with stop:step_cap_hit.
    script = [
        ProviderOutput(tool_calls=[{"name": "snapshot", "arguments": {}}])
        for _ in range(4)
    ]
    provider = MockLlmProvider(script)
    config = EvalConfig(step_cap=3)
    result = run_one_scenario(
        provider=provider,
        dispatcher_factory=_loader_dispatcher,
        scenario=scenario,
        tools=_TOOLS_LIST,
        config=config,
        registry=_REGISTRY,
    )
    assert len(result.turns) == 3
    assert all(t.kind == "tool_calls" for t in result.turns)
    # The synthetic system message landed exactly once.
    stops = [
        m for m in result.messages
        if m.get("role") == "system"
        and str(m.get("content", "")).startswith("stop:")
    ]
    assert len(stops) == 1
    assert stops[0]["content"] == "stop:step_cap_hit"
    # Verifier picks up the driver-level stop.
    assert result.verifier_result.failure_mode == "step_cap_hit"


# ---------------------------------------------------------------------------
# 3. Per-turn timeout → "stop:timeout" → verifier reports "timeout".
# ---------------------------------------------------------------------------


def test_run_one_scenario_per_turn_timeout_emits_stop_system_message() -> None:
    scenario = _scenario_by_id("bs-001")
    provider = MockLlmProvider(
        [ProviderOutput(tool_calls=[{"name": "snapshot", "arguments": {}}])],
        sleep_s=0.15,
    )
    config = EvalConfig(per_turn_timeout_s=0.05, step_cap=8)
    result = run_one_scenario(
        provider=provider,
        dispatcher_factory=_loader_dispatcher,
        scenario=scenario,
        tools=_TOOLS_LIST,
        config=config,
        registry=_REGISTRY,
    )
    # Exactly one error turn, then the loop bails.
    assert len(result.turns) == 1
    assert result.turns[0].kind == "error"
    assert result.turns[0].error_kind == "timeout"
    stops = [
        m for m in result.messages
        if m.get("role") == "system"
        and str(m.get("content", "")).startswith("stop:")
    ]
    assert len(stops) == 1
    assert stops[0]["content"] == "stop:timeout"
    assert result.verifier_result.failure_mode == "timeout"


# ---------------------------------------------------------------------------
# 4. Rollout record shape pin — every posttraining-dataset.md §1 key.
# ---------------------------------------------------------------------------


def test_write_rollout_record_emits_canonical_shape(tmp_path: Path) -> None:
    scenario = _scenario_by_id("bs-001")
    provider = MockLlmProvider(
        [
            ProviderOutput(
                tool_calls=[{"name": "load", "arguments": {"root": scenario.fixture}}]
            ),
            ProviderOutput(final_text="loaded."),
        ]
    )
    config = EvalConfig()
    result = run_one_scenario(
        provider=provider,
        dispatcher_factory=_loader_dispatcher,
        scenario=scenario,
        tools=_TOOLS_LIST,
        config=config,
        registry=_REGISTRY,
    )
    out_path = tmp_path / "rollouts.jsonl"
    with out_path.open("w") as f:
        write_rollout_record(
            f,
            scenario=scenario,
            messages=result.messages,
            verifier_result=result.verifier_result,
            tools=_TOOLS_LIST,
            config=config,
            provider_meta={"name": "mock"},
        )

    lines = out_path.read_text().splitlines()
    assert len(lines) == 1
    record = json.loads(lines[0])

    # Every §1 key is present.
    required = {
        "id", "fixture", "intent_id", "instruction", "instruction_source",
        "tools", "messages", "tool_calls_flat", "verifier", "provider",
        "split",
    }
    assert required.issubset(record.keys()), (
        f"missing required keys: {required - record.keys()}"
    )

    # Field values.
    assert record["id"] == scenario.id
    assert record["fixture"] == scenario.fixture
    assert record["intent_id"] == scenario.intent_id
    assert record["instruction"] == scenario.instruction
    assert record["instruction_source"] == INSTRUCTION_SOURCE_V0
    assert record["split"] == "eval"

    # tools is a sorted list of tool names.
    assert isinstance(record["tools"], list)
    assert record["tools"] == sorted(record["tools"])
    assert "load" in record["tools"]

    # tool_calls_flat is parsed-dict shape (not JSON-stringified
    # arguments — that's the wire form for `messages`).
    assert record["tool_calls_flat"] == [
        {"name": "load", "arguments": {"root": scenario.fixture}}
    ]
    # The flat list's argument MUST be a dict, not a string.
    for entry in record["tool_calls_flat"]:
        assert isinstance(entry["arguments"], dict), (
            "tool_calls_flat carries the parsed dict form (the dedup key); "
            "the JSON-stringified form lives only in `messages`"
        )

    # Verifier carries max_tier, reward, failure_mode, postcondition.
    assert record["verifier"]["max_tier"] == 3
    assert record["verifier"]["failure_mode"] is None
    assert record["verifier"]["reward"] == 1.0
    assert record["verifier"]["postcondition"] == scenario.postcondition.to_json()

    # Provider carries the falsifiability pin.
    assert record["provider"]["name"] == "mock"
    assert record["provider"]["config_hash"] == compute_system_prompt_hash(
        config.system_prompt
    )


# ---------------------------------------------------------------------------
# 5. extract_tool_calls_flat — preserves declared order across turns
#    AND within a turn (the N-calls-per-turn case).
# ---------------------------------------------------------------------------


def test_extract_tool_calls_flat_preserves_order_across_and_within_turns() -> None:
    scenario = Scenario(
        id="t-multi",
        fixture="d3samp6",
        intent_id="material",
        instruction="hide mats 3 and 5 then snapshot",
        postcondition=Postcondition(
            kind="materials_visible", expect={"hidden_materials": [3, 5]}
        ),
    )

    def handler(name: str, arguments: dict[str, Any]) -> dict[str, Any]:
        if name == "material":
            return {"ok": True, "hidden_materials": [arguments["material"]]}
        return {"ok": True, "state": 1, "num_states": 101}

    provider = MockLlmProvider(
        [
            # Turn 0: two tool calls in declared order.
            ProviderOutput(
                tool_calls=[
                    {"name": "material", "arguments": {"enable": False, "material": 3}},
                    {"name": "material", "arguments": {"enable": False, "material": 5}},
                ]
            ),
            # Turn 1: one tool call.
            ProviderOutput(tool_calls=[{"name": "snapshot", "arguments": {}}]),
            ProviderOutput(final_text="done."),
        ]
    )
    config = EvalConfig()
    result = run_one_scenario(
        provider=provider,
        dispatcher_factory=lambda s: FakeDispatcher(handler=handler),
        scenario=scenario,
        tools=_TOOLS_LIST,
        config=config,
        registry=_REGISTRY,
    )
    flat = extract_tool_calls_flat(result.messages)
    assert flat == [
        {"name": "material", "arguments": {"enable": False, "material": 3}},
        {"name": "material", "arguments": {"enable": False, "material": 5}},
        {"name": "snapshot", "arguments": {}},
    ]


def test_extract_tool_calls_flat_parses_wire_string_arguments() -> None:
    """The wire shape stores ``function.arguments`` as a JSON string;
    the dedup key (``tool_calls_flat``) requires the parsed dict."""
    messages = [
        {
            "role": "assistant",
            "tool_calls": [
                {
                    "id": "call_0_0",
                    "type": "function",
                    "function": {
                        "name": "set_state",
                        "arguments": json.dumps({"state": 5}),
                    },
                }
            ],
        },
        {"role": "tool", "tool_call_id": "call_0_0", "name": "set_state",
         "content": "{}"},
    ]
    flat = extract_tool_calls_flat(messages)
    assert flat == [{"name": "set_state", "arguments": {"state": 5}}]
    assert isinstance(flat[0]["arguments"], dict)


# ---------------------------------------------------------------------------
# 6. Summary writer completeness — every FAILURE_MODES key present.
# ---------------------------------------------------------------------------


def test_build_summary_covers_every_failure_mode_and_counts_tiers(
    tmp_path: Path,
) -> None:
    """4 scenarios: one L3 pass, one schema_mismatch, one wrong_result,
    one timeout. Every entry of ``verifier.FAILURE_MODES`` must appear
    in ``by_failure_mode`` (zero-init for the modes we did not trigger);
    ``l3_pass_rate == 0.25``."""
    load_scenario = _scenario_by_id("bs-001")  # state_index, fixture=d3samp6
    show_scenario = _scenario_by_id("bs-014")  # active_result, expect=sx

    schema_mismatch_scenario = Scenario(
        id="sm-001",
        fixture="d3samp6",
        intent_id="load",
        instruction="load d3samp6",
        postcondition=Postcondition(kind="state_index", expect={"state": 1}),
    )

    timeout_scenario = Scenario(
        id="to-001",
        fixture="d3samp6",
        intent_id="load",
        instruction="load d3samp6",
        postcondition=Postcondition(kind="state_index", expect={"state": 1}),
    )

    scenarios = [
        load_scenario,         # → L3 pass
        schema_mismatch_scenario,  # → schema_mismatch (L0)
        show_scenario,         # → wrong_result (L2, post-condition miss)
        timeout_scenario,      # → timeout (driver-level)
    ]

    def provider_factory(scenario: Scenario) -> MockLlmProvider:
        if scenario.id == load_scenario.id:
            return MockLlmProvider(
                [
                    ProviderOutput(
                        tool_calls=[
                            {"name": "load", "arguments": {"root": scenario.fixture}}
                        ]
                    ),
                    ProviderOutput(final_text="loaded."),
                ]
            )
        if scenario.id == schema_mismatch_scenario.id:
            # `load` requires root: str; passing an int trips L0/L1
            # schema_mismatch.
            return MockLlmProvider(
                [
                    ProviderOutput(
                        tool_calls=[{"name": "load", "arguments": {"root": 42}}]
                    ),
                    ProviderOutput(final_text="oops."),
                ]
            )
        if scenario.id == show_scenario.id:
            # Calls `show` with the WRONG result name — dispatch ok at
            # L2, post-condition fails with wrong_result.
            return MockLlmProvider(
                [
                    ProviderOutput(
                        tool_calls=[{"name": "show", "arguments": {"result": "wrong"}}]
                    ),
                    ProviderOutput(final_text="shown."),
                ]
            )
        # timeout
        return MockLlmProvider(
            [ProviderOutput(tool_calls=[{"name": "snapshot", "arguments": {}}])],
            sleep_s=0.15,
        )

    def dispatcher_factory(_scenario: Scenario) -> FakeDispatcher:
        def handler(name: str, arguments: dict[str, Any]) -> dict[str, Any]:
            if name == "load":
                return _load_response_for(arguments.get("root", ""))
            if name == "show":
                return {
                    "ok": True,
                    "result": arguments.get("result"),
                    "range": [0.0, 1.0],
                }
            return {"ok": True}

        return FakeDispatcher(handler=handler)

    config = EvalConfig(per_turn_timeout_s=0.05, step_cap=4)
    out_dir = tmp_path / "run"
    summary = run_eval(
        scenarios,
        provider_factory=provider_factory,
        dispatcher_factory=dispatcher_factory,
        config=config,
        out_dir=out_dir,
        provider_name="mock",
        registry=_REGISTRY,
        tools=_TOOLS_LIST,
    )

    # Totals and tier counts.
    assert summary["total"] == 4
    # L3 pass on the load scenario; L0 schema_mismatch; L2 wrong_result;
    # L0 timeout (no tool call ever dispatched).
    assert summary["by_max_tier"]["3"] == 1
    assert summary["by_max_tier"]["2"] == 1
    assert summary["by_max_tier"]["0"] == 2
    assert summary["by_max_tier"]["1"] == 0

    # Every FAILURE_MODES key is present (zero-init for unused ones).
    assert set(summary["by_failure_mode"].keys()) == set(verifier.FAILURE_MODES)
    assert summary["by_failure_mode"]["schema_mismatch"] == 1
    assert summary["by_failure_mode"]["wrong_result"] == 1
    assert summary["by_failure_mode"]["timeout"] == 1
    # Unused modes are zero, not missing.
    assert summary["by_failure_mode"]["nonexistent_material"] == 0
    assert summary["by_failure_mode"]["wrong_range"] == 0

    assert summary["l3_pass_rate"] == 0.25
    assert summary["mean_turns_to_completion"] > 0
    assert summary["total_wall_ms"] >= 0
    # Pinned config echoed verbatim into the summary.
    assert summary["config"]["step_cap"] == 4
    assert summary["config"]["per_turn_timeout_s"] == 0.05
    assert summary["config"]["temperature"] == 0.0
    assert summary["config"]["seed"] == 0
    assert summary["config"]["system_prompt_sha256"] == compute_system_prompt_hash(
        config.system_prompt
    )

    # Summary file is pretty-printed and round-trips.
    summary_path = out_dir / "summary.json"
    on_disk = json.loads(summary_path.read_text())
    assert on_disk == summary
    # Pretty-printed → contains an indented "by_max_tier" block.
    raw = summary_path.read_text()
    assert "  " in raw
    assert "by_max_tier" in raw


# ---------------------------------------------------------------------------
# 7. End-to-end smoke — run_eval over bootstrap.jsonl[0:3] with a Mock
#    factory that yields a perfect rollout per scenario.
# ---------------------------------------------------------------------------


def test_run_eval_end_to_end_smoke_three_scenarios(tmp_path: Path) -> None:
    all_scenarios = load_scenarios(default_bootstrap_path())
    scenarios = all_scenarios[:3]
    # bs-001 / bs-002 / bs-003 are all "load d3samp6" scenarios →
    # state_index expect=1. A perfect rollout per scenario is one
    # `load` call + final_text.

    def provider_factory(scenario: Scenario) -> MockLlmProvider:
        return MockLlmProvider(
            [
                ProviderOutput(
                    tool_calls=[
                        {"name": "load", "arguments": {"root": scenario.fixture}}
                    ]
                ),
                ProviderOutput(final_text="loaded."),
            ]
        )

    config = EvalConfig()
    out_dir = tmp_path / "run"
    summary = run_eval(
        scenarios,
        provider_factory=provider_factory,
        dispatcher_factory=_loader_dispatcher,
        config=config,
        out_dir=out_dir,
        provider_name="mock",
        registry=_REGISTRY,
        tools=_TOOLS_LIST,
    )

    rollouts_path = out_dir / "rollouts.jsonl"
    summary_path = out_dir / "summary.json"
    assert rollouts_path.exists()
    assert summary_path.exists()

    lines = rollouts_path.read_text().splitlines()
    assert len(lines) == 3
    records = [json.loads(line) for line in lines]
    # Records appear in input order.
    assert [r["id"] for r in records] == [s.id for s in scenarios]
    # Every record round-trips through json.loads (no trailing junk).
    for r in records:
        assert r["verifier"]["max_tier"] == 3
        assert r["verifier"]["failure_mode"] is None
        assert r["split"] == "eval"
        assert r["instruction_source"] == INSTRUCTION_SOURCE_V0

    assert summary["l3_pass_rate"] == 1.0
    assert summary["total"] == 3
    assert summary["by_max_tier"]["3"] == 3


# ---------------------------------------------------------------------------
# 8. System-prompt content-hash stability pin — the falsifiability hook.
# ---------------------------------------------------------------------------


def test_system_prompt_hash_pins_default_prompt() -> None:
    """The v0 baseline number is only meaningful against one pinned
    system prompt. A future prompt tweak MUST bump this hash (and
    thus invalidate the previous number on sight). If this test
    fails after a deliberate prompt change, update the expected
    hash and the rebaselining is explicit.
    """
    default_prompt = EvalConfig().system_prompt
    expected = hashlib.sha256(default_prompt.encode("utf-8")).hexdigest()[:16]
    assert compute_system_prompt_hash(default_prompt) == expected
    # Prefix length is configurable but defaults to 16 hex chars (8
    # bytes — collision-free for the scale of v0 ablations).
    assert len(compute_system_prompt_hash(default_prompt)) == 16
    # Tweaking the prompt bumps the hash.
    tweaked = default_prompt + " "
    assert compute_system_prompt_hash(tweaked) != compute_system_prompt_hash(
        default_prompt
    )


def test_write_summary_returns_same_dict_it_writes(tmp_path: Path) -> None:
    """``write_summary`` returns the same dict it serializes so callers
    don't need to re-read ``summary.json``."""
    scenario = _scenario_by_id("bs-001")
    provider = MockLlmProvider(
        [
            ProviderOutput(
                tool_calls=[{"name": "load", "arguments": {"root": scenario.fixture}}]
            ),
            ProviderOutput(final_text="loaded."),
        ]
    )
    config = EvalConfig()
    result = run_one_scenario(
        provider=provider,
        dispatcher_factory=_loader_dispatcher,
        scenario=scenario,
        tools=_TOOLS_LIST,
        config=config,
        registry=_REGISTRY,
    )
    path = tmp_path / "summary.json"
    returned = write_summary(path, [result], config)
    on_disk = json.loads(path.read_text())
    assert returned == on_disk


def test_eval_config_is_frozen_and_pins_baseline_defaults() -> None:
    """The pinned caps from baseline.md §W4b are the defaults; the
    dataclass is frozen so mutation is a deliberate
    ``dataclasses.replace`` not a silent attribute write."""
    cfg = EvalConfig()
    assert cfg.step_cap == 8
    assert cfg.max_new_tokens == 256
    assert cfg.temperature == 0.0
    assert cfg.seed == 0
    assert cfg.per_turn_timeout_s == 60.0
    # m7 Delta 3 — bench loop runs to natural termination by default.
    assert cfg.allow_oracle_early_exit is False
    with pytest.raises(Exception):
        cfg.step_cap = 99  # type: ignore[misc]
