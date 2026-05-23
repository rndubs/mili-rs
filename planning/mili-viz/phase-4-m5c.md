# Phase 4 M5c — landed (surfstrain + nodal-time derived families)

> **Status: ✅ COMPLETE.** This milestone is landed and frozen.
> Full live status in [`status.md`](status.md); this file is retained
> for the decision-number cross-references in the rest of the tree.

## What landed

- `show <name>` for the nodal time-derived families
  (`disp_x/y/z`, `disp_mag`, `disp_rad_mag_xy`, `vel_x/y/z`,
  `acc_x/y/z`) and the per-face Hex surface strains
  (`surfstrain{x,y,z,xy,yz,zx}`), via the parity-exact `mili-rs`
  kernels.
- **Nodal-time routing**: a new node-direct branch group mirrors the
  `crates/mili-py` `query()` nodal dispatch for the single current
  state (calls `mili_rs::compute_node_{displacement,
  displacement_magnitude,velocity,acceleration}` +
  `nodal_reference_from_coords` at `reference_state == 0`, with the
  same velocity/acceleration state stencil as upstream). M3's
  node→vertex mapping was factored into a shared helper so the M3
  primal nodal path is byte-stable.
- **`surfstrain*` routing**: a separate
  `MeshTopology::scatter_hex_faces` per-face Hex connectivity gather
  over `mili_rs::Database::surface_strain_query`, nodal-averaged via
  a viz-local canonical hex face table transcribed from
  `reference/mili-python/src/mili/miliinternal.py:675-682` (a
  connectivity constant, like the existing `triangulation()` table —
  not a derived-formula re-port). The M5/M5b element scatter is not
  touched.
- `*_alt` re-deferred — no parity-exact `mili-rs` kernel existed at
  the time (M5d adds it).
- No proto change; no `parity` feature in `mili-viz-server`.

## Gating test

`crates/mili-viz-server/tests/m5c_derived.rs::derived_surfstrain_and_nodal_time`
— single-shared-gather invariants only: the exact
displacement-magnitude norm identities
`disp_mag ≈ ‖(disp_x,disp_y,disp_z)‖` and
`disp_rad_mag_xy ≈ ‖(disp_x,disp_y)‖` per vertex within f32
tolerance, `vel_x` at state 1 is identically zero (kernel-defined),
structural + state-tracking + `ResultState.{min,max}` bracketing for
`surfstrain*` and the kinematic families, plus
unknown / empty / re-deferred-`*_alt` → bare-hull totality checks.
Kernel numerics are core-parity owned.

## Decisions

- Decisions 28–31 for this milestone are recorded in this file's
  git history; the index lives in [`status.md`](status.md). Any
  decision that *superseded* an earlier one is called out in
  status.md's TL;DR (the `*_alt`→bare-hull check here was discharged
  by M5d Decision 34).
