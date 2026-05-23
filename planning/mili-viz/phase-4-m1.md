# Phase 4 M1 — landed (proto crate + in-process transport, full wire contract frozen)

> **Status: ✅ COMPLETE.** This milestone is landed and frozen.
> Full live status in [`status.md`](status.md); this file is retained
> for the decision-number cross-references in the rest of the tree.

## What landed

- `crates/mili-viz-proto` (the wire types) and a `tonic`
  `mili-viz-server` reachable over an in-process transport
  (`spawn_in_process`, `tokio::io::duplex`). The frozen `MiliViz`
  service: `Hello`, `Execute`, `Subscribe`, `Query`, plus the agent
  surface (`AgentChat`, `Interrupt`, `CaptureFrame`).
- The full M1 wire contract — base griz vocabulary + scripting
  multi-client surface (subscription / `StateDelta` / handshake) +
  agent surface — frozen in one drop via deltas Δ1–Δ9 to the original
  proto draft (adds `AgentChat`/`Interrupt`/`CaptureFrame` RPCs, the
  `DELTA_AGENT` `DeltaKind`, the `StateDelta.payload` agent arm, the
  `AgentTranscript` field of `Snapshot`, and the `agent` capability
  flag; resolves the `Snapshot` name collision by naming the
  framebuffer RPC `CaptureFrame`).
- A protoc-free build (`protox` + `tonic-prost-build`) so
  `setup-parity.sh` stays the single provisioning source of truth.
- Frozen-but-`UNIMPLEMENTED` stubs for the agent/capture surface
  (Phase 4/5 M6 implements them).
- A pinned many-to-one `Command → DeltaKind` mapping
  (`mili_viz_server::command_delta_kind`) — the 17 command arms fold
  onto 10 state-aspect delta kinds.

## Gating tests

`crates/mili-viz-server/tests/acceptance.rs` — six tests:
`handshake_match_and_mismatch`, `capability_agent_present_absent`,
`layer0_equals_raw`, `subscription_fanout`,
`frozen_stubs_unimplemented`, `conformance_all_command_arms`.

## Decisions

- Decisions 1–9 for this milestone are recorded in this file's
  git history; the index lives in [`status.md`](status.md). Any
  decision that *superseded* an earlier one is called out in
  status.md's TL;DR (e.g. Decision 5's pre-committed validation
  fallback is superseded by M5 Decision 19).
