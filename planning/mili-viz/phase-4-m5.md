# Phase 4 M5 — landed (derived results: scalar stress invariants slice)

> **Status: ✅ COMPLETE.** This milestone is landed and frozen.
> Full live status in [`status.md`](status.md); this file is retained
> for the decision-number cross-references in the rest of the tree.

## What landed

- `show <stress-invariant>` for the four scalar invariants
  `pressure` / `eff_stress` / `triaxiality` / `norm_press` resolves
  via `mili_rs::stress_invariant_spec`, queries the component
  stress primals on each prepped element class with
  `Database::query_full` at the current state, computes per element
  with `mili_rs::compute_stress_invariant`, and feeds the
  per-element `label → value` map into M3's unchanged nodal-average
  scatter (same `MVG2` blob, same `ResultState.{min,max}` autoscale,
  same `NaN`-untouched / component-0 conventions). Unknown derived
  names fall back to the M3 bare hull.
- **Supersedes M1 Decision 5.** That pre-commitment assumed no
  upstream oracle for viz derived results and pre-committed to a
  griz golden + tolerance. The oracle in fact already exists:
  Phases 1–3 ported every derived expression from
  `reference/mili-python/src/mili/derived.py` into
  `crates/mili-rs/src/derived.rs`, bit-exact validated against the
  `mili` Python package by the frozen `mili-rs`/`milox` parity
  suite. M5-viz reuses that kernel verbatim — no formula port, no
  griz golden, no `parity` feature in `mili-viz-server` (preserves
  the M2 boundary).
- Eigenvalue families, per-face Hex, and nodal-time families are
  explicitly deferred as later sub-slices (M5b/M5c/M5d).

## Gating test

`crates/mili-viz-server/tests/m5_derived.rs::derived_stress_invariants`
— gates the routing (not the kernel) via the linear-pressure
identity `pressure ≈ -1/3·(sx+sy+sz)` per node within f32 tolerance
(averaging commutes with the linear combination), plus structural +
state-tracking + bare-hull-fallback assertions for the nonlinear
invariants.

## Decisions

- Decisions 19–21 for this milestone are recorded in this file's
  git history; the index lives in [`status.md`](status.md). Any
  decision that *superseded* an earlier one is called out in
  status.md's TL;DR (notably Decision 19 supersedes M1 Decision 5).
