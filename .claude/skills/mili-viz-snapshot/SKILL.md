---
name: mili-viz-snapshot
description: Take a screenshot of the running mili-viz-client GUI (mesh viewport + egui chrome) so you can see what the user sees. Use when the user asks "what does the app look like", "take a screenshot", "show me the current state of the viz", or when you've made a UI change and want to verify it visually.
---

# mili-viz-snapshot

The `mili-viz-client` windowed app exposes a screenshot mechanism so an
agent (you) can see the composited GUI — mesh viewport, toolbar, left
dock, AI panel, bottom tabs, overlays — without needing the user to
manually capture and paste an image.

## When to use

- The user asks for a screenshot, image, or "what the app looks like now".
- You just edited UI code in `crates/mili-viz-client/` and want to verify
  visually that the change is correct.
- You're debugging layout, theming, render-mode, or colormap issues that
  are hard to reason about from code alone.
- The user describes a visual bug ("the left dock is too narrow",
  "labels are clipped") and you need to see it.

Do not use for purely backend / server-side issues (`mili-viz-server`,
`mili-rs`); the screenshot only captures the client window.

## Prerequisites

There must be a `mili-viz-client` window already running on the user's
machine. If not, ask the user to start one — do NOT launch one yourself
(it opens a window on their desktop, which is intrusive).

Typical user-side launch:

```bash
cargo run -p mili-viz-client                       # empty session
cargo run -p mili-viz-client -- -i <plotfile>      # with a database
```

## How to take a snapshot

Run the CLI:

```bash
cargo run -q -p mili-viz-client -- snapshot
```

On success it prints **one line**: the absolute path of the freshly
written PNG. Read it with the Read tool:

```
<path printed by snapshot> e.g. /Users/<user>/.griz/snapshots/cli-1716583942000.png
```

Then read with the Read tool (Claude Code's Read can ingest PNG
directly):

```
Read(file_path="/Users/<user>/.griz/snapshots/cli-1716583942000.png")
```

## Useful flags

- `--out PATH` — write the PNG to `PATH` instead of the default
  timestamped name. Use this when you want a stable filename (e.g.
  for repeated comparison across edits).
- `--timeout SECS` — how long to wait for the running window to pick
  up the request. Default `5.0` seconds. Increase if the user's app is
  busy mid-render.
- `snapshot --help` — prints the inline help.

## How it works (so you can debug if it fails)

The CLI drops a request file at `~/.griz/snapshots/.capture-request`;
the running window polls that file ~once a second and, when it appears,
re-renders the mesh + egui passes into an offscreen RGBA8 texture, reads
it back, and writes a PNG to the requested path. It also overwrites
`~/.griz/snapshots/latest.png` so you can always grab the most recent
frame from that fixed path.

`$GRIZ_SNAPSHOTS_DIR` overrides `~/.griz/snapshots` (used in tests; also
useful if `~/` is mounted noexec / read-only on a particular box).

## Failure modes and what they mean

- **`no running mili-viz-client serviced the request within Ns`** — no
  window is up, or it's a different `mili-viz-client` binary that
  doesn't have the snapshot machinery (predates this feature). Confirm
  with the user that a window is open; if so, check
  `~/.griz/snapshots/.capture-request` — if it's still there after
  several seconds with a window open, the window-side polling is broken.
- **PNG path printed but `Read` of it fails** — almost always means a
  stale path is in `latest.png` from a previous session. Use the path
  the CLI just printed, not `latest.png`, for the freshest result.
- **The image shows the mesh viewport but missing chrome / overlays** —
  the windowed app shipped before the F12 / `snapshot` feature; rebuild
  and rerun.

## Hotkey alternative (user-driven)

`F12` in the running window writes a snapshot to
`~/.griz/snapshots/hotkey-<timestamp>.png`. Suggest this when the user
wants to capture a transient state (e.g. mid-drag) the CLI request
roundtrip can't catch.
