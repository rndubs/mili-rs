"""W4b — eval driver (loop on top of the harness, caps, rollout writer).

The v0 eval driver: per-scenario, builds a fresh dispatcher (via the
caller-supplied factory) + provider (likewise), runs ``harness.run_turn``
up to ``step_cap`` times, surfaces driver-level stops to the W3
verifier via the synthetic ``"stop:<reason>"`` ``system`` message
convention, calls ``verifier.verify``, and emits one canonical rollout
record per scenario in the
``planning/mili-viz/posttraining-dataset.md`` §1 shape.

Three load-bearing design pins:

1. **Factory seam for dispatchers + providers.** The always-on test
   path supplies a ``FakeDispatcher`` factory and a ``MockLlmProvider``
   factory — zero pygriz / transformers / network deps. The PR-5 CLI
   plugs in a ``PygrizDispatcher``-per-scenario factory + a live LLM
   provider factory without re-architecting the driver. The factory
   signature ``(Scenario) -> X`` is the seam.
2. **Driver-level stops ride on system messages.** The harness stays
   neutral on driver bookkeeping (it returns
   ``TurnResult(kind="error", error_kind="timeout")`` and leaves
   ``messages`` alone); the driver appends ``"stop:timeout"`` /
   ``"stop:step_cap_hit"`` so the W3 verifier's ``_detect_driver_stop``
   picks them up and grades the rollout with the right
   ``failure_mode``. ``token_cap_hit`` is reserved in the closed enum
   but **not emitted in v0** — the provider's per-call ``max_new_tokens``
   bound is sufficient; a per-rollout token budget is a deliberate
   non-goal for v0 (baseline §W4b "Caps and determinism").
3. **System-prompt content-hash is the falsifiability hook.** The v0
   baseline number is meaningless without a pinned system prompt;
   ``compute_system_prompt_hash`` is recorded on every rollout's
   ``provider.config_hash`` and on the run-level
   ``summary.config.system_prompt_sha256`` so a deliberate prompt
   tweak bumps the hash and the previous number becomes
   re-baselineable on sight.
"""

from __future__ import annotations

import hashlib
import json
import time
from dataclasses import dataclass, field
from pathlib import Path
from typing import Any, Callable, TextIO

from . import harness, verifier
from .harness import Dispatcher, Registry, TurnResult
from .providers.base import LlmProvider
from .scenarios import Scenario


# ---------------------------------------------------------------------------
# Pinned system prompt (content-hashed for falsifiability — see module
# docstring). Short, factual, tool-use-oriented; an LLM-native chat
# template will accept it as a ``developer`` / ``system`` message.
# ---------------------------------------------------------------------------

_DEFAULT_SYSTEM_PROMPT = (
    "You are an assistant that operates the Griz post-processor for the "
    "Mili finite-element format. You drive Griz by emitting JSON function "
    "calls into the supplied tool inventory. Inspect the user's request, "
    "call exactly the tools that satisfy it, and reply with one short "
    "final text message only after the request is fully complete. Do not "
    "narrate plans; emit a tool call instead. Prefer the typed tools "
    "over the `griz_raw` fallback when a typed tool exists for the task."
)


def compute_system_prompt_hash(prompt: str, *, prefix_len: int = 16) -> str:
    """SHA-256 of the system prompt, truncated to ``prefix_len`` hex
    characters.

    Recorded on each rollout's ``provider.config_hash`` and on the
    summary's ``config.system_prompt_sha256``. A prompt tweak deliberately
    bumps the hash; the v0 baseline number is only meaningful against
    one pinned prompt.
    """
    return hashlib.sha256(prompt.encode("utf-8")).hexdigest()[:prefix_len]


# ---------------------------------------------------------------------------
# EvalConfig — the frozen run knobs (baseline §W4b "Caps and
# determinism"). All defaults pinned; consumers override deliberately.
# ---------------------------------------------------------------------------


@dataclass(frozen=True)
class EvalConfig:
    """v0 caps + run metadata; defaults pinned by baseline.md §W4b."""

    step_cap: int = 8
    max_new_tokens: int = 256
    temperature: float = 0.0
    seed: int = 0
    per_turn_timeout_s: float = 60.0
    system_prompt: str = _DEFAULT_SYSTEM_PROMPT


