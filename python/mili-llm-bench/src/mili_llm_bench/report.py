"""W6 — ``report.md`` generator.

Pure-Python, no Jinja / no MD library — one f-string template per
section. The report is the human-facing surface that the post-v0
decision tree
(``planning/mili-viz/agent-local-llm-baseline.md`` §"After v0") branches
on: failure-mode breakdown + L0/L1 vs L3 split + per-intent breakdown
are the inputs, not a courtesy.

Required sections (pinned by ``test_report.py``):

* Headline — provider name, model id, L3 pass rate, the
  ``system_prompt_sha256`` + ``scenarios.jsonl`` sha so the number is
  unambiguous on sight.
* ``by_max_tier`` — counts per tier 0..3.
* ``by_failure_mode`` — counts per closed FAILURE_MODES entry; sorted
  by count descending, then alphabetically (deterministic for the
  zero-count tail).
* Mean turns to completion + total wall clock.
* Per-intent breakdown — count of scenarios per intent_id + L3 pass
  rate per intent_id. Post-v0 the decision tree gates on this.
* Raw-fallback rate — fraction of rollouts whose ``tool_calls_flat``
  contains at least one ``griz_raw`` entry. The open Q in
  baseline.md §"Open questions" calls this out as a visible metric.
* Pointer block — paths to ``rollouts.jsonl`` + ``config.yaml`` +
  ``summary.json`` so the operator can re-grade / replay.
"""

from __future__ import annotations

import json
from pathlib import Path
from typing import Any


# ---------------------------------------------------------------------------
# Per-section helpers (pure functions; the writer composes them).
# ---------------------------------------------------------------------------


def _read_rollouts(rollouts_path: Path) -> list[dict[str, Any]]:
    records: list[dict[str, Any]] = []
    with Path(rollouts_path).open() as f:
        for line in f:
            line = line.strip()
            if not line:
                continue
            records.append(json.loads(line))
    return records


def _per_intent_breakdown(
    records: list[dict[str, Any]],
) -> list[tuple[str, int, int, float]]:
    """Per intent_id: (intent_id, count, l3_count, l3_rate). Sorted by
    intent_id for deterministic output."""
    by_intent: dict[str, list[int]] = {}
    for r in records:
        intent = r.get("intent_id", "<unknown>")
        tier = int((r.get("verifier") or {}).get("max_tier", 0))
        by_intent.setdefault(intent, []).append(tier)
    rows: list[tuple[str, int, int, float]] = []
    for intent in sorted(by_intent.keys()):
        tiers = by_intent[intent]
        count = len(tiers)
        l3 = sum(1 for t in tiers if t == 3)
        rate = (l3 / count) if count else 0.0
        rows.append((intent, count, l3, rate))
    return rows


def _raw_fallback_count(records: list[dict[str, Any]]) -> int:
    """Number of rollouts whose ``tool_calls_flat`` contains at least one
    ``griz_raw`` entry."""
    count = 0
    for r in records:
        flat = r.get("tool_calls_flat") or []
        if any((entry or {}).get("name") == "griz_raw" for entry in flat):
            count += 1
    return count


def _failure_mode_rows(by_failure_mode: dict[str, int]) -> list[tuple[str, int]]:
    """Sort by count desc, name asc — deterministic for the zero-count
    tail (every closed-set entry appears so this is the post-v0
    decision input baseline.md §"After v0" gates on)."""
    return sorted(by_failure_mode.items(), key=lambda kv: (-kv[1], kv[0]))


# ---------------------------------------------------------------------------
# Section renderers — one f-string per concern, composed by ``render``.
# ---------------------------------------------------------------------------


