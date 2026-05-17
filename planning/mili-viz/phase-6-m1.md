# `mili-viz` Phase 6 M1 — `pygriz` package scaffold + stubs + connect/handshake (buildable scope)

> Scope doc for **Phase 6 Milestone 1**. Phase 6 builds the
> pip-installable Python scripting client whose *design* was resolved
> in [`scripting.md`](scripting.md) (open question #1) but which had
> **no implementation home in the milestone roadmap** — Phase 4 is the
> pure-Rust server (✅ complete), Phase 5 is the `wgpu`/`egui`
> renderer. The scripting client is a **third client** of the frozen
> `mili-viz-proto` wire contract, gated **only on Phase 4 M1** (long
> landed) — it does **not** depend on the Phase 5 renderer and runs as
> an independent track.
>
> Read [`status.md`](status.md) first (the live tracker), then
> [`scripting.md`](scripting.md) (the resolved design + API sketch)
> and [`phase-4-m1.md`](phase-4-m1.md) (the frozen contract this
> consumes). The decision log is global and monotonic across
> `mili-viz` (M1: 1–9; M2: 10–12; M3: 13–15; M4: 16–18; M5: 19–21;
> M5b: 22–24; M6: 25–27; M5c: 28–31; M5d: 32–34;
> Phase 5 M3/M3.5: 44–52). **Phase 6 M1 scope = 35–37; the M1
> implementation decisions continue the global log at 53–55**
> (Phase 5 M3.5 ended at 52).

## Goal

Stand up the `pygriz` distribution (import namespace `griz`) as a
pure-Python second/third client of the **already-frozen, unchanged**
`crates/mili-viz-proto/proto/mili_viz.proto`, and prove the
connection + handshake + Layer-0 command path end-to-end against the
running `mili-viz-server`. M1 ships the package, the generated gRPC
stubs, `griz.connect(...)`, the `Hello` version/capability handshake,
and the Layer-0 escape hatch (`session.command(...)` /
`session.run_script(path)` → `Command.raw`). It does **not** ship the
Layer-1 object API, the `attach()`/`launch()` connection modes, live
`StateDelta` subscription, or the query/render surface — those are
Phase 6 M2–M6. **No change to the frozen proto.**

## Phase 6 milestone breakdown

These expand `scripting.md` into a buildable order. Only M1 is
specified in detail here; later milestones get their own
`phase-6-mN.md` when built (the Phase 4 discipline).

- **M1 — package scaffold + stubs + connect/handshake** (this doc).
  `griz.connect(host, port, token=...)`, `Hello` handshake,
  Layer-0 `command()` / `run_script()`.
- **M2 — connection model.** `griz.attach()` (priority path: read the
  newest `~/.griz/sessions/<id>.json`), `griz.attach(id=...)`,
  `griz.launch(gui=...)` (spawn `mili-viz-server`),
  `griz.list_sessions()`. Server-side session-file writing is in
  scope here if the server does not already emit it.
- **M3 — Layer-1 object API.** `s.open/state/next/select/show/
  isosurface/contour/materials/cutplane`, `s.view.*`
  (server-authoritative), typed handles (`Result`, `Isosurface`).
  Carries the **Layer-0 ≡ Layer-1 integration test**
  (`scripting.md` API-sketch conventions): every Layer-1 call MUST
  lower to the exact `Command` the raw stream produces.
- **M4 — live sync.** `Subscribe` stream → `@s.on("state_changed")`
  callbacks; the open GUI and a script stay in sync (camera mirrors).
- **M5 — query payoff.** `db.query(...)` / `current_result
  .to_dataframe()` returning the **same numpy/pandas types as the
  milox query layer**; Arrow Flight for large results.
- **M6 — output + remote tuning.** `s.render()` / `s.save_animation()`
  / `s.snapshot()` via `CaptureFrame`; HPC-latency buffer tuning.

## Decisions

### Decision 35 — a top-level `python/` tree, distribution `pygriz`, import `griz`, pure-Python (no pyo3)

The repo had no home for non-crate Python packages: `crates/mili-py`
is the Phase 1–3 **`milox` pyo3 extension** (a Rust crate built with
maturin — `[lib] name = "_native"`), unrelated to viz scripting.
`scripting.md` is explicit the viz client is **pure-Python — no
maturin/pyo3, no ABI coupling, a universal wheel**, so it cannot live
under `crates/`.

Resolution: a new **top-level `python/` directory**, the parallel of
`crates/` for distributions that are not Rust crates, with one package
per subdirectory. The first is `python/pygriz/`:

- Distribution name **`pygriz`**, import namespace **`griz`**
  (`import griz; s = griz.attach()` — matches `scripting.md` verbatim
  and griz muscle memory). `src/`-layout, `setuptools` backend.
- Not a Cargo workspace member; nothing to `exclude`. The fast
  `cargo` jobs never see it. A future `test-pygriz` CI job runs
  `pip install -e python/pygriz[dev]` + `pytest`, mirroring the
  `test-milox` split.
- `python/` is the designated drop-zone for future pure-Python
  packages so this decision is not relitigated per package.

### Decision 36 — stubs are generated build output from the single canonical proto, never hand-edited

`crates/mili-viz-proto/proto/mili_viz.proto` is the **one** canonical
copy (its header already says so; the Rust side codegens protoc-free
via `protox`). The Python client MUST consume that same file, not a
fork, so the wire contract cannot drift between clients.

Resolution: M1 generates `griz._proto` from
`crates/mili-viz-proto/proto/mili_viz.proto` with `grpcio-tools`
(pinned in the `dev` extra). Generated modules are **build output** —
regenerated, `.gitignore`d (the root `.gitignore` Python block this
milestone adds), never hand-edited — exactly the single-source rule
the Rust `protox` path follows. A
`scripts/gen-pygriz-stubs.sh` (or a `pyproject` hook) is the
repeatable generator; it is the Python analogue of the proto
`build.rs`. Version skew is caught at runtime by the existing `Hello`
handshake (`HelloReply.compatible` / `mismatch_detail`), never a
crash — this is the Visit "API matches the engine" guarantee bought
with a wire contract, and the M1 gate exercises the mismatch branch.

### Decision 37 — M1 is Layer-0 only; Layer-1 is M3 (no duplicate parser, reuse the server's)

`scripting.md` defines two layers; the server **already** parses the
Layer-0 griz line stream (`crates/mili-viz-server/src/raw.rs`
`parse_raw`, reached via `Command.raw`). M1 must not grow a second
parser in Python.

Resolution: M1 ships **only** `session.command("show sx; state 10")`
and `session.run_script(path)`, both of which send `Command { raw }`
and let the server's existing dispatcher parse. The typed Layer-1
object API (and the Layer-0 ≡ Layer-1 equivalence test that guards the
migration aid) is **M3**. This keeps M1 a thin transport+handshake
slice and preserves the single-parser invariant: griz-line parsing
lives in the server, once.

## Decision 53 — the stub generator: `grpc_tools.protoc` + a package-relative import rewrite, gitignore citation corrected

Implementation detail of Decision 36. `scripts/gen-pygriz-stubs.sh`
(the Python analogue of the Rust `protox` `build.rs`) is the single
repeatable generator: it `rm -rf`s `python/pygriz/src/griz/_proto`,
runs `python -m grpc_tools.protoc` (`--python_out`/`--grpc_python_out`/
`--pyi_out`) on the **one** canonical
`crates/mili-viz-proto/proto/mili_viz.proto`, and drops a generated
`_proto/__init__.py`. `grpc_tools` emits a bare top-level
`import mili_viz_pb2` in the `_grpc` module; the script rewrites that
one line to `from griz._proto import mili_viz_pb2` so `griz._proto` is
self-contained on `sys.path` without leaking a top-level module name.
The root `.gitignore` Python block cited a stale "Decision 33"
(that number is `phase-4-m5d.md`); corrected in passing to
"Decisions 36 & 53". `griz._proto` stays gitignored build output;
`griz._stubs()` raises an actionable error pointing at the script if
it was never run, and the M1 gate's autouse fixture regenerates it
(skip-on-absent when `grpcio-tools` is not installed).

## Decision 54 — `run_script` sends the whole file as one verbatim `Command{raw}`; no Python line-splitting

`scripting.md` says `session.run_script(path)` "streams the lines to
the server's existing command dispatcher." Decision 37's single-parser
invariant makes the literal reading (split lines in Python, send each)
wrong — that *is* a second Python-side parser. The server's `parse_raw`
(`crates/mili-viz-server/src/raw.rs`) already splits on `;`/newline and
skips blank lines and `#`/`//` comments. Resolution: `run_script`
reads the file and sends its **entire contents byte-verbatim** as a
single `Command{raw}`; the grizinit splitting/comment-skipping is the
server's, once. This is the M1 form of Layer-0 ≡ raw (Decision 37) and
is pinned always-on by `test_run_script_is_one_verbatim_raw` (a fake
stub asserts exactly one `Command`, `WhichOneof == "raw"`, `raw` ==
the file byte-for-byte) — no server needed.

## Decision 55 — the gate's connect leg spawns the real `mili-viz-server` TCP binary; the `load` assertion is corpus-independent

The acceptance gate needs a live server on an ephemeral TCP port. The
M6 `serve_tcp` is exposed by `mili-viz-server`'s `main` (`argv[1]`
bind, `127.0.0.1:0` → an OS-assigned port printed as
`tcp://127.0.0.1:<port>`). Resolution: the gate's connect/handshake/
Layer-0 leg uses a prebuilt `target/{release,debug}/mili-viz-server`
(or `cargo build -p mili-viz-server`), spawns it on `127.0.0.1:0`, and
parses the bound port from stdout — **skip-on-absent** (never failed)
when `cargo`/the binary is unavailable, exactly the CLAUDE.md corpus
skip. The `load <fixture>` assertion relies only on the server's
graceful never-error `load` (a non-openable root falls back to the
stub `LoadedState`, still `ok == true` — `phase-4-m2.md` Decision 12),
so it is corpus-independent; the real `serial/basic1/basic1.pltA`
fixture is used when present so the realistic path is exercised too.
No proto/server change — this only consumes the frozen M6 transport.

