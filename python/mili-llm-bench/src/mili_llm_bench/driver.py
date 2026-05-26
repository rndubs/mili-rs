"""W4b — eval driver (loop on top of the harness, caps, rollout writer).

The v0 eval driver: per-scenario, builds a fresh dispatcher (via the
caller-supplied factory) + provider (likewise), runs ``harness.run_turn``
up to ``step_cap`` times, surfaces driver-level stops to the W3
verifier via the synthetic ``"stop:<reason>"`` ``system`` message
convention, calls ``verifier.verify``, and emits one canonical rollout
record per scenario in the
``planning/mili-viz/mili-agent/posttraining-dataset.md`` §1 shape.

Stage-5 surface (rev 11 — pilot teacher rollouts). The same ``run_eval``
loop doubles as the Stage 5 K-pass teacher-rollout writer: when ``k > 1``
each scenario is run K times with per-pass seed ``config.seed + k_idx``,
and every rollout is written to ``rollouts.jsonl`` with a ``k_idx`` +
``retained`` field. ``retain="passing"`` marks only ``max_tier == 3``
rollouts as retained (the Stage 6 SFT-corpus filter key);
``retain="all"`` retains every rollout. The summary aggregates
per-category Anthropic ``usage`` into ``cost_estimate_usd`` so the
$50 pilot budget gate is checkable without manual math. K=1 (the
default) preserves the bench-as-eval shape byte-for-byte — no
``k_idx`` / ``retained`` / ``usage`` keys land on those records.
See ``planning/mili-viz/mili-agent/m5-sft-pipeline.md`` Stage 5.

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
    "over the `griz_raw` fallback when a typed tool exists for the task.\n\n"
    "UNDERSTANDING TOOL RESPONSES:\n"
    "When a tool response includes 'action_complete': true, the action has succeeded and you should move on.\n"
    "For state-changing tools (set_state): compare 'requested_state' with 'state' to verify completion.\n"
    "Do not repeat the same tool call with identical arguments if you already received a successful response.\n"
    "Only call a tool again if you need to verify something or if the previous response indicated an error (ok: false).\n\n"
    "JSON TOOL CALL FORMAT (REQUIRED):\n"
    "Emit tool calls ONLY as valid JSON objects with 'name' and 'arguments' keys:\n"
    "{\"name\": \"tool_name\", \"arguments\": {\"param1\": value1, \"param2\": value2}}\n"
    "Do NOT wrap in markdown, comments, or extra text. Emit only the raw JSON object.\n"
    "Ensure all argument values match their expected types (strings quoted, numbers unquoted, booleans as true/false).\n\n"
    "KEY TOOL MAPPINGS:\n"
    "- Load/open a database: use `load` with root parameter (e.g., {\"name\": \"load\", \"arguments\": {\"root\": \"cylinder\"}})\n"
    "- Display/show/color a result: use `show` with result parameter (e.g., {\"name\": \"show\", \"arguments\": {\"result\": \"vx\"}})\n"
    "- Enable/disable materials: use `material` with enable (true/false) and material/class_name\n"
    "- Select elements: use `select` or `clrsel` (clear selection)\n"
    "- Change states: use `set_state` or `step`\n"
    "- Adjust view: use `colormap`, `view`, `named_view`, `legend`\n\n"
    "TASK COMPLETION:\n"
    "When you have completed ALL sub-tasks in the user's request, emit the final text message and STOP.\n"
    "Do not call extra verification tools. Do not loop. If no more actions are needed, just send the final message."
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
    # m7 Delta 3 — when False (the honest default), the driver loop
    # runs until the model emits final_text or the step_cap fires;
    # the postcondition oracle is *only* used for grading after the
    # rollout completes, never to short-circuit the loop. When True,
    # the legacy auto-terminate behavior is restored (the rollout
    # stops as soon as the verifier grades L3 mid-loop). Off by
    # default so bench numbers reflect live-UX behavior rather than
    # oracle-truncated tool-call sequences.
    allow_oracle_early_exit: bool = False


# Constant string for v0 — every record in the W4b rollout writer
# carries it. Future teacher rollouts (Stage 5,
# ``posttraining-dataset.md``) flip this to ``"teacher-paraphrase"``.
INSTRUCTION_SOURCE_V0 = "bootstrap-handauthored"


# Retain modes for Stage 5 K-pass rollouts. ``all`` (default) tags
# every rollout retained=True — used by bench-as-eval (K=1) and by
# "keep everything for inspection" sweeps. ``passing`` tags only
# rollouts whose verifier returns max_tier == 3 — used by Stage 5 so
# Stage 6's assembler filters on ``retained == True`` instead of
# re-grading. See ``m5-sft-pipeline.md`` Stage 5.
RETAIN_MODES: tuple[str, ...] = ("all", "passing")


# Claude Sonnet 4.5 pricing per million tokens (2026-05-24). Inputs:
# $3 / Mtok. Outputs: $15 / Mtok. Cache reads: $0.30 / Mtok (90% off
# inputs). Cache creation (5-min ephemeral): $3.75 / Mtok (25% premium
# over inputs). The cost estimator combines these to surface the
# $50-pilot / $200-full-sweep budget gates from the planning doc.
_PRICING_PER_MTOK_USD: dict[str, dict[str, float]] = {
    "claude-sonnet-4-5": {
        "input_tokens": 3.00,
        "output_tokens": 15.00,
        "cache_read_input_tokens": 0.30,
        "cache_creation_input_tokens": 3.75,
    },
}


def _usage_categories() -> tuple[str, ...]:
    return (
        "input_tokens",
        "output_tokens",
        "cache_read_input_tokens",
        "cache_creation_input_tokens",
    )


def estimate_cost_usd(usage_totals: dict[str, int], model_id: str) -> float:
    """Return the USD cost estimate for a usage totals dict against the
    pinned model's per-Mtok pricing. Unknown model ids return 0.0 — a
    deliberate silent zero so non-Anthropic providers (mock, llamacpp,
    transformers) report ``cost_estimate_usd == 0.0`` without a
    branching call site."""
    pricing = _PRICING_PER_MTOK_USD.get(model_id)
    if pricing is None:
        return 0.0
    total = 0.0
    for cat, per_mtok in pricing.items():
        total += float(usage_totals.get(cat, 0)) * (per_mtok / 1_000_000.0)
    return total


# ---------------------------------------------------------------------------
# Result dataclasses.
# ---------------------------------------------------------------------------


@dataclass
class ScenarioRunResult:
    """One scenario's outcome: the mutated ``messages`` transcript,
    every per-turn ``TurnResult``, the verifier's grading, total
    wall-clock.

    ``usage_sum`` is the per-category Anthropic token totals summed
    across all turns of this scenario, or ``None`` when no turn
    reported a usage dict (e.g. mock / replay / llamacpp). Stage 5
    cost telemetry aggregates these into the summary's
    ``cost_estimate_usd``.
    """

    scenario: Scenario
    messages: list[dict[str, Any]]
    verifier_result: verifier.VerifierResult
    turns: list[TurnResult] = field(default_factory=list)
    wall_ms_total: int = 0
    usage_sum: dict[str, int] | None = None


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
        terminated_cleanly = False
        postcondition = scenario.postcondition.to_json()

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
                terminated_cleanly = True
                break
            if turn.kind == "error" and turn.error_kind == "timeout":
                _append_stop(messages, "timeout")
                break
            # tool_calls turn — keep looping. m7 Delta 3: by default we
            # let the loop run to natural termination (final_text or
            # step_cap), since the live agent has no postcondition
            # oracle. The legacy auto-terminate behavior is preserved
            # behind ``EvalConfig.allow_oracle_early_exit`` for callers
            # that need to reproduce pre-M7 bench numbers. Under the
            # opt-in flag the loop uses the pre-M7 pc-satisfied
            # contract (``wrong_termination`` did not exist), so a
            # rollout that meets the postcondition but doesn't close
            # on content still short-circuits the loop.
            if config.allow_oracle_early_exit:
                vr_mid = verifier.verify(messages, postcondition)
                pre_m7_pc_satisfied = vr_mid.max_tier == 3 or (
                    vr_mid.max_tier == 2
                    and vr_mid.failure_mode == "wrong_termination"
                )
                if pre_m7_pc_satisfied:
                    terminated_cleanly = True
                    break

        if not terminated_cleanly and not any(
            t.kind == "error" and t.error_kind == "timeout" for t in turns
        ):
            # Loop exhausted ``step_cap`` without ``final_text``.
            _append_stop(messages, "step_cap_hit")

        wall_ms_total = int((time.monotonic() - start) * 1000)

        vr = verifier.verify(messages, scenario.postcondition.to_json())
        usage_sum: dict[str, int] | None = None
        for t in turns:
            if t.usage is None:
                continue
            if usage_sum is None:
                usage_sum = {k: 0 for k in _usage_categories()}
            for cat in _usage_categories():
                usage_sum[cat] = usage_sum.get(cat, 0) + int(t.usage.get(cat, 0))
        return ScenarioRunResult(
            scenario=scenario,
            messages=messages,
            verifier_result=vr,
            turns=turns,
            wall_ms_total=wall_ms_total,
            usage_sum=usage_sum,
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
    *,
    k_idx: int | None = None,
    retained: bool | None = None,
    usage: dict[str, int] | None = None,
) -> None:
    """Emit one JSONL line for ``rollouts.jsonl`` in the canonical
    ``posttraining-dataset.md`` §1 shape.

    ``provider_meta`` carries the provider name; ``config_hash`` is
    filled in from ``compute_system_prompt_hash`` if the caller did
    not supply one (the writer is responsible for the falsifiability
    pin — see module docstring).

    Stage-5 optional fields: ``k_idx`` (the 0-based rollout index when
    K > 1), ``retained`` (Stage 6's SFT-filter key — true when the
    verifier graded L3 under ``retain="passing"``, or always true
    under ``retain="all"``), and ``usage`` (per-category Anthropic
    token totals for this rollout). All three default to ``None`` so
    bench-as-eval records (K=1, mock/replay/local providers) stay
    byte-identical to pre-rev-11.
    """
    meta = dict(provider_meta)
    meta.setdefault("config_hash", compute_system_prompt_hash(config.system_prompt))

    tool_names = sorted(t["name"] for t in tools)

    record: dict[str, Any] = {
        "id": scenario.id,
        "fixture": scenario.fixture,
        "intent_id": scenario.intent_id,
        "instruction": scenario.instruction,
        "instruction_source": scenario.instruction_source or INSTRUCTION_SOURCE_V0,
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
    if k_idx is not None:
        record["k_idx"] = int(k_idx)
    if retained is not None:
        record["retained"] = bool(retained)
    if usage is not None:
        record["usage"] = dict(usage)
    out.write(json.dumps(record))
    out.write("\n")


# ---------------------------------------------------------------------------
# Summary writer.
# ---------------------------------------------------------------------------


def build_summary(
    scenario_results: list[ScenarioRunResult],
    config: EvalConfig,
    *,
    k: int = 1,
    retained_flags: list[bool] | None = None,
    scenario_ids: list[str] | None = None,
    intent_ids: list[str] | None = None,
    model_id: str | None = None,
) -> dict[str, Any]:
    """Compute the summary dict per baseline.md §"Acceptance gate" #6.

    Every entry in ``verifier.FAILURE_MODES`` is zero-initialized in
    ``by_failure_mode`` so "we don't know which mode dominates" is
    structurally impossible — the gate fails if a mode is missing,
    not if a mode is zero.

    JSON-friendly key types: ``by_max_tier`` keys are stringified
    ints (``"0"``..``"3"``) so the summary round-trips through
    ``json.dump`` / ``json.load`` without type drift.

    Stage-5 telemetry (``k``, ``retained_flags``, ``scenario_ids``,
    ``intent_ids``, ``model_id``). When ``k > 1``, the summary carries
    ``retention_rate`` (fraction of *scenarios* with ≥1 retained
    rollout) and ``retention_by_intent`` (the same fraction broken out
    by intent_id — the Stage 6 ≥40-row-per-intent gate input). When
    any scenario reports a ``usage_sum``, the summary aggregates them
    into ``usage_totals`` (per-category tokens) plus a
    ``cost_estimate_usd`` against ``model_id``'s per-Mtok pricing.
    Defaults preserve the bench-as-eval shape.
    """
    by_max_tier: dict[str, int] = {str(t): 0 for t in range(4)}
    by_failure_mode: dict[str, int] = {m: 0 for m in verifier.FAILURE_MODES}

    total = len(scenario_results)
    l3_passes = 0
    total_wall_ms = 0
    total_turns = 0
    usage_totals: dict[str, int] | None = None

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
        if sr.usage_sum is not None:
            if usage_totals is None:
                usage_totals = {k_: 0 for k_ in _usage_categories()}
            for cat in _usage_categories():
                usage_totals[cat] = usage_totals.get(cat, 0) + int(
                    sr.usage_sum.get(cat, 0)
                )

    l3_pass_rate = (l3_passes / total) if total else 0.0
    mean_turns = (total_turns / total) if total else 0.0

    out: dict[str, Any] = {
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
            "k": int(k),
        },
    }

    if k > 1 and retained_flags is not None and scenario_ids is not None:
        # The retention gate is per scenario, not per rollout — a scenario
        # whose K=3 rollouts include at least one passing trajectory counts
        # as retained for the SFT corpus.
        retained_per_scenario: dict[str, bool] = {}
        intent_by_scenario: dict[str, str] = {}
        ids_iter = scenario_ids
        intents_iter = intent_ids if intent_ids is not None else [""] * len(ids_iter)
        for sid, intent, flag in zip(ids_iter, intents_iter, retained_flags):
            retained_per_scenario[sid] = retained_per_scenario.get(sid, False) or bool(
                flag
            )
            intent_by_scenario.setdefault(sid, intent)
        n_scenarios = len(retained_per_scenario)
        n_retained = sum(1 for v in retained_per_scenario.values() if v)
        out["scenarios_total"] = n_scenarios
        out["scenarios_retained"] = n_retained
        out["retention_rate"] = (n_retained / n_scenarios) if n_scenarios else 0.0
        by_intent: dict[str, dict[str, int]] = {}
        for sid, was_retained in retained_per_scenario.items():
            intent = intent_by_scenario.get(sid, "<unknown>")
            slot = by_intent.setdefault(intent, {"count": 0, "retained": 0})
            slot["count"] += 1
            if was_retained:
                slot["retained"] += 1
        out["retention_by_intent"] = {
            intent: {
                "count": s["count"],
                "retained": s["retained"],
                "rate": (s["retained"] / s["count"]) if s["count"] else 0.0,
            }
            for intent, s in sorted(by_intent.items())
        }

    if usage_totals is not None:
        out["usage_totals"] = usage_totals
        out["cost_estimate_usd"] = estimate_cost_usd(usage_totals, model_id or "")

    return out


def write_summary(
    path: Path,
    scenario_results: list[ScenarioRunResult],
    config: EvalConfig,
    *,
    k: int = 1,
    retained_flags: list[bool] | None = None,
    scenario_ids: list[str] | None = None,
    intent_ids: list[str] | None = None,
    model_id: str | None = None,
) -> dict[str, Any]:
    """Write the summary dict (pretty-printed JSON) and return it.

    Pretty-printing makes ``cat summary.json | less`` readable; the
    rollouts file stays one-record-per-line for streaming
    consumption.
    """
    summary = build_summary(
        scenario_results,
        config,
        k=k,
        retained_flags=retained_flags,
        scenario_ids=scenario_ids,
        intent_ids=intent_ids,
        model_id=model_id,
    )
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
    k: int = 1,
    retain: str = "all",
    model_id: str | None = None,
) -> dict[str, Any]:
    """Run every scenario in order; write ``rollouts.jsonl`` +
    ``summary.json`` under ``out_dir``; return the summary dict.

    The factories are the seam that keeps the always-on test path
    free of pygriz / live-LLM deps and lets PR-5's CLI swap in a
    real ``PygrizDispatcher`` per scenario (which opens the fixture
    before the harness loop) plus a live ``LlmProvider`` (FunctionGemma
    / Anthropic) without touching this orchestrator.

    Stage-5 K-pass surface (``k``, ``retain``, ``model_id``). When
    ``k > 1`` each scenario is run K times with per-pass seed
    ``config.seed + k_idx``; each rollout is written separately with a
    ``k_idx`` and ``retained`` field. ``retain="passing"`` marks only
    L3 rollouts as retained (Stage 6's SFT-corpus filter key);
    ``retain="all"`` marks every rollout retained. The summary
    aggregates per-category Anthropic token usage into
    ``cost_estimate_usd`` when ``model_id`` matches a pinned-pricing
    entry. K=1 (default) preserves the bench-as-eval shape byte-for-byte.

    Returns the summary dict so callers don't need to re-read
    ``summary.json`` from disk.
    """
    if retain not in RETAIN_MODES:
        raise ValueError(
            f"unknown retain={retain!r}; expected one of {RETAIN_MODES}"
        )
    if k < 1:
        raise ValueError(f"k must be >= 1, got {k!r}")

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
    retained_flags: list[bool] = []
    scenario_ids: list[str] = []
    intent_ids: list[str] = []
    from dataclasses import replace as _dc_replace
    from tqdm import tqdm

    total_rollouts = len(scenarios) * k

    with rollouts_path.open("w") as rollouts_file:
        pbar = tqdm(total=total_rollouts, desc="Running rollouts", unit="rollout")
        for scenario in scenarios:
            provider = provider_factory(scenario)
            scenario_retained_any = False
            for k_idx in range(k):
                per_pass_config = (
                    _dc_replace(config, seed=config.seed + k_idx) if k > 1 else config
                )
                sr = run_one_scenario(
                    provider=provider,
                    dispatcher_factory=dispatcher_factory,
                    scenario=scenario,
                    tools=tool_list,
                    config=per_pass_config,
                    registry=reg,
                )
                results.append(sr)
                # Retention: under ``passing`` only L3 rollouts qualify
                # for the Stage 6 SFT corpus; under ``all`` (the
                # bench-as-eval default) every rollout is retained.
                this_retained = (
                    retain == "all" or sr.verifier_result.max_tier == 3
                )
                if k > 1:
                    write_rollout_record(
                        rollouts_file,
                        scenario=scenario,
                        messages=sr.messages,
                        verifier_result=sr.verifier_result,
                        tools=tool_list,
                        config=per_pass_config,
                        provider_meta=dict(provider_meta_template),
                        k_idx=k_idx,
                        retained=this_retained,
                        usage=sr.usage_sum,
                    )
                else:
                    # K=1: preserve the pre-rev-11 record shape exactly
                    # (no k_idx / retained / usage keys).
                    write_rollout_record(
                        rollouts_file,
                        scenario=scenario,
                        messages=sr.messages,
                        verifier_result=sr.verifier_result,
                        tools=tool_list,
                        config=per_pass_config,
                        provider_meta=dict(provider_meta_template),
                    )
                rollouts_file.flush()

                retained_flags.append(this_retained)
                scenario_ids.append(scenario.id)
                intent_ids.append(scenario.intent_id)
                scenario_retained_any = scenario_retained_any or this_retained

                l3_count = sum(1 for r in results if r.verifier_result.max_tier == 3)
                pbar.update(1)
                pbar.set_postfix(
                    {
                        "last": f"{scenario.id}/k{k_idx}:{sr.verifier_result.failure_mode}",
                        "L3": f"{l3_count}/{len(results)}",
                    }
                )
        pbar.close()

    summary = write_summary(
        summary_path,
        results,
        config,
        k=k,
        retained_flags=retained_flags if k > 1 else None,
        scenario_ids=scenario_ids if k > 1 else None,
        intent_ids=intent_ids if k > 1 else None,
        model_id=model_id,
    )
    return summary


__all__ = [
    "EvalConfig",
    "INSTRUCTION_SOURCE_V0",
    "RETAIN_MODES",
    "ScenarioRunResult",
    "build_summary",
    "compute_system_prompt_hash",
    "estimate_cost_usd",
    "extract_tool_calls_flat",
    "run_eval",
    "run_one_scenario",
    "write_rollout_record",
    "write_summary",
]
