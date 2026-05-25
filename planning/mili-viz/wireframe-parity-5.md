# Wireframe-parity #5 — scripting tab attach-into-*this*-GUI

Closes the second half of [`wireframe-parity.md`](wireframe-parity.md)
row #5: a sibling `pygriz` script invoked by the scripting-tab Run
button (or from any external process) can now `griz.attach()` into
the **in-process** server the windowed client spawns, not just into a
separately-launched `mili-viz-server` binary.

The runner half (editor + Run button + streamed output + `venv:…·
attach:…` status line → `UiAction::RunScript`) was already in place
before this change. The blocker was that `attach()` reads
`~/.griz/sessions/<id>.json` to find a running server — and the
in-process server publishes no such file. `mili-viz-server`'s
`main.rs` writes one immediately after `serve_tcp` binds, but the
windowed client never goes through that path (it calls
`spawn_in_process` directly, which routes RPCs over a tonic in-memory
duplex with no TCP socket and no on-disk artefact).

Two paths were on the table (see the design questions captured at the
top of [`wireframe-parity.md`](wireframe-parity.md) #5):

- **(a)** Discriminator field on the existing Decision-56 session-file
  shape: keep `{host, port, session_id, …}`, add an optional
  `transport: "in-process"` + `socket_path` pair the loader checks
  first. Existing `attach(host=..., port=...)` keeps working
  byte-identically — important for VB-001 parity with Phase 6 M2.
- **(b)** Drop `host`/`port` for in-process sessions and add a
  top-level `in_process: true` flag. Cleaner shape but breaks the
  Decision-56 implicit invariant that a session file always carries a
  TCP endpoint; older pygriz versions would fail to parse.

The maintainer picked **(a)** — minimal blast radius, byte-stable for
the TCP arm, forward-compat for unknown-field-tolerant readers.

Handshake mechanism, three options considered:

- **(i)** **Unix domain socket** published in the session file. Zero
  new deps; works on macOS/Linux today. Windows would need a
  named-pipe follow-up. **Picked.**
- (ii) The `interprocess` crate for cross-platform parity from day
  one. One new dep we don't currently carry and more code paths to
  test. Deferred.
- (iii) `tokio::io::duplex` direct in-memory handoff. Only works when
  pygriz runs *inside* the GUI's own Python interpreter, not a sibling
  process — defeats the purpose of the scripting-tab Run button which
  spawns a subprocess. Rejected.

## What landed

- Server library (`mili-viz-server`):
  - New `serve_uds(svc, path) -> (PathBuf, JoinHandle)` —
    Unix-only (`#[cfg(unix)]`) analog of `serve_tcp`. Binds a
    `tokio::net::UnixListener` at `path` and runs the same
    `MiliVizServer` + `FlightServiceServer` router over
    `tokio_stream::wrappers::UnixListenerStream`. Best-effort removes a
    stale socket file before binding so a previous crashed instance
    that skipped Drop never causes EADDRINUSE. Frozen wire contract
    untouched — UDS carries the same HTTP/2 frames TCP does.
- Client (`mili-viz-client`):
  - New private module slot in `session.rs` —
    `publish_in_process_session(svc)` picks a UDS path under `/tmp`,
    calls `serve_uds` on the same `VizService` instance the in-process
    `MiliVizClient` channel speaks to, then writes the session JSON
    with the two new fields (`transport: "in-process"`,
    `socket_path: "/tmp/griz-<id>.sock"`) using the same atomic
    temp-then-rename the binary uses.
  - `InProcessSessionGuard` (Drop-removes-both-files) is held on
    `App` so a clean GUI exit leaves `~/.griz/sessions/` empty. A
    force-killed GUI leaves the JSON behind; the read-side pid-
    liveness filter (Decision 57 / Decision 111) excludes it from
    newest-live picks.
  - `app::run` calls the publisher only on the no-transport branch
    (the default in-process arm). `--remote`/`--attach` consume an
    *external* server's session file and must not write one of their
    own. Publish failure is non-fatal — the GUI still works locally;
    pygriz-attach is the loss surfaced via stderr.
- Python client (`pygriz`):
  - `SessionInfo` gains two new optional fields (`transport`,
    `socket_path`), default `""`. Legacy session files (pre-Decision-
    109, TCP-only) parse unchanged.
  - New `_connect_uds(socket_path, token, …)` builds the
    gRPC channel via `grpc.insecure_channel(f"unix:{socket_path}")` and
    runs the same `Hello` handshake `connect()` does. The two share a
    factored `_handshake(channel, …)` helper so the post-channel path
    stays single-sourced.
  - `attach()` dispatches on the resolved session's `transport`:
    `"in-process"` → `_connect_uds(info.socket_path, info.token)`;
    anything else (including the legacy empty string) → the existing
    TCP `connect(info.host, info.port, info.token)`. The explicit
    `host=`/`port=` escape hatch still wins ahead of the
    file-resolution branch (Decision 57 precedence intact).
- Wire format: zero `.proto` edit, zero `crates/mili-viz-proto`
  edit. The session file is a connection-discovery side-channel; the
  transport on the wire is the same MiliViz + Flight as TCP.

## Decisions

### Decision 109 — Transport discriminator field, additive on Decision 56

`{host, port}` stays in the session file. Two new optional fields land
next to them:

```json
{
  "id": "ab12cd34",
  "pid": 12345,
  "host": "127.0.0.1",
  "port": 0,
  "transport": "in-process",
  "socket_path": "/tmp/griz-ab12cd34.sock",
  …
}
```

`attach()` checks `transport` first and routes to UDS when it equals
`"in-process"`; the TCP arm runs verbatim otherwise.
Forward-compatible: an older pygriz that ignores `transport` lands on
the TCP arm with `host: 127.0.0.1 / port: 0` and fails loud at
`connect` time (port 0 is an OS sentinel — there is no "0 connect")
rather than mis-routing to a random listener.

### Decision 110 — UDS path lives under `/tmp`, not `$TMPDIR` or `<sessions_dir>`

macOS's `sun_path` field caps Unix socket paths at 104 bytes. macOS
`$TMPDIR` is typically `/var/folders/<u>/<h>/T/` (≥ 40 chars before
the file name); a session id (8 hex + `griz-` + `.sock` = 18 chars)
fits but with no margin. `/tmp` is one of the few path roots that is
always short, always exists, and is universally writable on a
single-user dev workstation (the only target the in-process arm cares
about). The publisher falls back to `std::env::temp_dir()` only when
`/tmp` does not exist (effectively never on a host with a sane Unix
filesystem). The session file's `~/.griz/sessions/` location is
unchanged — only the socket goes to `/tmp`.

Putting the socket alongside the JSON in `<sessions_dir>` would have
been "tidier" but `$HOME` can exceed the sun_path budget (think
`/Users/very-long-account-name/.griz/sessions/<id>.sock` — 50+ chars
of prefix alone), and the user has no way to tell why their attach
failed.

### Decision 111 — Drop guard cleans on clean exit; pid-liveness filters force-kills

`InProcessSessionGuard::drop` removes both `<sessions_dir>/<id>.json`
and the `/tmp/griz-<id>.sock` it points at. A graceful GUI exit (the
common case) therefore leaves the sessions dir empty.

A force-killed GUI (Cmd+. / kill -9 / panic) skips Drop and leaves
the JSON behind. The read side already handles this for the TCP arm
(`_pid_alive` in pygriz, `pid_alive` in the Rust attach resolver
through `kill(pid, 0)` — Decision 57): newest-live-only excludes
stale entries from `attach()` with no args; an explicit
`attach(id=...)` for a stale id raises with the existing
`no readable griz session` error from the parse path or a UDS-connect
failure from `_connect_uds`.

This is intentionally simpler than tracking `socket_path` liveness:
the pid check already discriminates correctly, and the socket file
will be reaped by tmp-clean on next reboot if nothing else does.

## Gating tests

Server (`crates/mili-viz-server/tests/uds_transport.rs`,
2 always-on Unix-only cases):

- `serve_uds_round_trips_a_hello` — bind a UDS, dial it with a
  tonic channel backed by `tokio::net::UnixStream`, assert the
  `Hello` reply comes back protocol-compatible.
- `serve_uds_clears_a_stale_socket_file` — leave a pre-existing
  file at the bind path, confirm `serve_uds` removes it and the
  listener is live afterwards.

Client (`crates/mili-viz-client/src/session.rs::tests`,
1 always-on Unix-only combined case):

- `publish_in_process_session_round_trip` — exercises the full
  publisher contract in one test (cargo runs cases in parallel and
  splitting them would race the shared `GRIZ_SESSIONS_DIR` env var,
  serialised by a module-local `ENV_LOCK` `Mutex` shared with the
  resolve_session_* cases):
  - exactly one `*.json` under the configured sessions dir,
  - the file contains `"transport": "in-process"` + `"socket_path":
    …` + the legacy `host: 127.0.0.1 / port: 0` sentinels,
  - the advertised UDS exists on disk,
  - dialling it returns a protocol-compatible `Hello`,
  - dropping the guard removes both files.

Python (`python/pygriz/tests/test_m2_attach.py`,
4 new always-on cases on top of the existing 7):

- `test_session_info_carries_transport_and_socket_path` — the new
  fields parse off both shapes (legacy → empty defaults; in-process
  → populated).
- `test_attach_in_process_routes_through_uds_path` — `attach(id=)`
  on an in-process session calls `_connect_uds(socket_path, token)`,
  not `connect(host, port, token)`. Monkey-patches both.
- `test_attach_in_process_without_socket_path_raises` — a
  discriminator without socket_path is a malformed file; raises
  rather than silently falling through to the `host: 127.0.0.1 /
  port: 0` sentinel mis-route.
- `test_attach_explicit_host_port_overrides_in_process` — the
  Decision-57 `attach(host=, port=)` escape hatch still wins ahead
  of the session-file resolution branch (used when forwarding the
  GUI's UDS over an SSH `-L unix:...` tunnel).

## Out of scope (follow-up)

- The other half of `wireframe-parity.md` #5 — `pip install`ed
  managed venv — is independent. The Run-button subprocess uses a
  PYTHONPATH-injected `griz` in the workspace today; productionising
  a venv strategy belongs in its own change.
- Windows — `serve_uds` is `#[cfg(unix)]`-gated and the publisher is
  too. A named-pipe arm (via `interprocess` or
  `tokio::net::windows::named_pipe`) is a clean follow-up that lands
  by adding a second `Transport` variant, not by reshaping the
  session file. Decision 109 explicitly accommodates this — the
  discriminator string is the extension point.
- Multi-user shared boxes — the UDS file in `/tmp` is world-readable
  by default (mode 0666 minus umask). Adequate for a single-user dev
  workstation; a hardened multi-tenant deployment would want either
  per-user `$XDG_RUNTIME_DIR` placement or explicit `chmod 0600`
  after bind. Tracked for a future hardening pass; the current
  threat model is "developer attaching their own VS Code script to
  their own GUI."
