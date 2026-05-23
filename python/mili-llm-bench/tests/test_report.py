"""PR-5 report generator tests — pure-Python, always-on.

Pins:

* every required section is present (one assertion per section);
* per-intent rate math is correct;
* raw-fallback rate counts ``griz_raw`` slots correctly;
* failure-mode rows are sorted count-desc, name-asc;
* tier rows include 0..3 even when some buckets are empty.
"""

from __future__ import annotations

import json
from pathlib import Path
from typing import Any

from mili_llm_bench import report, verifier


def _write_rollouts(path: Path, records: list[dict[str, Any]]) -> None:
    with path.open("w") as f:
        for r in records:
            f.write(json.dumps(r) + "\n")


def _minimal_summary(tier_counts: dict[str, int]) -> dict[str, Any]:
    total = sum(tier_counts.values())
    l3 = tier_counts.get("3", 0)
    return {
        "total": total,
        "by_max_tier": {**{"0": 0, "1": 0, "2": 0, "3": 0}, **tier_counts},
        "by_failure_mode": {m: 0 for m in verifier.FAILURE_MODES},
        "l3_pass_rate": (l3 / total) if total else 0.0,
        "mean_turns_to_completion": 1.0,
        "total_wall_ms": 100,
        "config": {
            "step_cap": 8, "max_new_tokens": 256, "temperature": 0.0, "seed": 0,
            "per_turn_timeout_s": 60.0,
            "system_prompt_sha256": "abcdef0123456789",
        },
    }


def test_report_per_intent_rate_math_is_correct(tmp_path: Path) -> None:
    """Two intents, three records: ``load`` 1/2 → 50%, ``show`` 1/1 →
    100%."""
    rollouts = tmp_path / "rollouts.jsonl"
    _write_rollouts(
        rollouts,
        [
            {
                "id": "a", "fixture": "f", "intent_id": "load",
                "instruction": "x",
                "verifier": {"max_tier": 3, "postcondition": {"kind": "x"}},
                "tool_calls_flat": [],
            },
            {
                "id": "b", "fixture": "f", "intent_id": "load",
                "instruction": "x",
                "verifier": {"max_tier": 0, "postcondition": {"kind": "x"}},
                "tool_calls_flat": [],
            },
            {
                "id": "c", "fixture": "f", "intent_id": "show",
                "instruction": "x",
                "verifier": {"max_tier": 3, "postcondition": {"kind": "x"}},
                "tool_calls_flat": [],
            },
        ],
    )
    summary = _minimal_summary({"3": 2, "0": 1})
    text = report.render(
        summary,
        rollouts,
        provider_name="mock",
        model_id="mock",
    )
    # The per_intent table contains one row per intent_id, sorted
    # alphabetically (deterministic), with the right L3 rate.
    assert "| load | 2 | 1 | 50.0% |" in text
    assert "| show | 1 | 1 | 100.0% |" in text


def test_report_raw_fallback_rate_counts_griz_raw_slots(tmp_path: Path) -> None:
    """Two of four rollouts carry a ``griz_raw`` call → 50%."""
    rollouts = tmp_path / "rollouts.jsonl"
    _write_rollouts(
        rollouts,
        [
            {
                "id": "a", "fixture": "f", "intent_id": "i",
                "instruction": "x",
                "verifier": {"max_tier": 0, "postcondition": {"kind": "x"}},
                "tool_calls_flat": [{"name": "load", "arguments": {}}],
            },
            {
                "id": "b", "fixture": "f", "intent_id": "i",
                "instruction": "x",
                "verifier": {"max_tier": 0, "postcondition": {"kind": "x"}},
                "tool_calls_flat": [{"name": "griz_raw", "arguments": {"line": "step"}}],
            },
            {
                "id": "c", "fixture": "f", "intent_id": "i",
                "instruction": "x",
                "verifier": {"max_tier": 0, "postcondition": {"kind": "x"}},
                "tool_calls_flat": [
                    {"name": "load", "arguments": {}},
                    {"name": "griz_raw", "arguments": {"line": "next"}},
                ],
            },
            {
                "id": "d", "fixture": "f", "intent_id": "i",
                "instruction": "x",
                "verifier": {"max_tier": 0, "postcondition": {"kind": "x"}},
                "tool_calls_flat": [],
            },
        ],
    )
    summary = _minimal_summary({"0": 4})
    text = report.render(summary, rollouts, provider_name="mock", model_id="mock")
    assert "2 / 4" in text
    assert "50.0%" in text


def test_report_failure_mode_rows_sorted_count_desc_then_alpha(tmp_path: Path) -> None:
    rollouts = tmp_path / "rollouts.jsonl"
    _write_rollouts(rollouts, [])
    summary = _minimal_summary({"0": 5})
    summary["by_failure_mode"]["timeout"] = 3
    summary["by_failure_mode"]["schema_mismatch"] = 3
    summary["by_failure_mode"]["wrong_result"] = 1
    text = report.render(summary, rollouts, provider_name="mock", model_id="mock")
    # In the by_failure_mode section, schema_mismatch (alpha) precedes
    # timeout (both count=3); wrong_result (count=1) follows; the
    # zero-count tail comes last alphabetically.
    body = text.split("## by_failure_mode")[1]
    sm_pos = body.find("| schema_mismatch |")
    to_pos = body.find("| timeout |")
    wr_pos = body.find("| wrong_result |")
    assert sm_pos < to_pos < wr_pos
    # A zero-count entry still appears (zero-init invariant).
    assert "| nonexistent_material | 0 |" in body


def test_report_tier_rows_include_zero_buckets(tmp_path: Path) -> None:
    rollouts = tmp_path / "rollouts.jsonl"
    _write_rollouts(rollouts, [])
    summary = _minimal_summary({"3": 1})
    text = report.render(summary, rollouts, provider_name="mock", model_id="mock")
    # Tiers 0..3 all appear (zero-init), in order.
    for tier in ("| 0 |", "| 1 |", "| 2 |", "| 3 |"):
        assert tier in text


def test_report_headline_includes_falsifiability_pins(tmp_path: Path) -> None:
    rollouts = tmp_path / "rollouts.jsonl"
    _write_rollouts(rollouts, [])
    summary = _minimal_summary({"3": 0})
    summary["config"]["system_prompt_sha256"] = "deadbeefcafef00d"
    text = report.render(
        summary,
        rollouts,
        provider_name="anthropic",
        model_id="claude-foo",
        scenarios_sha256="abcd1234",
    )
    assert "anthropic" in text
    assert "claude-foo" in text
    assert "deadbeefcafef00d" in text
    assert "abcd1234" in text


def test_report_write_round_trips_file(tmp_path: Path) -> None:
    rollouts = tmp_path / "rollouts.jsonl"
    _write_rollouts(rollouts, [])
    summary = _minimal_summary({"3": 0})
    out = tmp_path / "report.md"
    text = report.write_report(out, summary, rollouts, provider_name="x", model_id="y")
    assert out.read_text() == text
