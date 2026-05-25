"""Round-trip tests for Stage 3 scenario synthesis.

The synthesis pipeline writes JSONL records that must (a) parse cleanly
through ``scenarios.load_scenarios`` and (b) be dispatchable through
the verifier's closed-kind handler set without raising. Drift in either
contract is the kind of bug that produces a green run today and a 0%
L3 surprise during Stage 5 — pin both here.

Also pins:
* the compound-ratio gate (≥20% per m5-sft-pipeline.md
  "Multi-step tool calls — first-class category" point 3),
* a sane total count (the v1 pilot scope target is ~200), and
* the absence of duplicate ``(instruction, fixture, intent_id)``
  triples in a single seed run.

Tests use a deterministic stub query oracle so the round-trip path is
pygriz-free and runs in the always-on test path.
"""

from __future__ import annotations

import json
from pathlib import Path
from typing import Any

import pytest

from mili_llm_bench import verifier as _verifier_mod
from mili_llm_bench.scenarios import VALID_POSTCONDITION_KINDS, load_scenarios
from mili_llm_bench.synth import run_synth


CATALOG_RELATIVE = Path("data/posttraining/intents/catalog.yaml")


def _repo_root() -> Path:
    here = Path(__file__).resolve()
    for parent in here.parents:
        if (parent / "CLAUDE.md").exists():
            return parent
    raise FileNotFoundError("could not locate repo root from tests")


def _stub_query_oracle(_fixture: Any, bound: dict[str, Any]) -> dict[str, Any]:
    """Deterministic stand-in for the live pygriz capture.

    Returns a shape-correct, content-deterministic ``table`` dict so
    every synthesized ``query`` row round-trips through the loader and
    the verifier handler without raising. The values do NOT pretend to
    be the parity-suite answers — Stage 6.5 (Claude data-quality gate)
    is where live rollouts will compare against real fixtures.
    """
    return {
        "result": bound["result_name"],
        "class_name": bound["class"],
        "labels": list(bound["labels"]),
        "states": list(bound["states"]),
        "values": [[0.0 for _ in bound["labels"]]],
    }


@pytest.fixture(scope="module")
def synth_run(tmp_path_factory: pytest.TempPathFactory) -> tuple[Path, Any]:
    out_dir = tmp_path_factory.mktemp("synth_round_trip")
    out_path = out_dir / "synth.jsonl"
    report_path = out_dir / "synth.report.md"
    report = run_synth(
        catalog_path=_repo_root() / CATALOG_RELATIVE,
        out_path=out_path,
        report_path=report_path,
        seed=42,
        query_oracle=_stub_query_oracle,
        confirm_fixtures=False,
    )
    return out_path, report


def test_synth_jsonl_round_trips_through_load_scenarios(synth_run):
    out_path, _report = synth_run
    scenarios = load_scenarios(out_path)
    # Every emitted row parses cleanly. ``load_scenarios`` already
    # rejects unknown postcondition kinds; this assertion just pins
    # the count > 0 contract so an empty file doesn't quietly pass.
    assert len(scenarios) > 0


def test_every_postcondition_kind_is_in_closed_set(synth_run):
    out_path, _report = synth_run
    for s in load_scenarios(out_path):
        assert s.postcondition.kind in VALID_POSTCONDITION_KINDS, (
            f"{s.id} has out-of-set kind {s.postcondition.kind!r}"
        )


def test_every_record_passes_verifier_handler_smoke(synth_run):
    out_path, _report = synth_run
    for s in load_scenarios(out_path):
        handler = _verifier_mod._PC_HANDLERS[s.postcondition.kind]
        # Empty calls — only assert the handler accepts the expect
        # shape. Real grading happens at Stage 5 / Stage 7.
        handler(s.postcondition.expect, [])


def test_compound_ratio_meets_gate(synth_run):
    _out_path, report = synth_run
    assert report.total > 0
    assert report.compound_ratio >= 0.20, (
        f"compound ratio {report.compound_ratio:.2%} below 20% gate "
        f"({report.compound_count}/{report.total})"
    )


def test_total_in_target_window(synth_run):
    _out_path, report = synth_run
    assert 160 <= report.total <= 240, (
        f"total {report.total} outside the v1-pilot window [160, 240]"
    )


def test_no_duplicate_instruction_fixture_intent_triples(synth_run):
    out_path, _report = synth_run
    seen: set[tuple[str, str, str]] = set()
    for s in load_scenarios(out_path):
        key = (s.instruction, s.fixture, s.intent_id)
        assert key not in seen, (
            f"duplicate (instruction, fixture, intent_id) at {s.id}: {key}"
        )
        seen.add(key)


def test_instruction_source_tagged_for_every_record(synth_run):
    out_path, _report = synth_run
    with out_path.open() as f:
        for line in f:
            rec = json.loads(line)
            src = rec.get("instruction_source")
            assert src in ("template", "manual-paraphrase"), (
                f"{rec['id']} has unexpected instruction_source {src!r}"
            )


def test_deterministic_at_fixed_seed(tmp_path: Path):
    out_a = tmp_path / "a.jsonl"
    out_b = tmp_path / "b.jsonl"
    for out in (out_a, out_b):
        run_synth(
            catalog_path=_repo_root() / CATALOG_RELATIVE,
            out_path=out,
            report_path=None,
            seed=42,
            query_oracle=_stub_query_oracle,
            confirm_fixtures=False,
        )
    assert out_a.read_text() == out_b.read_text()
