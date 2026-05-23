# Phase 5 M8 — cut-plane gizmo + Rendering→Cut UI

> **Status: 🟡 PLANNED.** Client UI for the server-side
> [`phase-4-m8.md`](phase-4-m8.md) (`cutpln` operator). Sibling to
> [`phase-5-m7.md`](phase-5-m7.md) (render modes) and
> [`phase-5-m9.md`](phase-5-m9.md) (slice gizmo). Live status in
> [`status.md`](status.md). Decisions start at **84**.

## Why

`Cmd::Cutplane` is in the frozen proto; M8 (server) implements it;
this milestone exposes it through the L1 wireframe (`Rendering`
menu + a viewport gizmo). The gizmo is the natural shape — griz
and VisIt both ship one — and the proto already provides every
operation it needs (the plane is `(origin, normal, relative)`; the
gizmo emits a `Cmd::Cutplane` per gesture-throttled frame; the
server is the authority on the resulting clip).

## What lands

- `Rendering` menu (`crates/mili-viz-client/src/shell.rs`,
  already the home for `RenderMode` per VB-003 / VB-004) gains a
  three-state **Cut** sub-menu:
  - **Cut: Off** (default; clears `Session.cut` server-side)
  - **Cut: Plane…** (opens the gizmo + lowers a `Cmd::Cutplane`
    with the gizmo's current plane)
  - **Cut: Reset to bbox centre** (places the gizmo at the mesh
    centroid with the camera's view-plane normal)
- The gizmo is an egui-overlay-rendered handle (a flat disk +
  arrow normal, drawn in the additive `egui` second pass
  established at [`phase-5-m3.md`](phase-5-m3.md) Decision 45 —
  **no new render pipeline**). The handle is drag-rotatable
  (changes normal) and drag-translatable along the normal
  (changes origin's projection onto the normal).
- Drag-end commits the `Cmd::Cutplane`; **drag-in-progress** emits
  a gesture-throttled preview command (≤ 30 Hz, the same throttle
  as the M4 camera reconcile path) so the user sees the cut
  update live without flooding the bus. A `Preferences →
  Interactive clip` toggle disables the preview path for low-
  bandwidth links (the cut commits only on drag-end).
- Status-bar readout: when `Session.cut` is set, the status bar
  shows `cut: o=(x,y,z) n=(nx,ny,nz)` (cribbing the existing
  picking-readout structure).

## Decisions

### Decision 84 — gizmo is **egui-overlay**, not a `wgpu` mesh pipeline; reuses the M3 additive-paint seam

The cut-plane gizmo is a UI affordance, not a renderable mesh
feature. Drawing it as a `wgpu` triangle/line set requires a new
pipeline, new uniforms, new draw call; drawing it through `egui` is
a `painter.add(LineSegment2D)` + a 3-D→2-D projection through the
live `Camera` (the projection math is already in
`crates/mili-viz-client/src/picking.rs` for the picking-glyph
overlay). One pattern, one paint, no `Renderer::new` movement.

**Trade-off recorded.** A `wgpu` gizmo would render correctly
under transformations the `egui` overlay does not (e.g. an
off-screen handle still casts shadows on the mesh). For a flat
disk + arrow, those failure modes do not visually matter; the
overlay shape is the cheaper, smaller-blast-radius landing.

### Decision 85 — interactive preview is **gesture-throttled at 30 Hz** and rides the existing `DELTA_RESULT` broadcast; drag-end is the canonical commit

The same predict-and-reconcile shape M4 used for the camera
(`Camera::from_orbit` overwrites a predicted camera field-for-field
on each `DELTA_CAMERA`) applies here: the client predicts the cut
plane locally (draws the gizmo at the dragged position immediately)
and lowers a `Cmd::Cutplane` at most every 33 ms; each broadcast
`DELTA_RESULT` re-uploads the (server-cut) geometry. Drag-end
guarantees one final, un-throttled commit so the steady-state cut
matches the gizmo position exactly.

A `Preferences → Interactive clip = off` toggle (Decision 84
follow-up) suppresses the preview commands — useful on remote
sessions where the round-trip cost dominates. The toggle is
cross-session-persisted via the existing `PersistedTweaks` config
(status.md item 23) — no new storage.

**Trade-off recorded.** Committing on **every** drag frame floods
the bus on remote sessions; committing **only on drag-end** loses
the live-preview feel. The throttled middle is the same compromise
M4's camera reconcile picked and validates well there.

### Decision 86 — the gizmo's initial position is the **mesh AABB centre**, with the **view normal** as the default plane normal; named-view-style snapping is a later polish

When the user picks `Cut: Plane…` the gizmo appears at
`Mesh::aabb` centre with the camera's view-plane normal. This
matches griz `cutpln` defaults and gives a sensible "starts in the
middle of the mesh, facing the screen" placement that the user
then drags from. Axis-aligned snapping (`Shift`-drag → snap normal
to nearest world axis) is a polish, not blocking.

**Trade-off recorded.** Picking a named view ("XZ slice") was
considered as the initial UI affordance but it presumes a saved
named-view convention this client does not yet have wired through;
the bbox-centre default works with zero session-state assumed.

## Gating test

`crates/mili-viz-client/tests/m8_cut_gizmo.rs` — always-on:
gizmo position → `Cmd::Cutplane` field-for-field lowering;
drag-throttle accumulator unit test (no command emitted within 33
ms window, exactly one emitted on drag-end after the window);
`Preferences → Interactive clip` toggle suppresses preview but
not commit. Skip-on-absent composite render: `Cut: Plane…` on
`bar71.pltA` produces a re-rendered mesh distinct from the
unclipped composite.

## Trade-off recorded (milestone-level)

The compute stays server-side per maintainer direction — the
client only sends the plane and renders the returned blob. The
preview path is the one architectural concession to interactivity
(gesture-throttled command emission); it is bounded above (30 Hz)
and gated behind a Preference (Decision 85). No proto change in
this milestone; the typed `Cmd::Cutplane` arm is the one the
client lowers.
