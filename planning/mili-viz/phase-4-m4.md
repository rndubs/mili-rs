# Phase 4 M4 — landed (selection + enable/disable)

> **Status: ✅ COMPLETE.** This milestone is landed and frozen.
> Full live status in [`status.md`](status.md); this file is retained
> for the decision-number cross-references in the rest of the tree.

## What landed

- `enable`/`disable` (`MaterialVisibility`, `DELTA_MATERIALS`) filters
  the emitted triangle list by per-triangle material on the next
  `show`. A material is visible unless `Session.materials` maps it
  to `false`. The per-vertex scalar array, `num_vertices`, and the
  `ResultState.{min,max}` range are unchanged (the legend clamp is
  the client's per M3 Decision 15) — only `indices`/`tri_material`
  and therefore `num_indices` shrink. Blob magic and layout string
  unchanged (`MVG1`/`MVG2`).
- `select`/`clrsel` stays metadata-only: broadcast via the existing
  `DELTA_SELECTION` `SelectionState{by_class}` and the late-joiner
  `Snapshot.selection`, no geometry blob change (a client highlights
  selected elements client-side, mirroring M1 Decision 2's picking
  model). `clrsel` with an empty `class_name` clears the entire
  selection map (griz `poof` fidelity); a named class clears just
  that class.
- One `StateDelta` per `Execute` invariant preserved: each of
  `enable`/`disable`/`select`/`clrsel` emits exactly its own one
  delta; the visual effect lands when the client re-issues `show`.

## Gating test

`crates/mili-viz-server/tests/m4_visibility.rs::material_visibility_and_selection`
— asserts disable shrinks `num_indices` (vertices unchanged), the
filter composes with `MVG2` (scalar + range byte-stable), enable
restores byte-identical blobs, and selection deltas reach
subscribers + late-joiner `Snapshot`.

## Decisions

- Decisions 16–18 for this milestone are recorded in this file's
  git history; the index lives in [`status.md`](status.md). Any
  decision that *superseded* an earlier one is called out in
  status.md's TL;DR.
