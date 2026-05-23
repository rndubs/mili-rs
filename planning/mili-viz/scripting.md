# `mili-viz` — Python scripting interface

> **Status: ✅ RESOLVED design doc.** This is the original
> architecture/rationale for `pygriz`. Implementation landed across
> Phase 6 M1/M2/M3 — see [`phase-6-m1.md`](phase-6-m1.md),
> [`phase-6-m2.md`](phase-6-m2.md), [`phase-6-m3.md`](phase-6-m3.md),
> and the live status in [`status.md`](status.md). Phase 6 M4
> (live sync), M5 (query/Arrow Flight), M6 (output/snapshot) are
> the remaining work.

A pip-installable Python package that drives a `mili-viz` session from
**any** interpreter — VS Code, a venv, a notebook — without the
Visit-style bundled-interpreter problem.

## The Visit problem, and why we don't have it

Visit ships its own Python interpreter because its `cli`/`pyside` API
is **linked in-process** with the C++ viewer: the Python objects are
SWIG wrappers over live C++ state, so the interpreter ABI is part of
the build. That is what makes it painful to mix Visit scripting with
an external venv.

The `mili-viz` split already removes this by construction. The server
owns all session and scene state and speaks the `mili-viz-proto`
RPC; the `egui` client is *a* client, not the owner. The Python
library is simply **a second client of the same RPC service**:

- The package is **pure-Python** — generated gRPC stubs plus a thin
  ergonomic layer. A universal wheel, no `maturin`/pyo3, no ABI
  coupling. Works in any CPython ≥ 3.11, conda, uv, notebooks.
- Engine/API compatibility is a **versioned wire contract**, not
  co-compilation. The handshake exchanges a protocol version +
  capability flags; a pip-upgraded client warns on mismatch instead
  of segfaulting. This buys Visit's "API matches the engine"
  guarantee without Visit's interpreter lock-in.

Do **not** add a second scripting mechanism. The Python layer lowers
to the exact proto the `egui` client emits.

> Note: `milox`'s existing `grizinterface.py` is the *opposite*
> direction — the legacy C griz embedding Python to *read* data
> (the in-process pattern we are moving away from). It is unrelated
> to this scripting client.

## Decisions (resolved 2026-05-17)

1. **Camera/view is server-authoritative.** Every connected client
   (the `egui` window, a script) is a peer. Mutations go to the
   server; the server broadcasts `StateDelta` events to *all*
   clients. A script's `view.rotate(...)` visibly moves an open GUI
   window — the live-driving experience users expect from Visit.
   Interactive drag in the GUI is client-side *prediction only*
   (predict locally for responsiveness, send the command, reconcile
   against the authoritative echo); it is not a second state owner.
   This supersedes the README's original "local view manipulation
   without round-tripping" framing — see README fix-ups.

2. **Optimize first for interactive use** (VS Code + a live GUI
   window). Prioritize the session-file/`attach()` path and live
   `StateDelta` sync. Headless/offscreen rendering and Arrow-Flight
   tuning come later, but the proto handshake must not bake in
   localhost-only assumptions so they drop in without a redesign.

## Protocol impact (`mili-viz-proto`, expands Phase 4 M1)

The command *vocabulary* is unchanged from the README (it is still
the griz command set). What is added:

- A **subscription RPC** + server→client streaming `StateDelta`
  messages (loaded run, state index, selection, active results,
  isosurfaces, **camera**). Required so multiple clients stay in
  sync.
- A **handshake** carrying `protocol_version` and capability flags.
- A **session/connection file** written by the server on startup:
  `~/.griz/sessions/<id>.json` with `host`, `port`, `token`,
  `protocol_version`, `pid`, and the loaded `db`. This is the
  Jupyter-connection-file pattern and is how a script attaches to a
  running GUI session.

This is a larger surface than "server accepts strings; client emits
strings," so it belongs in M1 explicitly rather than being assumed.

