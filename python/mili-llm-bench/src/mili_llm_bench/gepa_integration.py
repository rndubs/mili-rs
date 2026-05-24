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
from .dispatchers.pygriz import PygrizDispatcher, pygriz_dispatcher_factory
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


# Prefix that marks a multi-field-candidate key as a tool-description
# override. GEPA's dict-shaped candidates are dict[str, str], so the only
# way to carry per-tool overrides is to namespace them in the key. Keep the
# prefix short — it appears in every reflection prompt rendered to the LM.
_TOOL_DESC_PREFIX = "tool:"


def artifact_tools(artifact: str | dict) -> list[dict[str, Any]] | None:
    """Extract tool description overrides from artifact if present.

    Supports two artifact shapes:

    1. Legacy bundle: ``{"tools": [{name, description, input_schema, ...}, ...]}``.
       Returned as-is; ``apply_artifact_tools`` merges onto the registry.

    2. Multi-field candidate (GEPA dict[str, str]):
       ``{"system_prompt": "...", "tool:<name>": "<description>", ...}``.
       Each ``tool:<name>`` key contributes a partial override (description
       only); input/output schemas come from the registry — see
       ``apply_artifact_tools`` which preserves any field the override
       doesn't set.

    Returns ``None`` if neither shape is present (artifact is a plain
    system-prompt string, or a dict with no tool fields).
    """
    if not isinstance(artifact, dict):
        return None

    if "tools" in artifact:
        return artifact["tools"]

    overrides: list[dict[str, Any]] = []
    for key, value in artifact.items():
        if key.startswith(_TOOL_DESC_PREFIX) and isinstance(value, str):
            overrides.append(
                {"name": key[len(_TOOL_DESC_PREFIX) :], "description": value}
            )
    return overrides or None


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


def _candidate_to_artifact(
    candidate: str | dict[str, str],
    baseline_tools: list[dict[str, Any]],
) -> dict[str, Any]:
    """Convert GEPA's returned candidate into the legacy serialization shape.

    GEPA hands back whatever was passed as `seed_candidate` — a string for
    single-field optimization, a `dict[str, str]` for multi-field. The
    on-disk format (best_artifact.json, best_tools.json) is always the
    legacy bundle, so this normalizes both shapes to that.

    For dict candidates, `tool:<name>` keys override descriptions on the
    matching baseline tool; input/output schemas are preserved verbatim
    from `baseline_tools` (they were never in the candidate).
    """
    if isinstance(candidate, str):
        return {
            "system_prompt": candidate,
            "step_cap": 8,
            "tools": baseline_tools,
        }

    desc_overrides: dict[str, str] = {
        key[len(_TOOL_DESC_PREFIX) :]: value
        for key, value in candidate.items()
        if key.startswith(_TOOL_DESC_PREFIX) and isinstance(value, str)
    }
    final_tools: list[dict[str, Any]] = []
    for tool in baseline_tools:
        merged = tool.copy()
        if merged["name"] in desc_overrides:
            merged["description"] = desc_overrides[merged["name"]]
        final_tools.append(merged)

    return {
        "system_prompt": candidate.get("system_prompt", driver._DEFAULT_SYSTEM_PROMPT),
        "step_cap": 8,
        "tools": final_tools,
    }


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


