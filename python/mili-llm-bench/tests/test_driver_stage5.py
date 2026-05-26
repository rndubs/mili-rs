"""Stage 5 — K-pass teacher-rollout pins (rev 11).

Always-on (no LLM, no GPU, no network). Mocks the Anthropic SDK
transport via a fake client that records every request; the K-pass
fan-out + retention filter + per-K seed plumbing is exercised through
the public ``run_eval`` surface, not via internal helpers, so a future
refactor that preserves the contract still passes.

Pin set (matches the rev-11 acceptance gate in
``planning/mili-viz/mili-agent/m5-sft-pipeline.md`` Stage 5):

1. K-pass fan-out — ``--k 3`` writes 3 rollouts per scenario into
   ``rollouts.jsonl`` with monotonically increasing ``k_idx``.
2. Retention filter — under ``retain="passing"`` only L3 rollouts
   carry ``retained=True``; non-L3 rollouts carry ``retained=False``
   (the SFT-corpus filter key Stage 6 reads).
3. Per-K seed — the per-pass seed forwarded to ``provider.generate``
   strictly differs across K rollouts of the same scenario
   (``seed = config.seed + k_idx``).

A fourth pin asserts the Anthropic cost-estimate math against pinned
Sonnet 4.5 prices, because the $50 pilot-budget gate is meaningless
without it.
"""

from __future__ import annotations

import json
from dataclasses import dataclass, field
from pathlib import Path
from typing import Any

import pytest

from mili_llm_bench.driver import (
    EvalConfig,
    estimate_cost_usd,
    run_eval,
)
from mili_llm_bench.harness import FakeDispatcher, Registry
from mili_llm_bench.providers.base import LlmProvider, ProviderOutput
from mili_llm_bench.scenarios import Postcondition, Scenario


_REGISTRY = Registry.load_from_artifact()
_TOOLS_LIST = _REGISTRY.all()


# ---------------------------------------------------------------------------
# Test seam — a provider that records every (per-pass) seed it sees and
# returns a scripted ProviderOutput list so we can pin K-pass fan-out
# without burning real Anthropic API tokens.
# ---------------------------------------------------------------------------


@dataclass
class _SeedRecordingProvider:
    """A scripted provider that records every (seed, scenario_id) pair
    its generate() is called with. Used to pin the per-K seed plumbing
    without standing up the live Anthropic client."""

    outputs: list[ProviderOutput]
    seeds_seen: list[int] = field(default_factory=list)
    _idx: int = 0

    def generate(
        self,
        messages: list[dict[str, Any]],
        tools: list[dict[str, Any]],
        *,
        temperature: float,
        max_new_tokens: int,
        seed: int,
    ) -> ProviderOutput:
        self.seeds_seen.append(seed)
        out = self.outputs[self._idx % len(self.outputs)]
        self._idx += 1
        return out


def _load_response_for(fixture: str) -> dict[str, Any]:
    return {
        "ok": True,
        "num_states": 101,
        "num_classes": 7,
        "classes": ["glob", "mat", "node", "beam", "brick", "shell", "cseg"],
        "state_time_range": [0.0, 1.0],
        "current_time": 0.0,
    }


def _loader_dispatcher_factory(_scenario: Scenario) -> FakeDispatcher:
    def handler(name: str, arguments: dict[str, Any]) -> dict[str, Any]:
        if name == "load":
            return _load_response_for(arguments.get("root", ""))
        return {"ok": True}

    return FakeDispatcher(handler=handler)


def _load_scenario(sid: str = "test-load-0") -> Scenario:
    return Scenario(
        id=sid,
        fixture="d3samp6",
        intent_id="load",
        instruction="load the d3samp6 database",
        postcondition=Postcondition(kind="state_index", expect={"state": 1}),
    )


# ---------------------------------------------------------------------------
# 1. K-pass fan-out — K=3 writes 3 rollouts per scenario with
#    monotonically increasing k_idx.
# ---------------------------------------------------------------------------


