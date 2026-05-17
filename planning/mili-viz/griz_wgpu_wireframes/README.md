# Handoff: mili-viz client wireframes

## Overview

Mid-fidelity wireframes for the `mili-viz` client — the `wgpu` + `egui`
viewer specified in `planning/mili-viz/client.md`. This package pins the
window shape, panel composition, viewport overlay set, and AI-assistant
panel behaviour as agreed during design review on 2026-05-17.

The goal of these wireframes is to lock the **layout, the states the UI
must handle, and the chosen patterns for two ambiguous areas** (inline
tool-call rendering, provenance UX) — *not* to dictate pixel-perfect
visuals. Final styling lives in `egui`'s visuals system, not in CSS.

## About the design files

The files in this bundle are **design references created in HTML/CSS/JSX**.
They are prototypes showing intended look and behaviour — they are *not*
production code to port. The implementation target is a native
Rust application using `wgpu` (renderer) and `egui` (immediate-mode UI).

When implementing, **recreate these layouts using `egui`'s native widgets**
(`SidePanel`, `TopBottomPanel`, `CentralPanel`, `CollapsingHeader`,
`ScrollArea`, `egui_plot`, etc.). Treat the HTML as a structural and
behavioural reference; let `egui`'s default visuals carry the look,
adjusted only where this doc calls out a specific choice.

## Fidelity

**Mid-fidelity.** Layout, panel composition, overlay set, agent-state
affordances, and copy are deliberate and should be implemented as
specified. Exact colours, font sizes, and spacing in the HTML are
*illustrative of an `egui`-flavoured look* — final values should come
from the `egui` theme. Where a value is load-bearing for legibility (HUD
overlay sizes, hit-target minimums) it is called out below.

---

## Window shape

A single top-level window. Six regions, top to bottom / left to right:

```
┌─ menu bar  ──────────────────────────────────────────────────────────┐
├─ toolbar   ──────────────────────────────────────────────────────────┤
│            │                                          │              │
│ left dock  │       3D render viewport (wgpu)          │  AI rail /   │
│            │                                          │  AI panel    │
│            │                                          │              │
├──────────────────────────────────────────────────────────────────────┤
│ bottom tabs (command line / scripting / time-history)                 │
├──────────────────────────────────────────────────────────────────────┤
│ status bar                                                            │
└──────────────────────────────────────────────────────────────────────┘
```

| Region        | egui mapping                                      |
| ------------- | ------------------------------------------------- |
| Menu bar      | `egui::menu::bar` in a `TopBottomPanel::top`      |
| Toolbar       | second `TopBottomPanel::top`                      |
| Left dock     | `SidePanel::left("dock").resizable(true)`         |
| Viewport      | `CentralPanel` containing a `wgpu` texture        |
| AI rail/panel | `SidePanel::right("ai").resizable(true)`          |
| Bottom tabs   | `TopBottomPanel::bottom("tabs").resizable(true)`  |
| Status bar    | `TopBottomPanel::bottom("status")`                |

Default widths/heights (initial sizes — user-resizable):
- Left dock: **230 px**
- AI rail (collapsed): **28 px**
- AI panel (expanded): **340 px**
- Bottom tabs: **200 px**
- Toolbar / status bar: **30 px / 20 px** (compact)

---

## Layouts (three configurations)

### L1 — Default (AI rail collapsed)

The canonical, minimal first-run layout. Left dock open, viewport
centre, AI as a thin 28 px rail on the right with a vertical "AI
ASSISTANT" label that expands the panel on click. Bottom tabs visible.

**The AI panel is collapsed by default and not surfaced unless the user
opens it.** This was an explicit design decision.

### L2 — AI panel expanded

Same as L1 but the AI rail has expanded into a 340 px right dock.
Triggered by clicking the rail label or invoking the assistant via menu.

### L3 — Focus mode

Stripped to the viewport. Left dock collapses to a 28 px icon rail
(R/M/S/P glyphs for Results/Materials/Surfaces/Picking). AI hidden.
Bottom tabs hidden. Useful for screenshot capture, presentation, and
"just looking at the data" sessions.

