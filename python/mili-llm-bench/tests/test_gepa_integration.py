"""Smoke tests for GEPA integration.

Tests the artifact abstraction and evaluator interface without requiring
actual GEPA, griz, or llama-server. Uses FakeDispatcher + MockLlmProvider
for fast, deterministic evaluation.
"""

from __future__ import annotations

import json
import tempfile
from pathlib import Path
from typing import Any

import pytest

from mili_llm_bench import driver, scenarios
from mili_llm_bench.harness import FakeDispatcher, Registry
from mili_llm_bench.gepa_integration import (
    _candidate_to_artifact,
    apply_artifact_tools,
    artifact_to_eval_config,
    artifact_tools,
    evaluate_artifact,
    evaluate_artifact_detailed,
)
from mili_llm_bench.providers.base import ProviderOutput
from mili_llm_bench.providers.mock import MockLlmProvider


# ---------------------------------------------------------------------------
# Test artifact_to_eval_config
# ---------------------------------------------------------------------------


class TestArtifactToEvalConfig:
    """Test the artifact abstraction layer."""

    def test_string_artifact(self) -> None:
        """String artifact becomes system prompt."""
        custom_prompt = "Custom system prompt for testing"
        config = artifact_to_eval_config(custom_prompt)

        assert config.system_prompt == custom_prompt
        assert config.step_cap == 8  # default
        assert config.temperature == 0.0  # default
        assert config.max_new_tokens == 256  # default

    def test_dict_artifact_minimal(self) -> None:
        """Dict with only system_prompt specified."""
        artifact = {"system_prompt": "Test prompt"}
        config = artifact_to_eval_config(artifact)

        assert config.system_prompt == "Test prompt"
        assert config.step_cap == 8

    def test_dict_artifact_full(self) -> None:
        """Dict with all fields specified."""
        artifact = {
            "system_prompt": "Test prompt",
            "step_cap": 12,
            "max_new_tokens": 512,
            "temperature": 0.5,
            "seed": 42,
            "per_turn_timeout_s": 120.0,
        }
        config = artifact_to_eval_config(artifact)

        assert config.system_prompt == "Test prompt"
        assert config.step_cap == 12
        assert config.max_new_tokens == 512
        assert config.temperature == 0.5
        assert config.seed == 42
        assert config.per_turn_timeout_s == 120.0

    def test_invalid_artifact_type(self) -> None:
        """Non-string, non-dict artifact raises ValueError."""
        with pytest.raises(ValueError, match="artifact must be str or dict"):
            artifact_to_eval_config(12345)  # type: ignore

    def test_dict_uses_defaults_for_missing_fields(self) -> None:
        """Dict missing some fields falls back to defaults."""
        artifact = {"system_prompt": "Test", "step_cap": 10}
        config = artifact_to_eval_config(artifact)

        assert config.system_prompt == "Test"
        assert config.step_cap == 10
        assert config.max_new_tokens == 256  # default
        assert config.temperature == 0.0  # default


# ---------------------------------------------------------------------------
# Test artifact_tools — multi-field candidate shape (tool:<name> keys)
# ---------------------------------------------------------------------------


