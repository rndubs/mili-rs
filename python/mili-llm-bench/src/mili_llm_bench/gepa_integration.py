"""GEPA integration for system prompt optimization.

Wraps the eval loop (driver + harness + verifier) to expose an evaluator
interface compatible with GEPA's optimize_anything API. See
planning/gepa-integration-plan.md for design notes.

Three load-bearing functions:

1. artifact_to_eval_config(artifact) -> EvalConfig
   - Convert GEPA's proposed artifact (string or dict) to EvalConfig

2. evaluate_artifact(artifact, **provider_setup) -> float
   - Run eval loop on scenarios with given artifact
   - Return aggregated score (mean_tier / 3.0)

3. run_gepa_optimization(**config) -> dict
   - High-level orchestration: load scenarios, set up providers,
     call GEPA, serialize results
"""

from __future__ import annotations

import json
import logging
from dataclasses import asdict, dataclass
from pathlib import Path
from typing import Any, Callable

from . import driver, scenarios
from .dispatchers.pygriz import PygrizDispatcher
from .harness import Registry
from .providers.base import LlmProvider

logger = logging.getLogger(__name__)


# ---------------------------------------------------------------------------
# Phase 1: Artifact abstraction layer
# ---------------------------------------------------------------------------


def artifact_to_eval_config(artifact: str | dict) -> driver.EvalConfig:
    """Convert GEPA's proposed artifact to an EvalConfig.

    Phase 1 optimizes system prompts (string); Phase 2+ optimizes
    structured configs (dicts with step_cap, tool definitions, etc.).

    Args:
        artifact: Either a system prompt string or a config dict.

    Returns:
        EvalConfig with the artifact's values, defaults for others.
    """
    if isinstance(artifact, str):
        return driver.EvalConfig(system_prompt=artifact)

    if isinstance(artifact, dict):
        return driver.EvalConfig(
            system_prompt=artifact.get("system_prompt", driver._DEFAULT_SYSTEM_PROMPT),
            step_cap=artifact.get("step_cap", 8),
            max_new_tokens=artifact.get("max_new_tokens", 256),
            temperature=artifact.get("temperature", 0.0),
            seed=artifact.get("seed", 0),
            per_turn_timeout_s=artifact.get("per_turn_timeout_s", 60.0),
        )

    raise ValueError(
        f"artifact must be str or dict, got {type(artifact).__name__}: {artifact!r}"
    )


def artifact_tools(artifact: str | dict) -> list[dict[str, Any]] | None:
    """Extract tool definitions from artifact if present.

    Returns:
        List of tool dicts (each with name, description, input_schema,
        output_schema) if artifact is a dict with 'tools' key; None otherwise.
    """
    if isinstance(artifact, dict) and "tools" in artifact:
        return artifact["tools"]
    return None


def apply_artifact_tools(registry: Registry, tools: list[dict[str, Any]]) -> Registry:
    """Create a new registry with tool definitions from artifact.

    Modifies only the descriptions and schemas; preserves other metadata.

    Args:
        registry: Original registry loaded from disk.
        tools: Tool definitions from artifact (list of dicts with name,
               description, input_schema, output_schema).

    Returns:
        New Registry with updated tool definitions.
    """
    updated_tools: dict[str, dict[str, Any]] = {}
    for tool in tools:
        name = tool["name"]
        if name in registry.tools:
            # Merge artifact tool with registry tool (artifact overrides description/schemas)
            updated_tool = registry.tools[name].copy()
            updated_tool["description"] = tool.get("description", updated_tool.get("description"))
            updated_tool["input_schema"] = tool.get("input_schema", updated_tool.get("input_schema"))
            updated_tool["output_schema"] = tool.get("output_schema", updated_tool.get("output_schema"))
            updated_tools[name] = updated_tool
        else:
            # New tool from artifact (rare, but allow it)
            updated_tools[name] = tool

    # Keep any tools from original registry that weren't in artifact
    for name, tool in registry.tools.items():
        if name not in updated_tools:
            updated_tools[name] = tool

    return Registry(tools=updated_tools)


# ---------------------------------------------------------------------------
# Phase 2: Evaluator function
# ---------------------------------------------------------------------------


@dataclass
class EvaluationResult:
    """Per-artifact evaluation result."""

    artifact: str | dict
    score: float
    mean_tier: float
    l3_pass_rate: float
    failure_modes: dict[str, int]
    num_scenarios: int
    wall_s: float


