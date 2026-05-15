"""milox vs. upstream `mili` — M3 primal `query()` parity.

Reuses the (svar, class, states) tuples the Rust `parity_corpus.rs`
table already proves bit-exact at the core layer, asserting the full
upstream QueryDict shape (class_name/source/title/components/labels/
states/data dtype+shape+values/times) over the corpus + xmilics
families.
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

# conftest case id -> (svar, class, 0-based state indices). Mirrors
# crates/mili-rs/tests/parity_corpus.rs.
SERIAL_QUERIES = {
    "serial-beam_udi": ("axf", "beam", [0, 10, 20]),
    "serial-d3samp4": ("sand", "brick", [0, 5, 10]),
    "serial-dbl_nodtang": ("nodpos", "node", [0, 60, 121]),
    "serial-fdamp1": ("stress", "brick", [0, 10, 20]),
    "serial-labeling": ("nodpos", "node", [0, 1, 2]),
    "serial-mstate": ("axf", "beam", [0, 50, 100]),
    "serial-rigid_body_1": ("sand", "brick", [0, 10, 20]),
    "serial-sstate": ("axf", "beam", [0, 50, 100]),
    "serial-tet": ("sand", "tet", [0, 40, 80]),
    "serial-vrt_BS": ("axf", "beam", [0, 5, 10]),
}


def _open_upstream(mili, base):
    return mili.open_database(base, suppress_parallel=True)


@pytest.fixture(params=CASE_PARAMS, ids=CASE_IDS)
def case(request, upstream_mili):
    cid = request._pyfuncitem.callspec.id
    base = request.param
    g = milox.open_database(base)
    w = _open_upstream(upstream_mili, base)
    if cid in SERIAL_QUERIES:
        svar, cls, st0 = SERIAL_QUERIES[cid]
    else:
        # xmilics multi-fragment families: nodal positions on `node`.
        svar, cls, st0 = ("nodpos", "node", [0, 1, 2])
    n = g.state_count()
    # 0-based -> 1-based state numbers, clamped + de-duped in order.
    nums = []
    for s in st0:
        v = min(s + 1, n)
        if v not in nums:
            nums.append(v)
    return g, w, svar, cls, nums


def test_query_dict(case):
    g, w, svar, cls, states = case
    gr = g.query(svar, cls, states=states)
    wr = w.query(svar, cls, states=states)

    assert set(gr.keys()) == set(wr.keys()) == {svar}
    ge, we = gr[svar], wr[svar]

    assert ge["class_name"] == we["class_name"]
    assert ge["source"] == we["source"] == "primal"
    assert ge["title"] == we["title"]

    gl, wl = ge["layout"], we["layout"]
    assert list(gl["components"]) == list(we["layout"]["components"])
    assert np.array_equal(
        np.asarray(gl["labels"], dtype=np.int64),
        np.asarray(wl["labels"], dtype=np.int64),
    ), f"labels: {gl['labels']!r} != {wl['labels']!r}"
    assert np.array_equal(
        np.asarray(gl["states"], dtype=np.int64),
        np.asarray(wl["states"], dtype=np.int64),
    ), f"states: {gl['states']!r} != {wl['states']!r}"
    assert np.allclose(
        np.asarray(gl["times"], dtype=np.float64),
        np.asarray(wl["times"], dtype=np.float64),
    ), "times differ"

    gd, wd = ge["data"], np.asarray(we["data"])
    assert gd.dtype == wd.dtype, f"data dtype {gd.dtype} != {wd.dtype}"
    assert gd.shape == wd.shape, f"data shape {gd.shape} != {wd.shape}"
    assert np.array_equal(gd, wd), "data values differ"