class TestArtifactToolsMultiField:
    """Verify the multi-field candidate shape GEPA hands back when
    `seed_candidate` is a dict[str, str] with `tool:<name>` keys.

    The reflection LM proposes new text per field; the harness has to
    rebuild a tool-list shape that `apply_artifact_tools` accepts, while
    preserving input/output schemas that were never in the candidate."""

    def test_string_artifact_returns_none(self) -> None:
        assert artifact_tools("just a system prompt") is None

    def test_dict_without_tool_keys_returns_none(self) -> None:
        assert artifact_tools({"system_prompt": "..."}) is None

    def test_legacy_tools_list_passes_through(self) -> None:
        tools = [{"name": "load", "description": "open db"}]
        assert artifact_tools({"tools": tools}) == tools

    def test_tool_prefix_keys_become_description_overrides(self) -> None:
        artifact = {
            "system_prompt": "...",
            "tool:load": "open a database by basename",
            "tool:show": "color the mesh by a result field",
        }
        out = artifact_tools(artifact)
        assert out is not None
        by_name = {t["name"]: t for t in out}
        assert by_name["load"]["description"] == "open a database by basename"
        assert by_name["show"]["description"] == "color the mesh by a result field"
        # Schemas are intentionally absent — apply_artifact_tools fills
        # them from the registry.
        assert "input_schema" not in by_name["load"]

    def test_apply_artifact_tools_preserves_schema_on_description_only_override(
        self,
    ) -> None:
        registry = Registry(
            tools={
                "load": {
                    "name": "load",
                    "description": "baseline desc",
                    "input_schema": {"type": "object", "properties": {"root": {"type": "string"}}},
                    "output_schema": {"type": "object"},
                }
            }
        )
        overrides = artifact_tools(
            {"tool:load": "edited description from reflection"}
        )
        assert overrides is not None
        merged = apply_artifact_tools(registry, overrides)
        assert merged.tools["load"]["description"] == "edited description from reflection"
        # input_schema must survive untouched — a wiped schema would break dispatch.
        assert merged.tools["load"]["input_schema"]["properties"]["root"]["type"] == "string"


# ---------------------------------------------------------------------------
# Test _candidate_to_artifact — GEPA-shape → serialization-shape conversion
# ---------------------------------------------------------------------------


class TestCandidateToArtifact:
    """End-to-run normalization: GEPA returns whatever shape we seeded
    (str or dict[str, str]). The on-disk best_artifact.json / best_tools.json
    format is always the legacy bundle — this is the converter."""

    def _baseline_tools(self) -> list[dict[str, Any]]:
        return [
            {
                "name": "load",
                "description": "baseline load",
                "input_schema": {"type": "object", "properties": {"root": {"type": "string"}}},
                "output_schema": {"type": "object"},
            },
            {
                "name": "show",
                "description": "baseline show",
                "input_schema": {"type": "object", "properties": {"result": {"type": "string"}}},
                "output_schema": {"type": "object"},
            },
        ]

    def test_string_candidate_wraps_with_baseline_tools(self) -> None:
        out = _candidate_to_artifact("new prompt", self._baseline_tools())
        assert out["system_prompt"] == "new prompt"
        assert out["step_cap"] == 8
        assert out["tools"] == self._baseline_tools()

    def test_dict_candidate_applies_description_overrides(self) -> None:
        candidate = {
            "system_prompt": "new prompt",
            "tool:load": "edited load",
            "tool:show": "edited show",
        }
        out = _candidate_to_artifact(candidate, self._baseline_tools())
        by_name = {t["name"]: t for t in out["tools"]}
        assert by_name["load"]["description"] == "edited load"
        assert by_name["show"]["description"] == "edited show"
        # Schemas preserved.
        assert by_name["load"]["input_schema"]["properties"]["root"]["type"] == "string"

    def test_dict_candidate_with_partial_overrides_keeps_baseline_for_rest(self) -> None:
        """If GEPA only edits some tool descriptions, the others stay at baseline."""
        candidate = {"system_prompt": "p", "tool:load": "only load edited"}
        out = _candidate_to_artifact(candidate, self._baseline_tools())
        by_name = {t["name"]: t for t in out["tools"]}
        assert by_name["load"]["description"] == "only load edited"
        assert by_name["show"]["description"] == "baseline show"

    def test_dict_candidate_does_not_mutate_baseline(self) -> None:
        baseline = self._baseline_tools()
        candidate = {"tool:load": "edited"}
        _candidate_to_artifact(candidate, baseline)
        # The original baseline list is unchanged — important because the
        # same list object is reused for every candidate evaluation.
        assert baseline[0]["description"] == "baseline load"


# ---------------------------------------------------------------------------
# Test evaluate_artifact (integration with mock providers)
# ---------------------------------------------------------------------------


