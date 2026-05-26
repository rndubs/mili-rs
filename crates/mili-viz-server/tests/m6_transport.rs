//! Phase 4 M6 acceptance — remote transport (gRPC + Arrow Flight
//! over TCP).
//!
//! Gating test for `planning/mili-viz/phase-4-m6.md` § "M6 acceptance
//! gate". Drives the **real TCP transport** end-to-end: a real
//! `MiliViz` gRPC channel and a real `arrow.flight.protocol.
//! FlightService` client over an OS-assigned ephemeral
//! `127.0.0.1:0` port. Proves M6 is a transport swap, not a contract
//! or format change — the Flight `DoGet` of the frozen
//! `GeometryRef.flight_ticket` returns a blob **byte-identical** to
//! the in-process `VizService::fetch_geometry`.
//!
//! Skip-on-absent per CLAUDE.md: early `return` + `eprintln!` when the
//! `serial/basic1` corpus fixture is missing.

#![allow(clippy::too_many_lines)] // one end-to-end acceptance scenario

use std::path::{Path, PathBuf};

use mili_viz_proto::flight as fpb;
use mili_viz_proto::v1 as pb;
use mili_viz_server::{spawn_tcp, VizService, CLIENT_ID_HEADER};
use tonic::Request;

fn corpus_path(rel: &[&str]) -> PathBuf {
    let mut p = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("reference")
        .join("mili-python")
        .join("tests")
        .join("data");
    for c in rel {
        p = p.join(c);
    }
    p
}

fn with_client_id(cmd: pb::command::Cmd, id: &str) -> Request<pb::Command> {
    let mut req = Request::new(pb::Command { cmd: Some(cmd) });
    req.metadata_mut()
        .insert(CLIENT_ID_HEADER, id.parse().unwrap());
    req
}

type VizClient = pb::mili_viz_client::MiliVizClient<tonic::transport::Channel>;
type FlightClient = fpb::flight_service_client::FlightServiceClient<tonic::transport::Channel>;

async fn exec(client: &mut VizClient, cmd: pb::command::Cmd) -> pb::CommandReply {
    client
        .execute(with_client_id(cmd, "m6-test"))
        .await
        .unwrap()
        .into_inner()
}

/// Pull a ticket's bytes back over a real Arrow Flight `DoGet`,
/// concatenating `data_body` across the stream (phase-4-m6.md
/// Decision 26 — the client reassembles regardless of framing).
async fn flight_get(flight: &mut FlightClient, ticket: &[u8]) -> Vec<u8> {
    let mut stream = flight
        .do_get(Request::new(fpb::Ticket {
            ticket: ticket.to_vec(),
        }))
        .await
        .expect("DoGet of a live ticket succeeds")
        .into_inner();
    let mut blob = Vec::new();
    while let Some(fd) = stream.message().await.expect("Flight stream ok") {
        blob.extend_from_slice(&fd.data_body);
    }
    blob
}

fn show_cmd(result: &str) -> pb::command::Cmd {
    pb::command::Cmd::Show(pb::Show {
        result: result.to_string(),
        component: String::new(),
        opts: std::collections::HashMap::new(),
    })
}

async fn show_geometry(
    client: &mut VizClient,
    sub: &mut tonic::Streaming<pb::StateDelta>,
    result: &str,
) -> (pb::GeometryRef, f64, f64) {
    let r = exec(client, show_cmd(result)).await;
    assert!(r.ok, "show {result:?} failed: {}", r.error);
    let d = sub.message().await.unwrap().unwrap();
    let Some(pb::state_delta::Payload::Result(res)) = d.payload else {
        panic!("show must broadcast a ResultState");
    };
    let g = res.geometry.expect("show carries a GeometryRef over TCP");
    (g, res.min, res.max)
}

