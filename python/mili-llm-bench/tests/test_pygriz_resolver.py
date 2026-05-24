"""Fixture-path resolver tests for ``dispatchers/pygriz``.

The bench scenarios carry bare fixture names (``"d3samp6"`` /
``"cylinder"``); mili-viz-server's ``Database::open`` takes an absolute
path and silently falls back to an empty M1 stub on lookup failure. The
resolver in ``dispatchers/pygriz`` maps the bare name to a checked-in
.A file, so the eval actually exercises a real corpus. Loud failure on
miss is part of the contract — these tests pin both halves.
"""

from __future__ import annotations

import pytest

from mili_llm_bench.dispatchers.pygriz import (
    _FIXTURE_PATHS,
    _resolve_fixture,
)


def test_known_fixtures_resolve_to_existing_files() -> None:
    for name in _FIXTURE_PATHS:
        path = _resolve_fixture(name)
        # Resolver returns a string for the gRPC wire; the underlying
        # path must exist (otherwise mili-viz-server silently stubs).
        from pathlib import Path
        assert Path(path).is_file(), f"fixture {name!r} resolved to absent {path}"


def test_unknown_fixture_raises_loudly() -> None:
    with pytest.raises(ValueError, match="unknown bench fixture"):
        _resolve_fixture("does-not-exist")


def test_all_bootstrap_scenarios_use_known_fixtures() -> None:
    """Drift between ``bootstrap.jsonl`` and ``_FIXTURE_PATHS`` would
    silently re-introduce the stub-fallback bug for the unknown name.
    Catch that at test time, not at eval time."""
    from mili_llm_bench.scenarios import default_bootstrap_path, load_scenarios

    fixtures = {s.fixture for s in load_scenarios(default_bootstrap_path())}
    unknown = fixtures - set(_FIXTURE_PATHS)
    assert not unknown, (
        f"bootstrap.jsonl references fixtures with no resolver entry: "
        f"{sorted(unknown)}. Add them to _FIXTURE_PATHS."
    )