class TestEvaluateArtifact:
    """Test the evaluator function with mocked infrastructure."""

    @pytest.fixture
    def mock_scenarios(self) -> list[scenarios.Scenario]:
        """Minimal set of test scenarios."""
        return [
            scenarios.Scenario(
                id="test-1",
                fixture="test_fixture",
                intent_id="intent_1",
                instruction="Load a database and show a result",
                postcondition=scenarios.Postcondition(
                    kind="active_result",
                    expect={"result": "vx"},
                ),
            ),
            scenarios.Scenario(
                id="test-2",
                fixture="test_fixture",
                intent_id="intent_2",
                instruction="Select some elements",
                postcondition=scenarios.Postcondition(
                    kind="selection_set",
                    expect={"selection": []},
                ),
            ),
        ]

    @pytest.fixture
    def mock_registry(self) -> Registry:
        """Minimal tool registry for testing."""
        return Registry(
            tools={
                "load": {
                    "name": "load",
                    "input_schema": {
                        "type": "object",
                        "properties": {"root": {"type": "string"}},
                        "required": ["root"],
                    },
                    "output_schema": {"type": "object"},
                },
                "show": {
                    "name": "show",
                    "input_schema": {
                        "type": "object",
                        "properties": {"result": {"type": "string"}},
                        "required": ["result"],
                    },
                    "output_schema": {"type": "object"},
                },
                "select": {
                    "name": "select",
                    "input_schema": {
                        "type": "object",
                        "properties": {"elements": {"type": "array"}},
                        "required": ["elements"],
                    },
                    "output_schema": {"type": "object"},
                },
            }
        )

    @pytest.fixture
    def mock_tools(self, mock_registry: Registry) -> list[dict[str, Any]]:
        """Tool definitions for harness."""
        return [mock_registry.tools[name] for name in sorted(mock_registry.tools.keys())]

    def test_evaluate_artifact_returns_float(
        self,
        mock_scenarios: list[scenarios.Scenario],
        mock_registry: Registry,
        mock_tools: list[dict[str, Any]],
    ) -> None:
        """evaluate_artifact returns a float in [0, 1]."""
        def provider_factory() -> Any:
            return MockLlmProvider([
                ProviderOutput(tool_calls=[{"name": "load", "arguments": {"root": "test"}}])
            ])

        def dispatcher_factory(scenario: scenarios.Scenario) -> Any:
            return FakeDispatcher(scenario)

        score = evaluate_artifact(
            "Test system prompt",
            provider_factory=provider_factory,
            dispatcher_factory=dispatcher_factory,
            scenarios_list=mock_scenarios,
            registry=mock_registry,
            tools=mock_tools,
        )

        assert isinstance(score, float)
        assert 0.0 <= score <= 1.0

    def test_evaluate_artifact_custom_prompt(
        self,
        mock_scenarios: list[scenarios.Scenario],
        mock_registry: Registry,
        mock_tools: list[dict[str, Any]],
    ) -> None:
        """evaluate_artifact accepts custom system prompt."""
        custom_prompt = "You are a test assistant"

        def provider_factory() -> Any:
            return MockLlmProvider([
                ProviderOutput(tool_calls=[{"name": "load", "arguments": {"root": "test"}}])
            ])

        def dispatcher_factory(scenario: scenarios.Scenario) -> Any:
            return FakeDispatcher(scenario)

        score = evaluate_artifact(
            custom_prompt,
            provider_factory=provider_factory,
            dispatcher_factory=dispatcher_factory,
            scenarios_list=mock_scenarios,
            registry=mock_registry,
            tools=mock_tools,
        )

        assert isinstance(score, float)
        assert 0.0 <= score <= 1.0

    def test_evaluate_artifact_detailed_returns_full_metrics(
        self,
        mock_scenarios: list[scenarios.Scenario],
        mock_registry: Registry,
        mock_tools: list[dict[str, Any]],
    ) -> None:
        """evaluate_artifact_detailed returns EvaluationResult with breakdown."""
        def provider_factory() -> Any:
            return MockLlmProvider([
                ProviderOutput(tool_calls=[{"name": "load", "arguments": {"root": "test"}}])
            ])

        def dispatcher_factory(scenario: scenarios.Scenario) -> Any:
            return FakeDispatcher(scenario)

        result = evaluate_artifact_detailed(
            "Test system prompt",
            provider_factory=provider_factory,
            dispatcher_factory=dispatcher_factory,
            scenarios_list=mock_scenarios,
            registry=mock_registry,
            tools=mock_tools,
        )

        assert result.score >= 0.0
        assert result.score <= 1.0
        assert result.mean_tier >= 0.0
        assert result.mean_tier <= 3.0
        assert 0.0 <= result.l3_pass_rate <= 1.0
        assert result.num_scenarios == len(mock_scenarios)
        assert isinstance(result.failure_modes, dict)
        assert result.wall_s >= 0.0


