# mili-viz bug tracker

Running log of bugs found exercising the `mili-viz` client/server on
real corpora (the kind that fixture/parity tests don't catch because
they assert wire/byte invariants, not "does the GUI look right").
Newest first. `status.md` remains the **milestone** tracker; this is
the **defect** tracker — symptom → root cause → fix → commit, so a
cold reader can tell what we already hit and how it was resolved.

Conventions:

- **ID** `VB-NNN`, monotonic.
- **Commit** the short hash that resolved it (or `—` / `branch:<name>`
  while in flight). Cross-link the milestone item in `status.md` when
  there is one.
- **Status** `open` · `fixed` · `wontfix` · `known-gap` (intentionally
  deferred, needs a milestone).
- A regression test is the goal for every `fixed` row; when the bug is
  GUI-visual and not unit-coverable, say so explicitly.

---

## VB-005 — `Edges`/`Wireframe` mode draws hex face diagonals as if they were element edges

- **Status:** known-gap (fix planned in
  [`phase-4-m7.md`](phase-4-m7.md) Decision 73 +
  [`phase-5-m7.md`](phase-5-m7.md) Decision 82)
- **Reported:** 2026-05-23 (maintainer feedback, `bar71.pltA` —
  Hex corpus — under the release binary's `Edges` mode looked
  "like tet elements")
- **Symptom:** A hex element in `Edges` or `Wireframe` mode draws
  12 cube edges **plus 6 face diagonals**. The diagonals slice each
  face into two triangles and the wireframe reads as a triangulated
  soup rather than a cube grid — visually indistinguishable from a
  tet-mesh wireframe.
