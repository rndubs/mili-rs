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

## VB-003 — mesh/element outlines unimplemented

- **Status:** known-gap
- **Reported:** 2026-05-18 (maintainer feedback, `bar71.pltA`)
- **Symptom:** no way to enable mesh / element edge outlines; the
  menu-bar `Rendering` menu does nothing.
- **Root cause:** never built. The renderer has only a filled
  `TriangleList` pass; the `Rendering` menu button is an empty
  placeholder (`shell.rs`, `ui.menu_button(m, |_| {})`); the toolbar
  overlay chips are HUD-only (`title/state/legend/axes/bbox`).
- **Fix:** none yet — needs a milestone: a hidden-line / element-edge
  render mode (extract unique edges, line pipeline or barycentric
  wireframe shader) plus a `Rendering`-menu or overlay-chip toggle.
  The wireframe "Tweaks" surface is the natural home.
- **Commit:** — · `status.md` item 23.

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
