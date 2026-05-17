# `mili-viz` Phase 4 M6 — remote transport (gRPC + Arrow Flight over TCP) (buildable scope)

> Scope doc for Phase 4 Milestone 6, the final Phase 4 milestone,
> continuing [`phase-4-m5b.md`](phase-4-m5b.md). M1–M5 + the M5
> follow-up froze the wire contract and built every server-side
> producer behind it over an **in-process** `tokio::io::duplex`
> channel, with bulk geometry resolved through an **in-process
> geometry store** keyed by the frozen `GeometryRef.flight_ticket`
> ([`phase-4-m2.md`](phase-4-m2.md) Decision 10). M6 swaps that
> in-process transport for a **real gRPC + Arrow Flight transport on
> TCP** — without changing the frozen `mili_viz.proto`, the M2-frozen
> blob format, or the ticket bytes.
>
> Read [`status.md`](status.md) first, then `phase-4-m2.md`
> Decision 10 (the deferral this doc redeems) and `phase-4-m1.md`
> Decision 7 (the frozen-stub `UNIMPLEMENTED` discipline this doc
> reuses for the unused Flight RPCs). Decisions continue the log
> (M1: 1–9; M2: 10–12; M3: 13–15; M4: 16–18; M5: 19–21; M5b: 22–24;
> this milestone starts at **25**).

## Goal

Serve the **same frozen `mili_viz.proto`** plus a real Arrow Flight
`FlightService` over a TCP socket, so a client on a remote host (an
HPC login node) connects over the network exactly as the in-process
client connects today:

- `serve_tcp(addr)` binds a TCP listener (supports an ephemeral
  `:0`), co-serves `MiliVizServer` **and** `FlightServiceServer` on
  the one port via tonic's multi-service router, and returns the
  resolved `SocketAddr` + a server `JoinHandle`.
- The frozen `GeometryRef.flight_ticket` resolves through a real
  Arrow Flight `DoGet` over TCP: `DoGet(Ticket{ ticket })` streams
  the **byte-identical** M2/M3 `MVG1`/`MVG2` blob back. The ticket
  bytes, the `layout` string, and the encoded blob are unchanged
  across the swap — M6 is a transport swap, not a contract or format
  change, exactly as `phase-4-m2.md` Decision 10 promised.
- Every existing invariant holds over the new transport: the `Hello`
  handshake (match + reported mismatch, never a crash), Layer-0 ≡
  raw, subscription fan-out, the frozen-stub `UNIMPLEMENTED`
  (`AgentChat`/`Interrupt`/`CaptureFrame`), `show` totality, the
  `MVG1`/`MVG2` blob, and `ResultState.{min,max}` autoscale.
- The in-process transport (`spawn_in_process`) and the in-process
  store accessor (`fetch_geometry`) are **kept** as a
  test/embedding seam (`README.md` § "`mili-viz-client`" run mode 1
  — the default local-workstation in-process server — depends on
  it); M6 adds the remote path, it does not delete the local one.

Out of scope (unchanged from the prior docs' "Out of scope", and
explicitly not regressed here): the deferred derived sub-slices
(`surfstrain*` per-face Hex, the `*_alt` trig strains, nodal
time-derived families — `phase-4-m5b.md` Decision 22); the agent
loop / LLM backend (`phase-4-m1.md` Decision 6 — still
`UNIMPLEMENTED`); the offscreen renderer / `CaptureFrame` (Phase 5);
TLS / auth hardening beyond the existing `Hello` session-token check
(a deployment concern, not a transport-correctness concern; the
session-token path is unchanged and works over TCP as-is).

## Decisions (continuing the log)

### Decision 25 — M6 stands up the real Arrow Flight + gRPC TCP transport, redeeming `phase-4-m2.md` Decision 10; that decision's tonic-version premise is now factually false but the deferral was still correct, and the ticket / blob / layout stay byte-stable across the swap

`phase-4-m2.md` Decision 10 deferred the real Flight wire to M6 on
two stated grounds: (1) "the `arrow-flight` crate pins an older
`tonic` than the frozen M1 stack (`tonic` 0.14 / `prost` 0.14)" so
co-serving Flight would drag a second, incompatible `tonic` major
into the tree; and (2) `README.md` explicitly scopes
Flight-over-TCP to M6.