# Constant string for v0 — every record in the W4b rollout writer
# carries it. Future teacher rollouts (Stage 5,
# ``posttraining-dataset.md``) flip this to ``"teacher-paraphrase"``.
INSTRUCTION_SOURCE_V0 = "bootstrap-handauthored"


# ---------------------------------------------------------------------------
# Result dataclasses.
# ---------------------------------------------------------------------------


@dataclass
class ScenarioRunResult:
    """One scenario's outcome: the mutated ``messages`` transcript,
    every per-turn ``TurnResult``, the verifier's grading, total
    wall-clock."""

    scenario: Scenario
    messages: list[dict[str, Any]]
    verifier_result: verifier.VerifierResult
    turns: list[TurnResult] = field(default_factory=list)
    wall_ms_total: int = 0


# ---------------------------------------------------------------------------
# Per-scenario loop.
# ---------------------------------------------------------------------------


def _append_stop(messages: list[dict[str, Any]], reason: str) -> None:
    """Append the synthetic ``"stop:<reason>"`` system message so the
    W3 verifier's ``_detect_driver_stop`` grades the rollout with the
    right driver-level ``failure_mode``."""
    messages.append({"role": "system", "content": f"stop:{reason}"})


def run_one_scenario(
    provider: LlmProvider,
    dispatcher_factory: Callable[[Scenario], Dispatcher],
    scenario: Scenario,
    tools: list[dict[str, Any]],
    config: EvalConfig,
    registry: Registry,
) -> ScenarioRunResult:
    """Drive one scenario end-to-end through ``harness.run_turn``.

    Mutates a fresh ``messages`` list that starts with the pinned
    ``developer`` system prompt and the scenario's ``user`` instruction.
    Stops on ``final_text`` (model done) or on the per-turn ``timeout``
    error from the harness. If the ``step_cap`` is exhausted without a
    ``final_text``, appends ``"stop:step_cap_hit"`` so the W3 verifier
    grades the rollout as ``step_cap_hit`` (driver-level stop dominates
    the failure label per ``verifier._detect_driver_stop``).
    """
    dispatcher = dispatcher_factory(scenario)

    try:
        messages: list[dict[str, Any]] = [
            {"role": "developer", "content": config.system_prompt},
            {"role": "user", "content": scenario.instruction},
        ]

        turns: list[TurnResult] = []
        start = time.monotonic()
        completed_with_final_text = False

        for step_index in range(config.step_cap):
            turn = harness.run_turn(
                provider,
                dispatcher,
                messages,
                tools,
                step_index=step_index,
                max_new_tokens=config.max_new_tokens,
                temperature=config.temperature,
                seed=config.seed,
                timeout_s=config.per_turn_timeout_s,
                registry=registry,
            )
            turns.append(turn)

            if turn.kind == "final_text":
                completed_with_final_text = True
                break
            if turn.kind == "error" and turn.error_kind == "timeout":
                _append_stop(messages, "timeout")
                break
            # tool_calls turn — keep looping.

        if not completed_with_final_text and not any(
            t.kind == "error" and t.error_kind == "timeout" for t in turns
        ):
            # Loop exhausted ``step_cap`` without ``final_text``.
            _append_stop(messages, "step_cap_hit")

        wall_ms_total = int((time.monotonic() - start) * 1000)

        vr = verifier.verify(messages, scenario.postcondition.to_json())
        return ScenarioRunResult(
            scenario=scenario,
            messages=messages,
            verifier_result=vr,
            turns=turns,
            wall_ms_total=wall_ms_total,
        )
    finally:
        # Best-effort dispatcher teardown — a live ``griz.Session``
        # leaks a process per scenario without this. Tests use
        # ``FakeDispatcher`` (no ``close`` method) so the getattr-guard
        # is a no-op for the always-on path.
        close = getattr(dispatcher, "close", None)
        if callable(close):
            try:
                close()
            except Exception:
                pass


