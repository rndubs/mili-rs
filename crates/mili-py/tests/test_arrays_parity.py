"""milox vs. upstream `mili` — M2 bulk-array parity.

Diffs `nodes()` and `connectivity()` (dtype, shape, values) for
`import milox` vs `import mili` over the parity corpus + xmilics
multi-fragment families. First arrays across the FFI boundary.
"""

from __future__ import annotations

import numpy as np
import pytest

import milox

from conftest import CASES

if not CASES:
    pytest.skip(
        "no parity fixtures present (run scripts/setup-parity.sh)",
        allow_module_level=True,
    )

CASE_IDS = [c[0] for c in CASES]
CASE_PARAMS = [c[1] for c in CASES]


def _open_upstream(mili, base):
    # suppress_parallel keeps multi-fragment opens deterministic
    # (LoopWrapper, no worker processes) while still merging results —
    # the same merge contract milox's DatabaseSet implements.
    return mili.open_database(base, suppress_parallel=True)


@pytest.fixture(params=CASE_PARAMS, ids=CASE_IDS)
def both(request, upstream_mili):
    base = request.param
    return (
        milox.open_database(base),
        _open_upstream(upstream_mili, base),
    )


def test_nodes(both):
    g, w = both
    gn = g.nodes()
    wn = np.asarray(w.nodes())
    assert gn.dtype == np.float32, f"nodes dtype {gn.dtype}"
    assert gn.shape == wn.shape, f"nodes shape {gn.shape} != {wn.shape}"
    assert np.array_equal(gn, wn), "nodes values differ"


def test_connectivity_dict(both):
    g, w = both
    gd = g.connectivity()
    wd = w.connectivity()
    assert set(gd.keys()) == set(wd.keys()), (
        f"connectivity classes {sorted(gd)} != {sorted(wd)}"
    )
    for cls in gd:
        ga = gd[cls]
        wa = np.asarray(wd[cls])
        assert ga.dtype == np.int32, f"{cls}: dtype {ga.dtype}"
        assert ga.shape == wa.shape, f"{cls}: shape {ga.shape} != {wa.shape}"
        assert np.array_equal(ga, wa), f"{cls}: connectivity values differ"


def test_connectivity_by_class(both):
    g, w = both
    for cls in w.connectivity():
        ga = g.connectivity(cls)
        wa = np.asarray(w.connectivity(cls))
        assert ga.dtype == np.int32, f"{cls}: dtype {ga.dtype}"
        assert ga.shape == wa.shape, f"{cls}: shape {ga.shape} != {wa.shape}"
        assert np.array_equal(ga, wa), f"{cls}: connectivity values differ"
