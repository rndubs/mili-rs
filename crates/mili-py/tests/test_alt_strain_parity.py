"""milox vs. upstream `mili` — `*_alt` griz closed-form principal-strain
parity (Part A of the derived `*_alt` slice).

The six `prin_strain[1-3]_alt` / `prin_dev_strain[1-3]_alt` variants are
a *distinct* closed-form load-angle algorithm (no eigensolver — see
`mili_rs::compute_principal_strain_alt`). Unlike the eigensolver
families they intrinsically need f32 `arccos`/`cos`; numpy's float32
`arccos`/`cos` are numpy's own SIMD single-precision polynomials, which
differ from system libm (what Rust `f64::{acos,cos}`-then-cast and any
cross-language port resolve to) by 1–2 ULP. The worst observed absolute
deviation on d3samp6 is ≈ 1.7e-10 against strain magnitudes ~1e-2, with
only sub-1e-9-magnitude jitter at the `J2` limit boundary.

So this gate is `np.allclose` to a tight f32 tolerance vs the real
`mili` oracle (not bitwise `np.array_equal`) — the structurally exact,
numerically faithful contract that matches what `*_alt` is *for*
(upstream's own docstrings: debug-only "alternate calculation methods …
to … check[] to see if the methods matched"; upstream ships no `*_alt`
*value* test, only listing). Rationale pinned in
planning/mili-py/m4.md Decision 27.

Skip-not-fail when the submodule corpus or the upstream oracle is
absent (CLAUDE.md skip-on-absent discipline; mirrors test_query_parity).
"""

from __future__ import annotations

import numpy as np
import pytest

import milox

from conftest import SERIAL, _has_afile

_ALT_NAMES = [
    "prin_strain1_alt",
    "prin_strain2_alt",
    "prin_strain3_alt",
    "prin_dev_strain1_alt",
    "prin_dev_strain2_alt",
    "prin_dev_strain3_alt",
]

# d3samp6 (serial/sstate) — the exact corpus upstream's
# SerialDerivedExpressions exercises; `brick`/`shell` are the classes
# the `*_alt` listing (test_derived.py BRICK_DERIVED/SHELL_DERIVED)
# resolves these names for.
_D3 = SERIAL / "sstate"
_CLASSES = ["brick", "shell"]
# Stressed transient states spanning the run (incl. state 22, which
# carries the largest non-alt/alt divergence — the f32-transcendental
# worst case — and the J2-limit boundary elements).
_STATES = [1, 11, 22, 50, 101]

# Tolerance: worst observed abs deviation ≈ 1.7e-10 (transcendental
# ULP) on strain magnitudes ~1e-2; atol comfortably above the noise and
# the ≤1e-9 J2-limit-boundary jitter, far below any physical signal.
_RTOL = 1e-5
_ATOL = 1e-6


def _present() -> bool:
    return _D3.is_dir() and _has_afile(_D3, "d3samp6.plt")


@pytest.mark.skipif(
    not _present(),
    reason="serial/sstate/d3samp6 absent (run scripts/setup-parity.sh)",
)
@pytest.mark.parametrize("cls", _CLASSES)
@pytest.mark.parametrize("name", _ALT_NAMES)
def test_alt_strain_oracle_tolerance(name, cls, upstream_mili):
    base = str(_D3 / "d3samp6.plt")
    g = milox.open_database(base)
    w = upstream_mili.open_database(base, suppress_parallel=True)

    n = g.state_count()
    states = sorted({min(s, n) for s in _STATES})

    gr = g.query(name, cls, states=states)
    wr = w.query(name, cls, states=states)

    assert set(gr.keys()) == set(wr.keys()) == {name}
    ge, we = gr[name], wr[name]

    assert ge["class_name"] == we["class_name"]
    assert ge["title"] == we["title"]
    assert ge["source"] == we["source"] == "derived"
    assert list(ge["layout"]["components"]) == list(
        we["layout"]["components"]
    ) == [name]
    # Structural parity is exact (same gather): labels + states identical.
    assert np.array_equal(
        np.asarray(ge["layout"]["labels"], dtype=np.int64),
        np.asarray(we["layout"]["labels"], dtype=np.int64),
    ), f"{name}/{cls} labels differ"
    assert np.array_equal(
        np.asarray(ge["layout"]["states"], dtype=np.int64),
        np.asarray(we["layout"]["states"], dtype=np.int64),
    ), f"{name}/{cls} states differ"

    gd = np.asarray(ge["data"])
    wd = np.asarray(we["data"])
    assert gd.dtype == wd.dtype, f"{name}/{cls} dtype {gd.dtype} != {wd.dtype}"
    assert gd.shape == wd.shape, f"{name}/{cls} shape {gd.shape} != {wd.shape}"

    # Finite structure must match exactly (no spurious NaN/Inf).
    fg, fw = np.isfinite(gd), np.isfinite(wd)
    assert np.array_equal(fg, fw), f"{name}/{cls} finite mask differs"

    # Numeric parity to the f32 tolerance over the finite values.
    assert np.allclose(
        gd[fw], wd[fw], rtol=_RTOL, atol=_ATOL
    ), (
        f"{name}/{cls} exceeds tolerance: max abs "
        f"{np.max(np.abs(gd[fw] - wd[fw])) if fw.any() else 0.0}"
    )

    # Sanity: the closed-form branch actually ran (the J2 limit mask is
    # not all-false → at least one non-zero element), so the tolerance
    # check is not a vacuous all-zeros pass. (prin_strain2_alt is
    # legitimately ~1e-9 on this corpus — a real, tiny signal, not a
    # degenerate zero, so the guard is "exercised", not a magnitude
    # floor.)
    assert np.any(wd[fw] != 0.0), (
        f"{name}/{cls}: oracle is identically zero — kernel not exercised"
    )