def evaluate_single_scenario(
    artifact: str | dict,
    scenario: scenarios.Scenario,
    *,
    provider_factory: Callable[[], LlmProvider],
    dispatcher_factory: Callable[[scenarios.Scenario], Any],
    registry: Registry,
    tools: list[dict[str, Any]],
) -> tuple[float, dict[str, Any]]:
    """Per-scenario evaluator for GEPA's dataset mode.

    GEPA's `optimize_anything(..., valset=<scenarios>)` calls the
    evaluator as `evaluator(candidate, scenario)` and aggregates the
    returned scores itself. Returning `(score, side_info)` puts the
    failure_mode and instruction in front of the reflection LM as
    actionable feedback — without this, reflection gets only a scalar
    and can't reason about *which* scenarios are failing or *how*.
    """
    config = artifact_to_eval_config(artifact)

    artifact_tools_list = artifact_tools(artifact)
    if artifact_tools_list:
        eval_registry = apply_artifact_tools(registry, artifact_tools_list)
        eval_tools = _load_tools_for_registry(eval_registry)
    else:
        eval_registry = registry
        eval_tools = tools

    result = driver.run_one_scenario(
        provider=provider_factory(),
        dispatcher_factory=dispatcher_factory,
        scenario=scenario,
        tools=eval_tools,
        config=config,
        registry=eval_registry,
    )

    tier = result.verifier_result.max_tier
    score = tier / 3.0
    side_info = {
        "scenario_id": scenario.id,
        "fixture": scenario.fixture,
        "intent_id": scenario.intent_id,
        "instruction": scenario.instruction,
        "max_tier": tier,
        "failure_mode": result.verifier_result.failure_mode,
    }
    return score, side_info


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

    # Set up dispatcher factory. `PygrizDispatcher` is a @dataclass with a
    # single `session` field — handing the class itself to the driver causes
    # `dispatcher_factory(scenario)` to assign the Scenario as the session,
    # and every tool call then raises AttributeError → caught as
    # dispatch_error. The factory below opens a real griz session and
    # pre-loads `scenario.fixture` before the model runs.
    dispatcher_factory = pygriz_dispatcher_factory()

    # Load registry and tools
    registry = Registry.load_from_artifact()
    tools = _load_tools_for_registry(registry)

    # Build evaluator closure
    wall_s_tracker: dict[str, float] = {}

    def evaluator(artifact: str | dict, example: scenarios.Scenario) -> tuple[float, dict[str, Any]]:
        """GEPA calls this once per (candidate, example) pair when a
        dataset is provided. The parameter name MUST be `example` — GEPA's
        optimize_anything adapter (`optimize_anything.py:~932`) filters
        kwargs by inspecting the evaluator's signature and forwards
        `example=<data_inst>` from its `dataset`/`valset`. Naming it
        anything else (e.g. `scenario`) drops the kwarg and raises
        `missing 1 required positional argument`.

        Returning (score, side_info) gives the reflection LM per-scenario
        failure data — that's what `<side_info>` in the default reflection
        template renders."""
        return evaluate_single_scenario(
            artifact,
            example,
            provider_factory=provider_factory,
            dispatcher_factory=dispatcher_factory,
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

    # GEPA's `optimize_anything` accepts a dict[str, str] as a multi-field
    # candidate (see `Candidate` type). We expose one field per tool's
    # description plus the system prompt — the reflection LM proposes new
    # text for each field independently, while cross-transfer across
    # scenarios happens via the dataset axis.
    #
    # Only top-level `description` strings are editable. Schemas
    # (input_schema/output_schema) are not in the candidate because a
    # malformed JSON proposal would break dispatch on every scenario.
    # `step_cap` likewise stays constant (GEPA fields are strings, not ints).
    seed_candidate: dict[str, str] = {"system_prompt": driver._DEFAULT_SYSTEM_PROMPT}
    for tool in seed_tools:
        seed_candidate[f"{_TOOL_DESC_PREFIX}{tool['name']}"] = tool.get(
            "description", ""
        )

    # Legacy bundle kept for on-disk serialization (best_artifact.json,
    # best_tools.json) and for `evaluate_artifact_detailed` at run end.
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
        # Single-threaded keeps llama-server (one local process, one
        # request at a time) from being trampled by parallel evaluators.
        parallel=False,
    )

    # `reflection_minibatch_size` is the number of scenarios GEPA shows
    # the reflection LM per round. Without it (None default), GEPA either
    # sends one example at a time or the entire set — neither gives the
    # proposer useful failure-mode breadth. 10 of 50 is a reasonable
    # compromise: enough variety for pattern-finding, small enough that
    # the reflection prompt stays under the engine's context budget.
    reflection_minibatch_size = min(10, len(scenarios_list))
    reflection_config = ReflectionConfig(
        reflection_lm=config.gepa_engine,
        reflection_minibatch_size=reflection_minibatch_size,
    )

    gepa_config = GEPAConfig(
        engine=engine_config,
        reflection=reflection_config,
    )

    # Pass scenarios as `dataset` so GEPA enters multi-task mode and calls
    # `evaluator(candidate, scenario)` per scenario, then aggregates. Without
    # this it stays in single-task mode, calling `evaluator(candidate)` once
    # and treating the returned scalar as the whole signal — that's how the
    # progress bar ends up reading "1 / 1 examples" on a 50-scenario set.
    # `valset` defaults to dataset, which is what we want with only 50
    # scenarios (no train/val split yet).
    gepa_result = optimize_anything(
        seed_candidate=seed_candidate,
        evaluator=evaluator,
        dataset=scenarios_list,
        objective=objective,
        background=background,
        config=gepa_config,
    )

    # Extract best result with detailed metrics
    best_candidate = gepa_result.best_candidate
    if not best_candidate:
        logger.warning("GEPA did not find a better candidate; using baseline")
        best_candidate = seed_candidate

    # Convert GEPA's returned candidate (str or dict[str, str]) into the
    # legacy serialization shape (system_prompt + step_cap + tools list).
    best_artifact = _candidate_to_artifact(best_candidate, seed_tools)

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