# ---------------------------------------------------------------------------
# Rollout record writer — ``posttraining-dataset.md`` §1 shape.
# ---------------------------------------------------------------------------


def extract_tool_calls_flat(
    messages: list[dict[str, Any]],
) -> list[dict[str, Any]]:
    """Denormalize the assistant ``tool_calls`` slots into
    ``[{name, arguments: dict}, ...]`` in declared order across turns.

    The wire shape stores ``function.arguments`` as a JSON-encoded
    string (matches OpenAI / FunctionGemma). The dedup key
    (``posttraining-dataset.md`` §1, "tool_calls_flat") requires the
    *parsed* dict, not the string — so the SFT/DPO projection in
    Stage 6 can hash on structural equality. A slot whose
    ``arguments`` is already a dict (hand-fabricated rollouts in unit
    tests) is passed through unchanged.
    """
    out: list[dict[str, Any]] = []
    for msg in messages:
        if msg.get("role") != "assistant":
            continue
        for tc in msg.get("tool_calls") or []:
            fn = tc.get("function") or {}
            name = fn.get("name", "")
            raw_args = fn.get("arguments", {})
            if isinstance(raw_args, str):
                try:
                    parsed = json.loads(raw_args)
                except json.JSONDecodeError:
                    parsed = {}
                args = parsed if isinstance(parsed, dict) else {}
            elif isinstance(raw_args, dict):
                args = raw_args
            else:
                args = {}
            out.append({"name": name, "arguments": args})
    return out


def write_rollout_record(
    out: TextIO,
    scenario: Scenario,
    messages: list[dict[str, Any]],
    verifier_result: verifier.VerifierResult,
    tools: list[dict[str, Any]],
    config: EvalConfig,
    provider_meta: dict[str, Any],
) -> None:
    """Emit one JSONL line for ``rollouts.jsonl`` in the canonical
    ``posttraining-dataset.md`` §1 shape.

    ``provider_meta`` carries the provider name; ``config_hash`` is
    filled in from ``compute_system_prompt_hash`` if the caller did
    not supply one (the writer is responsible for the falsifiability
    pin — see module docstring).
    """
    meta = dict(provider_meta)
    meta.setdefault("config_hash", compute_system_prompt_hash(config.system_prompt))

    tool_names = sorted(t["name"] for t in tools)

    record: dict[str, Any] = {
        "id": scenario.id,
        "fixture": scenario.fixture,
        "intent_id": scenario.intent_id,
        "instruction": scenario.instruction,
        "instruction_source": INSTRUCTION_SOURCE_V0,
        "tools": tool_names,
        "messages": messages,
        "tool_calls_flat": extract_tool_calls_flat(messages),
        "verifier": {
            "max_tier": verifier_result.max_tier,
            "reward": verifier_result.reward,
            "failure_mode": verifier_result.failure_mode,
            "postcondition": scenario.postcondition.to_json(),
        },
        "provider": meta,
        "split": "eval",
    }
    out.write(json.dumps(record))
    out.write("\n")


# ---------------------------------------------------------------------------
# Summary writer.
# ---------------------------------------------------------------------------