def test_run_eval_k_pass_writes_k_rollouts_per_scenario(tmp_path: Path) -> None:
    """Stage 5's primary contract: K=3 against 2 scenarios produces
    6 rollouts in rollouts.jsonl, grouped by scenario in input order,
    with k_idx ∈ {0, 1, 2} within each group."""
    scenarios = [
        _load_scenario("sc-A"),
        _load_scenario("sc-B"),
    ]
    passing = ProviderOutput(
        tool_calls=[{"name": "load", "arguments": {"root": "d3samp6"}}]
    )

    # Reuse one provider across scenarios (mirrors AnthropicProvider in
    # production — the SDK client is created once).
    provider = _SeedRecordingProvider(outputs=[passing])

    out_dir = tmp_path / "k3-fanout"
    run_eval(
        scenarios,
        provider_factory=lambda _s: provider,
        dispatcher_factory=_loader_dispatcher_factory,
        # m7 Delta 3 — Stage 5 unit tests scripted around the
        # auto-terminate mock pathway; opt back into the oracle so the
        # retention mechanics under test are preserved without giving
        # every mock script an explicit final_text turn.
        config=EvalConfig(allow_oracle_early_exit=True),
        out_dir=out_dir,
        provider_name="mock",
        registry=_REGISTRY,
        tools=_TOOLS_LIST,
        k=3,
        retain="all",
    )

    rollouts = out_dir / "rollouts.jsonl"
    lines = rollouts.read_text().splitlines()
    assert len(lines) == 6, "K=3 × 2 scenarios = 6 rollouts"
    records = [json.loads(line) for line in lines]

    # Records grouped by scenario, K rollouts each, in input order.
    assert [r["id"] for r in records] == ["sc-A", "sc-A", "sc-A", "sc-B", "sc-B", "sc-B"]
    # k_idx monotonically increases within each scenario group.
    assert [r["k_idx"] for r in records] == [0, 1, 2, 0, 1, 2]
    # Every K>1 rollout carries the retention key.
    for r in records:
        assert "retained" in r
        assert isinstance(r["retained"], bool)


# ---------------------------------------------------------------------------
# 2. Retention filter — under retain="passing" only L3 rollouts are
#    marked retained=True; failed rollouts carry retained=False.
# ---------------------------------------------------------------------------


def test_run_eval_retention_filters_by_l3_verdict_under_passing(
    tmp_path: Path,
) -> None:
    """Stage 6 reads ``retained == True`` as the SFT-corpus filter key.
    Under ``retain="passing"`` a rollout that grades L3 is retained;
    one that fails (here: parse_error from a malformed tool call) is
    not. We script K=2 against one scenario where the first pass
    passes and the second deliberately fails, then assert the
    retention pattern."""
    scenario = _load_scenario("sc-mixed")
    passing_call = ProviderOutput(
        tool_calls=[{"name": "load", "arguments": {"root": "d3samp6"}}]
    )
    # A non-canonical tool call (arguments not a dict) → harness routes
    # it to <parse_error>, dispatch fails, postcondition unmet → L0.
    failing_call = ProviderOutput(
        tool_calls=[{"name": "load", "arguments": "not-a-dict"}]  # type: ignore[arg-type]
    )
    # m7 Delta 1/2 — the rollout must close on a content-only
    # assistant message for the verifier to award L3, so the first
    # pass's second turn emits a short ack.
    final_text_call = ProviderOutput(final_text="done.")

    @dataclass
    class _AlternatingProvider:
        _pass_idx: int = 0

        def generate(
            self,
            messages: list[dict[str, Any]],
            tools: list[dict[str, Any]],
            *,
            temperature: float,
            max_new_tokens: int,
            seed: int,
        ) -> ProviderOutput:
            # When the model has already responded to a tool call in
            # this pass, emit the terminating ack so the pass closes
            # cleanly.
            has_tool_response = any(m.get("role") == "tool" for m in messages)
            if has_tool_response:
                self._pass_idx += 1
                return final_text_call
            return passing_call if self._pass_idx == 0 else failing_call

    provider = _AlternatingProvider()

    out_dir = tmp_path / "retention"
    summary = run_eval(
        [scenario],
        provider_factory=lambda _s: provider,
        dispatcher_factory=_loader_dispatcher_factory,
        # m7 Delta 3 — natural-termination default. The mock provider
        # above scripts an explicit final_text turn after the tool
        # response so the passing pass grades L3 under the strict
        # verifier.
        config=EvalConfig(),
        out_dir=out_dir,
        provider_name="mock",
        registry=_REGISTRY,
        tools=_TOOLS_LIST,
        k=2,
        retain="passing",
    )

    records = [json.loads(l) for l in (out_dir / "rollouts.jsonl").read_text().splitlines()]
    assert len(records) == 2
    # Pass k=0, fail k=1.
    by_k = {r["k_idx"]: r for r in records}
    assert by_k[0]["verifier"]["max_tier"] == 3
    assert by_k[0]["retained"] is True
    assert by_k[1]["verifier"]["max_tier"] < 3
    assert by_k[1]["retained"] is False
    # Scenario-level retention rate = 1/1 (the scenario has ≥1 passing
    # rollout — Stage 6's "≥1 retained rollout per scenario" gate).
    assert summary["scenarios_retained"] == 1
    assert summary["scenarios_total"] == 1
    assert summary["retention_rate"] == 1.0


