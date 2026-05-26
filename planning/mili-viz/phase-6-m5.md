# Phase 6 M5 — pygriz `Query` payoff (`db.query` → `pd.DataFrame`)

> **Status: ✅ LANDED.** Full live status in
> [`status.md`](status.md); this file is the M5 decisions log + the
> rationale anyone reading future history needs to make sense of the
> shape choices.

## Scope

The pygriz half of the `Query` arm whose server side already landed
(`wireframe-parity.md` "What's still left" #4 — the server now
dispatches to `mili_rs::Database::query_full` for primal svars, and
the Rust client wraps it via `Session::query`; both shipped in
`crates/mili-viz-server/tests/query_rpc.rs` (5 server-side cases) +
`crates/mili-viz-client/tests/plot_element_series.rs` (7 client-side
cases)).

A script must now be able to write:

```python
import griz

with griz.launch() as s:
    db = s.open("d3samp6.plt")
    df = db.query("sand", "brick", states=[1, 2, 3]).to_dataframe()
    # df is a pandas DataFrame: index = states, columns = labels,
    # cells = the scalar values (or per-cell ndarray for multi-comp).
```

The shape is deliberately the same one `mili.utils.
query_data_to_dataframe` produces (index=states, columns=labels;
scalar → flat, multi-component → per-cell ndarray via the
`DataFrame.from_records` arm) so a script that already speaks the
Python oracle's DataFrame layout drops in unchanged. The
`scripting.md` framing: "viz and analysis mix freely — the main win
over legacy griz."

## What landed

- `Session.query(result, class_name, *, labels=None, states=None,
  component="") → QueryResult` — builds the typed `QueryRequest`
  directly (no griz string formatted; M3's "no second emitter"
  invariant generalises from `Command` to `Query`), dispatches over
  the in-band `Query` RPC, surfaces `ok=false` as a typed
  `QueryError` carrying the verbatim server message, and hands the
  inline carrier to `QueryResult` for numpy/pandas conversion. Empty
  `labels`/`states` pass through verbatim so the server fills in (all
  labels / current cursor — `proto/mili_viz.proto:379` contract).
- `Database.query(...)` — thin alias for `Session.query` so the
  `scripting.md` sketch (`db.query(...)`) and the Rust-side
  `Session::query` parity shape are spelled the same way. One
  dispatcher, no parallel path (mirrors M3's one-`Command`
  dispatcher).
- `QueryResult` (frozen dataclass) — preserves the proto's row-major
  `[len(states) × len(labels) × components]` payload verbatim
  (`.labels`/`.states`/`.values`/`.components` + the request context
  `.result`/`.class_name`/`.component`), plus convenience views:
  - `.values_3d` → `numpy.ndarray` reshaped to
    `(len(states), len(labels), components)` (the milox
    `query_data_to_dataframe` input shape — preserved so callers can
    reshape on numpy when the pandas default doesn't fit).
  - `.to_dataframe()` → `pandas.DataFrame` matching
    `mili.utils.query_data_to_dataframe`: index = states, columns =
    labels. `components == 1` lays flat; `components > 1` uses the
    `from_records` arm with a per-cell 1-D `ndarray` of length
    `components`.
- `QueryError(RuntimeError)` — single typed exception class. The
  server returns `ok=false` with a typed string (not a transport
  `Status`), so the client maps that surface 1:1; callers can branch
  on the verbatim message (`"no run loaded"`, `"not yet supported"`,
  `"out of range"`, ...).
- The proto's `oneof data { InlineTable inline; bytes flight_ticket
  }` reserves the Flight ticket for the large-result path. pygriz M5
  ships only the inline arm: a `flight_ticket` reply raises a clear
  `QueryError` ("server returned a Flight ticket, not implemented in
  pygriz M5 yet"). Wiring the Flight client is M6 (`render`/`snapshot`
  already need the same Flight transport for the framebuffer carrier,
  so the two share that follow-up).
- Zero `crates/` edit. Zero `.proto` change. The shape was already
  designed by the server PR; M5 is the Python ergonomic wrapper over
  the frozen wire.

## Decisions

### Decision 67 — typed `QueryRequest`, typed `QueryError`, single dispatcher.

`Session.query` builds the proto `QueryRequest` directly (never a
griz string, never via `Command.raw`). The server's `ok=false` +
typed `error` shape (not a transport `Status`) maps to a single
`QueryError(RuntimeError)` exception class carrying the verbatim
message; callers can branch on it the same way the Rust client
branches on `QueryReply.ok` + `reply.error`. `Database.query` is a
1-line alias so there is exactly one `Query` dispatcher — same
discipline M3 pinned for `Command` (one parser server-side; one
typed-emitter client-side).

**Why this and not e.g. raising on `flight_ticket` silently:** the
server can today return only `inline`; the unimplemented `flight_ticket`
arm of the proto's `oneof` is a real failure mode for callers, not a
silently-dropped payload. `QueryError` makes the missing follow-up
visible (`"server returned a Flight ticket, not implemented in
pygriz M5 yet"`); they get one consistent exception type to catch.