def build_summary(
    scenario_results: list[ScenarioRunResult],
    config: EvalConfig,
) -> dict[str, Any]:
    """Compute the summary dict per baseline.md §"Acceptance gate" #6.

    Every entry in ``verifier.FAILURE_MODES`` is zero-initialized in
    ``by_failure_mode`` so "we don't know which mode dominates" is
    structurally impossible — the gate fails if a mode is missing,
    not if a mode is zero.

    JSON-friendly key types: ``by_max_tier`` keys are stringified
    ints (``"0"``..``"3"``) so the summary round-trips through
    ``json.dump`` / ``json.load`` without type drift.
    """
    by_max_tier: dict[str, int] = {str(t): 0 for t in range(4)}
    by_failure_mode: dict[str, int] = {m: 0 for m in verifier.FAILURE_MODES}

    total = len(scenario_results)
    l3_passes = 0
    total_wall_ms = 0
    total_turns = 0

    for sr in scenario_results:
        tier = sr.verifier_result.max_tier
        by_max_tier[str(tier)] = by_max_tier.get(str(tier), 0) + 1
        fm = sr.verifier_result.failure_mode
        if fm is not None:
            by_failure_mode[fm] = by_failure_mode.get(fm, 0) + 1
        if tier == 3:
            l3_passes += 1
        total_wall_ms += sr.wall_ms_total
        total_turns += len(sr.turns)

    l3_pass_rate = (l3_passes / total) if total else 0.0
    mean_turns = (total_turns / total) if total else 0.0

    return {
        "total": total,
        "by_max_tier": by_max_tier,
        "by_failure_mode": by_failure_mode,
        "l3_pass_rate": l3_pass_rate,
        "mean_turns_to_completion": mean_turns,
        "total_wall_ms": total_wall_ms,
        "config": {
            "step_cap": config.step_cap,
            "max_new_tokens": config.max_new_tokens,
            "temperature": config.temperature,
            "seed": config.seed,
            "per_turn_timeout_s": config.per_turn_timeout_s,
            "system_prompt_sha256": compute_system_prompt_hash(config.system_prompt),
        },
    }


def write_summary(
    path: Path,
    scenario_results: list[ScenarioRunResult],
    config: EvalConfig,
) -> dict[str, Any]:
    """Write the summary dict (pretty-printed JSON) and return it.

    Pretty-printing makes ``cat summary.json | less`` readable; the
    rollouts file stays one-record-per-line for streaming
    consumption.
    """
    summary = build_summary(scenario_results, config)
    Path(path).write_text(json.dumps(summary, indent=2) + "\n")
    return summary


# ---------------------------------------------------------------------------
# Top-level orchestrator.
# ---------------------------------------------------------------------------


def run_eval(
    scenarios: list[Scenario],
    provider_factory: Callable[[Scenario], LlmProvider],
    dispatcher_factory: Callable[[Scenario], Dispatcher],
    config: EvalConfig,
    out_dir: Path,
    *,
    provider_name: str = "unknown",
    registry: Registry | None = None,
    tools: list[dict[str, Any]] | None = None,
) -> dict[str, Any]:
    """Run every scenario in order; write ``rollouts.jsonl`` +
    ``summary.json`` under ``out_dir``; return the summary dict.

    The factories are the seam that keeps the always-on test path
    free of pygriz / live-LLM deps and lets PR-5's CLI swap in a
    real ``PygrizDispatcher`` per scenario (which opens the fixture
    before the harness loop) plus a live ``LlmProvider`` (FunctionGemma
    / Anthropic) without touching this orchestrator.

    Returns the summary dict so callers don't need to re-read
    ``summary.json`` from disk.
    """
    reg = registry if registry is not None else Registry.load_from_artifact()
    tool_list = tools if tools is not None else reg.all()

    out_path = Path(out_dir)
    out_path.mkdir(parents=True, exist_ok=True)
    rollouts_path = out_path / "rollouts.jsonl"
    summary_path = out_path / "summary.json"

    provider_meta_template = {
        "name": provider_name,
        "config_hash": compute_system_prompt_hash(config.system_prompt),
    }

    results: list[ScenarioRunResult] = []
    with rollouts_path.open("w") as rollouts_file:
        for scenario in scenarios:
            provider = provider_factory(scenario)
            sr = run_one_scenario(
                provider=provider,
                dispatcher_factory=dispatcher_factory,
                scenario=scenario,
                tools=tool_list,
                config=config,
                registry=reg,
            )
            results.append(sr)
            write_rollout_record(
                rollouts_file,
                scenario=scenario,
                messages=sr.messages,
                verifier_result=sr.verifier_result,
                tools=tool_list,
                config=config,
                provider_meta=dict(provider_meta_template),
            )

    summary = write_summary(summary_path, results, config)
    return summary


__all__ = [
    "EvalConfig",
    "INSTRUCTION_SOURCE_V0",
    "ScenarioRunResult",
    "build_summary",
    "compute_system_prompt_hash",
    "extract_tool_calls_flat",
    "run_eval",
    "run_one_scenario",
    "write_rollout_record",
    "write_summary",
]