def _render_headline(
    summary: dict[str, Any],
    provider_name: str,
    model_id: str,
    scenarios_sha256: str | None,
) -> str:
    cfg = summary.get("config", {})
    sysprompt_hash = cfg.get("system_prompt_sha256", "")
    total = summary.get("total", 0)
    pass_rate = summary.get("l3_pass_rate", 0.0)
    pct = pass_rate * 100.0
    lines = [
        "# mili-llm-bench v0 baseline report",
        "",
        f"**L3 pass rate: {pass_rate:.3f} ({pct:.1f}%) — {summary.get('by_max_tier', {}).get('3', 0)} / {total} scenarios.**",
        "",
        f"* provider: `{provider_name}`",
        f"* model: `{model_id}`",
        f"* system_prompt_sha256: `{sysprompt_hash}`",
    ]
    if scenarios_sha256:
        lines.append(f"* scenarios_sha256: `{scenarios_sha256}`")
    return "\n".join(lines)


def _render_by_max_tier(summary: dict[str, Any]) -> str:
    by_tier = summary.get("by_max_tier", {})
    total = summary.get("total", 0) or 1
    rows = []
    for tier in ("0", "1", "2", "3"):
        n = int(by_tier.get(tier, 0))
        pct = (n / total) * 100.0
        rows.append(f"| {tier} | {n} | {pct:.1f}% |")
    body = "\n".join(rows)
    return (
        "## by_max_tier\n\n"
        "| tier | count | pct |\n"
        "|------|-------|-----|\n"
        f"{body}"
    )


def _render_by_failure_mode(summary: dict[str, Any]) -> str:
    by_fm = summary.get("by_failure_mode", {})
    rows = []
    for name, count in _failure_mode_rows(by_fm):
        rows.append(f"| {name} | {count} |")
    body = "\n".join(rows)
    return (
        "## by_failure_mode\n\n"
        "Sorted by count desc, then name asc. Every closed-set entry "
        "appears (zero-init) so a missing mode is structurally impossible.\n\n"
        "| failure_mode | count |\n"
        "|--------------|-------|\n"
        f"{body}"
    )


def _render_timing(summary: dict[str, Any]) -> str:
    mean_turns = float(summary.get("mean_turns_to_completion", 0.0))
    wall_ms = int(summary.get("total_wall_ms", 0))
    return (
        "## timing\n\n"
        f"* mean turns to completion: **{mean_turns:.2f}**\n"
        f"* total wall clock: **{wall_ms} ms** ({wall_ms / 1000.0:.2f} s)"
    )


def _render_per_intent(records: list[dict[str, Any]]) -> str:
    rows = _per_intent_breakdown(records)
    body_lines = []
    for intent, count, l3, rate in rows:
        pct = rate * 100.0
        body_lines.append(f"| {intent} | {count} | {l3} | {pct:.1f}% |")
    body = "\n".join(body_lines) if body_lines else "| _no records_ | 0 | 0 | 0.0% |"
    return (
        "## per_intent\n\n"
        "L3 pass rate broken down by intent_id; the post-v0 decision tree "
        "(baseline.md §\"After v0\") branches on this.\n\n"
        "| intent_id | count | l3 | l3_rate |\n"
        "|-----------|-------|----|---------|\n"
        f"{body}"
    )