The concrete draft lives at `proto/mili_viz.proto` — a design
artifact, not a built crate yet. It moves into
`crates/mili-viz-proto/proto/` with a `tonic` `build.rs` when
Phase 4 M1 starts. Layer-1 API calls lower to the typed `Command`
variants there; `session.command(...)` / `run_script(...)` use the
`Command.raw` escape hatch, and the two MUST stay equivalent
(integration test).

## Connection model

Three modes; interactive `attach()` is the priority.

1. **Attach (priority):** server writes the session file on startup;
   `griz.attach()` picks the newest local session,
   `griz.attach(id=...)` / `attach(host, port, token=...)` is
   explicit. Open the GUI, attach a VS Code script to it.
2. **Launch (local default later):** `griz.launch()` spawns
   `mili-viz-server` (+ optional GUI) on a free port — like
   `visit -cli`.
3. **Remote:** `griz.connect(host, port, token=...)` over gRPC, bulk
   geometry on Arrow Flight, for the HPC case.

`grizinit`-style batch files keep working: `session.run_script(path)`
streams the lines to the server's existing command dispatcher.

## API sketch

Two layers. **Layer 0** is a literal 1:1 binding of the griz command
vocabulary (migration aid + power-user command line). **Layer 1** is
the object API people should write. An integration test must assert
Layer 0 ≡ raw command stream so the migration aid cannot drift.

```python
import griz

# --- connect / session lifecycle -----------------------------------
s = griz.attach()                      # newest local session (priority path)
s = griz.attach(id="ab12cd")
s = griz.connect("login01", 50051, token=...)   # remote
s = griz.launch(gui=True)              # spawn server (+ GUI)
griz.list_sessions()                   # -> [SessionInfo(id, pid, host, port, db)]
with griz.launch() as s: ...           # context-managed close

# --- data / scene (server-authoritative) ---------------------------
db = s.open("d3samp6.plt")
s.state = 10                           # property -> 'state 10'
s.next(); s.prev(); s.first(); s.last()
s.select("brick", "1-100"); s.selection.clear("brick")
r = s.show("sx")                       # Result handle
r = s.show("stress", component="eff")  # derived (server, rayon)
r.range                                # (min, max)
s.colormap("cool"); s.legend.limits = (0, 5e4)
iso = s.isosurface("sx", levels=[1e4, 2e4]); iso.remove()
con = s.contour("sx")
s.materials.disable(3); s.materials.enable("brick", mat=2)
s.cutplane(origin=(0,0,0), normal=(1,0,0))

# --- view (server state; GUI mirrors it live) ----------------------
s.view.rotate(x=30, y=15); s.view.translate(0.1, 0, 0); s.view.zoom(1.5)
s.view.set(azimuth=45, elevation=20, distance=3.0)
s.view.reset(); s.view.save("v1"); s.view.restore("v1")

# --- output --------------------------------------------------------
s.render("frame.png", width=1920, height=1080)   # offscreen-capable
s.save_animation("run.mp4", states=range(0, 100, 2))
img = s.snapshot()                     # -> RGBA ndarray (notebooks)

# --- query: the real payoff (data back into Python) ----------------
vals = db.query("sx", "brick", labels=[1, 2, 3], states=[10, 20])
df = s.current_result.to_dataframe()   # same types as the milox query layer

# --- escape hatches ------------------------------------------------
s.command("show sx; state 10; rot x 30")   # Layer 0 raw stream
s.run_script("legacy_grizinit")            # verbatim batch migration

# --- live sync -----------------------------------------------------
@s.on("state_changed")
def _(ev): ...                             # fed by server StateDelta
```

Conventions: properties for state (`s.state = 10`), methods for
actions (`s.next()`); handles not stringly-typed everything
(`show()` → `Result`, `isosurface()` → `Isosurface`); `query()`
returns the same numpy/pandas types as the `milox` query layer so
viz and analysis mix freely — the main win over legacy griz.
