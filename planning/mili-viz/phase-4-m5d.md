# Phase 4 M5d — landed (alt principal strain; closes deferred `*_alt`)

> **Status: ✅ COMPLETE.** This milestone is landed and frozen.
> Full live status in [`status.md`](status.md); this file is retained
> for the decision-number cross-references in the rest of the tree.

## What landed (two-part slice)

- **Part A — `mili-rs` core**: a new parity-validated kernel.
  `compute_principal_strain_alt` + `principal_strain_alt_spec` /
  `_primals` + a `PrincipalStrainAlt` enum added to
  `crates/mili-rs/src/derived.rs` (re-exported from `lib.rs`) and
  wired into `crates/mili-py/src/database.rs` (the `query()` derived
  dispatch arm + per-fragment guard). Mirrors upstream's separate
  `compute_function`s — a distinct closed-form `J2`/`J3` load-angle
  algorithm, not an extension of the eigensolver-based
  `PrincipalStrain`. Parity gated by
  `crates/mili-py/tests/test_alt_strain_parity.py` (`np.allclose`,
  rtol 1e-5 / atol 1e-6 — see `../mili-py/m4.md` Decision 27 for why
  it is not bitwise: numpy's float32 `arccos`/`cos` are numpy's own
  SIMD single-precision polynomials, ≠ system libm).
- **Part B — viz routing**: `show prin_strain[1-3]_alt` /
  `prin_dev_strain[1-3]_alt` routes through the now-parity-gated
  kernel into the unchanged M3/M5b element nodal-average scatter.
  The routing is a verbatim copy of the M5b
  `principal_strain_spec` branch, inserted immediately after it,
  with only `principal_strain_alt_spec` / `_primals` /
  `compute_principal_strain_alt` substituted. `MVG1`/`MVG2`,
  `flight_ticket`, and autoscale stay byte-stable.
- M5c Decision 28's `*_alt`→bare-hull assertion is intentionally
  removed (superseded by Decision 34, not regressed): `*_alt` now
  *resolves*, so that fallback check moves to `m5d_alt_strain.rs`.

## Gating test

`crates/mili-viz-server/tests/m5d_alt_strain.rs::derived_alt_principal_strain`
— on `serial/sstate/d3samp6` at state 22: structural + finite
samples + `ResultState.{min,max}` bracketing, the per-vertex
principal-ordering identity `1_alt ≥ 2_alt ≥ 3_alt` (and the `dev`
triple) within f32 slack, a non-vacuous finite-nonzero sample,
state tracking (22→60), and unknown / empty / `prin_strain1_alt`
→ `MVG2` totality + Decision-28 closure. Kernel numerics core-parity
owned; no `parity` feature in `mili-viz-server`.

## Decisions

- Decisions 32–34 for this milestone are recorded in this file's
  git history; the index lives in [`status.md`](status.md). Any
  decision that *superseded* an earlier one is called out in
  status.md's TL;DR (Decision 34 discharges M5c Decision 28).