def evaluate_artifact(
    artifact: str | dict,
    *,
    provider_factory: Callable[[], LlmProvider],
    dispatcher_factory: Callable[[scenarios.Scenario], Any],
    scenarios_list: list[scenarios.Scenario],
    registry: Registry,
    tools: list[dict[str, Any]],
) -> float:
    """Run eval loop with given artifact, return aggregated score.

    This is the evaluator function GEPA will call on each iteration.
    It runs the full driver loop (harness + verifier) for each scenario
    with the proposed artifact as the system prompt and/or tool definitions.

    Args:
        artifact: System prompt string (Phase 1) or config dict (Phase 2+).
                  Dict can include: system_prompt, step_cap, tools[], etc.
        provider_factory: Callable that returns a fresh LlmProvider.
        dispatcher_factory: Callable(Scenario) -> Dispatcher.
        scenarios_list: List of Scenario objects to evaluate.
        registry: Tool registry for schema validation.
        tools: List of tool definitions (baseline).

    Returns:
        Aggregated score in [0, 1]. Phase 2+ uses mean_tier / 3.0.
    """
    config = artifact_to_eval_config(artifact)

    # Apply artifact's tools if present, otherwise use baseline
    artifact_tools_list = artifact_tools(artifact)
    if artifact_tools_list:
        eval_registry = apply_artifact_tools(registry, artifact_tools_list)
        eval_tools = _load_tools_for_registry(eval_registry)
    else:
        eval_registry = registry
        eval_tools = tools

    results: list[driver.ScenarioRunResult] = []
    for scenario in scenarios_list:
        result = driver.run_one_scenario(
            provider=provider_factory(),
            dispatcher_factory=dispatcher_factory,
            scenario=scenario,
            tools=eval_tools,
            config=config,
            registry=eval_registry,
        )
        results.append(result)

    # Phase 2+ aggregation: mean tier / 3.0 (dense signal)
    mean_tier = sum(r.verifier_result.max_tier for r in results) / len(results)
    score = mean_tier / 3.0

    logger.debug(
        f"Evaluated artifact with mean_tier={mean_tier:.2f}, score={score:.3f}"
    )

    return score


def evaluate_artifact_detailed(
    artifact: str | dict,
    *,
    provider_factory: Callable[[], LlmProvider],
    dispatcher_factory: Callable[[scenarios.Scenario], Any],
    scenarios_list: list[scenarios.Scenario],
    registry: Registry,
    tools: list[dict[str, Any]],
    wall_s_tracker: dict[str, float] | None = None,
) -> EvaluationResult:
    """Evaluate artifact and return detailed metrics.

    Useful for iteration logging and diagnostics. Same as
    evaluate_artifact() but returns full EvaluationResult with
    failure mode breakdown and wall-clock time.
    """
    import time

    start = time.monotonic()

    config = artifact_to_eval_config(artifact)

    # Apply artifact's tools if present, otherwise use baseline
    artifact_tools_list = artifact_tools(artifact)
    if artifact_tools_list:
        eval_registry = apply_artifact_tools(registry, artifact_tools_list)
        eval_tools = _load_tools_for_registry(eval_registry)
    else:
        eval_registry = registry
        eval_tools = tools

    results: list[driver.ScenarioRunResult] = []

    for scenario in scenarios_list:
        result = driver.run_one_scenario(
            provider=provider_factory(),
            dispatcher_factory=dispatcher_factory,
            scenario=scenario,
            tools=eval_tools,
            config=config,
            registry=eval_registry,
        )
        results.append(result)

    wall_s = time.monotonic() - start
    if wall_s_tracker is not None:
        wall_s_tracker["last_eval_s"] = wall_s

    # Aggregate metrics
    mean_tier = sum(r.verifier_result.max_tier for r in results) / len(results)
    l3_pass_rate = sum(
        1 for r in results if r.verifier_result.max_tier == 3
    ) / len(results)

    # Failure mode histogram
    failure_modes: dict[str, int] = {}
    for r in results:
        mode = r.verifier_result.failure_mode
        if mode:
            failure_modes[mode] = failure_modes.get(mode, 0) + 1

    score = mean_tier / 3.0

    return EvaluationResult(
        artifact=artifact,
        score=score,
        mean_tier=mean_tier,
        l3_pass_rate=l3_pass_rate,
        failure_modes=failure_modes,
        num_scenarios=len(results),
        wall_s=wall_s,
    )


# ---------------------------------------------------------------------------
# Phase 3: GEPA orchestration
# ---------------------------------------------------------------------------


