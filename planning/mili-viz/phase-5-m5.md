## Phase 5 M5 — remote mode (client → real `mili-viz-server` over gRPC + Arrow Flight TCP)

> Status: 🟢 in progress (drafted 2026-05-23). Decisions 90–93 below;
> the live tracker is [`status.md`](status.md), this file holds the
> M5-specific scope/contract.

## What lands

The Phase 4 M6 transport — `serve_tcp` / `spawn_tcp` co-serving
`MiliViz` + `arrow.flight.protocol.FlightService` on one TCP port —
has been live since Phase 4. Phase 5 M1–M4 only consumed the
**in-process** seam (`spawn_in_process` + `VizService::fetch_geometry`).
M5 wires the client to the **real wire**:

- `Session::connect_tcp(endpoint: &str, root)` constructs a session
  against a running `mili-viz-server` (one tonic `Channel` cloned for
  both `MiliVizClient` and `FlightServiceClient`).
- `Session::attach(id, root)` reuses the same
  `~/.griz/sessions/<id>.json` resolver pygriz already wrote (Phase 6
  M2, [`phase-6-m2.md`](phase-6-m2.md) Decision 56). A `None` `id`
  picks the **newest live** session (pid alive, `kill(pid, 0)`); an
  explicit `id` loads `<id>.json` directly.
- `Session::resolve_geometry` / `Session::fetch_catalog` learn a
  remote arm that streams the blob via a real Flight `DoGet` and
  concatenates `FlightData.data_body` (the M6 test pattern); in-process
  arm unchanged.
- CLI gains `-r <host:port>` (also `--remote`) and `--attach [<id>]`.
  In-process stays the default.
- HPC-latency tuning on the tonic channel: `tcp_nodelay`, TCP/HTTP/2
  keep-alives, explicit `connect_timeout`.

The frozen `mili_viz.proto` is **untouched**. The server-side `serve_tcp`
+ Flight `DoGet` from M6 is untouched. Every prior Phase 4/5 gating
test stays byte-stable (verified at end of milestone with a full
`cargo test --workspace --exclude mili-py`).

## Decisions

### Decision 90 — `Transport` enum carried by `Session`; resolvers become `async`

`Session` currently stores `svc: VizService` for the direct
`fetch_geometry`/`fetch_catalog` calls. M5 replaces that with an
internal `Transport`:

```rust
enum Transport {
    InProcess(VizService, JoinHandle<()>),
    Remote(FlightServiceClient<Channel>),
}
```

- `resolve_geometry(&self, gref)` and `fetch_catalog(&self)` become
  `async`. In-process arm delegates to `VizService::fetch_geometry`
  exactly as before; remote arm calls `flight.do_get(...)` and
  concatenates `data_body` (Decision 92).
- `app.rs` callsites use the existing `rt.block_on(...)` pattern, the
  same way `execute` is already driven from the synchronous winit
  redraw loop. No new runtime; no new thread.

This keeps the in-process **byte-stable** for every existing test
(M1–M4 + every MVP-polish sub-gate); the only behavioural change is
the new remote arm.

### Decision 91 — CLI surface mirrors pygriz `connect`/`attach`

Two new flags on top of the portable griz subset
([`phase-4-m1.md`](phase-4-m1.md) Decision 4 / Phase 5 M4
Decision 63):

- `-r <host:port>` (long: `--remote`) — direct TCP endpoint.
- `--attach [<id>]` — resolve through the `~/.griz/sessions/` JSON
  files the binary's `main` writes (Phase 6 M2 Decision 56). Bare
  `--attach` picks the newest **live** session (pid liveness via
  `kill(pid, 0)` on Unix; absent process detection is best-effort).

Mutual exclusion: at most one of `{-r host:port, --attach}` may be
given; either implies remote mode. With neither, M4's in-process
default is preserved verbatim (the windowed `run()` spawns a server
in-process as before).

`$GRIZ_SESSIONS_DIR` honoured so the gate is hermetic (same env var
pygriz `_sessions_dir` reads, same server `sessions_dir` writes).

### Decision 92 — Flight blob fetch helper

The Flight `DoGet` response is a server stream of `FlightData` —
`data_body` is the raw bytes M6 emits (zero schema; the blob is opaque
by design). The client must concatenate across chunks (HTTP/2 framing
is not promised one-message-per-blob). One helper in
`crate::session::flight_get`:

