"""Stage 3 orchestrator: glue from catalog.yaml to synth.jsonl + report.

Public entry point ``run_synth`` loads the catalog, samples tuples,
resolves slots, validates every record, and writes the two output
files. Callers are the CLI ``synth`` subcommand and the round-trip
test.

Pygriz is *optional* — the synthesizer falls back to a deterministic
stub query oracle when pygriz is not available, and surfaces the
skipped query rows in the report. The CLI invocation should use the
live oracle (pygriz session per fixture); the round-trip test uses
the stub.
"""

from __future__ import annotations

import json
from dataclasses import dataclass, field
from pathlib import Path
from typing import Any

from .. import verifier as _verifier_mod
from ..scenarios import Postcondition, Scenario, _parse_scenario
from .catalog import Catalog, FixtureFacts, IntentRow, load_catalog
from .sample import SampledTuple, sample_tuples
from .slots import (
    QueryOracle,
    ResolvedScenario,
    format_paraphrase,
    resolve_expect,
    substitute,
)


# ---------------------------------------------------------------------------
# Report.
# ---------------------------------------------------------------------------


@dataclass
class SynthReport:
    """Audit trail one ``run_synth`` invocation produces.

    Surfaced by the CLI as a sibling ``synth.report.md`` and consumed
    by the round-trip test for its compound-ratio + total-count
    assertions.
    """

    seed: int
    total: int = 0
    compound_count: int = 0
    by_cell: dict[tuple[str, str], int] = field(default_factory=dict)
    by_instruction_source: dict[str, int] = field(default_factory=dict)
    fixture_facts_confirmed: list[str] = field(default_factory=list)
    skipped: list[str] = field(default_factory=list)
    notes: list[str] = field(default_factory=list)

    @property
    def compound_ratio(self) -> float:
        return self.compound_count / self.total if self.total else 0.0


# ---------------------------------------------------------------------------
# Orchestrator.
# ---------------------------------------------------------------------------


def run_synth(
    *,
    catalog_path: Path,
    out_path: Path,
    report_path: Path | None = None,
    seed: int = 42,
    target_total: int = 200,
    compound_ratio: float = 0.20,
    query_oracle: QueryOracle | None = None,
    confirm_fixtures: bool = True,
) -> SynthReport:
    """End-to-end Stage 3 synthesis run.

    Writes the JSONL corpus to ``out_path`` and (optionally) a Markdown
    audit report to ``report_path``. Returns the ``SynthReport`` so
    tests + the CLI can assert against it without re-parsing files.
    """
    catalog = load_catalog(catalog_path)

    report = SynthReport(seed=seed)

    confirmed_fixtures: dict[str, FixtureFacts] = {}
    if confirm_fixtures:
        try:
            confirmed_fixtures = _confirm_fixture_facts(catalog, report)
        except _PygrizUnavailable as exc:
            report.notes.append(
                f"fixture-fact confirmation skipped: {exc}. "
                "Falling back to catalog placeholders verbatim."
            )

    # Build the query oracle if we have pygriz and any query intents
    # need <derived:from_parity_suite>.
    intent_ids = {row.intent_id for row in catalog.intents}
    has_query_path = "query" in intent_ids
    if has_query_path and query_oracle is None:
        try:
            query_oracle = _build_live_query_oracle(catalog)
            report.notes.append("query oracle: live pygriz capture (per fixture)")
        except _PygrizUnavailable as exc:
            report.notes.append(
                f"query oracle: pygriz unavailable ({exc}); query rows dropped"
            )

    tuples = sample_tuples(
        catalog,
        seed=seed,
        target_total=target_total,
        compound_ratio=compound_ratio,
    )

    by_id = {row.intent_id: row for row in catalog.intents}

    scenarios: list[Scenario] = []
    counter = 0
    for tup in tuples:
        intent = by_id[tup.intent_id]
        fixture = catalog.fixtures[tup.fixture]
        try:
            scenario = _build_scenario(
                tup, intent, fixture, counter, query_oracle
            )
        except _DropScenario as exc:
            report.skipped.append(f"{tup.intent_id}/{tup.fixture}: {exc}")
            continue
        except Exception as exc:
            # Any per-row resolution failure (e.g. a missing pygriz
            # read-path method) drops the row but keeps the run going.
            report.skipped.append(
                f"{tup.intent_id}/{tup.fixture}: resolution failed: {exc!r}"
            )
            continue

        # Round-trip validation. ``_parse_scenario`` mirrors what the
        # public ``load_scenarios`` will do at consume time; failing
        # here means the record would have failed at load.
        try:
            _parse_scenario(scenario.to_json())
        except Exception as exc:  # pragma: no cover — would catch a bug in build
            report.skipped.append(
                f"{tup.intent_id}/{tup.fixture}: round-trip parse failed: {exc!r}"
            )
            continue

        # Verifier-handler smoke. Empty calls list — we only assert the
        # handler accepts the expect shape, not that it grades L3.
        handler = _verifier_mod._PC_HANDLERS[scenario.postcondition.kind]
        try:
            handler(scenario.postcondition.expect, [])
        except Exception as exc:  # pragma: no cover — would catch a bug in build
            report.skipped.append(
                f"{tup.intent_id}/{tup.fixture}: verifier handler raised: {exc!r}"
            )
            continue

        scenarios.append(scenario)
        report.total += 1
        if intent.shape == "compound":
            report.compound_count += 1
        cell = (intent.intent_id, tup.fixture)
        report.by_cell[cell] = report.by_cell.get(cell, 0) + 1
        report.by_instruction_source[tup.instruction_source] = (
            report.by_instruction_source.get(tup.instruction_source, 0) + 1
        )
        counter += 1

    # Write the JSONL.
    out_path.parent.mkdir(parents=True, exist_ok=True)
    with out_path.open("w") as f:
        for scenario in scenarios:
            f.write(json.dumps(scenario.to_json()))
            f.write("\n")

    if report_path is not None:
        report_path.write_text(_render_report(report, catalog))

    return report