## M1 acceptance gate

A single gating test (`python/pygriz/tests/test_m1_connect.py`,
run by the `test-pygriz` job) against a `mili-viz-server` on an
ephemeral TCP port (`serve_tcp(":0")`, the M6 transport):

- [x] `pip install -e python/pygriz[dev]` succeeds; `import griz`
      works on CPython ≥ 3.11; stubs generate from the canonical
      proto with zero edits (`scripts/gen-pygriz-stubs.sh`,
      Decision 53; `test_import_and_proto_pinned`).
- [x] `griz.connect(host, port, token=...)` completes the `Hello`
      handshake; a matching `protocol_version` → `compatible == True`;
      a deliberately bumped client version → `compatible == False`
      with a non-empty `mismatch_detail` and a Python **warning, not
      an exception** (Decision 36 / Visit guarantee;
      `test_connect_handshake_and_layer0` +
      `test_handshake_mismatch_warns_not_raises`).
- [x] `session.command("load <fixture>; state 2; show sx")` returns
      `ok`, and `session.run_script(path)` streams a `grizinit`-style
      batch (comments/blank lines skipped) to the same dispatcher —
      both via `Command.raw`, no Python-side griz parser
      (Decisions 37 & 54).
- [x] The Phase 4 server acceptance suite + M2–M6 gating tests are
      **unchanged and green** (Phase 6 adds a client; it does not
      touch the frozen proto or the server —
      `cargo test --workspace --exclude mili-py` green).

## Out of scope for M1 (later Phase 6 milestones)

`attach()`/`launch()`/`list_sessions()` + the session/connection file
(M2); the Layer-1 object API + handles + Layer-0 ≡ Layer-1 test (M3);
`Subscribe`/`@s.on(...)` live sync (M4); `query`/`to_dataframe`/Arrow
Flight (M5); `render`/`save_animation`/`snapshot` (M6). None require a
proto change — the M1 contract already froze all of it.