```rust
async fn flight_get(flight: &mut FlightServiceClient<Channel>,
                    ticket: &[u8]) -> Result<Vec<u8>, BoxErr> {
    let mut stream = flight
        .do_get(Request::new(fpb::Ticket { ticket: ticket.to_vec() }))
        .await?
        .into_inner();
    let mut out = Vec::new();
    while let Some(fd) = stream.message().await? {
        out.extend_from_slice(&fd.data_body);
    }
    Ok(out)
}
```

This mirrors the `m6_transport.rs` test's `flight_get` exactly; the
acceptance gate is "same blob, byte-identical, across in-process vs
remote".

### Decision 93 — HPC-latency channel tuning

The tonic `Endpoint` defaults are fine for LAN but not for an HPC
login node where idle subscriptions may sit for minutes and clusters
have stateful NAT/firewalls. M5 sets:

- `.tcp_nodelay(true)` — `Execute` and `Hello` are sub-MTU, Nagle
  buys nothing and adds 40 ms on each round-trip.
- `.tcp_keepalive(Some(Duration::from_secs(30)))` — TCP-level keep so
  the OS prunes the connection promptly if the cluster drops it.
- `.http2_keep_alive_interval(Duration::from_secs(20))` and
  `.http2_keep_alive_timeout(Duration::from_secs(10))` — HTTP/2 PING
  keep-alive on the `Subscribe` stream (the long-lived RPC).
- `.connect_timeout(Duration::from_secs(10))` — explicit instead of
  tonic's default ("wait forever"). 10 s is enough for an in-cluster
  hop; a misconfigured `-r host:port` fails loud.

One `Channel` is built and **cloned** for the `MiliVizClient` and the
`FlightServiceClient` — tonic Channels are cheap to clone, share the
underlying HTTP/2 connection (so a single TCP socket carries both
services exactly as `serve_tcp` muxes them).

## M5 acceptance gate

`crates/mili-viz-client/tests/m5_remote_mode.rs`:

1. **Always-on** — `cli::parse_args` extends correctly:
   - `-r 1.2.3.4:50051` parses to `Remote("http://1.2.3.4:50051")`.
   - `--attach` parses to `Attach(None)`.
   - `--attach abc123` parses to `Attach(Some("abc123"))`.
   - `-r foo --attach` errors (mutually exclusive).
   - `-r foo --attach bar` errors (same).
   - bare default (no `-r`/`--attach`) is `InProcess` (M4 byte-stable).
2. **Always-on** — `Session::attach` resolver:
   - empty `$GRIZ_SESSIONS_DIR` errors with the expected message.
   - explicit `id` on a missing file errors with the expected message.
   - a fabricated session-file list with a synthetic newest-live entry
     resolves through `attach()` to a callable `connect_tcp` URL
     (verified by URL string; the connect itself is the skip-on-absent
     leg).
3. **Skip-on-absent** — spawn a real `mili-viz-server` over TCP via
   `mili_viz_server::spawn_tcp(...)`, drive a client `Session::
   connect_tcp` end-to-end against `serial/basic1`, then:
   - `load` + `show ""` over the wire returns a `GeometryRef` whose
     ticket starts with `b"geom:"`.
   - `Session::resolve_geometry` (remote arm — Flight `DoGet`) decodes
     the same `Mesh` as the in-process `VizService::fetch_geometry`
     (vertex/index counts identical; first few floats bit-identical).
   - `Session::fetch_catalog` (remote arm — Flight `DoGet` against
     `CATALOG_TICKET`) decodes a non-empty primal+derived catalog.
4. **Skip-on-absent** — `attach()` round-trip: write a session file
   under `$GRIZ_SESSIONS_DIR` (with the spawned server's host/port /
   pid), call `Session::attach(None, root)`, assert the same load +
   show + decode works.

Every prior gating test in `crates/mili-viz-client/tests/` and
`crates/mili-viz-server/tests/` stays unchanged and green
(`cargo test --workspace --exclude mili-py`).

## Out of scope

- The **agent** chat surface stays `UNIMPLEMENTED` per M1 Decision 7;
  agent integration polish is Phase 5 M6
  ([`client.md`](client.md) § "Phasing").
- TLS / mTLS on the wire — deferred. Today's `serve_tcp` is plaintext
  HTTP/2; a deployment that needs TLS terminates it at a fronting
  proxy. When TLS lands it is an additive Endpoint builder change with
  no contract impact.
- A `--token <hex>` flag — the server's session-file token field is
  written but unenforced ([`phase-6-m2.md`](phase-6-m2.md)
  Decision 56). The client passes through whatever token `attach()`
  reads from the JSON, but does not prompt for one.