### Decision 68 — inline carrier only in M5; defer the Flight ticket arm.

The proto pin: `oneof data { InlineTable inline; bytes
flight_ticket }`. The server returns only `inline` today (the
`query_full` outputs go through `pb::query_reply::Data::Inline`,
verified by `crates/mili-viz-server/tests/query_rpc.rs`). Wiring
the pygriz Flight reader requires an `arrow`/`pyarrow` dependency
+ a Flight client; that lands jointly with M6's
`render`/`snapshot`/`save_animation` (which need the same Flight
plumbing for the framebuffer carrier).

**Why now:** decoupling the Flight client from this milestone keeps
M5's dep tree clean (numpy + pandas only, both already declared in
`pygriz/pyproject.toml`); a noisy `pyarrow` add here would prejudge
M6's transport choice.

### Decision 69 — `to_dataframe()` mirrors `mili.utils.query_data_to_dataframe`.

The DataFrame layout: index = `states`, columns = `labels`, scalar
values flat, multi-component values per-cell `ndarray` via
`from_records`. This is the *exact* shape mili-python's
`query_data_to_dataframe` produces (the function lives at
`reference/mili-python/src/mili/utils.py:68-95`), so a script that
already speaks `milox`/`mili`'s DataFrame layout drops in unchanged.

**Why match milox/mili-python and not invent a new shape:** the
`scripting.md` framing — "same types as the milox query layer so
viz and analysis mix freely" — is M5's whole point. Inventing a
third layout would defeat it.

**Why not return the DataFrame directly from `query()`:** the proto
ships labels/states/values/components as four equal-class fields; a
`QueryResult` wrapper exposes them without conjuring a DataFrame on
every call. `values_3d` gives a numpy view for the analytic path;
`to_dataframe()` is one method call when the milox shape fits.

## M5 acceptance gate

`python/pygriz/tests/test_m5_query.py` (run by the existing
`test-pygriz` harness; same skip-on-absent contract as M1/M2/M3):

**Always-on pure logic** (7 cases):

1. `Session.query` lowers to a typed `QueryRequest` with every
   field (`result`/`class_name`/`labels`/`states`/`component`) set
   verbatim. The returned `QueryResult` carries the request
   context.
2. Empty `labels`/`states` pass through as empty repeated fields
   (the server fills them in; the client must not invent a
   default).
3. Server `ok=false` raises `QueryError` with the verbatim
   `reply.error`.
4. The proto's `flight_ticket` arm raises a clear `QueryError`
   pointing at the deferred Arrow-Flight follow-up.
5. `Database.query` and `Session.query` send the *identical*
   `QueryRequest` (alias contract — exactly one Query goes out).
6. `to_dataframe()` for `components == 1` produces a DataFrame
   indexed by states, columned by labels, with the row-major
   `[state][label]` interpretation pinned on a spot value.
7. `to_dataframe()` for `components > 1` uses the `from_records`
   arm: each cell is a 1-D `ndarray` of length `components`
   (`values_3d.shape == (states, labels, components)`).

**Skip-on-absent** (2 cases, gated on `serial/basic1.pltA`):

8. `db.query("sand", "brick", states=[1, 2, 3])` against a real
   launched server returns finite values; the DataFrame is indexed
   by exactly those three states and columned by every brick label
   (the M1 stub returned an empty vec — this leg pins that we are
   now talking to the real `mili-rs` arm landed in
   `crates/mili-viz-server/tests/query_rpc.rs`).
9. `s.query("pressure", "brick", states=[1])` raises a
   `QueryError` containing the server's `"not yet supported"`
   message (the geometry-path derived routing is the documented
   forward path — `wireframe-parity.md` #4 follow-up).

All prior gates (`test_m1_connect.py`, `test_m2_attach.py`,
`test_m3_layer1.py`) and all `crates/` tests unchanged and green.

## What this unblocks

- `wireframe-parity.md` #4 (text-input variant) is **fully closed
  on the wire**: the egui Plot tab, the Rust client wrapper, the
  server's primal arm, **and** the pygriz wrapper are all green
  against the same `Query` RPC. The picking-driven variant (#6)
  remains design-first and independent.
- The Phase 6 narrative becomes: M1 connect/handshake → M2
  `attach`/`launch` → M3 typed Layer-1 → **M5 data back into
  Python**. M4 (live `Subscribe` → `@s.on(...)`) and M6 (`render`/
  `snapshot` + Arrow Flight for the large arm) remain.