@dataclass
class GepaRunConfig:
    """Configuration for a GEPA optimization run."""

    dataset_path: Path | str
    output_dir: Path | str
    provider_name: str = "llamacpp"
    num_scenarios: int | None = None
    artifact_mode: str = "config"  # "config" includes system_prompt + step_cap + tools
    max_iterations: int = 5
    gepa_engine: str = "claude-opus-4-7"
    gepa_reflection: str = "medium"
    gepa_background: str | None = None
    gepa_objective: str | None = None
    seed_artifact_dir: Path | str | None = None  # Load best tools from a previous run


def run_gepa_optimization(config: GepaRunConfig) -> dict[str, Any]:
    """Run GEPA optimization loop.

    High-level orchestration: load scenarios, instantiate providers,
    call GEPA's optimize_anything(), serialize results.

    Args:
        config: GepaRunConfig with all run parameters.

    Returns:
        Dict with keys:
        - best_artifact: Winning system prompt
        - best_score: Best score achieved
        - best_result: Full EvaluationResult for best artifact
        - history: List of iteration records
    """
    try:
        from gepa.optimize_anything import optimize_anything
    except ImportError as e:
        raise ImportError(
            "GEPA library not installed. Install with: pip install gepa"
        ) from e

    dataset_path = Path(config.dataset_path)
    output_dir = Path(config.output_dir)
    output_dir.mkdir(parents=True, exist_ok=True)

    # Load scenarios
    scenarios_list = scenarios.load_scenarios(dataset_path)
    if config.num_scenarios:
        scenarios_list = scenarios_list[: config.num_scenarios]
    logger.info(f"Loaded {len(scenarios_list)} scenarios from {dataset_path}")

    # Set up provider factory
    if config.provider_name == "llamacpp":
        from .providers.llamacpp import LlamaCppProvider

        def provider_factory() -> LlmProvider:
            return LlamaCppProvider()

    elif config.provider_name == "anthropic":
        from .providers.anthropic import AnthropicProvider

        def provider_factory() -> LlmProvider:
            return AnthropicProvider()

    else:
        raise ValueError(f"Unknown provider: {config.provider_name}")

    # Set up dispatcher factory
    dispatcher_factory = PygrizDispatcher

    # Load registry and tools
    registry = Registry.load_from_artifact()
    tools = _load_tools_for_registry(registry)

    # Build evaluator closure
    wall_s_tracker: dict[str, float] = {}

    def evaluator(artifact: str | dict) -> float:
        """GEPA calls this for each proposed artifact."""
        return evaluate_artifact(
            artifact,
            provider_factory=provider_factory,
            dispatcher_factory=dispatcher_factory,
            scenarios_list=scenarios_list,
            registry=registry,
            tools=tools,
        )

    # Prepare seed artifact: system prompt + step_cap + tools
    seed_tools = None
    seed_source = None

    # 1. Try explicit seed_artifact_dir if provided
    if config.seed_artifact_dir:
        seed_tools = _load_tools_from_result_dir(config.seed_artifact_dir)
        if seed_tools:
            seed_source = str(config.seed_artifact_dir)
            logger.info(f"Seeding with tools from {seed_source}")
        else:
            logger.warning(f"No best_tools.json in {config.seed_artifact_dir}, trying auto-discovery")

    # 2. If not found, auto-discover the most recent GEPA run in the same parent directory
    if not seed_tools:
        previous_run = _find_previous_gepa_run(output_dir)
        if previous_run:
            seed_tools = _load_tools_from_result_dir(previous_run)
            if seed_tools:
                seed_source = str(previous_run)
                logger.info(f"Auto-discovered previous run: {seed_source}")

    # 3. Fall back to baseline if no seed found
    if not seed_tools:
        seed_tools = tools
        logger.info("No previous GEPA run found; using baseline tools")

    seed_artifact: dict[str, Any] = {
        "system_prompt": driver._DEFAULT_SYSTEM_PROMPT,
        "step_cap": 8,
        "tools": seed_tools,
    }

    # Default background and objective if not provided
    background = (
        config.gepa_background
        or """
Griz post-processor for Mili finite-element format.

Tools:
- load: Open a fixture database
- show: Display/color a result field
- material: Enable/disable materials
- select: Manage element selections
- set_state: Change analysis state
- step: Advance to next step
- view, colormap, named_view, legend: Adjust visualization

Tunable hyperparameters:
- system_prompt: Instructions and guidelines for tool use
- step_cap: Maximum steps before stopping (currently 8)
- tools: Tool definitions (names, descriptions, schemas)

Failure modes (in priority order):
1. step_cap_hit (40%): Model needs more steps to complete tasks
2. dispatch_error (42%): Tool execution failures (wrong params, invalid state)
3. schema_mismatch (8%): Argument type mismatches
4. parse_error (8%): Malformed tool calls

Model: FunctionGemma-270M via llama-server.

Previous baseline: 2% L3 pass rate, 58% reach tier 2.
Goal: Improve tool descriptions and system prompt to guide better tool use.
"""
    )

    objective = (
        config.gepa_objective
        or """Improve L3 pass rate by optimizing system prompt guidance and tool
descriptions. Reduce dispatch_error (42%) and step_cap_hit (40%) failures by
clarifying tool semantics and decision-making in the prompt."""
    )

    logger.info(f"Starting GEPA optimization with max_iterations={config.max_iterations}")
    logger.info(f"Evaluating on {len(scenarios_list)} scenarios per iteration")
    logger.info(f"Seed artifact: step_cap=8, system_prompt hash={driver.compute_system_prompt_hash(seed_artifact['system_prompt'])}, tools={len(seed_tools)}")

    # Call GEPA
    from gepa.optimize_anything import GEPAConfig, EngineConfig, ReflectionConfig

    engine_config = EngineConfig(
        display_progress_bar=True,
        max_candidate_proposals=config.max_iterations,
    )

    reflection_config = ReflectionConfig(
        reflection_lm=config.gepa_engine,
    )

    gepa_config = GEPAConfig(
        engine=engine_config,
        reflection=reflection_config,
    )

    # GEPA's reflective mutation expects the seed_candidate to be a string
    # (the instruction/system_prompt), not a dict. Pass the system prompt
    # directly; step_cap and tools are constant during optimization.
    gepa_result = optimize_anything(
        seed_candidate=seed_artifact["system_prompt"],
        evaluator=evaluator,
        objective=objective,
        background=background,
        config=gepa_config,
    )

    # Extract best result with detailed metrics
    best_artifact = gepa_result.best_candidate
    if not best_artifact:
        logger.warning("GEPA did not find a better candidate; using baseline")
        best_artifact = driver._DEFAULT_SYSTEM_PROMPT

    # GEPA returns the system_prompt string; wrap it back into a dict with
    # step_cap and tools for consistent serialization and seeding future runs.
    if isinstance(best_artifact, str):
        best_artifact = {
            "system_prompt": best_artifact,
            "step_cap": 8,
            "tools": seed_tools,
        }

    best_result = evaluate_artifact_detailed(
        best_artifact,
        provider_factory=provider_factory,
        dispatcher_factory=dispatcher_factory,
        scenarios_list=scenarios_list,
        registry=registry,
        tools=tools,
        wall_s_tracker=wall_s_tracker,
    )

    # Build iteration history from GEPA candidate_tree
    # Note: GEPA's result structure is complex; we capture the best candidate
    history = []

    # Serialize results
    _serialize_gepa_results(
        output_dir,
        best_artifact=best_artifact,
        best_score=best_result.score,
        best_result=best_result,
        history=history,
        config=config,
    )

    logger.info(f"GEPA run complete. Best score: {best_result.score:.3f}")
    logger.info(f"Results saved to {output_dir}")

    return {
        "best_artifact": best_artifact,
        "best_score": best_result.score,
        "best_result": best_result,
        "history": history,
    }


