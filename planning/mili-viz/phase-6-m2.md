# Phase 6 M2 — landed (pygriz connection model + session file)

> **Status: ✅ COMPLETE.** This milestone is landed and frozen.
> Full live status in [`status.md`](status.md); this file is retained
> for the decision-number cross-references in the rest of the tree.

## What landed

- Server-side session file (the Jupyter-connection-file pattern):
  the `mili-viz-server` **binary** (only — `main.rs`, never the
  frozen library transport) writes `$GRIZ_SESSIONS_DIR/<id>.json`
  (default `~/.griz/sessions/`) immediately after `serve_tcp`
  returns the bound `SocketAddr`. Fields: `host`, `port`, `token`,
  `protocol_version`, `pid`, `db`. Wire contract untouched
  (no proto edit, no `lib.rs` edit); token written but not enforced
  so the frozen M1 gate's tokenless `connect()` still passes.
- `griz.attach()` with deterministic precedence: explicit
  `(host, port)` > `id=` > newest-live-local (highest mtime whose
  `pid` is alive). Every branch lowers to the M1 `connect()` —
  one transport, never a parallel client.
- `griz.launch(...)` discovers the binary
  (`$GRIZ_SERVER_BIN` → `target/{release,debug}/mili-viz-server` →
  `PATH`), spawns on `127.0.0.1:0`, parses the bound port from the
  binary's `tcp://127.0.0.1:<port>` stdout line, then attaches via
  the session file the child just wrote. `gui=` is accepted but
  emits `GuiUnavailableWarning` (renderer track is independent).
- `griz.list_sessions()` returns newest-first; malformed/partial
  JSON files are skipped, not crashed on; dead-pid sessions are
  filtered in the `attach()` newest-pick.
- Landing M2 fully discharges the Phase 5 M3.5 scripting-runner
  cross-milestone dependency.

## Gating test

`python/pygriz/tests/test_m2_attach.py` (run by `test-pygriz`) —
always-on pure logic over a fabricated tmp `GRIZ_SESSIONS_DIR`
(newest-first ordering, `id=` selection, malformed-skip,
empty-dir error) + skip-on-absent leg spawning the real
`mili-viz-server` and verifying it writes a valid session file
that `attach()`/`launch()` consume end-to-end.

## Decisions

- Decisions 56–58; index lives in [`status.md`](status.md).