# ---------------------------------------------------------------------------
# Serialization tests
# ---------------------------------------------------------------------------


class TestGepaResultSerialization:
    """Test result serialization (without full GEPA)."""

    def test_can_roundtrip_artifact_string(self, tmp_path: Path) -> None:
        """Best artifact can be serialized and deserialized."""
        from mili_llm_bench.gepa_integration import _serialize_gepa_results
        from mili_llm_bench.gepa_integration import EvaluationResult, GepaRunConfig

        artifact = "Test system prompt for serialization"
        result = EvaluationResult(
            artifact=artifact,
            score=0.5,
            mean_tier=1.5,
            l3_pass_rate=0.0,
            failure_modes={"step_cap_hit": 50},
            num_scenarios=50,
            wall_s=100.0,
        )
        config = GepaRunConfig(
            dataset_path="dummy.jsonl",
            output_dir=tmp_path,
        )

        _serialize_gepa_results(
            tmp_path,
            best_artifact=artifact,
            best_score=result.score,
            best_result=result,
            history=[],
            config=config,
        )

        # Check files exist
        assert (tmp_path / "best_artifact.txt").exists()
        assert (tmp_path / "best_score.txt").exists()
        assert (tmp_path / "best_result.json").exists()
        assert (tmp_path / "metadata.json").exists()

        # Check artifact was serialized correctly
        restored = (tmp_path / "best_artifact.txt").read_text()
        assert restored == artifact

        # Check score was serialized
        score_text = (tmp_path / "best_score.txt").read_text()
        assert float(score_text.strip()) == 0.5

        # Check result metrics
        result_json = json.loads((tmp_path / "best_result.json").read_text())
        assert result_json["score"] == 0.5
        assert result_json["mean_tier"] == 1.5
        assert result_json["l3_pass_rate"] == 0.0

    def test_serialization_with_dict_artifact(self, tmp_path: Path) -> None:
        """Dict artifact is serialized as JSON."""
        from mili_llm_bench.gepa_integration import _serialize_gepa_results
        from mili_llm_bench.gepa_integration import EvaluationResult, GepaRunConfig

        artifact = {
            "system_prompt": "Test",
            "step_cap": 12,
            "temperature": 0.5,
        }
        result = EvaluationResult(
            artifact=artifact,
            score=0.5,
            mean_tier=1.5,
            l3_pass_rate=0.0,
            failure_modes={},
            num_scenarios=1,
            wall_s=10.0,
        )
        config = GepaRunConfig(
            dataset_path="dummy.jsonl",
            output_dir=tmp_path,
        )

        _serialize_gepa_results(
            tmp_path,
            best_artifact=artifact,
            best_score=result.score,
            best_result=result,
            history=[],
            config=config,
        )

        # Check dict artifact was serialized as JSON
        assert (tmp_path / "best_artifact.json").exists()
        restored = json.loads((tmp_path / "best_artifact.json").read_text())
        assert restored["system_prompt"] == "Test"
        assert restored["step_cap"] == 12


# ---------------------------------------------------------------------------
# Smoke tests (quick integration check)
# ---------------------------------------------------------------------------


class TestSmokeIntegration:
    """Quick integration checks without full GEPA."""

    def test_artifact_config_roundtrip(self) -> None:
        """artifact_to_eval_config can handle all phases."""
        # Phase 1: String
        config1 = artifact_to_eval_config("Test prompt")
        assert config1.system_prompt == "Test prompt"

        # Phase 2: Dict
        config2 = artifact_to_eval_config(
            {
                "system_prompt": "Test",
                "step_cap": 12,
            }
        )
        assert config2.system_prompt == "Test"
        assert config2.step_cap == 12

        # Check they're both frozen (immutable)
        with pytest.raises((AttributeError, Exception)):
            config1.step_cap = 999  # type: ignore
