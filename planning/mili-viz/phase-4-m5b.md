# Phase 4 M5b — landed (eigenvalue-based derived families)

> **Status: ✅ COMPLETE.** This milestone is landed and frozen.
> Full live status in [`status.md`](status.md); this file is retained
> for the decision-number cross-references in the rest of the tree.

## What landed

- `show <name>` for the 14 eigensolver-on-already-prepped-element-classes
  families: `prin_stress[1-3]`, `prin_dev_stress[1-3]`,
  `max_shear_stress`, `prin_strain[1-3]`, `prin_dev_strain[1-3]`,
  `vol_strain`.
- Routing reuses the M5 seam verbatim: two new branches in
  `MeshTopology::vertex_scalar` (`principal_stress_spec` then
  `principal_strain_spec`) inserted before the primal
  `classes_of_state_variable` lookup, each swapping only
  `*_spec` / `*_primals` / `compute_*` (calls
  `mili_rs::compute_principal_stress` /
  `mili_rs::compute_principal_strain`). `scatter_elements`,
  `finite_range`, `component0_map`, the `MVG2` blob, the M5
  stress-invariant branch, and the M3 primal path are byte-stable.
  No proto change, no `parity` feature in `mili-viz-server`.
- `surfstrain*`, the `*_alt` trig strains, and the nodal-time
  families stay deferred (M5c/M5d address them in follow-up slices).

## Gating test

`crates/mili-viz-server/tests/m5b_principal.rs::derived_principal_families`
— validates routing only via **single-shared-gather** algebraic
invariants (rejects cross-cardinality "trace" phrasings, which
were tried and fail at ~1.5e-3 on the IP-inconsistent `serial/basic1`
corpus for routings that are in fact correct): eigenvalue descending
order (`prin_stress1 ≥ 2 ≥ 3` etc.), relative deviatoric
tracelessness `|Σdev| ≤ 1e-3·Σ|dev|`, the max-shear relation
`max_shear_stress ≈ ½·(prin_stress1 − prin_stress3)`, plus
structural + state-tracking + bare-hull fallback. The eigensolver
itself is core-parity owned, not re-validated here.

## Decisions

- Decisions 22–24 for this milestone are recorded in this file's
  git history; the index lives in [`status.md`](status.md). Any
  decision that *superseded* an earlier one is called out in
  status.md's TL;DR.
