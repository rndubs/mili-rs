"""milox vs. upstream `mili` — M1 metadata-accessor parity.

Diffs every M1 accessor (`import milox` vs `import mili`) on the parity
corpus + xmilics multi-fragment families, plus the explicit decision-4
rank-0 `state_maps()` assertion on a multi-fragment family.
"""

from __future__ import annotations

import numpy as np
import pytest

import milox

from conftest import CASES, MULTI_FRAGMENT_STATE_MAP_FAMILY, _xmilics_base

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


def _as_int_array(x):
    return np.asarray(list(x) if not isinstance(x, np.ndarray) else x, dtype=np.int64)


def _assert_dict_of_arrays_eq(name, got, want):
    gk, wk = set(got.keys()), set(want.keys())
    assert gk == wk, f"{name}: key mismatch got={sorted(gk)} want={sorted(wk)}"
    for k in gk:
        ga, wa = _as_int_array(got[k]), _as_int_array(want[k])
        assert np.array_equal(ga, wa), f"{name}[{k}]: {ga!r} != {wa!r}"


def test_times(both):
    g, w = both
    assert np.array_equal(
        np.asarray(g.times(), dtype=np.float64),
        np.asarray(w.times(), dtype=np.float64),
    )


def test_state_count(both):
    g, w = both
    assert g.state_count() == w.state_count()


def test_mesh_dimensions(both):
    g, w = both
    assert g.mesh_dimensions() == w.mesh_dimensions()


def test_class_names(both):
    g, w = both
    # Upstream order is __MO_class_data insertion; milox is mesh
    # class-declaration order. Compare order-insensitively (the set of
    # classes is the contract; ordering is not load-bearing for any
    # downstream call).
    assert set(g.class_names()) == set(w.class_names())


def test_labels(both):
    g, w = both
    gl = g.labels()
    wl = w.labels()
    # Upstream may carry a class with an empty label array that milox
    # omits when the core returns None; intersect on shared keys and
    # require any extra upstream keys to be empty.
    shared = set(gl) & set(wl)
    for k in shared:
        assert np.array_equal(_as_int_array(gl[k]), _as_int_array(wl[k])), f"labels[{k}]"
    for k in set(wl) - set(gl):
        assert _as_int_array(wl[k]).size == 0, f"labels: missing non-empty class {k}"
    for k in set(gl) - set(wl):
        assert _as_int_array(gl[k]).size == 0, f"labels: extra non-empty class {k}"


def test_materials(both):
    g, w = both
    _assert_dict_of_arrays_eq("materials", g.materials(), w.materials())


def test_material_numbers(both):
    g, w = both
    gm = np.unique(_as_int_array(g.material_numbers()))
    wm = np.unique(_as_int_array(w.material_numbers()))
    assert np.array_equal(gm, wm)


def test_element_sets(both):
    g, w = both
    _assert_dict_of_arrays_eq("element_sets", g.element_sets(), w.element_sets())


def test_integration_points(both):
    g, w = both
    _assert_dict_of_arrays_eq(
        "integration_points", g.integration_points(), w.integration_points()
    )


def test_state_maps_length(both):
    g, w = both
    assert len(g.state_maps()) == w.state_count()


def test_state_maps_rank0_parity(upstream_mili):
    """Decision-4: lock milox's rank-0 `state_maps()` reduction equal
    to upstream on a multi-fragment family, so future reduction drift
    is caught."""
    base = _xmilics_base(MULTI_FRAGMENT_STATE_MAP_FAMILY)
    if base is None:
        pytest.skip(f"{MULTI_FRAGMENT_STATE_MAP_FAMILY} xmilics family absent")

    g = milox.open_database(base)
    w = _open_upstream(upstream_mili, base)

    gm = g.state_maps()
    wm = w.state_maps()
    assert len(gm) == len(wm)
    for i, (a, b) in enumerate(zip(gm, wm)):
        # Phase I.3: merge_results=True now routes through the
        # _MiliInternal adapter over the Set backend, so state_maps()
        # returns upstream-shaped StateMap objects (attribute access),
        # not the old raw FFI dicts — matching upstream w exactly.
        assert a.time == pytest.approx(b.time), f"state {i} time"
        assert a.file_number == b.file_number, f"state {i} file_number"
        assert a.file_offset == b.file_offset, f"state {i} file_offset"