Ground (2) still holds and is the milestone we are now executing.
**Ground (1) is now factually outdated and is explicitly recorded
here as superseded:** `arrow-flight` 57.0.0+ (current: 58.3.0)
depends on `tonic ^0.14.1` / `prost ^0.14.1` — *exactly* the frozen
M1 stack (the workspace resolves `tonic` 0.14.6 / `prost` 0.14.3).
There is no longer any version conflict from a real Flight
transport on the frozen tonic-0.14 server. The M2 deferral was
nonetheless the right call at M2 (the in-process client never
needed the wire, and Flight-over-TCP is a named M6 deliverable, not
M2 scope) — this entry records the supersession so a cold reader is
not misled by Decision 10's now-stale rationale.

**Decision: M6 replaces the in-process `tokio::io::duplex` transport
with a real TCP transport that co-serves `MiliVizServer` and a real
Arrow Flight `FlightServiceServer`. The frozen
`GeometryRef.flight_ticket` (the `geom:{seq}` bytes assigned in
`phase-4-m2.md` Decision 10), the `layout` string (`MVG1:...` /
`MVG2:...`), and the encoded blob bytes are unchanged — the Flight
`DoGet` resolves the *same* in-process geometry store and streams
the *same* bytes. `spawn_in_process` and `VizService::fetch_geometry`
are kept verbatim as the in-process test/embedding seam (the
`README.md` default local run mode and every prior gating test
depend on them).** M6 is therefore a transport swap precisely as
Decision 10 promised: no proto change, no blob-format change, no
ticket change.

**Trade-off recorded.** Deleting the in-process path to "have one
transport" was rejected: `README.md` § "`mili-viz-client`"
enumerates *two* run modes (in-process for local workstation use,
remote for HPC), and every M1–M5b gating test drives
`spawn_in_process` + `fetch_geometry`; removing them would force a
rewrite of the entire frozen test suite for zero behavioral gain
and would delete the documented default local mode. Keeping both
costs one extra small `serve_tcp` entry point and a Flight server
adapter over the *same* store — a localized addition, exactly the
"small, localized" cost Decision 10 predicted.

### Decision 26 — the Flight transport is the **canonical Apache Arrow `Flight.proto`** compiled through the existing protoc-free `protox` path (zero change to the frozen `mili_viz.proto`); only `DoGet` is implemented, every other Flight RPC returns `UNIMPLEMENTED`; `DoGet` streams the verbatim opaque blob in `FlightData.data_body`

The frozen `mili_viz.proto` defines only the `MiliViz` service and
deliberately carries **no** bulk bytes — `GeometryRef` is a pointer
(`flight_ticket` + `layout` + counts) and `README.md` names Arrow
Flight as the bulk transport. Arrow Flight is a *separate,
well-known, standardized* gRPC service (`arrow.flight.protocol.
FlightService`) with its own canonical IDL; serving it is **additive
and does not touch `mili_viz.proto`** (zero diff to the frozen
contract — confirmed: the M1 Δ1–Δ9 surface is unchanged). Two ways
to obtain that service were considered; this is an architecturally
significant choice and is pinned with its trade-off:

**Decision: vendor the canonical Apache Arrow `Flight.proto`
(Apache-2.0, license header retained) into
`crates/mili-viz-proto/proto/Flight.proto` and compile it alongside
`mili_viz.proto` through the crate's **existing protoc-free `protox`
+ `tonic-prost-build` path** — no protoc, no new codegen mechanism.
This produces a real, wire-interoperable `arrow.flight.protocol.
FlightService` server **and** client (a `pyarrow` Flight client or
the `arrow-flight` crate interoperate on the wire — the wire format
is defined by the `.proto`, which is byte-for-byte the canonical
one). `mili-viz-server` implements **only `DoGet`**; every other
Flight RPC (`Handshake`/`ListFlights`/`GetFlightInfo`/
`PollFlightInfo`/`GetSchema`/`DoPut`/`DoExchange`/`DoAction`/
`ListActions`) returns `Status::unimplemented` naming the geometry
path — the *same frozen-stub discipline* `phase-4-m1.md` Decision 7
applies to the unused agent RPCs. `DoGet(Ticket{ ticket })` looks
the ticket up in the in-process geometry store and streams the
**verbatim** `MVG1`/`MVG2` blob as the `data_body` of `FlightData`
messages (no Arrow schema, no `RecordBatch` — `phase-4-m2.md`
Decision 11's blob is an opaque self-describing little-endian buffer
by design); an unknown ticket is `Status::not_found`. The client
concatenates `data_body` across the stream, so single-message vs.
chunked framing is a transparent server-side detail.**

