"""PR-5 CLI tests — always-on (no LLM, no GPU, no pygriz, no network).

Every path is exercised via ``FakeDispatcher`` + ``MockLlmProvider``
injected through the public ``build_factories``/``build_replay_factories``
test seam (option (c) from the PR-5 review).
"""

from __future__ import annotations

import hashlib
import importlib
import json
import sys
from pathlib import Path
from typing import Any

import pytest
import yaml

from mili_llm_bench import cli, driver, verifier
from mili_llm_bench.cli import (
    SUPPORTED_PROVIDERS,
    build_factories,
    build_parser,
    build_replay_factories,
    main,
    write_config_yaml,
)
from mili_llm_bench.driver import EvalConfig, compute_system_prompt_hash, run_eval
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
# 1. derive-schemas --check is byte-exact against the pinned artifact.
# ---------------------------------------------------------------------------


def test_derive_schemas_check_passes_on_clean_tree(capsys: Any) -> None:
    rc = main(["derive-schemas", "--check"])
    assert rc == 0
    captured = capsys.readouterr()
    assert "ok" in captured.out


# ---------------------------------------------------------------------------
# 2. derive-schemas --out PATH writes byte-equal to the pinned artifact.
# ---------------------------------------------------------------------------


def test_derive_schemas_writes_byte_equal_to_pinned(tmp_path: Path) -> None:
    out = tmp_path / "tools.json"
    rc = main(["derive-schemas", "--out", str(out)])
    assert rc == 0
    from mili_llm_bench.schemas import default_artifact_path

    pinned = default_artifact_path().read_bytes()
    assert out.read_bytes() == pinned


# ---------------------------------------------------------------------------
# 3. `run --provider mock` end-to-end against bootstrap[0:3]: writes
#    the four expected files; summary.json is valid; report.md has the
#    L3 rate; config.yaml carries the pinned hashes.
# ---------------------------------------------------------------------------


def _loader_dispatcher(_scenario: Scenario) -> FakeDispatcher:
    def handler(name: str, arguments: dict[str, Any]) -> dict[str, Any]:
        if name == "load":
            return {
                "ok": True,
                "num_states": 101,
                "num_classes": 7,
                "classes": ["glob", "mat", "node", "beam", "brick", "shell", "cseg"],
                "state_time_range": [0.0, 1.0],
                "current_time": 0.0,
            }
        return {"ok": True}

    return FakeDispatcher(handler=handler)


def _perfect_loader_provider(scenario: Scenario) -> MockLlmProvider:
    return MockLlmProvider(
        [
            ProviderOutput(
                tool_calls=[{"name": "load", "arguments": {"root": scenario.fixture}}]
            ),
            ProviderOutput(final_text="done."),
        ]
    )


