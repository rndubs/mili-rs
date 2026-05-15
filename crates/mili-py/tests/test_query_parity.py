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


# --------------------------------------------------------------------------
# M4 — full query() filter-combination parity (the cross-validatable
# "Slice A" surface: material=/labels=/states=(±/dedupe)/multi-svar/
# subrec=/array-subscript). Slice B — bare-component-of-VEC_ARRAY,
# ip-label semantics, and cross-material InconsistentIpCounts — is
# split to M4-followup (planning/mili-py/m4.md decision 17/§Slice B);
# its test_bugfixes cases are xfail-marked below with the exact
# upstream oracle values so the follow-up has a ready gate.
# --------------------------------------------------------------------------

from pathlib import Path  # noqa: E402

from conftest import SERIAL, REPO_ROOT, _has_afile  # noqa: E402


def _base_present(base: str) -> bool:
    """True if `<base>(\\d*)A` exists (the milox/mili open rule); the
    on-disk fixtures carry the trailing `A`, so a plain `.exists()`
    on the suffix-less base wrongly reports absent."""
    p = Path(base)
    return p.parent.is_dir() and _has_afile(p.parent, p.name)

_FDAMP1 = SERIAL / "fdamp1" / "fdamp1.plt"
_HX = REPO_ROOT / "reference" / "mili-python" / "tests" / "data" / "th" / "serial" / "d3samp6.th"


# (base, svar_names, class, kwargs) — every combo here is bit-exact
# cross-validatable against upstream `mili` (verified during M4).
_FILTER_TABLE = [
    ("fdamp1", str(_FDAMP1), "stress", "brick", dict(states=[1, 11])),
    ("fdamp1-negstate", str(_FDAMP1), "stress", "brick", dict(states=-1)),
    ("fdamp1-dedup", str(_FDAMP1), "stress", "brick", dict(states=[5, 1, 5, 2])),
    ("fdamp1-mat-int", str(_FDAMP1), "stress", "brick", dict(material=1, states=[1])),
    ("fdamp1-mat-str", str(_FDAMP1), "stress", "brick", dict(material="1", states=[1])),
    ("fdamp1-labels", str(_FDAMP1), "stress", "brick", dict(labels=[1, 3], states=[1])),
    ("fdamp1-multi", str(_FDAMP1), ["stress", "sx"], "brick", dict(states=[1])),
    ("fdamp1-subrec", str(_FDAMP1), "stress", "brick", dict(states=[1, 5], subrec="1hex_mmsvn_rec")),
    ("fdamp1-subrec-sx", str(_FDAMP1), "sx", "brick", dict(states=[1, 5], subrec="1hex_mmsvn_rec")),
    ("hx-sub3", str(_HX), "hx[3]", "brick", dict(states=[1])),
    ("hx-sub8", str(_HX), "hx[8]", "brick", dict(states=[1])),
    ("hx-full", str(_HX), "hx", "brick", dict(states=[1])),
]


@pytest.mark.parametrize(
    "case", _FILTER_TABLE, ids=[c[0] for c in _FILTER_TABLE]
)
def test_filter_combination_parity(case, upstream_mili):
    _id, base, svar, cls, kwargs = case
    if not _base_present(base):
        pytest.skip(f"fixture {base} absent (run scripts/setup-parity.sh)")

    g = milox.open_database(base)
    w = upstream_mili.open_database(base, suppress_parallel=True)

    gr = g.query(svar, cls, **kwargs)
    wr = w.query(svar, cls, **kwargs)

    assert set(gr.keys()) == set(wr.keys())
    for k in gr:
        ge, we = gr[k], wr[k]
        assert ge["class_name"] == we["class_name"]
        assert ge["title"] == we["title"]
        assert list(ge["layout"]["components"]) == list(we["layout"]["components"]), (
            f"{k} components {ge['layout']['components']} != "
            f"{we['layout']['components']}"
        )
        assert np.array_equal(
            np.asarray(ge["layout"]["labels"], dtype=np.int64),
            np.asarray(we["layout"]["labels"], dtype=np.int64),
        ), f"{k} labels differ"
        assert np.array_equal(
            np.asarray(ge["layout"]["states"], dtype=np.int64),
            np.asarray(we["layout"]["states"], dtype=np.int64),
        ), f"{k} states differ"
        gd, wd = np.asarray(ge["data"]), np.asarray(we["data"])
        assert gd.shape == wd.shape, f"{k} data shape {gd.shape} != {wd.shape}"
        assert np.array_equal(gd, wd), f"{k} data values differ"


