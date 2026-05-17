# `mili-viz` Phase 5 M3.5 — bottom tabs (buildable scope)

> Scope doc for Phase 5 Milestone 3.5, the wireframes'
> §"Implementation order" item 2 (the explicit milestone after the
> M3 shell). M3 stood up the `egui` shell — toolbar, left dock, the
> five viewport overlays — and deliberately left the bottom-tabs
> region a **collapsed disabled stub** (`Panel::bottom("tabs")` with
> three `add_enabled(false)` buttons + a `(M3.5)` weak label) and the
> AI rail a 28 px placeholder (M6). M3.5 replaces the stub with the
> three real peer tabs: the **Layer-0 command line** (functional
> now), the **scripting runner** (structured placeholder, gated on
> the uncoded Phase 6 `pygriz`), and the **`egui_plot`
> time-history** (functional now, fed by the already-implemented
> `Subscribe` stream). No proto change — the Phase 4 M1 contract is
> frozen.
>
> Read [`status.md`](status.md) first, then
> [`griz_wgpu_wireframes/README.md`](griz_wgpu_wireframes/README.md)
> §§ "Bottom tabs"/"Implementation order", `client.md` § "Bottom
> tabs", `scripting.md` ("Layer 0 ≡ raw stream"), and
> [`phase-5-m3.md`](phase-5-m3.md) (the M3 seam M3.5 must keep
> intact). Decision entries continue the global log (Phase 4 ended at
> 34; Phase 6 M1 took 35–37; Phase 5 M1 38–40, M2 41–43, M3 44–47;
> Phase 5 M3.5 starts at **48**).

## Goal

Wireframes §"Bottom tabs": three peer tabs in a
`TopBottomPanel::bottom`, a **22 px tab strip**, the region
**hideable**. Concretely, `crates/mili-viz-client`'s
`build_shell_ui` grows a real bottom-tabs panel in place of the M3
collapsed stub:

- **Command line** — monospace, green `griz>` prompt, echoed
  commands, dim response lines. Each submitted line lowers to
  `Execute(Command{ raw })` over the existing in-process `Session`
  (verbatim Layer-0, no typed re-parse — `scripting.md` "Layer 0 ≡
  raw stream"). Echo + response history is client-side transcript
  state on `ShellState`.
- **Scripting** — editor + Run + output pane + venv indicator,
  rendered **disabled** with a clear "requires `pygriz` (Phase 6)"
  affordance. The functional subprocess+`attach()` runner
  (`client.md` decision 3) is **blocked on Phase 6 `pygriz`**, which
  is only scaffolded (`phase-6-m1.md` not coded). Ships as the
  structured chrome that lights up when Phase 6 lands (Decision 49).
- **Time-history** — an `egui_plot` host plotting the active
  result's data-range series accumulated from the broadcast
  `Subscribe`/`ResultState` stream as states are visited (Decision
  50). The `Query`-fed per-element/per-node series is the documented
  forward path.

Out of scope (unchanged from the wireframes' milestone split): the
AI panel (M6), local view manipulation reconciled against the
server-authoritative camera beyond emitting the command (M4),
**remote** mode (M5). M3.5 uses the **in-process** transport only
and is **purely client-side** — no Phase 4 server crate is touched.

## Decisions (continuing the global log)

### Decision 48 — the command-line tab is verbatim Layer-0: it lowers each submitted line to `Execute(Command{ raw })` over the existing live `Session`; echo + response history is client-side transcript state on `ShellState`

`scripting.md` pins "Layer 0 ≡ raw stream": the command line is the
power-user / migration surface and must be the *exact* `Command.raw`
escape hatch, never a typed re-parse in the client. The submitted
line is sent verbatim as `pb::command::Cmd::Raw(line)` over the M3
live in-process `Session::execute` (the server already owns
`parse_raw`; the Phase 4 M1 `layer0_equals_raw` acceptance test
covers the equivalence). `CommandReply` carries only `ok` /
`error` / `delta_seq` — griz commands have no text payload; their
effect is the broadcast `StateDelta` the rest of the shell already
ingests. So the transcript is purely **client-side**: the shell
echoes the entered line as a green `griz>` row immediately and the
windowed app appends a dim outcome row (`ok`, or the `error`
string) after the `Execute` returns. This keeps `build_shell_ui`
pure/GPU-free (the transcript is `Vec<TranscriptLine>` on
`ShellState`; the input buffer is a `String` field), with the new
`UiAction::RunCommand(String)` the only transport-affecting variant.
Rejected: a client-side griz mini-parser to render structured
responses — it would *be* the second mechanism `scripting.md`
forbids and drift from the server's `parse_raw`.

### Decision 49 — the scripting tab ships as a structured, disabled placeholder (editor + Run + venv indicator), not a functional subprocess runner, because the runner is blocked on the uncoded Phase 6 `pygriz`

`client.md` decision 3 makes the scripting runner a managed-venv
subprocess that `attach()`es to *this* session via the
`scripting.md` session file and runs the pure-Python `griz`
(`pygriz`) package. `pygriz` is only **scaffolded** —
`phase-6-m1.md` (the `connect`/`Hello`/Layer-0 path) is not yet
coded, and Phase 6 M2 (`attach()` + the session-file discovery the
runner needs) is further out. A functional runner is therefore not
buildable at M3.5 without either coding Phase 6 inline (out of this
milestone's scope and branch) or shipping a runner that cannot
attach. M3.5 ships the **chrome** — a monospace editor area, a Run
button, an output pane, and the `venv: … · attach: …` indicator —
all rendered **disabled** with a single explicit affordance
(`scripting runner requires pygriz (Phase 6) — not yet available`).
No subprocess is spawned, no `UiAction` is emitted from this tab.
When Phase 6 M1+M2 land, the runner wiring is a contained follow-up
(spawn the venv child, stream stdout into the existing output pane);
the placeholder's shape is chosen to make that a fill-in, not a
redesign. This mirrors the wireframes' "AI rail is a placeholder
until M6" precedent and keeps M3.5 unblocked and independent
exactly as `status.md` item 17 frames it. Recorded as the explicit
decision the task asked for.

### Decision 50 — the time-history tab is fed by the already-implemented `Subscribe`/`ResultState` stream (the active result's `{min,max}` accumulated over visited states), not the stubbed `Query` RPC; the `Query`-fed per-element series is the documented forward path

`phase-4-m1.md` Decision 3 nominates a client-side `egui_plot`
time-history "fed by the existing `Query`". But the Phase-4 server's
`Query` RPC is **shape/plumbing only** — `MiliViz::query` returns an
`InlineTable` with `values: vec![]` (`mili-viz-server/src/lib.rs`,
"real values need `mili-rs` wired in at M2/M3 (Decision 7 table)";
that wiring was never done — M2–M5 wired geometry/derived through
`show`, not `query`). A literally `Query`-fed plot would render
nothing, and filling the server `Query` body is a Phase-4 data
milestone, not a client tab — and Phase 4 is **frozen/complete**
(`status.md` TL;DR; the task forbids regressing the frozen server
gating tests). So M3.5's time-history plots a **client-accumulated
series** built from the broadcast `ResultState` the shell *already*
ingests every `DELTA_RESULT`: each visited state with an active
result contributes a `(state_time, min, max)` sample, drawn as two
`egui_plot` lines (data-range envelope vs. simulation time). This is
a real, working, testable time-history fed entirely by the
**already-implemented** `Subscribe` stream — zero proto change, zero
Phase-4 change, no dependency on the stubbed `Query`. The
`time_hist.c` X-Y per-element/per-node series (Decision 3's eventual
model) is the documented **forward path**: it lights up unchanged in
this tab when the server `Query` body is implemented (a future
Phase-4 follow-up / Phase-5 query milestone), exactly mirroring the
scripting-tab placeholder reasoning (Decision 49). The series buffer
is `Vec<TimeSample>` on `ShellState`, fed by the app's existing
`apply_result`; `build_shell_ui` stays pure/GPU-free (`egui_plot` is
immediate-mode UI, no `wgpu`).

### Decision 51 — the bottom-tabs panel is a 22 px tab strip that is **always present**, with a collapsed body by default; the body region expands on tab click and collapses on re-click, keeping the M3 render seam (Decision 45) byte-stable so `m3_egui_shell.rs` does not regress

Two constraints collide: the wireframes want a resizable ~200 px
bottom-tabs region, and `phase-5-m3.md`'s acceptance gate / the
"do not regress" instruction require `m3_egui_shell.rs`
(`composite_render`, a 200×160 off-screen render asserting the
viewport centre is still the mesh) to stay green **unchanged**. A
200 px bottom panel in a 160 px-tall test target would starve the
central viewport and flip that assertion. Resolution: the bottom
panel is **always** the 22 px tab strip (structurally the same
footprint as the M3 collapsed stub it replaces — wireframes: "Tab
strip is 22 px tall"), and the tab **body** is collapsed by default
(`bottom_tab: Option<BottomTab> = None`). Clicking a tab opens its
body (a resizable region, default 200 px); clicking the active tab
collapses back to the strip — this *is* the wireframes' "hideable
region", reusing the AI-rail-collapsed-by-default minimal-first-run
precedent. Because `ShellState::default()` leaves `bottom_tab =
None`, the M3 test (which builds `..ShellState::default()` and never
opens a tab) sees the same ~22 px bottom footprint as the M3 stub →
`m3_egui_shell.rs` stays green **unchanged**, and the M3
additive-composition seam (Decision 45: unchanged mesh pass → egui
pass; the transparent `CentralPanel`) is preserved verbatim — M3.5
only fills the strip's buttons and adds an optional body panel above
the status bar. The M3.5 gating test opens each tab explicitly and
renders at a target tall enough to verify the body. New
client-only `UiAction`s (`SelectBottomTab`,`CollapseBottomTabs`) are
applied to `ShellState` in the pure fn and returned for
observability, exactly like `ToggleOverlay`/`SetStride` (Decision
46); only `RunCommand` is transport-affecting. The AI-rail M6
placeholder and every M3 panel are untouched.

### Decision 52 — pin `egui_plot` 0.35.0, verified compatible with the frozen `egui` 0.34.2 via the sparse index (crates.io JSON API blocked, Decision 44's method)

`egui_plot`'s version line does **not** track `egui`'s: read off
the sparse index (`https://index.crates.io/eg/ui/egui_plot`, the
crates.io JSON API being blocked here as in Decision 44),
`egui_plot` 0.34.0/0.34.1 declare `egui ^0.33.0` (incompatible with
our pinned `egui` 0.34.2), while **`egui_plot` 0.35.0 declares
`egui ^0.34.0`** — satisfied by the already-resolved `egui` 0.34.2
with **no `egui`/`wgpu`/`winit` bump and no churn to the frozen
Phase 4 crates**. So the M3.5 dependency add is exactly
`egui_plot = "0.35.0"`. Rejected: `egui_plot` 0.34.x (the
name-matching version) — it would force an `egui` 0.33 downgrade,
breaking the whole M1–M3 `egui`/`wgpu` stack for no gain.

## Crate layout (delta from M3)

```
crates/mili-viz-client/
├── Cargo.toml          # + egui_plot 0.35.0
├── src/
│   ├── shell.rs        # bottom-tabs panel replaces the M3 stub:
│   │                   #   BottomTab enum, TranscriptLine, TimeSample,
│   │                   #   cmdline/scripting/time-history bodies;
│   │                   #   ShellState += bottom_tab/transcript/
│   │                   #   cmdline_input/time_history; new UiActions
│   ├── app.rs          # lower RunCommand → Execute(raw); append the
│   │                   #   dim outcome row; feed TimeSample from
│   │                   #   apply_result; the M3 paths unchanged
│   ├── session.rs      # unchanged (Session::execute already raw-capable)
│   └── … (camera/mesh/colormap/renderer/egui_layer unchanged)
└── tests/
    ├── m1_renderer.rs              # unchanged
    ├── m2_render_server_output.rs  # unchanged
    ├── m3_egui_shell.rs            # unchanged (Decision 51 keeps it green)
    └── m3_5_bottom_tabs.rs         # NEW (always-on tab logic +
                                    #      skip-on-absent composite render)
```

## Acceptance gate

- `cargo test --workspace --exclude mili-py` builds `mili-viz-client`;
  the always-on `m3_5_bottom_tabs` assertions pass with no GPU: the
  pure `build_shell_ui` opens each tab via synthetic `RawInput`,
  a command-line submit emits exactly one
  `UiAction::RunCommand(raw)` with the raw line verbatim and echoes
  a `griz>` transcript row, the scripting tab emits no action, and a
  fed `time_history` series produces plot shapes.
- `cargo fmt --all --check` and
  `cargo clippy --workspace --all-targets -- -D warnings` are clean.
- The skip-on-absent composite render (`serial/basic1` + a `wgpu`
  adapter) opens the command-line tab and renders the mesh **and**
  the egui chrome incl. the expanded bottom-tabs body into one
  off-screen texture, asserting the body region is opaque chrome
  while the (shrunken) viewport centre is still the mesh —
  Decision 45's additive composition preserved with a real bottom
  panel. Printed-and-skipped (not failed) when the corpus or a
  `wgpu` adapter is absent (CLAUDE.md convention).
- `m1_renderer.rs`, `m2_render_server_output.rs`, **and
  `m3_egui_shell.rs` are unchanged and green** (Decision 51 keeps the
  default bottom footprint ≈ the M3 22 px stub; Decision 45's seam is
  untouched). No Phase 4 crate is touched; every Phase 4 server
  gating test is unchanged. No `mili_viz.proto`/blob/ticket change.

## Decision log (this doc)

| # | Title | Resolves |
|---|---|---|
| 48 | Command-line tab = verbatim Layer-0 `Execute(Command{raw})` over the live `Session`; echo + response are client-side transcript state on `ShellState` | M3.5 command line |
| 49 | Scripting tab ships as a structured **disabled placeholder** (blocked on the uncoded Phase 6 `pygriz`); functional runner is a contained Phase-6 follow-up | M3.5 scripting |
| 50 | Time-history tab fed by the already-implemented `Subscribe`/`ResultState` stream (range-vs-time series), **not** the stubbed `Query`; `Query`-fed per-element series is the documented forward path | M3.5 time-history |
| 51 | Bottom panel = always-present 22 px tab strip + default-collapsed body; M3 default footprint ≈ the M3 stub so `m3_egui_shell.rs` + the Decision-45 seam stay byte-stable | M3.5 layout / no-regress |
| 52 | Pin `egui_plot` 0.35.0, sparse-index-verified vs the frozen `egui` 0.34.2 (0.34.x egui_plot wrongly needs egui 0.33) | M3.5 deps |
</content>
</invoke>