def test_run_mock_end_to_end_smoke_writes_four_artifacts(tmp_path: Path) -> None:
    """The W6 acceptance-gate smoke (baseline.md §"Acceptance gate" #3)
    on the always-on test path: build factories via the public seam
    (option (c)), run ``run_eval`` directly, assert the four files
    landed and the report.md headline contains the L3 pass rate."""
    config = EvalConfig()
    bundle = build_factories(
        "mock",
        config=config,
        provider_factory_override=_perfect_loader_provider,
        dispatcher_factory_override=_loader_dispatcher,
    )
    assert bundle.provider_name == "mock"

    scenarios = load_scenarios(default_bootstrap_path())[:3]
    out_dir = tmp_path / "v0-mock"

    # CLI's run subcommand uses build_factories + run_eval + write_report
    # + write_config_yaml in sequence; here we exercise the same
    # composition without going through argparse so a single test
    # pins both the success path and the file shapes.
    from mili_llm_bench import __version__ as bench_version
    from mili_llm_bench.cli import _sha256_file

    out_dir.mkdir(parents=True, exist_ok=True)
    tools_path = Path(_default_tools_path())
    scenarios_path = default_bootstrap_path()
    scenarios_sha = _sha256_file(scenarios_path)
    tools_sha = _sha256_file(tools_path)

    write_config_yaml(
        out_dir / "config.yaml",
        config=config,
        provider_name=bundle.provider_name,
        model_id=bundle.model_id,
        scenarios_sha256=scenarios_sha,
        tools_sha256=tools_sha,
        bench_version=bench_version,
        run_timestamp=1234567890.0,
    )

    summary = run_eval(
        scenarios,
        provider_factory=bundle.provider_factory,
        dispatcher_factory=bundle.dispatcher_factory,
        config=config,
        out_dir=out_dir,
        provider_name=bundle.provider_name,
        registry=_REGISTRY,
        tools=_TOOLS_LIST,
    )

    from mili_llm_bench import report as report_module

    report_module.write_report(
        out_dir / "report.md",
        summary,
        out_dir / "rollouts.jsonl",
        config_yaml_path=out_dir / "config.yaml",
        summary_path=out_dir / "summary.json",
        provider_name=bundle.provider_name,
        model_id=bundle.model_id,
        scenarios_sha256=scenarios_sha,
    )

    # Four expected files.
    assert (out_dir / "rollouts.jsonl").exists()
    assert (out_dir / "summary.json").exists()
    assert (out_dir / "config.yaml").exists()
    assert (out_dir / "report.md").exists()

    # summary.json is valid JSON.
    on_disk = json.loads((out_dir / "summary.json").read_text())
    assert on_disk["total"] == 3
    assert on_disk["l3_pass_rate"] == 1.0

    # report.md mentions the L3 pass rate string.
    md = (out_dir / "report.md").read_text()
    assert "L3 pass rate" in md
    assert "1.000" in md or "100.0%" in md


def _default_tools_path() -> Path:
    from mili_llm_bench.schemas import default_artifact_path

    return default_artifact_path()


# ---------------------------------------------------------------------------
# 4. replay round-trip identity — fabricate a rollouts.jsonl with 3
#    pinned records; replay reproduces the tier counts on an
#    unchanged verifier.
# ---------------------------------------------------------------------------