**Trade-off recorded.** Depending on the real `arrow-flight` crate
(58.3.0, now tonic-0.14-aligned per Decision 25) was the obvious
alternative and was rejected: it pulls ~12 non-optional `arrow-*`
crates (`arrow-array`/`-buffer`/`-cast`/`-ipc`/`-schema`/…) whose
entire purpose is building/encoding `RecordBatch`es — and we
*never* build one (Decision 11 froze the payload as an opaque
buffer). That is a large build-weight and dependency-surface
increase to move `bytes` in and `bytes` out, and it contradicts the
crate's deliberate protoc-free / lean-codegen discipline
(`crates/mili-viz-proto/build.rs` already refuses protoc and
compiles via `protox` precisely so the parity/web runners need no
extra toolchain). Vendoring the canonical IDL gives a *real*,
standards-conformant, wire-interoperable Flight transport with zero
new heavy deps and on the codegen path the crate already uses. The
residual cost — a vendored ~680-line standard `.proto` that tracks
upstream — is bounded and explicit (the file carries its Apache-2.0
header and is the stable, rarely-changing canonical Flight IDL); the
alternative (modifying the **frozen** `mili_viz.proto` to add a
bespoke geometry-stream RPC) was rejected outright as a frozen-proto
change for a problem Arrow Flight already solves on the wire.

### Decision 27 — bind/port handling: `serve_tcp(SocketAddr)` binds a `TcpListener` (ephemeral `:0` supported), co-serves both services on the one port via tonic's multi-service router, returns the resolved `SocketAddr` + a `JoinHandle`; the M6 gating test binds a real ephemeral `127.0.0.1:0` and drives **both** services over real `tonic::transport::Channel`s end-to-end

A remote deployment needs an explicit bind address; a hermetic test
needs an OS-assigned free port with no race. Both are served by
binding the `std`/`tokio` `TcpListener` *first* (so `:0` resolves to
a concrete port we can read back via `local_addr()`), then handing
its accept stream to tonic.

**Decision: add `mili_viz_server::serve_tcp(svc, addr) -> (SocketAddr,
JoinHandle<()>)` which (a) binds a `tokio::net::TcpListener` on
`addr`, (b) reads back `local_addr()` (the concrete port even when
`addr` requested `:0`), (c) spawns a tonic `Server` with **both**
`MiliVizServer::new(svc.clone())` *and*
`FlightServiceServer::new(svc.flight_service())` added to the same
router, served over the listener's accept stream, and (d) returns
the resolved `SocketAddr` + the server `JoinHandle`. Add a
`spawn_tcp(svc)` test/embedding helper (mirroring the existing
`spawn_in_process`) that calls `serve_tcp` on `127.0.0.1:0` and
returns connected real `MiliVizClient<Channel>` **and**
`FlightServiceClient<Channel>` over TCP plus the addr + handle. The
binary entry point (`main.rs`) serves over TCP on a bind address
taken from `argv[1]` (default `127.0.0.1:50051`) instead of the M1
"in-process only" stub message.** Both services share one
`Arc<Inner>` (hence one geometry store, one session, one broadcast
bus) — the Flight server is a thin adapter over the existing store,
not a second source of truth.

**Trade-off recorded.** Serving Flight on a *second* port / second
`Server` was rejected: tonic multiplexes services on one HTTP/2
listener by design, one port is the standard Flight deployment
shape, and a second port doubles the firewall/connection surface on
an HPC login node for no benefit. Letting tonic bind the address
itself (`Server::serve(addr)`) instead of pre-binding a
`TcpListener` was rejected for the test path only: it cannot return
an OS-assigned ephemeral port without a TOCTOU race, and a fixed
test port flakes under parallel `cargo test`. Pre-binding the
listener is the standard hermetic-test pattern and costs one extra
line in the non-test path.

## M6 acceptance gate

- [x] `crates/mili-viz-proto` compiles the **canonical vendored
      `Flight.proto`** through the existing `protox` +
      `tonic-prost-build` path (no protoc); the frozen
      `proto/mili_viz.proto` is **byte-for-byte unchanged** (zero
      diff — the M1 Δ1–Δ9 surface is frozen); the crate exposes the
      generated `arrow.flight.protocol` client + server.
- [x] `mili_viz_server::serve_tcp(svc, addr)` binds a `TcpListener`
      (ephemeral `:0` resolves to a concrete port via `local_addr`)
      and co-serves `MiliVizServer` + `FlightServiceServer` on the
      one port; returns the resolved `SocketAddr` + a `JoinHandle`.