def _render_stage5(summary: dict[str, Any]) -> str | None:
    """Stage 5 K-pass + retention + cost section.

    Returns ``None`` when the run was bench-as-eval (K=1, no
    retention metadata) so the report stays byte-identical to
    pre-rev-11 reports. When ``k > 1`` or ``cost_estimate_usd`` is
    populated, emits a section with the budget-gate-relevant
    numbers: retention rate, retention-by-intent, total tokens by
    category, and the USD cost estimate (against the model_id's
    pinned per-Mtok pricing).
    """
    cfg = summary.get("config", {})
    k = int(cfg.get("k", 1))
    has_cost = "cost_estimate_usd" in summary
    has_retention = "retention_rate" in summary
    if k <= 1 and not has_cost and not has_retention:
        return None

    lines = ["## stage_5_telemetry", ""]
    lines.append(f"* K (rollouts per scenario): **{k}**")
    if has_retention:
        scenarios_retained = int(summary.get("scenarios_retained", 0))
        scenarios_total = int(summary.get("scenarios_total", 0))
        rate = float(summary.get("retention_rate", 0.0))
        lines.append(
            f"* retention (≥1 passing rollout / scenario): "
            f"**{scenarios_retained} / {scenarios_total}** ({rate * 100.0:.1f}%)"
        )
    if has_cost:
        cost = float(summary.get("cost_estimate_usd", 0.0))
        lines.append(f"* cost estimate (USD, against pinned pricing): **${cost:.2f}**")
    totals = summary.get("usage_totals")
    if isinstance(totals, dict) and totals:
        in_t = int(totals.get("input_tokens", 0))
        out_t = int(totals.get("output_tokens", 0))
        cr_t = int(totals.get("cache_read_input_tokens", 0))
        cc_t = int(totals.get("cache_creation_input_tokens", 0))
        lines.append(
            f"* tokens — input {in_t:,} / output {out_t:,} / "
            f"cache_read {cr_t:,} / cache_creation {cc_t:,}"
        )
    by_intent = summary.get("retention_by_intent")
    if isinstance(by_intent, dict) and by_intent:
        lines.append("")
        lines.append("### retention_by_intent")
        lines.append("")
        lines.append("| intent_id | scenarios | retained | retention_rate |")
        lines.append("|-----------|-----------|----------|----------------|")
        for intent, slot in sorted(by_intent.items()):
            count = int(slot.get("count", 0))
            retained = int(slot.get("retained", 0))
            rate = float(slot.get("rate", 0.0)) * 100.0
            lines.append(f"| {intent} | {count} | {retained} | {rate:.1f}% |")
    return "\n".join(lines)


def _render_raw_fallback(records: list[dict[str, Any]]) -> str:
    total = len(records) or 1
    raw_count = _raw_fallback_count(records)
    pct = (raw_count / total) * 100.0
    return (
        "## raw_fallback_rate\n\n"
        f"Rollouts containing at least one `griz_raw` call: "
        f"**{raw_count} / {len(records)}** ({pct:.1f}%). "
        f"The v0 verifier treats `griz_raw` as a fair pass; this rate "
        f"tells us how often the model bypassed the typed-tool surface."
    )


def _render_pointers(
    rollouts_path: Path, config_yaml_path: Path | None, summary_path: Path | None
) -> str:
    lines = ["## artifacts", ""]
    lines.append(f"* rollouts: `{rollouts_path}`")
    if summary_path is not None:
        lines.append(f"* summary: `{summary_path}`")
    if config_yaml_path is not None:
        lines.append(f"* config: `{config_yaml_path}`")
    return "\n".join(lines)


# ---------------------------------------------------------------------------
# Public surface.
# ---------------------------------------------------------------------------


def render(
    summary: dict[str, Any],
    rollouts_path: Path,
    *,
    config_yaml_path: Path | None = None,
    summary_path: Path | None = None,
    provider_name: str = "unknown",
    model_id: str = "unknown",
    scenarios_sha256: str | None = None,
) -> str:
    """Render the full ``report.md`` text (without writing it)."""
    records = _read_rollouts(rollouts_path)
    sections = [
        _render_headline(summary, provider_name, model_id, scenarios_sha256),
        _render_by_max_tier(summary),
        _render_by_failure_mode(summary),
        _render_timing(summary),
        _render_per_intent(records),
        _render_raw_fallback(records),
    ]
    stage5 = _render_stage5(summary)
    if stage5 is not None:
        sections.append(stage5)
    sections.append(_render_pointers(rollouts_path, config_yaml_path, summary_path))
    return "\n\n".join(sections) + "\n"


def write_report(
    out_path: Path,
    summary: dict[str, Any],
    rollouts_path: Path,
    *,
    config_yaml_path: Path | None = None,
    summary_path: Path | None = None,
    provider_name: str = "unknown",
    model_id: str = "unknown",
    scenarios_sha256: str | None = None,
) -> str:
    """Render and write the report; return the text written."""
    text = render(
        summary,
        rollouts_path,
        config_yaml_path=config_yaml_path,
        summary_path=summary_path,
        provider_name=provider_name,
        model_id=model_id,
        scenarios_sha256=scenarios_sha256,
    )
    Path(out_path).write_text(text)
    return text


__all__ = ["render", "write_report"]