def _hand_fabricated_rollouts(path: Path) -> list[dict[str, Any]]:
    """Three hand-built records, each pinned to a known max_tier."""
    records = [
        # bs-001-style: a perfect ``load d3samp6`` → L3 pass.
        {
            "id": "rt-001",
            "fixture": "d3samp6",
            "intent_id": "load",
            "instruction": "load d3samp6",
            "instruction_source": "bootstrap-handauthored",
            "tools": ["load"],
            "messages": [
                {"role": "user", "content": "load d3samp6"},
                {
                    "role": "assistant",
                    "tool_calls": [
                        {
                            "id": "call_0_0",
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
                    "tool_call_id": "call_0_0",
                    "name": "load",
                    "content": json.dumps({"ok": True, "num_states": 101}),
                },
                {"role": "assistant", "content": "done."},
            ],
            "tool_calls_flat": [{"name": "load", "arguments": {"root": "d3samp6"}}],
            "verifier": {
                "max_tier": 3,
                "reward": 1.0,
                "failure_mode": None,
                "postcondition": {"kind": "state_index", "expect": {"state": 1}},
            },
            "provider": {"name": "mock", "config_hash": "x"},
            "split": "eval",
        },
        # rt-002: same shape — second L3 pass.
        {
            "id": "rt-002",
            "fixture": "d3samp6",
            "intent_id": "load",
            "instruction": "load d3samp6 again",
            "instruction_source": "bootstrap-handauthored",
            "tools": ["load"],
            "messages": [
                {"role": "user", "content": "load d3samp6"},
                {
                    "role": "assistant",
                    "tool_calls": [
                        {
                            "id": "call_0_0",
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
                    "tool_call_id": "call_0_0",
                    "name": "load",
                    "content": json.dumps({"ok": True, "num_states": 101}),
                },
                {"role": "assistant", "content": "done."},
            ],
            "tool_calls_flat": [{"name": "load", "arguments": {"root": "d3samp6"}}],
            "verifier": {
                "max_tier": 3,
                "reward": 1.0,
                "failure_mode": None,
                "postcondition": {"kind": "state_index", "expect": {"state": 1}},
            },
            "provider": {"name": "mock", "config_hash": "x"},
            "split": "eval",
        },
        # rt-003: a parse-error rollout (final_text only, never called
        # `load`) → grades L0 wrong_final_state since post-condition
        # expects state=1 and no state was established.
        {
            "id": "rt-003",
            "fixture": "d3samp6",
            "intent_id": "load",
            "instruction": "load d3samp6",
            "instruction_source": "bootstrap-handauthored",
            "tools": ["load"],
            "messages": [
                {"role": "user", "content": "load d3samp6"},
                {"role": "assistant", "content": "I don't know how."},
            ],
            "tool_calls_flat": [],
            "verifier": {
                "max_tier": 0,
                "reward": 0.0,
                "failure_mode": "wrong_final_state",
                "postcondition": {"kind": "state_index", "expect": {"state": 1}},
            },
            "provider": {"name": "mock", "config_hash": "x"},
            "split": "eval",
        },
    ]
    with path.open("w") as f:
        for r in records:
            f.write(json.dumps(r) + "\n")
    return records


def test_replay_round_trip_reproduces_max_tier_per_scenario(tmp_path: Path) -> None:
    rollouts = tmp_path / "rollouts.jsonl"
    originals = _hand_fabricated_rollouts(rollouts)

    bundle = build_replay_factories(
        rollouts, dispatcher_factory_override=_loader_dispatcher
    )
    assert bundle.provider_name == "replay"

    scenarios = cli._scenarios_from_rollouts(rollouts)
    out_dir = tmp_path / "replayed"
    config = EvalConfig()
    summary = run_eval(
        scenarios,
        provider_factory=bundle.provider_factory,
        dispatcher_factory=bundle.dispatcher_factory,
        config=config,
        out_dir=out_dir,
        provider_name=bundle.provider_name,
        registry=_REGISTRY,
        tools=_TOOLS_LIST,
    )

    # Round-trip identity on max_tier per scenario.
    new_records = [
        json.loads(line)
        for line in (out_dir / "rollouts.jsonl").read_text().splitlines()
        if line.strip()
    ]
    by_id = {r["id"]: r for r in new_records}
    for orig in originals:
        assert orig["id"] in by_id
        assert (
            by_id[orig["id"]]["verifier"]["max_tier"]
            == orig["verifier"]["max_tier"]
        ), f"replay drift on {orig['id']}: original {orig['verifier']['max_tier']} vs replayed {by_id[orig['id']]['verifier']['max_tier']}"

    # Summary counts match.
    assert summary["by_max_tier"]["3"] == 2
    assert summary["by_max_tier"]["0"] == 1


# ---------------------------------------------------------------------------
# 5. replay detects verifier drift — change the postcondition so the
#    previously-passing rollouts now fail.
# ---------------------------------------------------------------------------


def test_replay_detects_verifier_drift_via_postcondition_change(tmp_path: Path) -> None:
    rollouts = tmp_path / "rollouts.jsonl"
    _hand_fabricated_rollouts(rollouts)

    # Mutate the records on disk to point at a postcondition the
    # original rollout would NO LONGER satisfy.
    raw_lines = (tmp_path / "rollouts.jsonl").read_text().splitlines()
    mutated: list[dict[str, Any]] = []
    for line in raw_lines:
        obj = json.loads(line)
        # Original load-d3samp6 rollouts established state=1; a
        # postcondition demanding state=5 must NOT pass on replay.
        obj["verifier"]["postcondition"] = {
            "kind": "state_index",
            "expect": {"state": 5},
        }
        mutated.append(obj)
    drift_path = tmp_path / "rollouts_drift.jsonl"
    drift_path.write_text(
        "\n".join(json.dumps(r) for r in mutated) + "\n"
    )

    bundle = build_replay_factories(
        drift_path, dispatcher_factory_override=_loader_dispatcher
    )
    scenarios = cli._scenarios_from_rollouts(drift_path)
    out_dir = tmp_path / "replay_drift"
    summary = run_eval(
        scenarios,
        provider_factory=bundle.provider_factory,
        dispatcher_factory=bundle.dispatcher_factory,
        config=EvalConfig(),
        out_dir=out_dir,
        provider_name=bundle.provider_name,
        registry=_REGISTRY,
        tools=_TOOLS_LIST,
    )

    # At least one row's max_tier differs from the original (the L3
    # passes should now fail wrong_final_state since state=1 != 5).
    new_records = [
        json.loads(line)
        for line in (out_dir / "rollouts.jsonl").read_text().splitlines()
        if line.strip()
    ]
    # rt-001 and rt-002 originally L3 → now < 3.
    by_id = {r["id"]: r for r in new_records}
    assert by_id["rt-001"]["verifier"]["max_tier"] < 3
    assert by_id["rt-002"]["verifier"]["max_tier"] < 3
    assert "wrong_final_state" in summary["by_failure_mode"]
    assert summary["by_failure_mode"]["wrong_final_state"] >= 1


# ---------------------------------------------------------------------------
# 6. config.yaml carries every pinned hash + run knob.
# ---------------------------------------------------------------------------


def test_config_yaml_carries_pinned_hashes_and_caps(tmp_path: Path) -> None:
    config = EvalConfig(step_cap=4, per_turn_timeout_s=12.5, seed=7)
    out = tmp_path / "config.yaml"
    payload = write_config_yaml(
        out,
        config=config,
        provider_name="anthropic",
        model_id="claude-test",
        scenarios_sha256="a" * 64,
        tools_sha256="b" * 64,
        bench_version="9.9.9",
        run_timestamp=1700000000.0,
    )
    loaded = yaml.safe_load(out.read_text())
    assert loaded == payload
    for key in (
        "bench_version", "provider", "model_id",
        "system_prompt_sha256", "tools_sha256", "scenarios_sha256",
        "step_cap", "max_new_tokens", "temperature", "seed",
        "per_turn_timeout_s", "run_timestamp",
    ):
        assert key in loaded, f"missing {key}"
    assert loaded["provider"] == "anthropic"
    assert loaded["model_id"] == "claude-test"
    assert loaded["scenarios_sha256"] == "a" * 64
    assert loaded["tools_sha256"] == "b" * 64
    assert loaded["step_cap"] == 4
    assert loaded["per_turn_timeout_s"] == 12.5
    assert loaded["seed"] == 7
    assert loaded["temperature"] == 0.0
    # The system_prompt_sha256 is the full 64-char hex (not the 16-char
    # prefix the rollout record uses) — config.yaml is the
    # canonical pin.
    assert len(loaded["system_prompt_sha256"]) == 64
    expected = hashlib.sha256(config.system_prompt.encode("utf-8")).hexdigest()
    assert loaded["system_prompt_sha256"] == expected
    assert loaded["run_timestamp"] == 1700000000.0
    assert loaded["bench_version"] == "9.9.9"


# ---------------------------------------------------------------------------
# 7. report.md contains every required section.
# ---------------------------------------------------------------------------


def test_report_md_contains_every_required_section(tmp_path: Path) -> None:
    from mili_llm_bench import report as report_module

    # Build a minimal but realistic summary + rollouts file.
    rollouts_path = tmp_path / "rollouts.jsonl"
    _hand_fabricated_rollouts(rollouts_path)

    summary = {
        "total": 3,
        "by_max_tier": {"0": 1, "1": 0, "2": 0, "3": 2},
        "by_failure_mode": {m: 0 for m in verifier.FAILURE_MODES},
        "l3_pass_rate": 2 / 3,
        "mean_turns_to_completion": 1.5,
        "total_wall_ms": 250,
        "config": {
            "step_cap": 8, "max_new_tokens": 256, "temperature": 0.0, "seed": 0,
            "per_turn_timeout_s": 60.0,
            "system_prompt_sha256": "deadbeefcafef00d",
        },
    }
    summary["by_failure_mode"]["wrong_final_state"] = 1

    out = tmp_path / "report.md"
    text = report_module.write_report(
        out,
        summary,
        rollouts_path,
        config_yaml_path=tmp_path / "config.yaml",
        summary_path=tmp_path / "summary.json",
        provider_name="anthropic",
        model_id="claude-test",
        scenarios_sha256="cafe" * 16,
    )

    # Each required section appears once.
    assert "L3 pass rate" in text  # headline
    assert "## by_max_tier" in text
    assert "## by_failure_mode" in text
    assert "## timing" in text
    assert "## per_intent" in text  # per-intent breakdown
    assert "## raw_fallback_rate" in text  # raw fallback rate
    assert "## artifacts" in text  # pointer to run dir
    # Provider + model id appear in the headline.
    assert "anthropic" in text
    assert "claude-test" in text
    # Scenarios sha appears.
    assert "cafe" in text
    # Per-intent breakdown contains the one intent_id from the
    # fabricated records.
    assert "load" in text


# ---------------------------------------------------------------------------
# 8. CLI help text mentions all four providers.
# ---------------------------------------------------------------------------


def test_cli_help_mentions_all_four_providers(capsys: Any) -> None:
    parser = build_parser()
    # Use the `run` subparser's help output — that's where the closed
    # set is visible to operators.
    with pytest.raises(SystemExit):
        parser.parse_args(["run", "--help"])
    captured = capsys.readouterr()
    text = captured.out + captured.err
    for name in SUPPORTED_PROVIDERS:
        assert name in text, f"{name} missing from CLI help"


# ---------------------------------------------------------------------------
# 9. Lazy-import gate — importing the CLI module does NOT load
#    transformers / torch / anthropic / pygriz.
# ---------------------------------------------------------------------------


def test_cli_import_does_not_load_heavy_deps() -> None:
    """Fresh-reload the cli module; assert no heavy module landed in
    ``sys.modules``."""
    heavy = ("transformers", "torch", "anthropic", "griz", "pygriz")
    # Remove any cached heavy modules so we measure what cli pulls in.
    for mod in heavy:
        sys.modules.pop(mod, None)
    # Also drop the cli module so its top-level imports re-run.
    sys.modules.pop("mili_llm_bench.cli", None)
    sys.modules.pop("mili_llm_bench.providers.transformers", None)
    sys.modules.pop("mili_llm_bench.providers.anthropic", None)
    sys.modules.pop("mili_llm_bench.dispatchers.pygriz", None)

    importlib.import_module("mili_llm_bench.cli")

    for mod in heavy:
        assert mod not in sys.modules, (
            f"{mod} loaded during `import mili_llm_bench.cli` — the CLI must "
            "lazy-import heavy provider deps so `derive-schemas` runs without "
            "them"
        )


# ---------------------------------------------------------------------------
# 10. Unknown provider name is rejected cleanly.
# ---------------------------------------------------------------------------


def test_build_factories_rejects_unknown_provider() -> None:
    with pytest.raises(ValueError, match="unknown provider"):
        build_factories("notaprovider", config=EvalConfig())


def test_build_factories_rejects_replay_provider() -> None:
    with pytest.raises(ValueError, match="replay"):
        build_factories("replay", config=EvalConfig())


# ---------------------------------------------------------------------------
# 11. derive-schemas --check exit code on drift.
# ---------------------------------------------------------------------------


def test_derive_schemas_check_fails_on_drift(
    tmp_path: Path, monkeypatch: Any
) -> None:
    """If the on-disk tools.json drifts from what derive_tools()
    produces, --check exits 1. Forge the drift by pointing the
    pinned-path helper at a tampered copy."""
    from mili_llm_bench import schemas as schemas_mod

    real_path = schemas_mod.default_artifact_path()
    tampered = tmp_path / "tools.json"
    text = real_path.read_text()
    tampered.write_text(text.replace("\"load\"", "\"loadX\"", 1))

    monkeypatch.setattr(schemas_mod, "default_artifact_path", lambda: tampered)
    # The CLI module imports default_artifact_path lazily inside the
    # handler; the monkeypatch lands on the module path the handler
    # reads from.
    rc = main(["derive-schemas", "--check"])
    assert rc == 1