# ---------------------------------------------------------------------------
# Per-tuple → scenario record.
# ---------------------------------------------------------------------------


class _DropScenario(Exception):
    """Raised by ``_build_scenario`` when a row should be dropped from
    the corpus (oracle unavailable, etc.). Surfaces in ``report.skipped``."""


def _build_scenario(
    tup: SampledTuple,
    intent: IntentRow,
    fixture: FixtureFacts,
    counter: int,
    query_oracle: QueryOracle | None,
) -> Scenario:
    instruction = format_paraphrase(tup.seed_text, tup.bound)

    # Drop ``query`` rows when the oracle is unavailable — the record
    # would carry an unverifiable ``expect.table``. The non-query path
    # ignores the oracle entirely.
    needs_oracle = intent.intent_id == "query"
    if needs_oracle and query_oracle is None:
        raise _DropScenario("query oracle unavailable")

    expect = resolve_expect(
        intent.expect_template,
        bound=tup.bound,
        intent=intent,
        fixture=fixture,
        query_oracle=query_oracle,
    )

    pc = Postcondition(kind=intent.postcondition_kind, expect=expect)
    scenario_id = f"synth-{counter:05d}"
    return Scenario(
        id=scenario_id,
        fixture=tup.fixture,
        intent_id=tup.intent_id,
        instruction=instruction,
        postcondition=pc,
        instruction_source=tup.instruction_source,
    )


# ---------------------------------------------------------------------------
# Fixture-fact confirmation + query oracle (pygriz-backed).
# ---------------------------------------------------------------------------


class _PygrizUnavailable(RuntimeError):
    """Sentinel for the pygriz-absent branch."""


def _confirm_fixture_facts(
    catalog: Catalog, report: SynthReport
) -> dict[str, FixtureFacts]:
    """Load each declared fixture and diff observed facts against the
    catalog's placeholders. Mismatch = ``ValueError`` (the catalog is
    the source of truth; if it's wrong, fix it before continuing).

    A read of ``snapshot.materials.visible`` is taken AFTER toggling the
    first declared mat id off-and-on, because pygriz only populates that
    map for materials that have been touched at least once.
    """
    griz = _import_griz()
    from ..dispatchers.pygriz import _resolve_fixture

    out: dict[str, FixtureFacts] = {}
    for name, fix in catalog.fixtures.items():
        session = griz.launch()
        try:
            session.open(_resolve_fixture(name))
            snap = session._snapshot()
            obs_num_states = int(snap.loaded.num_states)
            class_names = list(snap.loaded.class_names)
            if obs_num_states != fix.num_states:
                raise ValueError(
                    f"fixture {name}: num_states mismatch — catalog says "
                    f"{fix.num_states}, fixture says {obs_num_states}. "
                    f"Update data/posttraining/intents/catalog.yaml."
                )
            missing_classes = [c for c in fix.classes if c not in class_names]
            if missing_classes:
                raise ValueError(
                    f"fixture {name}: catalog classes {missing_classes!r} not "
                    f"present in fixture class_names {class_names!r}. "
                    f"Update data/posttraining/intents/catalog.yaml."
                )
            # Material-id probe: each declared id must accept a
            # disable/enable cycle without raising.
            for mid in fix.material_ids:
                session.materials.disable(mat=mid)
                session.materials.enable(mat=mid)
            out[name] = fix
            report.fixture_facts_confirmed.append(
                f"{name}: num_states={obs_num_states} ✓, classes ⊇ "
                f"{list(fix.classes)} ✓, mat_ids {list(fix.material_ids)} ✓"
            )
        finally:
            try:
                session.close()
            except Exception:
                pass
    return out