# ---------------------------------------------------------------------------
# 3. Per-K seed — seed = config.seed + k_idx, strictly distinct across
#    K rollouts of the same scenario.
# ---------------------------------------------------------------------------


def test_run_eval_per_k_seed_differs_across_passes(tmp_path: Path) -> None:
    """The per-pass seed forwarded to the provider equals
    ``config.seed + k_idx``. Even though Anthropic itself ignores the
    seed parameter, the plumbing must thread distinct seeds so the
    rollout artifacts are traceable and a future seedable provider
    inherits diversity for free."""
    scenarios = [_load_scenario("sc-seed")]
    passing = ProviderOutput(
        tool_calls=[{"name": "load", "arguments": {"root": "d3samp6"}}]
    )
    provider = _SeedRecordingProvider(outputs=[passing])

    out_dir = tmp_path / "per-k-seed"
    run_eval(
        scenarios,
        provider_factory=lambda _s: provider,
        dispatcher_factory=_loader_dispatcher_factory,
        config=EvalConfig(seed=42, allow_oracle_early_exit=True),
        out_dir=out_dir,
        provider_name="mock",
        registry=_REGISTRY,
        tools=_TOOLS_LIST,
        k=3,
        retain="all",
    )

    # Three generate() calls (K=3 × 1 scenario), each with a distinct
    # seed = 42 + k_idx, in declared order.
    assert provider.seeds_seen == [42, 43, 44]
    assert len(set(provider.seeds_seen)) == 3


# ---------------------------------------------------------------------------
# 4. Cost-estimate math — pinned Sonnet 4.5 prices ($3/Mtok input,
#    $15/Mtok output, $0.30/Mtok cache_read, $3.75/Mtok cache_creation).
#    The $50 pilot-budget gate depends on this being exact.
# ---------------------------------------------------------------------------


def test_estimate_cost_usd_matches_pinned_sonnet_pricing() -> None:
    """1 M input + 1 M output + 1 M cache_read + 1 M cache_creation
    tokens at Sonnet 4.5 pricing = $3 + $15 + $0.30 + $3.75 = $22.05.
    Unknown model ids return 0.0 (silent — non-Anthropic providers
    don't have cost telemetry)."""
    usage = {
        "input_tokens": 1_000_000,
        "output_tokens": 1_000_000,
        "cache_read_input_tokens": 1_000_000,
        "cache_creation_input_tokens": 1_000_000,
    }
    cost = estimate_cost_usd(usage, "claude-sonnet-4-5")
    assert cost == pytest.approx(3.00 + 15.00 + 0.30 + 3.75)

    # Unknown model id is a silent zero, not an error.
    assert estimate_cost_usd(usage, "functiongemma-270m") == 0.0
    assert estimate_cost_usd(usage, "") == 0.0

    # Zero usage → zero cost, regardless of model.
    assert (
        estimate_cost_usd(
            {k: 0 for k in usage}, "claude-sonnet-4-5"
        )
        == 0.0
    )


def test_run_eval_k_eq_1_preserves_pre_rev11_record_shape(tmp_path: Path) -> None:
    """The K=1 path must keep the rollout record byte-identical to
    pre-rev-11 — no k_idx, retained, or usage keys land on those
    records. Stage 6 / replay / report assume that shape; rev-11 is
    additive only."""
    scenarios = [_load_scenario("sc-k1")]
    passing = ProviderOutput(
        tool_calls=[{"name": "load", "arguments": {"root": "d3samp6"}}]
    )
    provider = _SeedRecordingProvider(outputs=[passing])

    out_dir = tmp_path / "k1-shape"
    run_eval(
        scenarios,
        provider_factory=lambda _s: provider,
        dispatcher_factory=_loader_dispatcher_factory,
        # m7 Delta 3 — Stage 5 unit tests scripted around the
        # auto-terminate mock pathway; opt back into the oracle so the
        # retention mechanics under test are preserved without giving
        # every mock script an explicit final_text turn.
        config=EvalConfig(allow_oracle_early_exit=True),
        out_dir=out_dir,
        provider_name="mock",
        registry=_REGISTRY,
        tools=_TOOLS_LIST,
        k=1,
        retain="all",
    )
    records = [
        json.loads(l)
        for l in (out_dir / "rollouts.jsonl").read_text().splitlines()
    ]
    assert len(records) == 1
    r = records[0]
    assert "k_idx" not in r
    assert "retained" not in r
    assert "usage" not in r
