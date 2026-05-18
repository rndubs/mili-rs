# `mili-viz` Phase 6 M2 — `pygriz` connection model + server-side session file (buildable scope)

> Scope doc for **Phase 6 Milestone 2**. M1
> ([`phase-6-m1.md`](phase-6-m1.md)) shipped the `pygriz` scaffold,
> the gitignored stub generator, `griz.connect(host, port, token=...)`
> + the `Hello` handshake, and the Layer-0 escape hatch. M2 builds the
> **connection model** from [`scripting.md`](scripting.md) § "Connection
> model" — the priority interactive path: open a GUI/server, attach a
> VS Code script to it via the Jupyter-connection-file pattern.
>
> Read [`status.md`](status.md) first (the live tracker), then
> [`scripting.md`](scripting.md) (the three modes + the session/
> connection-file contract) and [`phase-6-m1.md`](phase-6-m1.md) (the
> landed M1 surface this extends). The decision log is global and
> monotonic across `mili-viz`; **M1 ended at Decision 55, M2 continues
> at 56**.

## Goal

Ship `griz.attach()` (priority path: the newest local
`~/.griz/sessions/<id>.json`), `griz.attach(id=...)` /
`griz.attach(host=, port=, token=)` (explicit), `griz.launch(gui=...)`
(spawn the `mili-viz-server` binary on a free port), and
`griz.list_sessions()`. This depends on the **server writing the
session/connection file on startup**, which the frozen Phase 4 server
does **not** currently do — that on-disk emission is in scope here
(Decision 56). **No change to the frozen `mili_viz.proto`** and no
change to the frozen `mili-viz-server` *library* transport — the only
server-side change is the binary's `main` emitting on-disk JSON.

Landing M2 **fully discharges** the Phase 5 M3.5 scripting-runner
cross-milestone dependency (`phase-5-m3.5.md` Decision 49): M1 only
unblocked it at the `connect()` level; the disabled subprocess +
`attach()` runner placeholder lights up once `attach()` exists.

## Decisions

### Decision 56 — the server writes `~/.griz/sessions/<id>.json` from the binary's `main`, not the frozen library transport; wire contract untouched; token written but not enforced