def _build_live_query_oracle(catalog: Catalog) -> QueryOracle:
    """Build a pygriz-backed query oracle that captures live tables.

    Caches one ``griz.Session`` per fixture so the per-scenario query
    overhead is one round-trip, not a full session bootstrap.
    """
    griz = _import_griz()
    from ..dispatchers.pygriz import _resolve_fixture

    sessions: dict[str, Any] = {}

    def oracle(fixture: FixtureFacts, bound: dict[str, Any]) -> dict[str, Any]:
        if fixture.name not in sessions:
            s = griz.launch()
            s.open(_resolve_fixture(fixture.name))
            sessions[fixture.name] = s
        s = sessions[fixture.name]
        kwargs: dict[str, Any] = {
            "result": bound["result_name"],
            "class_name": bound["class"],
            "labels": list(bound.get("labels") or []),
            "states": list(bound.get("states") or []),
        }
        table = s.query(**kwargs)
        return table if isinstance(table, dict) else {}

    return oracle


def _import_griz() -> Any:
    try:
        import griz  # type: ignore[import-not-found]
    except ImportError as exc:  # pragma: no cover — exercised on no-pygriz boxes
        raise _PygrizUnavailable(str(exc)) from exc
    try:
        # ``launch()`` raises ``FileNotFoundError`` if mili-viz-server isn't
        # built; surface that to the caller's ``_PygrizUnavailable`` branch
        # too so the synth pass degrades cleanly.
        s = griz.launch()
        s.close()
    except Exception as exc:
        raise _PygrizUnavailable(str(exc)) from exc
    return griz


# ---------------------------------------------------------------------------
# Report rendering.
# ---------------------------------------------------------------------------


def _render_report(report: SynthReport, catalog: Catalog) -> str:
    lines: list[str] = []
    lines.append("# Stage 3 synthesis report")
    lines.append("")
    lines.append(f"- seed: `{report.seed}`")
    lines.append(f"- total scenarios: **{report.total}**")
    lines.append(
        f"- compound scenarios: **{report.compound_count}** "
        f"(ratio {report.compound_ratio:.2%}; ≥20% gate)"
    )
    lines.append("")

    lines.append("## paraphrase source breakdown")
    lines.append("")
    for src, n in sorted(report.by_instruction_source.items()):
        lines.append(f"- `{src}`: {n}")
    lines.append("")

    lines.append("## per-cell count")
    lines.append("")
    lines.append("| intent_id | fixture | count |")
    lines.append("| --- | --- | --- |")
    intent_order = [row.intent_id for row in catalog.intents]
    for intent_id in intent_order:
        for fixture in sorted(catalog.fixtures.keys()):
            count = report.by_cell.get((intent_id, fixture), 0)
            lines.append(f"| `{intent_id}` | `{fixture}` | {count} |")
    lines.append("")

    if report.fixture_facts_confirmed:
        lines.append("## fixture-fact confirmation")
        lines.append("")
        for line in report.fixture_facts_confirmed:
            lines.append(f"- {line}")
        lines.append("")

    if report.skipped:
        lines.append("## skipped rows")
        lines.append("")
        for line in report.skipped:
            lines.append(f"- {line}")
        lines.append("")

    if report.notes:
        lines.append("## notes")
        lines.append("")
        for line in report.notes:
            lines.append(f"- {line}")
        lines.append("")

    return "\n".join(lines)


# Re-export the verifier handlers dict for the test seam — synthesis is
# part of mili_llm_bench so we don't strictly need to re-export, but
# this keeps the round-trip test's import surface explicit.
__all__ = ["SynthReport", "run_synth"]
