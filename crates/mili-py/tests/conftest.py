"""Shared discovery for the milox metadata-parity harness.

Skip-not-fail when the corpus submodules or the upstream ``mili``
oracle are absent — mirrors the Rust parity tests' behaviour so a bare
local run reads green while CI (which runs ``scripts/setup-parity.sh``)
exercises full coverage.
"""

from __future__ import annotations

import os
import re
from pathlib import Path

import pytest


def _has_afile(directory: Path, base: str) -> bool:
    """True if any `<base>(\\d*)A` file exists (the milox/mili rule)."""
    rx = re.compile(re.escape(base) + r"(\d*)A$")
    return any(rx.match(f) for f in os.listdir(directory))

# crates/mili-py/tests -> repo root
REPO_ROOT = Path(__file__).resolve().parents[3]
SERIAL = REPO_ROOT / "reference" / "mili-python" / "tests" / "data" / "serial"
XMILICS = REPO_ROOT / "reference" / "mili" / "test" / "xmilics"

# (id, relative dir under serial/, base filename). Mirrors the
# fixtures the Rust `parity_corpus.rs` table opens.
SERIAL_CASES = [
    ("beam_udi", "beam_udi", "beam_udi.plt"),
    ("d3samp4", "d3samp4", "d3samp4.plt"),
    ("dbl_nodtang", "dbl_nodtang", "dblplt000"),
    ("fdamp1", "fdamp1", "fdamp1.plt"),
    ("labeling", "labeling", "dblplt003"),
    ("mstate", "mstate", "d3samp6.plt_c"),
    ("rigid_body_1", "rigid_body_1", "rigid_body1.plt"),
    ("sstate", "sstate", "d3samp6.plt"),
    ("tet", "tet", "tet1_t4.plt"),
    ("vrt_BS", "vrt_BS", "vrt_BS.plt"),
    ("basic1", "basic1", "basic1.plt"),
]

# Multi-fragment MPI families (`<base>.plt00<r>A` per rank).
XMILICS_FAMILIES = [
    "bar1",
    "bar5",
    "basic2",
    "cylinder",
    "cylinder_4hex",
    "d3samp6",
    "shell_mat2",
]

# A multi-fragment family used for the decision-4 rank-0 state_maps
# parity assertion.
MULTI_FRAGMENT_STATE_MAP_FAMILY = "d3samp6"


def _serial_base(rel_dir: str, base: str) -> str | None:
    a = SERIAL / rel_dir
    if not a.is_dir() or not _has_afile(a, base):
        return None
    return str(a / base)


def _xmilics_base(family: str) -> str | None:
    d = XMILICS / family
    base = f"{family}.plt"
    if not d.is_dir() or not _has_afile(d, base):
        return None
    return str(d / base)


def discover_cases():
    """Yield (case_id, base_path) for every present fixture."""
    for cid, rel, base in SERIAL_CASES:
        p = _serial_base(rel, base)
        if p is not None:
            yield (f"serial-{cid}", p)
    for fam in XMILICS_FAMILIES:
        p = _xmilics_base(fam)
        if p is not None:
            yield (f"xmilics-{fam}", p)


@pytest.fixture(scope="session")
def upstream_mili():
    mili = pytest.importorskip("mili.reader", reason="upstream `mili` oracle not installed")
    return mili


CASES = list(discover_cases())


def pytest_sessionfinish(session, exitstatus):
    """Closeout hard gate — strict 0-xfail.

    The milox redirect surface is complete (Phase 3 landed; every
    upstream read-path module redirected-and-promoted or consciously
    excluded). With the xfail bucket empty, *any* xfail/xpass in the
    milox suite means parity silently rotted (a redirected case quietly
    started differing, or a passing case is still marked xfail). Fail
    the whole session in that case so CI's ``test-milox`` job is a hard
    gate, not best-effort.

    Skip-not-fail is preserved: an absent submodule/oracle yields
    *skips*, never xfails, so a bare local run (no
    ``scripts/setup-parity.sh``) still reads green — same convention as
    the rest of this harness and the Rust parity jobs."""
    tr = session.config.pluginmanager.get_plugin("terminalreporter")
    if tr is None:
        return
    xfailed = len(tr.stats.get("xfailed", []))
    xpassed = len(tr.stats.get("xpassed", []))
    if xfailed or xpassed:
        tr.write_sep(
            "=",
            f"strict 0-xfail gate FAILED: {xfailed} xfailed / "
            f"{xpassed} xpassed (milox redirect surface is declared "
            f"complete — promote or consciously re-scope)",
            red=True,
            bold=True,
        )
        session.exitstatus = 1
