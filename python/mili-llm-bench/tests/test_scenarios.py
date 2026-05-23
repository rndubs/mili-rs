"""W2 — bootstrap scenarios tests.

Always-on (no pygriz, no LLM, no GPU). Four groups:

* Round-trip: load ``bootstrap.jsonl``, re-serialize, byte-identical.
* Uniqueness: every scenario id is unique; count is exactly 50.
* Closed kinds: every ``postcondition.kind`` is in the W3 closed set
  (the scenarios <-> verifier contract — drift fails loudly).
* Coverage: each of the 10 required intents appears in >= 1 scenario
  per fixture.
"""

from __future__ import annotations

import json
import tempfile
from pathlib import Path

import pytest

from mili_llm_bench.scenarios import (
    VALID_POSTCONDITION_KINDS,
    Postcondition,
    Scenario,
    default_bootstrap_path,
    dump_scenarios,
    load_scenarios,
)


# The 10 required intents the W2 coverage gate enforces (compound is
# additive and not required per-fixture).
_REQUIRED_INTENTS: tuple[str, ...] = (
    "load",
    "set-state",
    "step",
    "select",
    "clrsel",
    "show-primal",
    "show-derived",
    "material",
    "view-reset",
    "colormap",
)

_REQUIRED_FIXTURES: tuple[str, ...] = ("d3samp6", "cylinder")


def _bootstrap_text() -> str:
    return default_bootstrap_path().read_text()


def _scenarios() -> list[Scenario]:
    return load_scenarios(default_bootstrap_path())


def test_round_trip_is_byte_identical() -> None:
    """Load + re-serialize == checked-in file (byte-identical).

    Catches accidental field reordering or stray whitespace before it
    breaks downstream diff-based artifact tracking.
    """
    scenarios = _scenarios()
    fresh = dump_scenarios(scenarios)
    assert fresh == _bootstrap_text()


def test_exactly_fifty_scenarios() -> None:
    assert len(_scenarios()) == 50


def test_every_id_is_unique() -> None:
    ids = [s.id for s in _scenarios()]
    assert len(set(ids)) == len(ids), "duplicate scenario ids"


def test_every_postcondition_kind_is_in_the_closed_set() -> None:
    for s in _scenarios():
        assert s.postcondition.kind in VALID_POSTCONDITION_KINDS, (
            f"scenario {s.id} carries unknown kind {s.postcondition.kind!r}"
        )


@pytest.mark.parametrize("fixture", _REQUIRED_FIXTURES)
@pytest.mark.parametrize("intent", _REQUIRED_INTENTS)
def test_intent_appears_in_each_fixture(fixture: str, intent: str) -> None:
    """The 10-intent x 2-fixture coverage matrix — any hole fails CI."""
    hits = [
        s for s in _scenarios() if s.fixture == fixture and s.intent_id == intent
    ]
    assert hits, f"no scenario for intent={intent!r} on fixture={fixture!r}"


def test_compound_intent_present() -> None:
    """The one multi-turn compound scenario stresses multi-tool chaining
    inside the W6 driver — pin its presence so it cannot be dropped."""
    compound = [s for s in _scenarios() if s.intent_id == "compound"]
    assert len(compound) >= 1, "no compound (multi-tool) scenario"


def test_loader_rejects_unknown_postcondition_kind() -> None:
    """Loader hard-fails on a kind not in the closed set — drift between
    the scenarios file and the verifier should never slip past loading."""
    bad = {
        "id": "bs-bad",
        "fixture": "d3samp6",
        "intent_id": "load",
        "instruction": "do a thing",
        "postcondition": {"kind": "totally_made_up", "expect": {}},
    }
    with tempfile.TemporaryDirectory() as td:
        p = Path(td) / "bad.jsonl"
        p.write_text(json.dumps(bad) + "\n")
        with pytest.raises(ValueError, match="unknown postcondition kind"):
            load_scenarios(p)


def test_loader_rejects_missing_required_field() -> None:
    bad = {
        # missing "fixture"
        "id": "bs-bad",
        "intent_id": "load",
        "instruction": "do a thing",
        "postcondition": {"kind": "state_index", "expect": {"state": 1}},
    }
    with tempfile.TemporaryDirectory() as td:
        p = Path(td) / "bad.jsonl"
        p.write_text(json.dumps(bad) + "\n")
        with pytest.raises(ValueError, match="missing required key"):
            load_scenarios(p)


def test_scenario_dataclass_round_trips_a_minimal_object() -> None:
    """Construct a Scenario directly, serialize, reparse — the dataclass
    layer is the in-memory shape both W4b and W3 hand around."""
    s = Scenario(
        id="bs-x",
        fixture="d3samp6",
        intent_id="load",
        instruction="open it",
        postcondition=Postcondition(kind="state_index", expect={"state": 1}),
    )
    text = dump_scenarios([s])
    with tempfile.TemporaryDirectory() as td:
        p = Path(td) / "x.jsonl"
        p.write_text(text)
        back = load_scenarios(p)
    assert back == [s]
