//! Phase 4 M2 acceptance — load + state navigation + real geometry.
//!
//! Gating test for `planning/mili-viz/phase-4-m2.md` § "M2 acceptance
//! gate". Follows the CLAUDE.md skip-on-absent discipline: when the
//! reference corpus fixture is missing it early-returns with an
//! `eprintln!` rather than failing, so a bare `cargo test` is honest
//! about coverage and CI (which runs `scripts/setup-parity.sh`) is
//! authoritative.

#![allow(clippy::too_many_lines)] // one end-to-end acceptance scenario

use std::path::{Path, PathBuf};

use mili_viz_proto::v1 as pb;
use mili_viz_server::{spawn_in_process, VizService, CLIENT_ID_HEADER};
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

/// Decode the frozen `MVG1` blob (phase-4-m2.md Decision 11).
struct Geom {
    verts: Vec<f32>,
    indices: Vec<u32>,
    tri_material: Vec<u32>,
}

fn decode(blob: &[u8]) -> Geom {
    assert_eq!(&blob[0..4], b"MVG1", "bad magic");
    let dims = u32::from_le_bytes(blob[4..8].try_into().unwrap());
    assert_eq!(dims, 3, "M2 pads to verts_f32x3");
    let n_verts = u64::from_le_bytes(blob[8..16].try_into().unwrap()) as usize;
    let n_idx = u64::from_le_bytes(blob[16..24].try_into().unwrap()) as usize;
    assert_eq!(n_idx % 3, 0, "index buffer is a triangle list");

    let mut off = 24;
    let verts: Vec<f32> = (0..n_verts * 3)
        .map(|i| f32::from_le_bytes(blob[off + i * 4..off + i * 4 + 4].try_into().unwrap()))
        .collect();
    off += n_verts * 3 * 4;
    let indices: Vec<u32> = (0..n_idx)
        .map(|i| u32::from_le_bytes(blob[off + i * 4..off + i * 4 + 4].try_into().unwrap()))
        .collect();
    off += n_idx * 4;
    let tri_material: Vec<u32> = (0..n_idx / 3)
        .map(|i| u32::from_le_bytes(blob[off + i * 4..off + i * 4 + 4].try_into().unwrap()))
        .collect();

    Geom {
        verts,
        indices,
        tri_material,
    }
}

async fn exec(
    client: &mut pb::mili_viz_client::MiliVizClient<tonic::transport::Channel>,
    cmd: pb::command::Cmd,
) -> pb::CommandReply {
    client
        .execute(with_client_id(cmd, "m2-test"))
        .await
        .unwrap()
        .into_inner()
}

#[tokio::test]
async fn load_state_nav_and_real_geometry() {
    let path = corpus_path(&["serial", "basic1", "basic1.pltA"]);
    if !path.exists() {
        eprintln!("skip: serial/basic1 absent (run scripts/setup-parity.sh)");
        return;
    }
    let svc = VizService::builder().build();
    let (mut client, _h) = spawn_in_process(svc.clone()).await.unwrap();

    // ── load: real LoadedState (num_states, times, classes) ──────────
    let reply = exec(
        &mut client,
        pb::command::Cmd::Load(pb::Load {
            root: path.to_string_lossy().into_owned(),
        }),
    )
    .await;
    assert!(reply.ok, "load failed: {}", reply.error);

    let mut sub = client
        .subscribe(Request::new(pb::SubscribeRequest::default()))
        .await
        .unwrap()
        .into_inner();
    let snap = sub.message().await.unwrap().unwrap();
    let Some(pb::state_delta::Payload::Snapshot(s)) = snap.payload else {
        panic!("subscribe must open with a Snapshot");
    };
    let loaded = s.loaded.expect("loaded state present after load");
    assert_eq!(loaded.num_states, 101, "basic1 ships 101 states");
    assert_eq!(loaded.state_times.len(), 101);
    assert!(
        !loaded.class_names.is_empty(),
        "basic1 declares element classes"
    );

    // ── state navigation clamps to [1, num_states] ───────────────────
    let r = exec(
        &mut client,
        pb::command::Cmd::SetState(pb::SetState { state: 999 }),
    )
    .await;
    assert!(r.ok);
    let r = exec(
        &mut client,
        pb::command::Cmd::Step(pb::Step {
            dir: pb::step::Dir::Next as i32,
        }),
    )
    .await;
    assert!(r.ok);
    // 999 clamps to 101; `next` from the clamped end stays at 101.
    let d = sub.message().await.unwrap().unwrap(); // SetState delta
    assert!(matches!(
        d.payload,
        Some(pb::state_delta::Payload::State(101))
    ));
    let d = sub.message().await.unwrap().unwrap(); // Step delta
    assert!(matches!(
        d.payload,
        Some(pb::state_delta::Payload::State(101))
    ));

    // first → 1
    exec(
        &mut client,
        pb::command::Cmd::Step(pb::Step {
            dir: pb::step::Dir::First as i32,
        }),
    )
    .await;
    let d = sub.message().await.unwrap().unwrap();
    assert!(matches!(
        d.payload,
        Some(pb::state_delta::Payload::State(1))
    ));

    // ── show → real GeometryRef + fetchable blob at state 1 ──────────
    let r = exec(
        &mut client,
        pb::command::Cmd::Show(pb::Show {
            result: String::new(),
            component: String::new(),
            opts: std::collections::HashMap::new(),
        }),
    )
    .await;
    assert!(r.ok);
    let d = sub.message().await.unwrap().unwrap();
    let Some(pb::state_delta::Payload::Result(res)) = d.payload else {
        panic!("show must broadcast a ResultState");
    };
    let gref = res.geometry.expect("M2 show carries a real GeometryRef");
    assert_eq!(gref.layout, "MVG1:verts_f32x3+idx_u32+trimat_u32");
    assert!(gref.num_vertices > 0, "mesh has vertices");
    assert!(gref.num_indices > 0 && gref.num_indices % 3 == 0);

    let blob1 = svc
        .fetch_geometry(&gref.flight_ticket)
        .expect("ticket resolves in the in-process store");
    let g1 = decode(&blob1);
    assert_eq!(g1.verts.len() as u64, gref.num_vertices * 3);
    assert_eq!(g1.indices.len() as u64, gref.num_indices);
    assert_eq!(g1.tri_material.len(), g1.indices.len() / 3);
    assert!(
        g1.indices.iter().all(|&i| (i as u64) < gref.num_vertices),
        "every triangle index is in range"
    );

    // ── per-state geometry: a later state deforms the hull ───────────
    exec(
        &mut client,
        pb::command::Cmd::SetState(pb::SetState { state: 101 }),
    )
    .await;
    let _ = sub.message().await.unwrap().unwrap(); // drain State delta
    let r = exec(
        &mut client,
        pb::command::Cmd::Show(pb::Show {
            result: String::new(),
            component: String::new(),
            opts: std::collections::HashMap::new(),
        }),
    )
    .await;
    assert!(r.ok);
    let d = sub.message().await.unwrap().unwrap();
    let Some(pb::state_delta::Payload::Result(res)) = d.payload else {
        panic!("show must broadcast a ResultState");
    };
    let gref2 = res.geometry.expect("GeometryRef at state 101");
    let blob2 = svc.fetch_geometry(&gref2.flight_ticket).unwrap();
    let g2 = decode(&blob2);

    // Topology is state-invariant; positions are not (basic1 is a
    // transient simulation — the hull deforms between state 1 and 101).
    assert_eq!(g1.indices, g2.indices, "topology is state-invariant");
    assert_ne!(
        g1.verts, g2.verts,
        "node positions deform across states (nodpos per state)"
    );
}
