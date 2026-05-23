# Phase 4 M6 — landed (remote transport: gRPC + Arrow Flight over TCP)

> **Status: ✅ COMPLETE.** This milestone is landed and frozen.
> Full live status in [`status.md`](status.md); this file is retained
> for the decision-number cross-references in the rest of the tree.

## What landed

- `mili_viz_server::serve_tcp(svc, addr) -> (SocketAddr, JoinHandle)`
  binds a `tokio::net::TcpListener` (ephemeral `:0` supported via
  `local_addr()` read-back) and co-serves `MiliVizServer` **and**
  `FlightServiceServer` on the one port through tonic's
  multi-service router. `spawn_tcp` mirrors the existing
  `spawn_in_process` test helper; `main.rs` now serves over TCP on a
  bind address from `argv[1]` (default `127.0.0.1:50051`).
- Arrow Flight uses the **canonical Apache Arrow `Flight.proto`**
  vendored into `crates/mili-viz-proto/proto/Flight.proto`
  (Apache-2.0 header retained) and compiled through the existing
  protoc-free `protox` + `tonic-prost-build` path — zero diff to the
  frozen `mili_viz.proto`, no new codegen mechanism, and no
  dependency on the heavy `arrow-flight` crate.
- Only `DoGet(Ticket{ticket})` is implemented: it looks the ticket up
  in the in-process geometry store and streams the **verbatim** M2/M3
  `MVG1`/`MVG2` blob as `FlightData.data_body` (no Arrow schema, no
  `RecordBatch` — the blob is opaque by design). Every other Flight
  RPC (`Handshake`/`ListFlights`/`GetFlightInfo`/`PollFlightInfo`/
  `GetSchema`/`DoPut`/`DoExchange`/`DoAction`/`ListActions`) returns
  `Status::unimplemented` — the same frozen-stub discipline M1
  Decision 7 applies to the agent RPCs. Unknown ticket →
  `Status::not_found`.
- Transport swap only: ticket bytes (`geom:{seq}`), `layout` string
  (`MVG1:...` / `MVG2:...`), and encoded blob are byte-stable across
  the swap, redeeming M2 Decision 10. `spawn_in_process` and
  `VizService::fetch_geometry` are kept verbatim as the local
  test/embedding seam (README's default local run mode and all prior
  gating tests depend on them; none were rewritten).

## Gating test

`crates/mili-viz-server/tests/m6_transport.rs::remote_transport_grpc_and_flight_over_tcp`
— binds a real `127.0.0.1:0`, drives both services over real
`tonic::transport::Channel`s end-to-end: `Hello` match/mismatch,
Layer-0 ≡ raw, subscription fan-out, frozen-stub `UNIMPLEMENTED`,
`show` after `load` yielding the same `geom:{seq}` ticket, and a
`DoGet` blob byte-identical to `fetch_geometry` for the same
scenario.

## Decisions

- Decisions 25–27 for this milestone are recorded in this file's
  git history; the index lives in [`status.md`](status.md). Decision
  25 explicitly records that M2 Decision 10's tonic-version premise
  is now factually superseded (arrow-flight 57+ aligns with the
  frozen tonic 0.14 stack), while the deferral itself was still the
  right call at M2.