Suggested keyboard toggle: `Ctrl+\`.

---

## Session states (must be handled)

All six render in the L1 (or L2 where the AI is involved) layout. The
implementation must visibly reflect each state — no silent failures.

| State        | Visual signal                                                     |
| ------------ | ----------------------------------------------------------------- |
| Not attached | Viewport shows an "attach to session" card; status bar reads `— not attached —`; pick reads `—` |
| Attached idle| Default L1 — all overlays drawn, status bar dot green             |
| Animating    | Toolbar `▶ animate` becomes `⏸ pause` (active style); state counter increments; bottom tab can switch to time-history plot |
| Agent thinking | AI panel header status pill is `thinking` with pulsing accent dot; transcript shows the user message + a "reading subscription stream…" placeholder; Send button replaced by **⏹ Stop** |
| Agent running tool | Status pill `running` (pulsing warn dot); transcript shows tool-call lines streaming in; an active tool line shows progress (`38 / 96`); **⏹ Stop** persistent |
| User interrupted | Transcript ends with a `✕ interrupted by user — turn cancelled` line in danger colour; composer placeholder reads "follow up… (turn was interrupted)"; status pill returns to `idle` |
| Multi-client (peer attached) | Banner above viewport: `● peer-name · viewing`; status bar shows `n peer(s)` with a warn dot |

The agent status pill colours:
- idle → `--ok` green
- thinking → `--accent` blue (pulse)
- running → `--warn` amber (pulse)
- interrupted → `--danger` red

---

## Menu bar

Items, left to right: `Control · Rendering · Picking · Results · Time ·
Plot · Help`. Mirrors the legacy griz Motif menus (see planning doc).
Long-tail commands not exposed in the left dock or toolbar should live
in these menus or be accessible via the Command line tab.

## Toolbar

Groups, left to right, separated by 1 px vertical dividers:

1. **Transport** — `⏮ ◀ ▶ ⏭` (first, prev, next, last state)
2. **Stride** — `stride [ 1 ]` (numeric input, 1-N states per step)
3. **Animate** — `▶ animate` (toggles to `⏸ pause`, active style when running); `⏹` stop
4. **View** — `⟲ view reset`, `⊞ fit`
5. **Overlays** — `overlays  title  state  legend  axes  bbox` — five toggle chips driving viewport HUD visibility. **All on by default**, user-toggleable. (Moved here from a floating bar over the viewport.)
6. *(spacer)*
7. **State counter** — `state 47 / 96` (right-aligned, monospace)

Hit target: each toolbar button is 22 px tall × ≥22 px wide. Stride input
is 28 px wide.

---

## Left dock — stacked collapsible sections

In order: **Runs/sessions**, **Results**, **Materials**, **Surfaces**.
Each section is a `CollapsingHeader` with a row count badge on the right
(e.g. `Results · 142`).

**Tree rows** are 20 px tall, with a single-character glyph column, a
name column, and an optional status dot (green = active, amber = warn,
grey = dim/disabled). Selected row gets an accent-tinted background.

Results sub-structure: `derived ▾`, `primal ▸`, `time-indep ▸`. Selecting
a result item is the same `Command` the command-line `show <result>`
emits.

---

## Viewport

A full-bleed `wgpu` render target rendered into the `CentralPanel`.
Overlays drawn on top in `egui`, gated by the toolbar Overlays toggles.

**Overlay layer (5 elements):**

| Overlay | Position           | Content                                                |
| ------- | ------------------ | ------------------------------------------------------ |
| title   | top-left           | `<run> · <result>` and `elements: N · nodes: N`        |
| state   | top-right          | `state N / total` and `t = 4.7000e-03 s`              |
| legend  | bottom-left        | Vertical colour bar + 5 numeric ticks + units label    |
| axes    | bottom-right       | Tri-axis gizmo (X red, Y green, Z blue)                |
| bbox    | inset on render    | Dashed rectangle showing model bounding box            |

Overlay typography: monospace, ~10.5 px, low-contrast white (≈ 85 %
alpha on the dark render). All five overlays are **on by default**.

**Multi-client peer banner** sits at top-centre when one or more peers
are attached: small pill with each peer's coloured dot + name + "viewing".

---

## AI Assistant panel

The headline feature. Thin subscriber to a server-hosted agent (see
`planning/mili-viz/client.md` decisions 1 & 2). The panel is collapsed
by default.

### Collapsed state (AI rail, 28 px)

- Vertical "AI ASSISTANT" label, click to expand.
- Tiny status word at bottom (`idle` / `thinking` / `running`) so the
  user can see activity without expanding.

### Expanded state (AI panel, 340 px)

Header → transcript → composer.

**Header** (26 px, uppercase 11 px label):
- Left: `AI ASSISTANT`
- Right: status pill (see Session states table above) + `›` collapse glyph

**Transcript** (scroll area, 8 px / 10 px padding):
- Messages alternate `you` / `claude` with a small timestamp role label.
- Body text is sans-serif 12.5 px.
- **Tool calls render as dense one-liners** (chosen during review):
  ```
  ▸ ran      state 47; show sx
  ▸ queried  sx range over states 40..60      → [0, 5.2e+04]
  ▸ queried  elements where sx > 4e+04 ...    → 12 elements
  ▸ captured frame (state 47, view: front)    → 812 KB png
  ```
  Monospace, 11 px, dim grey text with the arrow + value in accent/text
  colour. Expanding a line (future) reveals the exact `Command`/`Query`
  payload and reply.

  *Decision:* chip and card densities were considered and rejected in
  favour of the one-liner — it scales best in a narrow panel and reads
  like a familiar dev log.

**Composer** (top border, 8 px padding):
- Attachment row (only when frames are pinned): `📷 frame · state 47 ×`
- Text input area (min 48 px), placeholder `ask…`
- Bottom row: left = `📷` attach-frame button, `⌨` run-code button; right = **Send ↵** primary button OR **⏹ Stop** danger button when a turn is in flight.

### Provenance / revert UX

Two surfaces, both showing **which `StateDelta`s came from the agent**
and offering one-click revert. (A dedicated "Journal" tab was
considered and removed — not needed for v1.)

- **P1 — inline turn boundary marker** (primary): a small row in the
  transcript between the user message and the assistant response,
  showing the captured pre-turn snapshot (`state=47, result=sx,
  view=front`) and an accent-coloured `↶ revert to here` link. This is
  the default surface — minimal, in-context, and the user already has
  their eye on the transcript.

- **P3 — timeline strip above the viewport** (secondary, opt-in): a
  one-line horizontal strip of recent events (`load run · rotate · view
  front · ask · state 47 · show sx · snapshot`) with agent vs. user
  events colour-coded, plus a `↶ revert turn` button on the right. Shown
  only when the user opens it (e.g. via Control menu); off by default
  to keep the chrome clean.

Granularity is **per-turn**, not per-command — snapshot session state
at each user-turn boundary, revert restores that snapshot.

---

## Bottom tabs

Three peer tabs in a `TopBottomPanel::bottom`:

1. **Command line** — Layer-0 raw griz / `grizinit` stream. Monospace,
   green prompt (`griz>`), echoed commands, dim response lines.
2. **Scripting** — Python editor + Run + output. Runs via the
   subprocess+`attach()` runner described in the planning doc; venv
   indicator at the bottom (`venv: griz-0.4.2 · attach: session-9f3a`).
3. **Time-history plot** — `egui_plot` host for time-history series.

Tab strip is 22 px tall; active tab gets a lighter panel background and
soft border, inactive tabs are flat text.

The whole bottom-tabs region is **hideable via tweak** (planning doc
calls this out — quick toggle for clean screenshots).

---

## Status bar

Single row, monospace, 10.5 px, dim grey:
```
● attached <session-id>@<host>    proto v1    pick: <class N>          (n peer(s))   fps 58
```
Spacer pushes peer count and fps to the right.

---

## Tweaks (settings)

In the wireframe these are exposed via an in-design Tweaks panel.
In the real app these correspond to **View / Preferences** settings:

| Tweak                  | Effect                                              |
| ---------------------- | --------------------------------------------------- |
| Theme                  | Dark / Light egui visuals (light = white window bg) |
| Left dock collapsed    | L1 ↔ left-rail-only (L3-style left side)            |
| Show bottom tabs       | Show / hide bottom panel                            |
| AI panel position      | Right dock (default) vs. floating window            |

Each toolbar overlay chip (`title · state · legend · axes · bbox`) is a
runtime toggle, not a preference — though their on/off state should
persist between sessions.

---

## Design tokens (illustrative)

Final values come from the `egui` theme; these are the wireframe values
that produced the agreed look. Use them as a starting point.

**Dark theme:**
- Window bg `#1c1d20`, panel `#25272b`, panel-2 `#2d3035`, hover `#353a40`
- Border `#15161a`, border-soft `#34373c`
- Text `#d5d7db`, dim `#9aa0a8`, faint `#6b7079`
- Accent `oklch(0.68 0.10 250)` (muted blue) + 22 % alpha soft variant
- Warn `oklch(0.78 0.13 75)`, danger `oklch(0.66 0.18 25)`, ok `oklch(0.72 0.11 145)`

