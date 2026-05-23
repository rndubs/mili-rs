# Phase 5 M3.5 — landed (bottom tabs: cmdline / scripting / time-history)

> **Status: ✅ COMPLETE.** This milestone is landed and frozen.
> Full live status in [`status.md`](status.md); this file is retained
> for the decision-number cross-references in the rest of the tree.

## What landed

- `egui_plot` 0.35.0 pinned, sparse-index-verified vs the frozen
  `egui` 0.34.2 (0.34.x egui_plot wrongly needs egui 0.33).
- Bottom-tabs panel replacing the M3 collapsed stub: always-present
  22 px tab strip + default-collapsed body (`bottom_tab:
  Option<BottomTab> = None`) so the M3 default footprint is byte-
  stable and `m3_egui_shell.rs` stays green unchanged.
- **Command line tab** — monospace green `griz>` prompt;
  submitted lines lower to `Execute(Command{ raw })` verbatim over
  the existing live in-process `Session` (Layer-0 ≡ raw, no Python/
  Rust client-side re-parse); echo + response are client-side
  `TranscriptLine`s on `ShellState`.
- **Scripting tab** — structured disabled placeholder (editor +
  Run + output pane + `venv:…·attach:…` indicator) blocked on the
  uncoded Phase 6 `pygriz`; no subprocess, no `UiAction`.
- **Time-history tab** — `egui_plot` host plotting the active
  result's `(state_time, min, max)` series accumulated from the
  broadcast `Subscribe`/`ResultState` stream (not the stubbed
  `Query`; per-element series via `Query` is the forward path).

## Gating test

`crates/mili-viz-client/tests/m3_5_bottom_tabs.rs` — always-on tab-
logic assertions (synthetic `RawInput` clicks; `RunCommand` emits
exactly one raw `UiAction` verbatim) + skip-on-absent composite
render with an opened tab body proving Decision 45's seam preserved.

## Decisions

- Decisions 48–52; index lives in [`status.md`](status.md).