def test_bogus_subrec_raises_like_upstream(upstream_mili):
    """A non-existent subrec name raises on both readers (no silent
    empty result) — the subrec= filter contract."""
    if not _base_present(str(_FDAMP1)):
        pytest.skip("fdamp1 absent")
    g = milox.open_database(str(_FDAMP1))
    w = upstream_mili.open_database(str(_FDAMP1), suppress_parallel=True)
    with pytest.raises(Exception):
        g.query("stress", "brick", states=[1], subrec="no_such_subrec")
    with pytest.raises(Exception):
        w.query("stress", "brick", states=[1], subrec="no_such_subrec")


def test_unexpected_kwarg_raises_mili_python_error():
    """Hidden-kwarg validation (miliinternal.py:1159) surfaced through
    the typed hierarchy."""
    if not _base_present(str(_FDAMP1)):
        pytest.skip("fdamp1 absent")
    g = milox.open_database(str(_FDAMP1))
    with pytest.raises(milox.MiliPythonError, match="unexpected keyword"):
        g.query("stress", "brick", states=[1], not_a_real_kwarg=3)


# Slice-A value cases lifted verbatim from upstream
# reference/mili-python/tests/test_bugfixes.py (scalar/vector +
# labels/states only — no VEC_ARRAY component resolution). Asserts
# milox against the hardcoded upstream-proven values, delivering the
# test_bugfixes coverage without the wrapper-layer import-redirect
# (that redirect is M4-followup; see m4.md decision 16).
_BUGFIX_SLICE_A = [
    # (id, rel base, svar, class, kwargs, [(idx_tuple, expected, tol)])
    ("vrt_BS-refrcx", "serial/vrt_BS/vrt_BS.plt", "refrcx", "node",
     dict(labels=[67], states=[5]), [((0, 0, 0), 749.95404, 1e-3)]),
    ("fdamp1-refrcx", "serial/fdamp1/fdamp1.plt", "refrcx", "node",
     dict(labels=[6], states=[1, 2, 3]),
     [((0, 0, 0), 0.0, 1e-9), ((1, 0, 0), -195.680618, 1e-2),
      ((2, 0, 0), -374.033813, 1e-2)]),
    ("beam_udi-nodpos", "serial/beam_udi/beam_udi.plt", "nodpos", "node",
     dict(labels=[6], states=[3]),
     [((0, 0, 0), 499.86799237808793, 1e-3),
      ((0, 0, 1), 100.0, 1e-3), ((0, 0, 2), 198.08431992103525, 1e-3)]),
]


@pytest.mark.parametrize(
    "bf", _BUGFIX_SLICE_A, ids=[b[0] for b in _BUGFIX_SLICE_A]
)
def test_bugfixes_slice_a_values(bf):
    _id, rel, svar, cls, kwargs, checks = bf
    base = REPO_ROOT / "reference" / "mili-python" / "tests" / "data" / rel
    if not _base_present(str(base)):
        pytest.skip(f"{rel} absent")
    g = milox.open_database(str(base))
    r = g.query(svar, cls, **kwargs)
    data = np.asarray(r[svar]["data"])
    for idx, expected, tol in checks:
        assert abs(float(data[idx]) - expected) <= tol, (
            f"{_id} {idx}: {float(data[idx])} != {expected}"
        )


