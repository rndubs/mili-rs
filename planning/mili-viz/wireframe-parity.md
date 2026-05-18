# mili-viz client — wireframe parity tracker

Region-by-region diff of the `mili-viz` client against the agreed
wireframe spec ([`griz_wgpu_wireframes/README.md`](griz_wgpu_wireframes/README.md)).
This is the **placeholder/partial inventory** that `status.md`
(milestone-level) and `bug-tracker.md` (defects) deliberately do not
track at this granularity. Move rows into the ✅ column as they land;
keep the cross-refs current.

Conventions:

- **Status** `✅ done` · `🟡 partial` · `🔴 placeholder` (a stub that
  looks like a feature but does nothing real) · `⬜ missing` ·
  `⏸️ deferred` (intentionally out of scope for now, has/needs a
  milestone).
- **Ref** the authoritative milestone/decision/defect cross-link
  (`status.md` item N, `phase-5-*.md` Decision N, `VB-NNN`), or `—`.
- Source of truth for *milestones* stays `status.md`; for *defects*
  `bug-tracker.md`. This doc is the *wireframe-coverage* view; when a
  row flips, update the cross-linked tracker too.

Derived 2026-05-18 by reading `crates/mili-viz-client/src/{shell,app,renderer}.rs`
against the wireframe README. Phase 5 M1–M4 + M3.5 landed; M5/M6 not
started.

---

## Window shape & layout