#[tokio::test]
async fn remote_transport_grpc_and_flight_over_tcp() {
    let path = corpus_path(&["serial", "basic1", "basic1.pltA"]);
    if !path.exists() {
        eprintln!("skip: serial/basic1 absent (run scripts/setup-parity.sh)");
        return;
    }

    let svc = VizService::builder().build();
    // Real ephemeral TCP port; real MiliViz + Flight clients.
    let (addr, mut viz, mut flight, _h) = spawn_tcp(svc.clone()).await.unwrap();
    assert!(addr.port() != 0, "serve_tcp resolves the ephemeral port");

    // ── Hello negotiates over the wire (match + reported mismatch) ───
    let ok = viz
        .hello(Request::new(pb::HelloRequest {
            protocol_version: pb::PROTOCOL_VERSION.to_string(),
            client_id: "egui-client/0.1.0".into(),
            ..Default::default()
        }))
        .await
        .unwrap()
        .into_inner();
    assert!(ok.compatible && ok.mismatch_detail.is_empty());
    let bad = viz
        .hello(Request::new(pb::HelloRequest {
            protocol_version: "99.0.0".into(),
            ..Default::default()
        }))
        .await
        .unwrap()
        .into_inner();
    assert!(!bad.compatible && !bad.mismatch_detail.is_empty());

    // ── agent surface is reachable over the wire ─────────────────────
    // Phase 5 M6 (phase-5-m6.md Decisions 94/95) — the M1 frozen stub
    // is gone. Without a backend the call returns ok=false with a
    // clear error (the wire is implemented, the deployment isn't
    // configured); with one it succeeds. The M6 server-side gating
    // gate (`m6_agent.rs`) covers the lit-up path; this M6 transport
    // gate just verifies the wire reaches the new dispatch.
    let reply = viz
        .agent_chat(Request::new(pb::AgentChatRequest {
            text: "hi".into(),
            ..Default::default()
        }))
        .await
        .expect("the wire surface is implemented in M6")
        .into_inner();
    assert!(!reply.ok, "no backend ⇒ clean reject");
    assert!(reply.error.to_lowercase().contains("backend"));

    // ── load + subscribe over the wire ───────────────────────────────
    let loaded = exec(
        &mut viz,
        pb::command::Cmd::Load(pb::Load {
            root: path.to_string_lossy().into_owned(),
        }),
    )
    .await;
    assert!(loaded.ok, "load over TCP failed: {}", loaded.error);

    let mut sub = viz
        .subscribe(Request::new(pb::SubscribeRequest::default()))
        .await
        .unwrap()
        .into_inner();
    let snap = sub.message().await.unwrap().unwrap();
    let Some(pb::state_delta::Payload::Snapshot(sess)) = snap.payload else {
        panic!("subscribe must open with a Snapshot");
    };
    assert_eq!(
        sess.loaded.expect("loaded after load over TCP").num_states,
        101
    );

    // ── show "" → bare hull (MVG3 since VB-005); Flight DoGet ==
    //    in-process fetch_geometry ────────────────────────────────────
    let (bare, _, _) = show_geometry(&mut viz, &mut sub, "").await;
    assert!(
        bare.layout.starts_with("MVG3:"),
        "default emit is MVG3 over the wire since the VB-005 promotion: {}",
        bare.layout
    );
    assert!(
        bare.flight_ticket.starts_with(b"geom:"),
        "the frozen geom:{{seq}} ticket form is unchanged (Decision 10)"
    );
    let via_flight = flight_get(&mut flight, &bare.flight_ticket).await;
    let via_inproc = svc
        .fetch_geometry(&bare.flight_ticket)
        .expect("in-process seam still resolves the same ticket");
    assert_eq!(
        via_flight, via_inproc,
        "Flight DoGet streams the byte-identical blob (transport swap, \
         not a format change)"
    );
    // The blob decodes per phase-4-m7.md Decision 72.
    assert_eq!(&via_flight[0..4], b"MVG3");
    let n_verts = u64::from_le_bytes(via_flight[8..16].try_into().unwrap());
    let n_idx = u64::from_le_bytes(via_flight[16..24].try_into().unwrap());
    assert_eq!(n_verts, bare.num_vertices);
    assert_eq!(n_idx, bare.num_indices);
    assert!(n_idx % 3 == 0 && n_verts > 0);
    {
        let mut off = 36 + (n_verts as usize) * 3 * 4;
        for _ in 0..n_idx {
            let idx = u32::from_le_bytes(via_flight[off..off + 4].try_into().unwrap());
            assert!((u64::from(idx)) < n_verts, "triangle index in range");
            off += 4;
        }
    }

    // ── show "sand" → MVG3 with scalar over the wire, range brackets ─
    let (mvg2, min, max) = show_geometry(&mut viz, &mut sub, "sand").await;
    assert!(
        mvg2.layout.starts_with("MVG3:"),
        "scalar-carrying blob is MVG3 over the wire: {}",
        mvg2.layout
    );
    let blob = flight_get(&mut flight, &mvg2.flight_ticket).await;
    assert_eq!(
        blob,
        svc.fetch_geometry(&mvg2.flight_ticket).unwrap(),
        "MVG3 blob byte-identical across the Flight wire"
    );
    assert_eq!(&blob[0..4], b"MVG3");
    let nv = u64::from_le_bytes(blob[8..16].try_into().unwrap()) as usize;
    let ni = u64::from_le_bytes(blob[16..24].try_into().unwrap()) as usize;
    let n_edges = u64::from_le_bytes(blob[24..32].try_into().unwrap()) as usize;
    let flags_mask = u32::from_le_bytes(blob[32..36].try_into().unwrap());
    let n_tri = ni / 3;
    let mut soff = 36 + nv * 3 * 4 + ni * 4 + n_tri * 4;
    if flags_mask & 2 != 0 {
        soff += n_tri * 4;
    }
    if flags_mask & 4 != 0 {
        soff += n_edges * 4;
    }
    assert_ne!(flags_mask & 1, 0, "scalar bit set when a result is mapped");
    let scalar: Vec<f32> = (0..nv)
        .map(|k| f32::from_le_bytes(blob[soff + k * 4..soff + k * 4 + 4].try_into().unwrap()))
        .collect();
    let mut consumed = soff + nv * 4;
    if flags_mask & 16 != 0 {
        consumed += n_tri * 4; // tri_member_id (wireframe-parity #6 path (a))
    }
    assert_eq!(consumed, blob.len(), "MVG3 blob fully consumed");
    let finite: Vec<f32> = scalar.iter().copied().filter(|v| v.is_finite()).collect();
    assert!(!finite.is_empty(), "sand colors resulted elements");
    let lo = finite.iter().copied().fold(f32::INFINITY, f32::min);
    let hi = finite.iter().copied().fold(f32::NEG_INFINITY, f32::max);
    assert!(
        min as f32 <= lo + 1e-3 && max as f32 >= hi - 1e-3,
        "ResultState.{{min,max}} brackets the scalar over the wire"
    );

    // ── unknown ticket → NotFound; a non-DoGet RPC → Unimplemented ──
    let nf = flight
        .do_get(Request::new(fpb::Ticket {
            ticket: b"geom:does-not-exist".to_vec(),
        }))
        .await
        .unwrap_err();
    assert_eq!(nf.code(), tonic::Code::NotFound);
    let ni = flight
        .list_actions(Request::new(fpb::Empty {}))
        .await
        .unwrap_err();
    assert_eq!(ni.code(), tonic::Code::Unimplemented);

    // ── subscription fan-out over the wire (two subscribers) ─────────
    let mut sub_a = viz
        .subscribe(Request::new(pb::SubscribeRequest::default()))
        .await
        .unwrap()
        .into_inner();
    let mut sub_b = viz
        .subscribe(Request::new(pb::SubscribeRequest::default()))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(
        sub_a.message().await.unwrap().unwrap().kind,
        pb::DeltaKind::DeltaSnapshot as i32
    );
    assert_eq!(
        sub_b.message().await.unwrap().unwrap().kind,
        pb::DeltaKind::DeltaSnapshot as i32
    );
    let reply = viz
        .execute(with_client_id(
            pb::command::Cmd::SetState(pb::SetState { state: 5 }),
            "alice",
        ))
        .await
        .unwrap()
        .into_inner();
    for sub in [&mut sub_a, &mut sub_b] {
        let delta = sub.message().await.unwrap().unwrap();
        assert_eq!(delta.kind, pb::DeltaKind::DeltaState as i32);
        assert_eq!(delta.seq, reply.delta_seq);
        assert_eq!(delta.origin_client_id, "alice");
        assert!(matches!(
            delta.payload,
            Some(pb::state_delta::Payload::State(5))
        ));
    }

    // ── Layer-0 ≡ raw holds over the wire ────────────────────────────
    let svc2 = VizService::builder().build();
    let (_a2, mut viz2, _f2, _h2) = spawn_tcp(svc2).await.unwrap();
    let mut sub2 = viz2
        .subscribe(Request::new(pb::SubscribeRequest::default()))
        .await
        .unwrap()
        .into_inner();
    let _ = sub2.message().await.unwrap().unwrap(); // snapshot
    viz2.execute(Request::new(pb::Command {
        cmd: Some(pb::command::Cmd::Raw("state 5".into())),
    }))
    .await
    .unwrap();
    let raw_d = sub2.message().await.unwrap().unwrap();
    assert_eq!(raw_d.kind, pb::DeltaKind::DeltaState as i32);
    assert!(matches!(
        raw_d.payload,
        Some(pb::state_delta::Payload::State(5))
    ));
}
