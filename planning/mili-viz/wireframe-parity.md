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

Refreshed 2026-05-24 against the same files. Phase 5 M5 (remote mode),
M6 (agent integration polish), M7 (render modes consuming `MVG3`),
M8 (cut-plane gizmo), and M9 (slice gizmo) have **all** landed since
the derive date — most "⬜ missing" / "🔴 placeholder" rows below for
those features have been flipped. The genuinely-still-stub rows
(empty `Results`/`Time`/`Plot`/`Help` menus, Surfaces, time-indep
results catalog, `Query`-fed time-history, scripting `venv:`/`attach:`
indicators, File→Open, picking class-N label) are unchanged.

---

## Window shape & layout

| Item | Status | Notes | Ref |
| ---- | ------ | ----- | --- |
| L1 default layout (6 regions, egui panel mapping, default sizes) | ✅ done | `build_shell_ui` | status 16 |
| L2 — AI panel expanded (28 px rail → 340 px dock) | ✅ done | `ai_dock` paints the 340 px expanded panel (header + transcript + composer + Send/Stop + revert) when `state.ai.expanded`; collapsed 28 px rail with expand caret is the default. Capability-gated on `state.ai.cap_agent` so the no-backend composite gate stays byte-stable (VB-001) | status 25 / phase-5-m6 |
| L3 — focus mode (dock→icon rail, AI/tabs hidden, `Ctrl+\`) | ✅ done | `Ctrl+\` toggles `focus_mode` (`set_focus_mode` also collapses the dock to the R/M/S/P rail); the AI rail + bottom tabs are hidden; a rail glyph or a second `Ctrl+\` restores full L1. Default off → byte-stable | status 23 |

## Menu bar

| Item | Status | Notes | Ref |
| ---- | ------ | ----- | --- |
| `Results · Time · Plot · Help` items | ✅ done | populated from `reference/griz/Src/gui.c::create_menu_bar` (the wireframe README defers menu contents to "the legacy griz Motif menus"). **Results** mirrors the left-dock catalog (`derived`/`primal`/`time-indep` submenus → same `Show` action the dock click does). **Time** runs `time_menu_items()` — Next/Prev/First/Last + Animate/Stop Animate, the legacy `Time` transport verbs (reuses `UiAction`s the toolbar / `Control` menu already lower; griz idiom of menus duplicating the toolbar). **Plot** is the legacy `Time Hist Plot` (opens the `TimeHistory` bottom tab via `SelectBottomTab`). **Help** is the honest port of `Display Griz Manual` — an `About mili-viz` submenu listing the crate version, the frozen-proto major (`mili_viz_proto::v1::PROTOCOL_VERSION`), and the `Ctrl+\` shortcut; no Rust-port manual yet | — |
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
| Multi-client peer banner | 🟡 partial | status-bar peer count is **live** off `AgentStatus.detail = "peers=N"` when `CAP_AGENT` advertised (`shell.rs:1819-1837`); honest `(1 peer)` default when not. A dedicated viewport peer **banner** with peer-name list is still missing — the count rides the status bar | status 25 / phase-5-m6 Dec 99 |
| Not-attached card | ✅ done | — | status 16 |

## AI Assistant panel

| Item | Status | Notes | Ref |
| ---- | ------ | ----- | --- |
| Collapsed rail / expand / transcript / composer / status pill / provenance | ✅ done | full chrome lives in `ai_panel.rs` + `shell.rs::ai_dock`: 28 px collapsed rail with vertical `AI ASSISTANT` label + status word + expand caret; 340 px expanded panel with header, scroll transcript (`User`/`Assistant`/`Tool`/`TurnBoundary`/`Interrupted` rows), composer with `📷 attach frame` toggle, `Send` / `⏹ Stop` swap on in-flight, `↶ revert to here` lowers to typed `SetState`/`Show`/`View(SetCamera)` (never `raw`). Capability-gated; default `MockAgent` lights it up on a vanilla `cargo run` — real LLM backend is a separate Cargo-feature follow-up | status 25 / phase-5-m6 Dec 94–99 |

## Bottom tabs

| Item | Status | Notes | Ref |
| ---- | ------ | ----- | --- |
| Command line (Layer-0 verbatim, transcript) | ✅ done | — | status 17 |
| Scripting runner | 🟡 partial | enabled: editor + Run + streamed output pane + `venv:…·attach:…` line → `UiAction::RunScript`, app spawns a `pygriz` subprocess (PYTHONPATH-injected `griz.launch()`). Forward path: a `pip install`ed managed venv + `attach()`-into-*this*-GUI (the latter gated on Phase 5 M5 remote mode — the in-process client writes no session file) | client.md dec 3 / phase-6-m2 / status 18–20, 23 |
| Time-history plot | ✅ done (text-input variant) | fed by `ResultState` min/max envelope **plus** per-element `Query`-fed lines. Server `Query` RPC dispatches `Database::query_full` for primal svars (`InlineTable` carrier with `[states × labels × components]` row-major shape; out-of-range / no-run-loaded surface as typed `ok=false` errors; derived results route to "not yet supported" until the geometry-path derived dispatch is replicated). Plot tab body renders each series as its own line (round-robin palette, distinct from the envelope colours); input row carries `class · id · svar · comp` fields + `+series` button → `UiAction::QueryElementSeries`. `app.rs` lowering issues the request over all states the run advertises, parses the inline reply, and pushes samples back via `ShellState::push_element_series`; failures drop the placeholder so the legend never accumulates empty rows. The picking-driven variant (click an element → plot its series) still needs the picking-class-N label catalog (punch-list item below) | phase-5-m3.5 Dec 50 |
| Whole-region hide (tweak) | ✅ done | `Preferences → Show bottom tabs` checkbox suppresses the whole `tabs` panel (strip + body) via `ShellState::show_bottom_tabs` (default `true` → L1 byte-stable) → `UiAction::SetShowBottomTabs` → persisted in `tweaks.json`. The per-tab `▾ hide` still collapses the body only (its own runtime mode). Regression-tested by `m3_5_bottom_tabs::show_bottom_tabs_false_suppresses_the_panel` | — |

## Status bar

| Item | Status | Notes | Ref |
| ---- | ------ | ----- | --- |
| attached / proto / pick / fps row | ✅ done | `proto v1` is now the **major** of the single-source `mili_viz_proto::v1::PROTOCOL_VERSION` (compile-time, not a `Hello` round-trip — Decision 68; byte-identical to the old literal so the VB-001 seam is unmoved); honest **local** peer count `(1 peer)` shown attached-state only (real `n peer(s)` fan-out is M6 — "Multi-client peer banner"); not-attached carries no peer cell (byte-stable). `pick:` is **live** (client-side ray-cast readout, off by default) | phase-5-m4 Dec 68 / status 23 |

## Tweaks / Preferences

| Item | Status | Notes | Ref |
| ---- | ------ | ----- | --- |
| Theme (dark/light), left-dock collapse, full bottom-tab hide, AI-panel position | ✅ done | `Preferences` menu surfaces **Theme** + **Left dock collapsed** + **Show bottom tabs** (pure-client, `SetTheme`/`SetDockCollapsed`/`SetShowBottomTabs`). The wireframe-named whole-region bottom-tabs hide now exists alongside the runtime `▾ hide` body collapse; **AI-panel position** is M6 (panel is a placeholder). Cross-session persistence built: a `serde` `PersistedTweaks` (the 5 overlay chips + theme + dock-collapse + show-bottom-tabs + interactive-clip — the wireframe-justified set) is loaded into `ShellState` at windowed startup from `$XDG_CONFIG_HOME`/`$HOME/.config/mili-viz/tweaks.json` and re-written when a persisted `UiAction` fires (`is_persisted_action`). No config ⇒ `PersistedTweaks::default` == default-shell snapshot, so the headless composite gate is disk-free + byte-stable (VB-001) | status 23 |

## Renderer / rendering modes

| Item | Status | Notes | Ref |
| ---- | ------ | ----- | --- |
| Filled `TriangleList` pass | ✅ done | `renderer.rs:149` | status 15 |
| Wireframe / element-edge / hidden-line mode | ✅ done | `Renderer::set_mode` covers `Shaded` (default, byte-stable) / `Edges` (hidden-line overlay) / `Wireframe` / `FeatureEdges` plus the M7 `Translucent`/`Xray` arms. VB-005 face-diagonal regression is closed: the server promoted `MVG3` to the only production layout so the per-element edge buffer ships on every blob; the client prefers `Mesh::element_edges` and only falls back to the triangle-extractor for un-upgraded servers | VB-003 / VB-005 / status 23 / phase-4-m7 Dec 73 / phase-5-m7 Dec 82 |
| Translucent whole-mesh / X-ray (see internal element structure) | ✅ done | `RenderMode::{Translucent, Xray}` arms shipped: alpha-blended fill pipeline with depth-test on / depth-write off; `Xray` additionally overlays element-edges. `Interior` is a separate `ShellState::interior_on` toggle lowered to `Cmd::Material` with the reserved `material: Some(u32::MAX)` sentinel so the server re-emits an `MVG3` blob carrying interior triangles. All compose with cut/slice | status 25 / phase-4-m7 Dec 74 / phase-5-m7 Dec 81 / 83 |
| `Rendering → Cut` (cut-plane operator) | ✅ done | `egui`-overlay gizmo (origin disc + normal arrow as additional egui shapes, no new `wgpu` pipeline — M3 additive seam untouched per VB-001), 30 Hz wall-clock-throttled preview + un-throttled drag-end commit, Rendering → Cut menu (show-gizmo toggle, clear-cut emits zero-normal `Cmd::Cutplane`), `Preferences → Interactive clip` low-bandwidth suppress (persisted via `tweaks.json`), status-bar `cut: o=(...) n=(...)` readout. Server arm at `crates/mili-viz-server/src/lib.rs:528` is now real (rayon per-element Sutherland–Hodgman clip; cap triangles tagged `tri_material == u32::MAX - 1`) | status 24 / phase-4-m8 / phase-5-m8 |
| `Rendering → Slice` (2-D cross-section operator) | ✅ done | thin sibling of Cut: shared gizmo machinery with distinct cyan colour, shared `CutThrottle` (one drag at a time), independent `ShellState::slice_plane` composing with `cut_plane`, Rendering → Slice menu (show-gizmo / clear-slice), distinct `slice: o=(...) n=(...)` status-bar readout. `slice_cmd` lowers to `Cmd::Cutplane { slice_only: Some(true) }` (the one additive post-M1 proto field). Slice always opaque by default — translucency is the `RenderMode` lever | status 24 / phase-4-m9 / phase-5-m9 |

## Cross-cutting gaps

| Item | Status | Notes | Ref |
| ---- | ------ | ----- | --- |
| File → Open / `rfd` picker | ⏸️ deferred | own milestone (maintainer decision); load via CLI `-i` / Layer-0 `load` only | status 21 |
| Picking (client-side from cached `GeometryRef`) | 🟡 partial | ray-cast vs. cached hull → live status-bar readout (node/tri/scalar) **+ viewport highlight glyph** (ring+crosshair over the cached hit, projected through the live camera so it tracks orbit/pan/zoom + deform; off-by-default/no-camera → no glyph, byte-stable). Remaining: the frozen proto has no label catalog, so still no `class N` mapping (design-first, deferred) | status 23 |
| Agent / multi-client session states | ✅ done | `AgentStatus::{Thinking, Idle, Interrupted, Error}` ingested by `AiPanelState` → drives the panel status pill + ⏹ Stop swap; `Interrupted` rows are typed transcript entries; peer count rides `Status.detail = "peers=N"` (see "Multi-client peer banner") | status 25 / phase-5-m6 |
| Remote mode (gRPC+Flight wired to `connect`/`attach`) | ✅ done | `Session::connect_tcp` + `Session::attach` over the Phase 4 M6 wire — one tuned `tonic::Channel` cloned for `MiliVizClient` + `FlightServiceClient`, Flight `DoGet` streams the byte-identical M2/M3 blob; CLI `-r`/`--remote <host:port>` + `--attach [<id>]` (mutually exclusive); `--attach` reuses the same `~/.griz/sessions/<id>.json` resolver pygriz uses (newest-live pid liveness via `kill(pid, 0)`). HPC-latency tuning: `tcp_nodelay`, TCP + HTTP/2 keep-alives, 10 s `connect_timeout` | status 22 / phase-5-m5 |

---

## What's still left (post Phase 5 M5/M6/M7/M8/M9)

With every Phase 5 milestone landed, the remaining stub/placeholder
inventory is **substantially smaller** than the May-18 MVP cut. By
leverage, ordered "ship-blocking first":

1. **`Results` / `Time` / `Plot` / `Help` empty menus** — ✅ done.
   Populated from the legacy griz Motif menus
   (`reference/griz/Src/gui.c::create_menu_bar` — what the wireframe
   README defers to): Results mirrors the left-dock catalog, Time is
   the transport-verb pulldown that re-uses already-lowered
   `UiAction`s (`time_menu_items()`), Plot opens the `TimeHistory`
   bottom tab, Help carries an `About mili-viz` submenu with the
   crate version + frozen-proto major. Regression-covered by
   `tests/menu_bar.rs`.
2. **Surfaces section** (`shell.rs:1588-1592`) — literal
   `(surfaces: M4+)` string. No surfaces data model yet; needs a
   server-side surface catalog (sibling to the M4 primal/derived
   catalog side-channel) **and** a UI affordance to toggle / hide.
   Substantial work — likely its own milestone.
3. **Time-indep results catalog** (`shell.rs:1499-1507`) — honest
   `(time-indep: no catalog path yet)` label. Blocked on a `mili-rs`
   core TI-results accessor + a `mili` Python oracle to gate parity
   (`TI_PARAM` is a junk-drawer, needs a TI-name grammar +
   TI-type-aware `ParamTable`). The reserved `T` tag in the catalog
   blob is the zero-proto-change forward seam. Substantial — `mili-rs`
   core first.
4. **Time-history plot fed by `Query` per-element series** — ✅
   done (text-input variant). Server `Query` RPC dispatches
   `Database::query_full` for primal svars (`InlineTable` carrier;
   typed `ok=false` errors; derived results route to "not yet
   supported" until the geometry-path derived dispatch is
   replicated). Client wraps the call in `Session::query`, lowers
   `UiAction::QueryElementSeries` over every state the run
   advertises, parses the inline reply, and pushes per-element
   samples back via `ShellState::push_element_series`. Plot tab
   body renders each series alongside the existing min/max
   envelope (distinct round-robin palette); input row hosts
   `class · id · svar · comp` fields + `+series` button.
   Regression-covered by `tests/query_rpc.rs` (5 server cases over
   `serial/basic1`, skip-on-absent) and
   `tests/plot_element_series.rs` (7 pure-client cases: input
   submit/clear, idempotent re-submit, component differentiates
   identical-otherwise series, push/drop round-trip, painted shell
   stays input-free). The **picking-driven** variant (click an
   element on the hull → plot its series) is still open — blocked
   on the picking-class-N label catalog (item #6).
5. **Scripting tab — managed venv + attach-into-*this*-GUI**
   (`shell.rs:1012`, the `venv: starting · attach: launch` line).
   Runner works; the `pip install`ed managed venv and
   `attach()`-into-this-GUI (the latter requires the in-process
   client to write a session file — a small but design-first
   change) remain. Self-contained UX improvement.
6. **Picking class-N label** — picking ray-cast, status-bar readout,
   and viewport highlight glyph all landed; only the `class N`
   mapping is missing (frozen proto has no label catalog, so this
   needs a new catalog side-channel tag or a `Query` round-trip).
   Small UX gain; design-first.
7. **Bottom-tabs whole-region hide** — ✅ done. `Preferences → Show
   bottom tabs` checkbox suppresses the whole `tabs` panel (strip +
   body); persisted via `tweaks.json`. The per-tab `▾ hide` retains
   its runtime body-only collapse.
8. **File → Open / `rfd` picker** — intentionally deferred
   (maintainer decision, own milestone). Lift if needed.
9. **VB-006** — ✅ fixed. `EguiPaint::set_visuals` pre-applies the
   theme's visuals on the paint context before `run_ui`, and
   `render_shell_to_image` calls it from `state.theme.visuals()`
   ahead of `egui.paint`. The menu-chrome relight assertion in
   both `preferences_tweaks::composite_render` and
   `tweaks_persistence::composite_render` is re-enabled, sampling
   mean grey-chrome luminance in the top 26 px menu-bar band.
10. **Phase 6 `pygriz` M4 / M6** — out of the client crate proper,
    but the scripting tab's "attach into this GUI" and the AI
    panel's future event-driven analysis depend on Phase 6 M4 (live
    subscribe → `@s.on(...)` callbacks). Independent track. **M5
    (`query`/`to_dataframe`) ✅ landed** — `Session.query` /
    `Database.query` build the typed `QueryRequest` directly,
    `QueryResult.to_dataframe()` returns pandas in the
    `mili.utils.query_data_to_dataframe` shape (index=states,
    columns=labels); the `flight_ticket` arm of the proto's `oneof`
    raises a clear `QueryError` and lands jointly with M6's
    `render`/`snapshot` (same Arrow-Flight plumbing). See
    [`phase-6-m5.md`](phase-6-m5.md) — closes #4 above on the wire
    end-to-end (server arm + Rust client arm + pygriz arm all green
    against the same `Query` RPC).

## MVP cut (historical — superseded by the section above)

The maintainer-scoped MVP excluded the AI panel (M6) and remote mode
(Phase 5 M5). All MVP rows are now ✅ done; this section is preserved
for the audit trail.

1. ✅ **Menu bar** — `Control` / `Rendering` / `Picking` /
   `Preferences` wired; `Results` / `Time` / `Plot` / `Help`
   intentional empty stubs.
2. ✅ **Wireframe / element-edge render mode** (VB-003) — done; VB-005
   diagonals closed by always-on `MVG3`.
3. ✅ **Materials enable/disable affordance** — done.
4. 🟡 **Picking** — ray-cast, status-bar readout, and viewport
   highlight glyph landed; only the `class N` label mapping remains.
5. ✅ **Real bbox overlay + camera-tracking axes gizmo** — done.
6. ⏸️ **File → Open** — deferred.
7. ✅ **L3 focus mode + theme/tweaks surface** — done.
8. 🟡 **Primal / time-indep result catalog** — primal/derived done;
   time-indep remains a labelled placeholder (mili-rs TI accessor +
   oracle blocker).
9. 🟡 **Scripting tab** — runner enabled; managed venv + attach-into-
   this-GUI remain.

## Update protocol

When a row lands: flip its **Status**, tighten the **Notes**, and
update the cross-linked tracker (`status.md` item / `bug-tracker.md`
`VB-NNN` / the phase doc Decision) in the same change so the trackers
do not drift. Add new wireframe deltas as new rows under the right
region.