| Item | Status | Notes | Ref |
| ---- | ------ | ----- | --- |
| L1 default layout (6 regions, egui panel mapping, default sizes) | ✅ done | `build_shell_ui` | status 16 |
| L2 — AI panel expanded (28 px rail → 340 px dock) | ⬜ missing | right panel is a hard 28 px rail, no expand path | M6 |
| L3 — focus mode (dock→icon rail, AI/tabs hidden, `Ctrl+\`) | ⬜ missing | — | — |

## Menu bar

| Item | Status | Notes | Ref |
| ---- | ------ | ----- | --- |
| `Control · Results · Time · Plot · Help` items | 🔴 placeholder | the other 5 are still `ui.menu_button(m, |_| {})` — open but empty | — |
| `Rendering` menu (wireframe/edge toggles) | ✅ done | real three-way `shaded / shaded+edges / wireframe` toggle → `UiAction::SetRenderMode` | VB-003 / status 23 |
| `Picking` menu (enable client-side picking) | ✅ done | `enable picking` toggle → `UiAction::TogglePicking`; ray-cast vs. cached hull | status 23 |
| View / Preferences (theme, layout tweaks) | ⬜ missing | no menu to host the Tweaks set | — |

## Toolbar

| Item | Status | Notes | Ref |
| ---- | ------ | ----- | --- |
| Transport `⏮◀▶⏭`, stride, animate/pause/stop, view reset/fit | ✅ done | — | status 16 |
| Overlay chips (`title state legend axes bbox`), state counter | ✅ done | chips drive HUD; `state N / total` right-aligned | status 16 |

## Left dock

| Item | Status | Notes | Ref |
| ---- | ------ | ----- | --- |
| Runs/sessions section | ✅ done | one row + status dot | status 16 |
| Results → `derived` | 🟡 partial | hard-coded 7-name `DERIVED_RESULTS`, not a real catalog | phase-5-m3 Dec 47 |
| Results → `primal` / `time-indep` | 🔴 placeholder | literal `"(catalog: M4+)"`; frozen proto has no svar catalog | phase-5-m3 Dec 47 |
| Colormap (ramp picker + manual legend limits) | ✅ done | extra vs wireframe but functional | phase-5-m4 Dec 66 |
| Materials section | 🟡 partial | lists class names w/ static `●`; **no enable/disable, no dots, no row interaction** | status 8 (server-side done) |
| Surfaces section | 🔴 placeholder | `"(surfaces: M4+)"` | — |
| Per-section row-count badges; Picking glyph row | ⬜ missing | only Results/Materials have a count | — |

## Viewport overlays

| Item | Status | Notes | Ref |
| ---- | ------ | ----- | --- |
| title / state / legend | ✅ done | data-driven | status 16 |
| axes gizmo | 🟡 partial | static triad — does **not** track camera orientation | — |
| bbox | 🔴 placeholder | fixed 18% dashed inset, not the real projected bbox | — |
| Multi-client peer banner | ⬜ missing | — | M6 |
| Not-attached card | ✅ done | — | status 16 |

## AI Assistant panel

| Item | Status | Notes | Ref |
| ---- | ------ | ----- | --- |
| Collapsed rail / expand / transcript / composer / status pill / provenance | 🔴 placeholder | renders only the text `"AI"` in a 28 px rail | M6 / client.md |

## Bottom tabs

| Item | Status | Notes | Ref |
| ---- | ------ | ----- | --- |
| Command line (Layer-0 verbatim, transcript) | ✅ done | — | status 17 |
| Scripting runner | 🔴 placeholder | "requires pygriz (Phase 6)"; **Phase 6 M1–M3 now landed → unblocked, not yet wired** | phase-5-m3.5 Dec 49 / status 18–20 |
| Time-history plot | 🟡 partial | fed by `ResultState` min/max envelope, not the `Query` per-element series; server `Query` is a stub | phase-5-m3.5 Dec 50 |
| Whole-region hide (tweak) | 🟡 partial | collapses body only; 22 px strip always present | — |

## Status bar

| Item | Status | Notes | Ref |
| ---- | ------ | ----- | --- |
| attached / proto / pick / fps row | 🟡 partial | `proto v1` hard-coded, no peer count; `pick:` now **live** (client-side ray-cast readout, off by default) | status 23 |

## Tweaks / Preferences

| Item | Status | Notes | Ref |
| ---- | ------ | ----- | --- |
| Theme (dark/light), left-dock collapse, full bottom-tab hide, AI-panel position | ⬜ missing | no surface at all; `let _ = Overlay::Title;` marks the unbuilt persistence hook | — |

## Renderer / rendering modes

| Item | Status | Notes | Ref |
| ---- | ------ | ----- | --- |
| Filled `TriangleList` pass | ✅ done | `renderer.rs:149` | status 15 |
| Wireframe / element-edge / hidden-line mode | ✅ done | `LineList` edge pass via `Mesh::edge_indices`; `Renderer::set_mode` → `Shaded` (default, byte-stable) / `Edges` (hidden-line overlay) / `Wireframe` | VB-003 / status 23 |

## Cross-cutting gaps

| Item | Status | Notes | Ref |
| ---- | ------ | ----- | --- |
| File → Open / `rfd` picker | ⏸️ deferred | own milestone (maintainer decision); load via CLI `-i` / Layer-0 `load` only | status 21 |
| Picking (client-side from cached `GeometryRef`) | 🟡 partial | ray-cast vs. cached hull → live status-bar readout (node/tri/scalar). Frozen proto has no label catalog, so no `class N` mapping; no highlight glyph yet | status 23 |
| Agent / multi-client session states | ⬜ missing | thinking/running/interrupted/peer | M6 |
| Remote mode (gRPC+Flight wired to `connect`/`attach`) | ⏸️ deferred | server transport done (status 11); client wiring is Phase 5 M5 | status 22 |

---

## MVP cut (no AI, no remote)

The maintainer-scoped MVP excludes the AI panel (M6) and remote mode
(Phase 5 M5). The parity gaps that remain in-scope for MVP, roughly by
leverage:

1. **Menu bar** — 🟡 `Rendering` is now wired (VB-003); still to do:
   `Control`, `Picking`, the View/Preferences host.
2. ✅ **Wireframe / element-edge render mode** (VB-003) + its
   `Rendering` toggle — done.
3. **Materials enable/disable affordance** (server side already done,
   status 8 — GUI only).
4. 🟡 **Picking** + live status-bar readout — ray-cast + readout
   landed; a viewport highlight glyph + label-catalog mapping remain.
5. **Real bbox overlay + camera-tracking axes gizmo.**
6. **File → Open** (lift the deferral if MVP needs it).
7. **L3 focus mode + theme/tweaks surface.**
8. **Primal / time-indep result catalog** (needs a non-frozen-proto
   catalog path — design first).
9. **Wire the scripting tab** (now unblocked by Phase 6 M1–M3).

## Update protocol

When a row lands: flip its **Status**, tighten the **Notes**, and
update the cross-linked tracker (`status.md` item / `bug-tracker.md`
`VB-NNN` / the phase doc Decision) in the same change so the trackers
do not drift. Add new wireframe deltas as new rows under the right
region.