`scripting.md` § "Protocol impact" makes the session/connection file a
named M1-era contract item ("written by the server on startup:
`~/.griz/sessions/<id>.json` with `host`, `port`, `token`,
`protocol_version`, `pid`, and the loaded `db`. This is the Jupyter-
connection-file pattern"). The Phase 4 server never implemented it:
`HelloReply.session` reports `id: "in-process"`, `port: 0`. M2's
`attach()` / `list_sessions()` depend on this file existing, so its
emission is in M2 scope. Constraints and resolution:

- **Wire contract untouched.** This is on-disk JSON, **not** a proto
  change — `crates/mili-viz-proto/proto/mili_viz.proto` is byte-for-
  byte unchanged. The `Hello` handshake / `HelloReply.session` echo is
  also left exactly as Phase 4 froze it (still `id: "in-process"`,
  `port: 0`): per `scripting.md` the **session file is the
  `attach()`/`list_sessions()` source of truth**, not the on-wire
  echo, and mutating the frozen handshake's reply values would be a
  behavioural change to the frozen Phase 4 server with no contract
  need. The session file is a strictly additive, side-channel artifact.

- **Written from `main`, not `serve_tcp`/`spawn_*` (library).** The
  library transport (`serve_tcp`, `spawn_tcp`, `spawn_in_process` in
  `mili-viz-server/src/lib.rs`) is exercised by the **entire frozen
  Phase 4 acceptance suite and every Phase 5 client gating test**.
  Writing the file there would (a) change frozen library behaviour and
  (b) make every in-process/`spawn_tcp` unit test scribble JSON into
  `~/.griz`. Resolution: only the **binary** (`mili-viz-server/src/
  main.rs`) — what `launch()` spawns and what a user runs to host a
  GUI session for `attach()` — writes the file, immediately after
  `serve_tcp` returns the concrete bound `SocketAddr` (no TOCTOU; the
  port is real). The library stays byte-identical, so
  `cargo test --workspace --exclude mili-py` is unaffected by
  construction (no `lib.rs`/proto edit at all).

- **`id` / `token` derivation.** Both are short lowercase-hex strings
  mixed from the process pid and the startup time in nanoseconds (no
  new crate — the server's dependency surface is frozen; the JSON is
  hand-formatted with a minimal string escaper). `id` is the file
  stem; `token` is the Jupyter-style connection secret.

- **Token written but not enforced.** The frozen M1 acceptance gate
  (`test_m1_connect.py`) spawns *this same binary* and calls
  `griz.connect(...)` **tokenless**; turning on `expected_token`
  enforcement in the binary would make the frozen M1 gate fail with
  `unauthenticated`. Resolution: `main` does **not** call the existing
  `.expected_token(...)` builder hook — the token is written into the
  file for the Jupyter-file contract and forward-compatibility, and
  `attach()` sends it, but server-side enforcement remains the
  pre-existing opt-in builder API a future milestone / hardened
  deployment turns on. Sending an unchecked token is harmless; it is
  correct the day enforcement is enabled.

- **`db` and staleness.** The file is written **once** at bind. The
  fresh binary has nothing loaded, so `db` is empty; live db-tracking
  in the file is deferred (the live db is already available via the
  `Hello` echo / `Subscribe` stream — the wire is the live source,
  the file is just the bootstrap coordinate). The server does not
  delete the file on exit (a SIGKILLed process cannot); **staleness is
  handled on the read side** — `list_sessions()` / `attach()` skip
  unparseable files and, for `attach()`'s newest-pick, dead-pid
  sessions, so an accumulation of stale files is functionally
  harmless and lazily ignored.

- **Redirectable.** Both the server writer and the Python reader honor
  `$GRIZ_SESSIONS_DIR` (default `~/.griz/sessions`) so the M2 gate is
  hermetic — the spawned binary writes into a tmp dir, the reader
  reads the same, and a developer/CI home is never touched by the
  test. (The frozen M1 gate does not set it and so writes one file per
  run into the real `~/.griz/sessions`; that is the intended
  production behaviour — one file per running server — and is
  harmless per the staleness rule above.)

### Decision 57 — `attach()` precedence: explicit endpoint > `id` > newest live local session; all lower to the M1 `connect()`

`scripting.md` API sketch: `griz.attach()` (newest local),
`griz.attach(id="ab12cd")`, and the explicit
`griz.attach(host, port, token=...)`. Resolution — one function,
deterministic precedence, **no second transport**:

1. `host` **and** `port` given → connect to that endpoint directly
   (the explicit-endpoint form; `token` optional). This is the
   `attach`-spelled alias of M1 `connect()` for symmetry with the
   sketch.
2. else `id` given → read `<GRIZ_SESSIONS_DIR>/<id>.json`; error
   (clear, actionable `FileNotFoundError`-derived) if absent.
3. else → the **newest live** session file (highest mtime whose `pid`
   is still alive), erroring with an actionable message listing the
   sessions dir if none. "Newest" = file mtime, matching the Jupyter
   pattern and `scripting.md` "picks the newest local session".

Every branch ends by calling the **M1** `connect(host, port, token)`
and returning a `Session` — `attach()` is a session-file resolver in
front of the one transport, never a parallel client (the single-client
invariant, mirroring M1 Decision 37's single-parser invariant).

### Decision 58 — `launch()` spawns the binary and attaches via the file it just wrote; `gui=` is accepted but the renderer is an independent track (deferred, warns)

`scripting.md`: `griz.launch(gui=True)` "spawns `mili-viz-server`
(+ optional GUI) on a free port — like `visit -cli`". Resolution:

- `launch()` discovers the `mili-viz-server` binary (`$GRIZ_SERVER_BIN`
  → `target/{release,debug}/mili-viz-server` → `mili-viz-server` on
  `PATH`), spawns it on `127.0.0.1:0`, parses the bound port from its
  stdout (`tcp://127.0.0.1:<port>` — the exact line `main` already
  prints, reused verbatim from the M1 gate's binary-discovery
  approach), then **attaches via the session file that binary just
  wrote** (matched by the child pid), so `launch()` exercises the
  Decision-56 file path end-to-end and inherits its token. The
  returned `Session` owns the child process; `Session.close()` /
  the `with` context terminates it (Decision 56's no-server-side-
  cleanup is why the launcher owns the lifecycle).

- **`gui=`.** The GUI is the Phase 5 `wgpu`/`egui` renderer — an
  **independent track** (`status.md`: Phase 6 is independent of the
  Phase 5 renderer). M2 deliberately does **not** spawn it: the
  renderer binary/launch contract is not a Phase 6 deliverable and
  wiring it here would couple the two tracks. `launch(gui=True)` is
  accepted (the sketch's signature is preserved) but emits a
  `GuiUnavailableWarning` and proceeds headless — honest about the
  deferral, no signature churn when the renderer track wires it up.

## M2 acceptance gate

A gating test (`python/pygriz/tests/test_m2_attach.py`, run by the
`test-pygriz` job), two halves mirroring the CLAUDE.md / M1
skip-on-absent convention:

- [ ] **Always-on pure logic** (no server, no `cargo`): fabricated
      session JSON files in a tmp `GRIZ_SESSIONS_DIR` →
      `list_sessions()` returns them **newest-first**;
      `attach(id=...)` selects the named one; `attach()` selects the
      newest; a malformed/partial JSON file is skipped, not crashed
      on; an empty/missing sessions dir raises a clear, actionable
      error.
- [ ] **Skip-on-absent** (spawns the real `mili-viz-server` TCP
      binary, skipped — never failed — when `cargo`/the binary is
      absent, exactly the CLAUDE.md corpus skip): the spawned binary
      **writes a valid session file** (`host`/`port` == the bound
      port parsed from stdout, `pid` == the child, non-empty `token`,
      `protocol_version` == the canonical proto const);
      `griz.attach()` against that dir completes the handshake and a
      Layer-0 `command(...)`; `griz.launch()` spawns + attaches +
      `close()` terminates the child.
- [ ] The frozen Phase 4 server acceptance suite + every Phase 5
      client gating test are **unchanged and green** (no `lib.rs`/
      proto edit; `cargo test --workspace --exclude mili-py` green),
      and the frozen M1 gate `test_m1_connect.py` still passes.

## Out of scope for M2 (later Phase 6 milestones)

The Layer-1 object API + typed handles + the Layer-0 ≡ Layer-1 test
(M3); `Subscribe`/`@s.on(...)` live sync (M4); `query`/`to_dataframe`/
Arrow Flight (M5); `render`/`save_animation`/`snapshot` (M6);
server-side token enforcement and live `db` rewriting of the session
file (a later hardening milestone — see Decision 56). None require a
proto change — the M1 contract already froze all of it.
