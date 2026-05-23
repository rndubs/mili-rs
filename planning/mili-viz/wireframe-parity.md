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
| L3 — focus mode (dock→icon rail, AI/tabs hidden, `Ctrl+\`) | ✅ done | `Ctrl+\` toggles `focus_mode` (`set_focus_mode` also collapses the dock to the R/M/S/P rail); the AI rail + bottom tabs are hidden; a rail glyph or a second `Ctrl+\` restores full L1. Default off → byte-stable | status 23 |

## Menu bar

| Item | Status | Notes | Ref |
| ---- | ------ | ----- | --- |
| `Results · Time · Plot · Help` items | 🔴 placeholder | the other 4 are still `ui.menu_button(m, |_| {})` — open but empty | — |
| `Control` menu (session-control verbs) | ✅ done | hosts the already-lowered transport / animate-stop / view-reset-fit `UiAction`s (`control_menu_items`), greyed when not attached; griz idiom of menus duplicating the toolbar/`Time` menu. No proto change, no new `UiAction` | status 23 |
| `Rendering` menu (wireframe/edge toggles) | ✅ done | real three-way `shaded / shaded+edges / wireframe` toggle → `UiAction::SetRenderMode`. The `LineList` edge pipeline carried an illegal non-zero depth bias that aborted startup on a real device — fixed (zero bias + `LessEqual`), now device-verified by `tests/vb004_edge_pipeline_validation.rs` | VB-003 / VB-004 / status 23 |
| `Picking` menu (enable client-side picking) | ✅ done | `enable picking` toggle → `UiAction::TogglePicking`; ray-cast vs. cached hull | status 23 |
| View / Preferences (theme, layout tweaks) | ✅ done | `Preferences` menu hosts the Tweaks set: `Theme` (dark/light → `UiAction::SetTheme`, applied via egui visuals in `build_shell_ui`) + `Left dock collapsed` (→ `UiAction::SetDockCollapsed`, L1 ↔ 28 px rail). Pure-client, no proto change | status 23 |

## Toolbar

| Item | Status | Notes | Ref |
| ---- | ------ | ----- | --- |
| Transport `⏮◀▶⏭`, stride, animate/pause/stop, view reset/fit | ✅ done | — | status 16 |
| Overlay chips (`title state legend axes bbox`), state counter | ✅ done | chips drive HUD; `state N / total` right-aligned | status 16 |

## Left dock

| Item | Status | Notes | Ref |
| ---- | ------ | ----- | --- |
| Runs/sessions section | ✅ done | one row + status dot | status 16 |
| Results → `derived` | ✅ done | **real DB-filtered catalog**: the maintainer-authorized `mili-rs` core derived-enumeration milestone landed (`../mili-py/m4.md` Decision 28 — oracle-gated `derived_variables_of_class`), so the server emits a `D\t<name>` section (union over classes, deduped) in the same `MVCAT1` blob / `CATALOG_TICKET` / Flight `DoGet` (no `.proto`/blob/ticket/RPC change); the client lists `ResultCatalog::derived` with a `derived · N` badge (selectable → same `Show` as `primal`). `None`/no-run falls back to the static `DERIVED_RESULTS` with the bare `derived` header + `Results · DERIVED_RESULTS.len()` badge, so the default composite gate is byte-stable (VB-001). The deferred `mili-py`/`milox` bridge follow-up is now closed — `milox.derived`'s duplicated enumeration replaced by thin pyo3 pass-throughs to this same parity-gated core (`../mili-py/m4.md` Decision 29) | phase-5-m3 Dec 47 / phase-5-m4 Dec 70 → 71 / m4 Dec 28 → 29 |
| Results → `primal` / `time-indep` | 🟡 partial | `primal` is now a **real catalog**: the server enumerates `Database::queriable_svars` into a self-describing blob fetched over the existing Flight `DoGet` by the conventional `CATALOG_TICKET` (no `.proto` change — `phase-5-m4.md` Decision 67); the client lists the names (selectable → same `Show` as the command line) with a `primal · N` badge. `time-indep` **stays** the honest labelled placeholder — a faithful TI-results enumeration is a substantive **re-port**, not the reshape `queriable_svars` was: `TI_PARAM` is a junk-drawer (also labels/materials/coords storage), mili-python exposes **no** TI-results oracle to gate parity, and a faithful filter needs the `mc_ti_get_metadata_from_name` TI-name grammar + a TI-type-aware `ParamTable` mili-rs lacks (`phase-5-m4.md` Decision 69; blocker: a `mili-rs` core TI-results accessor + `mili` oracle). The reserved `T` tag + `decode_catalog`'s unknown-tag tolerance keep it a zero-proto-change forward seam. `None`/no-run keeps the static `(catalog: M4+)` so the default composite gate is byte-stable | phase-5-m4 Dec 67 / 69 |
| Colormap (ramp picker + manual legend limits) | ✅ done | extra vs wireframe but functional | phase-5-m4 Dec 66 |
| Materials section | ✅ done | per-class row toggles visibility (● shown / ○ hidden, weak when off) → `UiAction::SetMaterialVisible` → frozen `Command::Material` | status 8 / 23 |
| Surfaces section | 🔴 placeholder | `"(surfaces: M4+)"` | — |
| Per-section row-count badges; Picking glyph row | 🟡 partial | all four wireframe sections (Runs/sessions, Results, Materials, Surfaces) carry a `· N` badge; the collapsed dock is the wireframe **R/M/S/P icon rail** (`dock_rail_glyphs`, `P` hint tracks live picking). The Results badge + `primal · N` are now real (Decision 67 catalog). Remaining: the Surfaces count is still a placeholder (no surface model yet) | status 23 |

## Viewport overlays

| Item | Status | Notes | Ref |
| ---- | ------ | ----- | --- |
| title / state / legend | ✅ done | data-driven | status 16 |
| axes gizmo | ✅ done | world X/Y/Z projected through the live camera basis (tracks orbit); static triad only on the headless/not-attached fallback | status 23 |
| bbox | ✅ done | real per-state world AABB projected through the live camera (12 edges, tracks orbit/pan/zoom + deform); placeholder inset only when no live camera | status 23 |
| Pick highlight glyph | ✅ done | ring+crosshair over the last ray-cast hit, projected through the live camera (tracks orbit/pan/zoom + deform); only when picking on + a cached hit + a live camera, so the headless/off default is byte-stable | status 23 |
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
| Scripting runner | 🟡 partial | enabled: editor + Run + streamed output pane + `venv:…·attach:…` line → `UiAction::RunScript`, app spawns a `pygriz` subprocess (PYTHONPATH-injected `griz.launch()`). Forward path: a `pip install`ed managed venv + `attach()`-into-*this*-GUI (the latter gated on Phase 5 M5 remote mode — the in-process client writes no session file) | client.md dec 3 / phase-6-m2 / status 18–20, 23 |
| Time-history plot | 🟡 partial | fed by `ResultState` min/max envelope, not the `Query` per-element series; server `Query` is a stub | phase-5-m3.5 Dec 50 |
| Whole-region hide (tweak) | 🟡 partial | collapses body only; 22 px strip always present | — |

## Status bar

| Item | Status | Notes | Ref |
| ---- | ------ | ----- | --- |
| attached / proto / pick / fps row | ✅ done | `proto v1` is now the **major** of the single-source `mili_viz_proto::v1::PROTOCOL_VERSION` (compile-time, not a `Hello` round-trip — Decision 68; byte-identical to the old literal so the VB-001 seam is unmoved); honest **local** peer count `(1 peer)` shown attached-state only (real `n peer(s)` fan-out is M6 — "Multi-client peer banner"); not-attached carries no peer cell (byte-stable). `pick:` is **live** (client-side ray-cast readout, off by default) | phase-5-m4 Dec 68 / status 23 |

## Tweaks / Preferences

| Item | Status | Notes | Ref |
| ---- | ------ | ----- | --- |
| Theme (dark/light), left-dock collapse, full bottom-tab hide, AI-panel position | ✅ done | `Preferences` menu surfaces **Theme** + **Left dock collapsed** (pure-client, `SetTheme`/`SetDockCollapsed`). Bottom-tab hide is already reachable via the tab strip's `▾ hide`; **AI-panel position** is M6 (panel is a placeholder). Cross-session persistence built: a `serde` `PersistedTweaks` (the 5 overlay chips + theme + dock-collapse — the wireframe-justified set) is loaded into `ShellState` at windowed startup from `$XDG_CONFIG_HOME`/`$HOME/.config/mili-viz/tweaks.json` and re-written when a persisted `UiAction` fires (`is_persisted_action`). No config ⇒ `PersistedTweaks::default` == default-shell snapshot, so the headless composite gate is disk-free + byte-stable (VB-001) | status 23 |

## Renderer / rendering modes

| Item | Status | Notes | Ref |
| ---- | ------ | ----- | --- |
| Filled `TriangleList` pass | ✅ done | `renderer.rs:149` | status 15 |
| Wireframe / element-edge / hidden-line mode | 🟡 partial | `LineList` edge pass via `Mesh::edge_indices`; `Renderer::set_mode` → `Shaded` (default, byte-stable) / `Edges` (hidden-line overlay) / `Wireframe` lands. Remaining gap: `Mesh::edge_indices` derives from the triangle list, so hex/quad/pyramid/wedge per-face triangulation diagonals leak into the wireframe (`bar71.pltA` reads as a triangulated soup). Fixed once `MVG3` element-edge buffer ships: prefer server-supplied `Mesh::element_edges`, fall back to the extractor (byte-stable for older servers) | VB-003 / VB-005 / status 23 / phase-4-m7 Dec 73 / phase-5-m7 Dec 82 |
| Translucent whole-mesh / X-ray (see internal element structure) | ⬜ missing | needs server-side interior triangles (`MVG3` interior flag) + client `Translucent`/`Xray` render modes; opt-in via the reserved `MaterialVisibility{ material: u32::MAX }` sentinel (no proto change) | phase-4-m7 Dec 74 / phase-5-m7 Dec 81 / 83 |
| `Rendering → Cut` (cut-plane operator) | ⬜ missing | `Cmd::Cutplane` typed-frozen since `phase-4-m1.md` Δ1; server arm has been a no-op stub at `crates/mili-viz-server/src/lib.rs:528`; closed-hull clip (kept-side ∪ cap), session-state, composes with `show`/state-step | phase-4-m8 Dec 75 / 76 / 77 / phase-5-m8 |
| `Rendering → Slice` (2-D cross-section operator) | ⬜ missing | griz/VisIt verb sister to cut; reuses M8 cap machinery, emits cap-only; additive `slice_only: bool` on `CutPlane` (the **second** post-M1 proto change after the catalog side-channel) | phase-4-m9 Dec 78 / 79 / 80 / phase-5-m9 |

## Cross-cutting gaps

| Item | Status | Notes | Ref |
| ---- | ------ | ----- | --- |
| File → Open / `rfd` picker | ⏸️ deferred | own milestone (maintainer decision); load via CLI `-i` / Layer-0 `load` only | status 21 |
| Picking (client-side from cached `GeometryRef`) | 🟡 partial | ray-cast vs. cached hull → live status-bar readout (node/tri/scalar) **+ viewport highlight glyph** (ring+crosshair over the cached hit, projected through the live camera so it tracks orbit/pan/zoom + deform; off-by-default/no-camera → no glyph, byte-stable). Remaining: the frozen proto has no label catalog, so still no `class N` mapping (design-first, deferred) | status 23 |
| Agent / multi-client session states | ⬜ missing | thinking/running/interrupted/peer | M6 |
| Remote mode (gRPC+Flight wired to `connect`/`attach`) | ⏸️ deferred | server transport done (status 11); client wiring is Phase 5 M5 | status 22 |

---

## MVP cut (no AI, no remote)

The maintainer-scoped MVP excludes the AI panel (M6) and remote mode
(Phase 5 M5). The parity gaps that remain in-scope for MVP, roughly by
leverage:

1. **Menu bar** — 🟡 `Rendering` (VB-003), `Picking`, and `Control`
   (session-control verbs, reusing the already-lowered
   transport/animate/view `UiAction`s) are now wired; still to do:
   the View/Preferences host.
2. ✅ **Wireframe / element-edge render mode** (VB-003) + its
   `Rendering` toggle — done.
3. ✅ **Materials enable/disable affordance** (server side already
   done, status 8 — GUI only) — done.
4. 🟡 **Picking** + live status-bar readout — ray-cast, readout
   **and the viewport highlight glyph** landed; only the
   label-catalog `class N` mapping remains (needs a non-frozen-proto
   catalog path — design-first, deferred).
5. ✅ **Real bbox overlay + camera-tracking axes gizmo** — done.
6. **File → Open** (lift the deferral if MVP needs it).
7. ✅ **L3 focus mode + theme/tweaks surface** — the `Preferences`
   menu + Theme + Left-dock-collapse, the R/M/S/P icon rail, full L3
   focus mode (`Ctrl+\` → dock rail + AI/tabs hidden), and
   cross-session tweak persistence (the `serde` `PersistedTweaks`
   config, replacing the `app.rs` hook) all landed.
8. 🟡 **Primal / time-indep result catalog** — `primal` landed: the
   maintainer-approved Flight catalog side-channel (`phase-5-m4.md`
   Decision 67, no `.proto` change) enumerates `queriable_svars` into
   a real, selectable left-dock list. `time-indep` remains a labelled
   placeholder (mili-rs has no TI accessor — follow-up).
9. 🟡 **Wire the scripting tab** — done as a `launch()`-based
   `pygriz` subprocess runner (enabled editor + Run + streamed
   output + venv/attach indicator). Remaining: a `pip install`ed
   managed venv and `attach()`-into-*this*-GUI (the latter needs
   Phase 5 M5 remote mode; the in-process client has no session
   file).

## Update protocol

When a row lands: flip its **Status**, tighten the **Notes**, and
update the cross-linked tracker (`status.md` item / `bug-tracker.md`
`VB-NNN` / the phase doc Decision) in the same change so the trackers
do not drift. Add new wireframe deltas as new rows under the right
region.