# ---------------------------------------------------------------------------
# Utilities
# ---------------------------------------------------------------------------


def _load_tools_for_registry(registry: Registry) -> list[dict[str, Any]]:
    """Load tool definitions from registry in the format expected by
    harness.run_turn() and driver.write_rollout_record()."""
    return [registry.tools[name] for name in sorted(registry.tools.keys())]


def _load_tools_from_json(path: Path | str) -> list[dict[str, Any]]:
    """Load tool definitions from a tools.json file."""
    return json.loads(Path(path).read_text())


def _load_tools_from_result_dir(result_dir: Path | str) -> list[dict[str, Any]] | None:
    """Load tools from a previous GEPA run's best_tools.json if it exists.

    Returns None if the file doesn't exist, allowing graceful fallback to baseline.
    """
    best_tools_path = Path(result_dir) / "best_tools.json"
    if best_tools_path.exists():
        return _load_tools_from_json(best_tools_path)
    return None


def _make_gepa_run_dir_name(base_dir: Path | str) -> Path:
    """Generate a timestamped GEPA run directory name.

    Format: gepa-run-YYYYMMDD-HHMMSS (sortable, human-readable)
    Location: within the base_dir's parent (alongside other gepa-runs)

    Returns the full Path to the new directory (not created yet).
    """
    from datetime import datetime
    from pathlib import Path

    base_path = Path(base_dir)
    timestamp = datetime.now().strftime("%Y%m%d-%H%M%S")
    return base_path.parent / f"gepa-run-{timestamp}"


