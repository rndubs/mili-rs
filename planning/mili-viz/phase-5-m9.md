# Phase 5 M9 — slice gizmo + Rendering→Slice UI

> **Status: 🟡 PLANNED.** Client UI for the server-side
> [`phase-4-m9.md`](phase-4-m9.md) (`slice_only`-flagged
> `cutpln`). Sibling to [`phase-5-m7.md`](phase-5-m7.md) (render
> modes) and [`phase-5-m8.md`](phase-5-m8.md) (cut gizmo, whose
> gizmo machinery this milestone reuses). Live status in
> [`status.md`](status.md). Decisions start at **87**.

## Why

Slice is the cut operator's "show me only the cross-section" sister
— a separate UI verb because:
- griz / VisIt expose them separately and users learn them
  separately;
- a slice cap can carry a scalar mapping the cut hull cannot — the
  natural UI is `Rendering → Slice…` then `show <result>`, with the
  scalar painted on the slice plane only.

The gizmo / drag / preview machinery from
[`phase-5-m8.md`](phase-5-m8.md) is reusable verbatim; the only new
pieces are the menu entry and the `slice_only: true` flag on the
lowered `Cmd::Cutplane`.

## What lands

- `Rendering` menu adds a three-state **Slice** sub-menu, parallel
  to the M8 Cut sub-menu:
  - **Slice: Off** (clears `Session.slice` server-side)
  - **Slice: Plane…** (opens the shared M8 gizmo, lowers
    `Cmd::Cutplane{ slice_only: true, .. }`)
  - **Slice: Reset to bbox centre**
- The gizmo overlay (M8 Decision 84) gains a colour-coded variant:
  cut handles render in the existing M8 colour, slice handles in a
  contrasting second colour (so when both are active — composition
  per [`phase-4-m9.md`](phase-4-m9.md) Decision 80 — the user can
  tell which gizmo controls which plane).
- `Mesh::tri_flags` (delivered by `MVG3`, decoded per
  [`phase-5-m7.md`](phase-5-m7.md) Decision 82's adjacent path)
  carries the slice-cap sentinel (`tri_material == u32::MAX - 2`,
  per [`phase-4-m9.md`](phase-4-m9.md) Decision 80). The renderer
  draws slice-cap triangles with the active colormap when a result
  is mapped, neutral grey otherwise. The renderer does **not**
  attempt to draw the slice cap with a different shader pass —
  the existing depth-tested filled-triangle pipeline handles it.
- Status-bar readout (parallel to M8): when `Session.slice` is
  set, the status bar shows `slice: o=(…) n=(…)`.

## Decisions

### Decision 87 — slice and cut share the gizmo overlay code but are **distinct session-state verbs**; the UI does not collapse them into one toggle

griz / VisIt's idiom is two menu items and a user expectation that
they can be composed (cut a wedge out **and** slice through the
remainder to see the interior). The composition is the server
contract ([`phase-4-m9.md`](phase-4-m9.md) Decision 80). The UI
matches: two menu sub-trees, two gizmo handles when both are on,
two status-bar lines.

**Trade-off recorded.** A single `Rendering → Cut/Slice` chooser
with a dropdown for mode was considered — it is one fewer menu
entry — but it forces "pick one" semantics that contradict the
server's composition. Two parallel menus with shared gizmo
machinery is the same shape griz/VisIt land on for the same
reason.

### Decision 88 — when both cut **and** slice are active, the slice gizmo can be positioned **outside the kept volume** (the slice cuts the **original mesh**, not the cut residue); status-bar readout makes this explicit

Server-side, the slice plane intersects the **un-cut** mesh; this
matches the `Session.cut` / `Session.slice` independence in
[`phase-4-m9.md`](phase-4-m9.md). Visually, this means a slice
placed in the "cut-away wedge" still draws — it shows the
cross-section through cells that have been cut away. Some users
expect "slice within the kept volume only"; that is a different
operator (a clip-then-slice composition) and is not what griz
implements. Status-bar readout calls out the live state
explicitly so the user is never surprised by where their slice is
versus their cut.

**Trade-off recorded.** Restricting slice to the kept volume
("clip-and-slice") removes a degree of freedom users have come to
expect; offering both is a follow-up if the survey-of-griz-users
flags it.

### Decision 89 — slice rendering is always **opaque** by default; translucency requires the M7 `Translucent` or `Xray` `RenderMode`

A slice cap is a 2-D surface; rendering it translucent by default
makes it invisible against any near-coincident geometry. Opaque
slice + the existing scalar colormap is the high-information
default. Users wanting "translucent slice" pick
`RenderMode::Translucent` (which applies session-wide).

**Trade-off recorded.** A `Slice → Translucent` independent toggle
duplicates `RenderMode` semantics for one verb; reusing
`RenderMode` is the right factoring.

## Gating test

`crates/mili-viz-client/tests/m9_slice_gizmo.rs` — always-on:
slice menu → `Cmd::Cutplane{ slice_only: true }` lowering;
composition state (cut + slice both set → two gizmos rendered,
distinct colours, two status-bar lines); `Mesh::tri_flags`
sentinel-to-colour mapping (cap triangles read the active
colormap when a result is mapped; neutral grey otherwise).
Skip-on-absent composite render: `Slice: Plane…` on
`bar71.pltA` after `show pressure` produces a slice composite
distinct from the unclipped composite and from the M8 cut
composite.

## Trade-off recorded (milestone-level)

This milestone is a thin sibling of [`phase-5-m8.md`](phase-5-m8.md)
— reuses the gizmo overlay, the drag-throttle, the preview-suppress
preference, the predict-and-reconcile path — and adds the one
`slice_only: true` flag plus the slice-cap colour mapping. The
incremental complexity is small precisely because M7 froze the
contract carrying it.