# Slice B — bare-component-of-VEC_ARRAY + ip-label semantics +
# cross-material InconsistentIpCounts (M4-followup, now landed; see
# m4.md §"Slice B" / decision 17). Exact upstream oracle values lifted
# verbatim from reference/mili-python/tests/test_bugfixes.py
# (`VectorsInVectorArrays`, `InconsistantIntPointsForElementClassResult`).
#
# NOTE: the `sx`/`brick` cross-material case uses `serial/basic1`, not
# `parallel/basic1`. Upstream's own `InconsistantIntPointsForElementClassResult`
# uses serial; the parallel fragments carry no element-set TI params
# (verified across all 8 parts) so upstream there treats `sx` as a
# plain scalar, ignores `ips`, and does NOT raise — there is no Slice-B
# oracle on the parallel base. Recorded in
# planning/mili-rs/status.md §"Surprises worth remembering".
_BUGFIX_SLICE_B = [
    ("d3samp4-eps-ip1", "serial/d3samp4/d3samp4.plt", "eps", "shell",
     dict(labels=[1], states=[2], ips=[1]), (0, 0, 0), 2.3293568e-02),
    ("d3samp4-eps-ip2", "serial/d3samp4/d3samp4.plt", "eps", "shell",
     dict(labels=[1], states=[2], ips=[2]), (0, 0, 0), 7.1215495e-03),
    ("d3samp4-sy-ip1", "serial/d3samp4/d3samp4.plt", "sy", "shell",
     dict(labels=[24], states=[10], ips=[1]), (0, 0, 0), -2.20756815e-03),
    ("basic1-sx-ip4", "serial/basic1/basic1.plt", "sx", "brick",
     dict(labels=[144, 212], states=[101], ips=4), (0, 0, 0), 3.36948112e-02),
]


@pytest.mark.parametrize(
    "bf", _BUGFIX_SLICE_B, ids=[b[0] for b in _BUGFIX_SLICE_B]
)
def test_bugfixes_slice_b_oracle(bf):
    _id, rel, svar, cls, kwargs, idx, expected = bf
    base = REPO_ROOT / "reference" / "mili-python" / "tests" / "data" / rel
    if not _base_present(str(base)):
        pytest.skip(f"{rel} absent")
    g = milox.open_database(str(base))
    r = g.query(svar, cls, **kwargs)
    assert abs(float(np.asarray(r[svar]["data"])[idx]) - expected) <= 1e-7


def test_bugfixes_slice_b_component_names():
    """Slice B also fixes the `f"{comp} ipt. {label}"` component
    naming (`miliinternal.py:1367`). Values + names lifted from
    `VectorsInVectorArrays` (d3samp4)."""
    base = (
        REPO_ROOT / "reference" / "mili-python" / "tests" / "data"
        / "serial" / "d3samp4" / "d3samp4.plt"
    )
    if not _base_present(str(base)):
        pytest.skip("d3samp4 absent")
    g = milox.open_database(str(base))
    r = g.query("eps", "shell", labels=[1], states=[2])
    assert list(r["eps"]["layout"]["components"]) == ["eps ipt. 1", "eps ipt. 2"]
    r = g.query("sy", "shell", labels=[24], states=[10])
    assert list(r["sy"]["layout"]["components"]) == ["sy ipt. 1", "sy ipt. 2"]
    r = g.query("sy", "shell", labels=[24], states=[10], ips=[1])
    assert list(r["sy"]["layout"]["components"]) == ["sy ipt. 1"]


def test_bugfixes_cross_material_inconsistent_ips_contract():
    """serial/basic1 sx/brick over materials 5 (8 IPs) & 7 (9 IPs)
    with no `ips` must raise (upstream ValueError; milox typed
    InconsistentIpCounts → MiliPythonError). Mirrors upstream's
    `InconsistantIntPointsForElementClassResult` (serial base). Must
    NOT silently return mismatched data."""
    base = (
        REPO_ROOT / "reference" / "mili-python" / "tests" / "data"
        / "serial" / "basic1" / "basic1.plt"
    )
    if not _base_present(str(base)):
        pytest.skip("basic1 absent")
    g = milox.open_database(str(base))
    with pytest.raises(milox.MiliPythonError):
        g.query("sx", "brick", labels=[144, 212], states=[101])