def _find_previous_gepa_run(output_dir: Path | str) -> Path | None:
    """Find the most recent GEPA run in the same directory tree.

    Looks for directories matching pattern `gepa-run-YYYYMMDD-HHMMSS` and
    returns the most recent one that contains best_tools.json.

    Returns None if no previous run is found.
    """
    import re
    from pathlib import Path

    output_path = Path(output_dir)
    parent_dir = output_path.parent

    # Pattern: gepa-run-YYYYMMDD-HHMMSS
    pattern = re.compile(r"^gepa-run-(\d{8})-(\d{6})$")

    candidates = []
    for item in parent_dir.iterdir():
        if item.is_dir():
            match = pattern.match(item.name)
            if match:
                best_tools = item / "best_tools.json"
                if best_tools.exists():
                    # Extract sortable timestamp: YYYYMMDD + HHMMSS
                    timestamp = f"{match.group(1)}{match.group(2)}"
                    candidates.append((timestamp, item))

    if not candidates:
        return None

    # Return the most recent (highest timestamp)
    candidates.sort(key=lambda x: x[0], reverse=True)
    return candidates[0][1]


def _serialize_gepa_results(
    output_dir: Path,
    best_artifact: str | dict,
    best_score: float,
    best_result: EvaluationResult,
    history: list[dict[str, Any]],
    config: GepaRunConfig,
) -> None:
    """Write GEPA results to disk in standard format.

    Structure:
    - best_artifact.json: Best artifact (includes system_prompt, step_cap, tools)
    - best_score.txt: Numeric score
    - best_result.json: Full EvaluationResult with failure breakdown
    - best_tools.json: Tool definitions extracted from best_artifact (for seeding next run)
    - best_system_prompt.txt: System prompt extracted from best_artifact (for reference)
    - best_step_cap.txt: Step cap extracted from best_artifact (for reference)
    - history.jsonl: Per-iteration records
    - metadata.json: Run configuration
    """
    output_dir = Path(output_dir)
    output_dir.mkdir(parents=True, exist_ok=True)

    # Full best artifact
    if isinstance(best_artifact, str):
        (output_dir / "best_artifact.txt").write_text(best_artifact)
    else:
        (output_dir / "best_artifact.json").write_text(
            json.dumps(best_artifact, indent=2)
        )

    # Best score
    (output_dir / "best_score.txt").write_text(f"{best_score:.6f}\n")

    # Full result metrics
    result_dict = {
        "score": best_result.score,
        "mean_tier": best_result.mean_tier,
        "l3_pass_rate": best_result.l3_pass_rate,
        "num_scenarios": best_result.num_scenarios,
        "wall_s": best_result.wall_s,
        "failure_modes": best_result.failure_modes,
    }
    (output_dir / "best_result.json").write_text(json.dumps(result_dict, indent=2))

    # Extract and save components for next run
    if isinstance(best_artifact, dict):
        # Save tools for seeding next run
        if "tools" in best_artifact:
            (output_dir / "best_tools.json").write_text(
                json.dumps(best_artifact["tools"], indent=2)
            )

        # Save system prompt for reference
        if "system_prompt" in best_artifact:
            (output_dir / "best_system_prompt.txt").write_text(
                best_artifact["system_prompt"]
            )

        # Save step_cap for reference
        if "step_cap" in best_artifact:
            (output_dir / "best_step_cap.txt").write_text(
                str(best_artifact["step_cap"]) + "\n"
            )

    # Iteration history
    (output_dir / "history.jsonl").write_text(
        "\n".join(json.dumps(entry) for entry in history) + "\n"
    )

    # Run metadata
    metadata = {
        "dataset_path": str(config.dataset_path),
        "provider": config.provider_name,
        "num_scenarios": best_result.num_scenarios,
        "artifact_mode": config.artifact_mode,
        "max_iterations": config.max_iterations,
        "gepa_engine": config.gepa_engine,
        "gepa_reflection": config.gepa_reflection,
    }
    (output_dir / "metadata.json").write_text(json.dumps(metadata, indent=2))

    logger.info(f"Results serialized to {output_dir}")
    if isinstance(best_artifact, dict) and "tools" in best_artifact:
        logger.info(f"Best tools saved to {output_dir}/best_tools.json for next run")