- [x] Over a **real TCP `tonic::transport::Channel`**: `Hello`
      negotiates (compatible match + reported mismatch, never a
      crash); `Execute` of a typed command **and** the equivalent
      `raw` line produce an identical `StateDelta` (Layer-0 ≡ raw
      over the wire); a second subscriber sees the same broadcast
      ordered with `seq == CommandReply.delta_seq` and the origin
      tagged (fan-out over the wire); `AgentChat`/`Interrupt`/
      `CaptureFrame` return `UNIMPLEMENTED` over the wire.
- [x] `show` after `load` over TCP yields `ResultState.geometry =
      Some(GeometryRef)`; the `flight_ticket` is the **same frozen
      `geom:{seq}` byte form** and `layout` is the **same**
      `MVG1:...` / `MVG2:...` string as the in-process path.
- [x] A real Arrow Flight `DoGet(Ticket{ flight_ticket })` over TCP
      streams a blob that is **byte-identical** to
      `VizService::fetch_geometry(ticket)` for the same scenario; it
      decodes per `phase-4-m2.md` Decision 11 (`MVG1` magic, dims,
      counts, in-range indices) and `phase-4-m3.md` (`MVG2` trailing
      `scalar_f32`); an unknown ticket is `Status::not_found`; a
      non-`DoGet` Flight RPC is `Status::unimplemented`.
- [x] `show <result>` over TCP carries the `MVG2` scalar and
      `ResultState.{min,max}` bracketing the finite samples (the M3
      autoscale invariant holds over the wire).
- [x] `spawn_in_process` and `VizService::fetch_geometry` are
      **kept and unchanged**; all six M1 `acceptance.rs` +
      `m2_geometry.rs` + `m3_primal.rs` + `m4_visibility.rs` +
      `m5_derived.rs` + `m5b_principal.rs` tests still pass
      **unchanged** (no proto/blob/ticket change; the in-process
      seam is byte-stable).
- [x] New test follows the CLAUDE.md skip-on-absent discipline
      (early `return` + `eprintln!` when the `serial/basic1` corpus
      fixture is absent).
      → `crates/mili-viz-server/tests/m6_transport.rs`
      `remote_transport_grpc_and_flight_over_tcp`
- [x] `cargo fmt --all --check` and `cargo clippy --workspace
      --all-targets -- -D warnings` both pass.
- [x] `status.md` updated (TL;DR, Phase 4 list M6 box flipped with
      the gating test named, "what is decided" table with a
      `phase-4-m6.md` row, "immediate next steps"); `README.md`
      open-questions table checked for impact (no proto change; the
      M6 transport is the README's named deliverable, no open
      question opened or closed).

## Out of scope for M6 (which milestone / doc owns it)

- The deferred derived sub-slice (`surfstrain*` / `*_alt` / nodal
  time-derived) — a later follow-up (`phase-4-m5b.md` Decision 22).
- The agent loop / LLM backend — still `UNIMPLEMENTED`
  (`phase-4-m1.md` Decision 6); the wire contract was frozen at M1.
- Offscreen renderer / `CaptureFrame` — Phase 5.
- TLS / auth hardening beyond the existing `Hello` session-token
  check — a deployment concern; the session-token path is transport
  agnostic and already works over TCP unchanged.

## Decision log (this doc)

| # | Title | Resolves |
|---|---|---|
| 25 | Real Arrow Flight + gRPC TCP transport redeems `phase-4-m2.md` Decision 10; that decision's tonic-version premise is now factually false (arrow-flight 57+ = tonic 0.14) but the deferral was still correct; ticket/blob/layout byte-stable; in-process `spawn_in_process`/`fetch_geometry` seam kept | M6 transport scope |
| 26 | Flight via the canonical vendored `Flight.proto` on the existing protoc-free `protox` path (zero change to frozen `mili_viz.proto`); only `DoGet` implemented, other Flight RPCs `UNIMPLEMENTED` (frozen-stub discipline); `DoGet` streams the verbatim opaque blob in `FlightData.data_body`; the heavy `arrow-flight` crate rejected for dependency surface | M6 Flight surface |
| 27 | `serve_tcp(addr)` pre-binds a `TcpListener` (ephemeral `:0`), co-serves both services on one port via tonic's router, returns resolved `SocketAddr` + `JoinHandle`; `spawn_tcp` test helper; the gating test binds a real `127.0.0.1:0` and drives both services over real `Channel`s | M6 bind/port + test |