**Light theme:**
- Window bg `#ffffff`, panel `#f5f6f8`, panel-2 `#ffffff`, hover `#e6e9ee`
- Border `#c4c9d0`, border-soft `#dfe2e7`
- Text `#1f2227`, dim `#555a63`, faint `#808791`
- Accent `oklch(0.55 0.12 250)` + 18 % alpha soft variant

**Scale:**
- Font size: 12.5 px (sans, base), 11 px (small labels), 10.5 px (overlay/mono)
- Row height: 22 px (tree row 20 px); header height 26 px
- Corner radius: 3 px
- Hit target minimum: 22 px

**Fonts (wireframe substitutes — real app uses egui's defaults):**
- Sans: Source Sans 3
- Mono: JetBrains Mono

---

## Implementation order (suggested, refines planning doc Phase 5)

This is the order the wireframes prefer; not normative.

1. M3 — toolbar + left dock + viewport overlay set (`title/state/legend/axes/bbox`)
2. M3.5 — bottom tabs (command line first, scripting runner, then time-history)
3. M6a — AI panel chrome (collapsed rail + expanded panel + transcript renderer + composer); status pill driven by `DELTA_AGENT`
4. M6b — agent loop wired to `AgentChat` + tool-call streaming → one-liners
5. M6c — `Snapshot` capability + `📷 attach frame` flow
6. M6d — provenance: per-turn snapshot, inline turn boundary marker, `↶ revert to here`
7. M6e — `Interrupt` proto + `⏹ Stop` button
8. M6f — multi-client peer banner + status bar peer count

---

## Out of scope (deliberately not designed)

- Journal tab in the AI panel — explicitly removed during review.
- Per-command undo — only per-turn revert.
- Explicit "agent has the floor" lock — last-writer-wins per planning doc.
- Long-tail command GUI affordances (`outvec`, `refave`, traction viz) — live in menus or command-line for now.
- Detailed scripting editor surface (autocomplete, debugger) — out of v1.

---

## Files in this bundle

| File                  | What it is                                                  |
| --------------------- | ----------------------------------------------------------- |
| `Wireframes.html`     | The design canvas. Open in a browser to explore artboards.  |
| `egui-style.css`      | egui-flavoured CSS tokens & component styles.               |
| `chrome.jsx`          | Reusable chrome components (menu, toolbar, dock, viewport, AI rail, status bar, bottom tabs). |
| `artboards.jsx`       | Full `App` composition + AI-panel-only study components.    |
| `design-canvas.jsx`   | Canvas host (pan/zoom, sections, artboards).                |
| `tweaks-panel.jsx`    | In-design tweaks panel — runtime toggles for the wireframe. |

The bundled HTML is a **design reference**, not code to compile into
the Rust binary. Recreate the same layouts in `egui`.