- **Root cause:** The geometry blob frozen at
  [`phase-4-m2.md`](phase-4-m2.md) Decision 11 (`MVG1`/`MVG2`) is a
  boundary-surface representation: per-superclass faces are
  tessellated into triangles (Hex → 12 tris, 2 per quad face) and
  the per-face triangulation diagonal becomes a real index-buffer
  edge. `Mesh::edge_indices`
  (`crates/mili-viz-client/src/mesh.rs:195-208`) derives the
  wireframe by walking the triangle list and deduplicating edges
  — the diagonals are dedupe-stable (each appears in exactly two
  triangles' shared edge) and survive as wireframe lines. There is
  no per-face-vs-diagonal signal in the `MVG1`/`MVG2` blob.
- **Fix (planned):** Server-side, ship the per-superclass element
  edges as an explicit buffer in the `MVG3` blob revision
  ([`phase-4-m7.md`](phase-4-m7.md) Decision 73 — Hex = 12, Tet =
  6, Quad = 4, etc., enumerated from a fixed table that mirrors
  `triangulation()`). Client-side, prefer `Mesh::element_edges`
  when present and fall back to the on-the-fly extractor when not
  ([`phase-5-m7.md`](phase-5-m7.md) Decision 82). The fallback
  preserves the (broken-but-known) behavior for older servers, so
  the M2/M3/M4 + MVP-polish composite gates stay byte-stable
  (VB-001).
- **Regression test:** part of the `phase-4-m7.md` gating test
  (`crates/mili-viz-server/tests/m7_mvg3.rs::volumetric_geometry_contract`,
  hex-emits-exactly-12-edges assertion) and the `phase-5-m7.md`
  gating test (`crates/mili-viz-client/tests/m7_render_modes.rs`,
  decoder + prefer-element-edges branch).
- **Commit:** `—` (in flight against the planned milestones)

---

## VB-004 — edge pipeline aborts startup (depth bias on `LineList`)

- **Status:** fixed
- **Reported:** 2026-05-19 (maintainer feedback, release binary on a
  real corpus, macOS/Metal)
- **Symptom:** the release binary aborts at startup on a real device:

  ```
  wgpu error: Validation Error
    In Device::create_render_pipeline, label = 'edge pipeline'
      Depth/stencil state is invalid
        Depth bias is not compatible with non-triangle topology LineList
  ```

  `Renderer::new` runs inside a non-unwinding winit callback, so the
  panicking uncaptured-error handler aborts the process.
- **Root cause:** the VB-003 edge pipeline (`renderer.rs`) is a
  `PrimitiveTopology::LineList` pass but carried a non-zero
  `DepthBiasState { constant: -1, slope_scale: -1.0, .. }` (added to
  pull the hidden-line overlay in front of the coincident faces).
  wgpu 29 validation rejects any non-zero depth bias on a non-triangle
  topology at pipeline-creation time. The `Shaded` triangle pipeline
  (`DepthBiasState::default()`) is fine, and the headless composite
  gate only exercises `Shaded`, so CI never built the edge pipeline.
- **Fix:** **client-side, no proto change.** The edge pipeline now uses
  `DepthBiasState::default()` (zero — legal for `LineList`). The
  overlay still draws on top because the edges are extracted from the
  triangle mesh and share its exact vertices: along a coincident face
  edge the interpolated fragment depth equals the triangle's, so the
  existing `depth_compare: LessEqual` already lets the line pass over
  the fill — the bias was redundant for the common (on-face-edge)
  case. **Tradeoff:** where an edge passes *in front of a different,
  near-coincident face* it no longer gets the ~1-unit pull toward the
  camera, so minor z-fighting is possible on such crossings; acceptable
  versus aborting at startup, and not observed on `bar71.pltA`. A
  legal slope-independent pull (constant offset in the line vertices /
  a depth-range tweak) is a future polish if z-fight is reported.
- **Commit:** `branch:claude/fix-depth-bias-validation-ZppGM` ·
  `status.md` item 23.
- **Regression test:** `tests/vb004_edge_pipeline_validation.rs` —
  builds a real `Renderer` via `headless_device()` inside a
  `wgpu::ErrorFilter::Validation` scope and asserts `pop_error_scope`
  is `None`, so the edge/wireframe pipeline is validated in CI even
  though the composite gate stays on `Shaded`. Skip-on-absent when no
  adapter (CLAUDE.md), always-on otherwise.

## VB-003 — mesh/element outlines unimplemented

- **Status:** fixed
- **Reported:** 2026-05-18 (maintainer feedback, `bar71.pltA`)
- **Symptom:** no way to enable mesh / element edge outlines; the
  menu-bar `Rendering` menu does nothing.
- **Root cause:** never built. The renderer had only a filled
  `TriangleList` pass; the `Rendering` menu button was an empty
  placeholder (`shell.rs`, `ui.menu_button(m, |_| {})`); the toolbar
  overlay chips are HUD-only (`title/state/legend/axes/bbox`).
- **Fix:** **client-side, no proto change.** `Mesh::edge_indices`
  extracts the unique undirected triangle edges; a second `LineList`
  pipeline (`edges.wgsl`, sharing the camera bind group + vertex
  buffer, depth-tested `LessEqual` with a small negative bias) draws
  them. `Renderer::set_mode` picks the mode:
  - `Shaded` (default) — byte-for-byte the original single filled
    pass, so the M3 composite gate (`render_shell_to_image`, always
    `Shaded`) and VB-001 are untouched;
  - `Edges` — depth-tested edge overlay on the filled hull, so only
    the visible front edges draw (hidden-line overlay);
  - `Wireframe` — edges only over the cleared background (see-through
    wireframe).
  The menu-bar `Rendering` menu now hosts the three-way toggle and
  emits the pure-client `UiAction::SetRenderMode`, lowered in `app.rs`
  to `Renderer::set_mode` (no frozen-proto command).
- **Commit:** `branch:claude/update-wireframe-parity-J0XIn` ·
  `status.md` item 23.
- **Regression test:** `tests/vb003_render_modes.rs` — always-on pure
  logic (mode default, the `ShellState` switch, `edge_indices`
  dedup) plus a skip-on-absent headless leg asserting `Shaded` is
  byte-identical to `render_mesh_to_image` while `Edges`/`Wireframe`
  change pixels. The GUI render itself is not headlessly verifiable
  in CI (no display).

## VB-002 — stepping/animation froze the mesh (time/state mismatch)

- **Status:** fixed
- **Reported:** 2026-05-18 (maintainer feedback, `bar71.pltA`)
- **Symptom:** `▶ animate` and the manual `⏮ ◀ ▶ ⏭` buttons advanced
  the state counter and the time-history plot, but the deformed hull
  and field colours stayed at the load state; stepping after an
  animation felt "stuck" (stride looked pinned at 1).
- **Root cause:** the frozen proto contract makes
  `state`/`next`/`prev`/`first`/`last` a bare `DELTA_STATE` — it moves
  the cursor but carries no `GeometryRef`. Nothing re-encoded the
  active result at the new state, so geometry never refreshed; the
  time-history series (fed off `DELTA_RESULT`) only had the load-state
  sample.
- **Fix:** **client-side, contract-preserving** (server `DELTA_STATE`
  stays `DELTA_STATE`, so Layer-0 ≡ raw and the m6/fan-out gates are
  untouched). `App::ingest_deltas` round-trips the active `show` once
  per delta drain when the cursor moved
  (`App::refresh_result_geometry`), coalescing a strided burst to the
  final state. This also makes the time-history accumulate while
  scrubbing/animating.
- **Rejected alternative:** enriching the server's `Step`/`SetState`
  to a combined `DELTA_SNAPSHOT` (state + re-evaluated result). It
  works end-to-end but changes the observable wire kind for a loaded
  result and red-lines the frozen `m6_transport` /
  `subscription_fanout` acceptance gates — a contract change, out of
  scope for a bug fix.
- **Commit:** `0a54eab` · `status.md` item 23.
- **Regression test:** GUI-visual; not unit-covered. The contract
  invariant it must not break *is* covered (server suites green).

## VB-001 — model framed and orbited off-centre

- **Status:** fixed
- **Reported:** 2026-05-18 (maintainer feedback, `bar71.pltA`)
- **Symptom:** the part opened off-centre (pushed under the left
  dock) and the orbit pivot was not the centre of the visible scene.
- **Root cause:** the `wgpu` mesh pass rendered to the *full* surface
  while the `egui` left dock / bottom tabs / AI rail occlude it
  asymmetrically. The camera focus (orbit centre) projected to the
  full-window centre, which sits left of — and above — the visible
  central viewport.
- **Fix:** `build_shell_ui` publishes the leftover central rect as
  resolution-independent screen fractions (`ShellState::scene_frac`);
  the app maps it onto the physical surface and `Renderer::render_in`
  restricts the pass to that sub-rect with the projection aspect taken
  from it. `App::viewport` returns the visible-scene size so
  orbit/pan/zoom sensitivity matches what the user sees. One-frame
  stale on a panel drag (invisible). The headless M3 composite path
  (`render_shell_to_image`) still renders full-surface, so its
  byte-stable gate is unaffected.
- **Commit:** `0a54eab` · `status.md` item 23.
- **Regression test:** GUI-visual; not unit-covered (no display in
  CI). The M3 composite byte-stability gate still passes.
