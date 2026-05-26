"""W1 — tool-schema artifact tests.

Always-on (no pygriz, no LLM, no GPU). Three groups:

* Honest-diff vs the checked-in ``data/posttraining/grammar/tools.json``.
* Coverage: every typed ``Command`` oneof variant has exactly one tool
  entry; ``query`` / ``snapshot`` / ``griz_raw`` are present; ``render``
  and ``raw`` are absent.
* Invariants (pre-enforced at the schema layer): no entry's
  ``output_schema`` contains ``state_times``, ``flight_ticket``, or
  ``agent`` properties anywhere in its JSON Schema tree (the W4a
  harness invariants from baseline.md §W1).
"""

from __future__ import annotations

import difflib
import json
from pathlib import Path
from typing import Any

import pytest

from mili_llm_bench.schemas import (
    EXCLUDED_COMMANDS,
    FORBIDDEN_OUTPUT_FIELDS,
    TYPED_COMMAND_TOOLS,
    default_artifact_path,
    derive_tools,
    dump_tools_json,
    find_proto_path,
)


def _checked_in_text() -> str:
    return default_artifact_path().read_text()


def test_honest_diff_against_committed_artifact() -> None:
    """Re-derive and diff. Drift fails with a regenerate hint."""
    fresh = dump_tools_json(derive_tools())
    committed = _checked_in_text()
    if fresh != committed:
        diff = "\n".join(
            difflib.unified_diff(
                committed.splitlines(),
                fresh.splitlines(),
                fromfile="committed tools.json",
                tofile="re-derived tools.json",
                lineterm="",
            )
        )
        pytest.fail(
            "tools.json is stale. regenerate via "
            "`python -m mili_llm_bench derive-schemas`.\n\n" + diff
        )


def test_every_typed_command_oneof_variant_has_one_tool_entry() -> None:
    tools = derive_tools()
    names = [t["name"] for t in tools]
    for tool_name, _oneof_name, _msg in TYPED_COMMAND_TOOLS:
        assert names.count(tool_name) == 1, f"tool {tool_name!r} not unique"


def test_read_tools_and_griz_raw_present() -> None:
    names = {t["name"] for t in derive_tools()}
    for required in ("query", "snapshot", "griz_raw"):
        assert required in names, f"{required!r} missing from tools"


def test_render_and_raw_are_absent() -> None:
    names = {t["name"] for t in derive_tools()}
    for absent in ("render", "raw"):
        assert absent not in names, f"{absent!r} must not be in tools"
    # The exclusion list also pins the proto-side intent.
    assert EXCLUDED_COMMANDS == {"raw", "render"}


def test_total_tool_count_is_nineteen() -> None:
    # 15 typed Command variants + 2 read tools + 1 fallback
    # + 1 agent-local lookup tool (m7 Delta 5 `list_results`) = 19.
    assert len(derive_tools()) == 19


def test_tools_are_sorted_by_name_for_stable_diffs() -> None:
    tools = derive_tools()
    names = [t["name"] for t in tools]
    assert names == sorted(names)


def test_every_tool_has_required_keys() -> None:
    for tool in derive_tools():
        assert set(tool.keys()) >= {
            "name",
            "description",
            "input_schema",
            "output_schema",
        }


def _walk_property_keys(schema: Any, into: set[str]) -> None:
    """Collect every ``properties`` key name anywhere in a JSON Schema tree."""
    if isinstance(schema, dict):
        if "properties" in schema and isinstance(schema["properties"], dict):
            for key, sub in schema["properties"].items():
                into.add(key)
                _walk_property_keys(sub, into)
        for key, value in schema.items():
            if key == "properties":
                continue
            _walk_property_keys(value, into)
    elif isinstance(schema, list):
        for item in schema:
            _walk_property_keys(item, into)


@pytest.mark.parametrize("forbidden", FORBIDDEN_OUTPUT_FIELDS)
def test_no_output_schema_carries_a_forbidden_field(forbidden: str) -> None:
    """Pre-enforce the W4a harness invariants at the schema layer:
    ``state_times`` / ``flight_ticket`` / ``agent`` never appear in
    any tool's ``output_schema`` JSON Schema tree."""
    for tool in derive_tools():
        keys: set[str] = set()
        _walk_property_keys(tool["output_schema"], keys)
        assert forbidden not in keys, (
            f"forbidden field {forbidden!r} appears in output_schema "
            f"for tool {tool['name']!r}"
        )


def test_proto_path_resolves() -> None:
    proto = find_proto_path()
    assert proto.exists()
    assert proto.name == "mili_viz.proto"


def test_artifact_path_resolves() -> None:
    path = default_artifact_path()
    assert path.parts[-3:] == ("posttraining", "grammar", "tools.json")
