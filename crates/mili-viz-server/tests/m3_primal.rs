//! Phase 4 M3 acceptance — primal result display.
//!
//! Gating test for `planning/mili-viz/phase-4-m3.md` § "M3 acceptance
//! gate". Skip-on-absent per CLAUDE.md.

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

struct Geom {
    layout: String,
    verts: usize,
    scalar: Vec<f32>,
}

fn decode(blob: &[u8], layout: &str) -> Geom {
    let magic = &blob[0..4];
    let n_verts = u64::from_le_bytes(blob[8..16].try_into().unwrap()) as usize;
    let n_idx = u64::from_le_bytes(blob[16..24].try_into().unwrap()) as usize;
    let (header, n_edges, flags_mask) = match magic {
        b"MVG1" | b"MVG2" => (24, 0, u32::from(magic == b"MVG2")),
        b"MVG3" => (
            36,
            u64::from_le_bytes(blob[24..32].try_into().unwrap()) as usize,
            u32::from_le_bytes(blob[32..36].try_into().unwrap()),
        ),
        _ => panic!("bad magic {magic:?}"),
    };
    let n_tri = n_idx / 3;
    let mut off = header + n_verts * 3 * 4 + n_idx * 4 + n_tri * 4;
    if magic == b"MVG3" && flags_mask & 2 != 0 {
        off += n_tri * 4; // tri_flags
    }
    if magic == b"MVG3" && flags_mask & 4 != 0 {
        off += n_edges * 4; // edges
    }
    let has_scalar = flags_mask & 1 != 0;
    let scalar = if has_scalar {
        let s: Vec<f32> = (0..n_verts)
            .map(|i| f32::from_le_bytes(blob[off + i * 4..off + i * 4 + 4].try_into().unwrap()))
            .collect();
        off += n_verts * 4;
        assert_eq!(off, blob.len(), "blob fully consumed");
        s
    } else {
        Vec::new()
    };
    Geom {
        layout: layout.to_string(),
        verts: n_verts,
        scalar,
    }
}

async fn show(
    client: &mut pb::mili_viz_client::MiliVizClient<tonic::transport::Channel>,
    sub: &mut tonic::Streaming<pb::StateDelta>,
    svc: &VizService,
    result: &str,
) -> Geom {
    let mut req = Request::new(pb::Command {
        cmd: Some(pb::command::Cmd::Show(pb::Show {
            result: result.to_string(),
            component: String::new(),
            opts: std::collections::HashMap::new(),
        })),
    });
    req.metadata_mut()
        .insert(CLIENT_ID_HEADER, "m3".parse().unwrap());
    let reply = client.execute(req).await.unwrap().into_inner();
    assert!(reply.ok, "show {result} failed: {}", reply.error);
    let d = sub.message().await.unwrap().unwrap();
    let Some(pb::state_delta::Payload::Result(res)) = d.payload else {
        panic!("show must broadcast a ResultState");
    };
    let g = res.geometry.expect("show carries a GeometryRef");
    let mut geom = decode(&svc.fetch_geometry(&g.flight_ticket).unwrap(), &g.layout);
    // Stash the data range for the caller via the scalar vec's
    // companion assertions; expose min/max through ResultState.
    // MVG3 default (post VB-005 promotion) carries scalar when a
    // result is mapped; MVG2 is the legacy form that still triggers
    // the bracket check via the same has-scalar invariant.
    let has_scalar =
        g.layout.starts_with("MVG2") || g.layout.starts_with("MVG3:") && !geom.scalar.is_empty();
    if has_scalar {
        assert!(res.min <= res.max, "{result}: min<=max");
        let finite: Vec<f32> = geom
            .scalar
            .iter()
            .copied()
            .filter(|v| v.is_finite())
            .collect();
        assert!(!finite.is_empty(), "{result}: some finite samples");
        let lo = finite.iter().copied().fold(f32::INFINITY, f32::min);
        let hi = finite.iter().copied().fold(f32::NEG_INFINITY, f32::max);
        assert!(
            res.min as f32 <= lo + 1e-3 && res.max as f32 >= hi - 1e-3,
            "{result}: ResultState range brackets the scalar data"
        );
    }
    geom.layout = g.layout;
    geom
}

async fn set_state(
    client: &mut pb::mili_viz_client::MiliVizClient<tonic::transport::Channel>,
    sub: &mut tonic::Streaming<pb::StateDelta>,
    state: u32,
) {
    let mut req = Request::new(pb::Command {
        cmd: Some(pb::command::Cmd::SetState(pb::SetState { state })),
    });
    req.metadata_mut()
        .insert(CLIENT_ID_HEADER, "m3".parse().unwrap());
    client.execute(req).await.unwrap();
    let _ = sub.message().await.unwrap().unwrap();
}

#[tokio::test]
async fn primal_result_colors_the_mesh() {
    let path = corpus_path(&["serial", "basic1", "basic1.pltA"]);
    if !path.exists() {
        eprintln!("skip: serial/basic1 absent (run scripts/setup-parity.sh)");
        return;
    }
    let svc = VizService::builder().build();
    let (mut client, _h) = spawn_in_process(svc.clone()).await.unwrap();

    let mut load = Request::new(pb::Command {
        cmd: Some(pb::command::Cmd::Load(pb::Load {
            root: path.to_string_lossy().into_owned(),
        })),
    });
    load.metadata_mut()
        .insert(CLIENT_ID_HEADER, "m3".parse().unwrap());
    assert!(client.execute(load).await.unwrap().into_inner().ok);

    let mut sub = client
        .subscribe(Request::new(pb::SubscribeRequest::default()))
        .await
        .unwrap()
        .into_inner();
    let _snap = sub.message().await.unwrap().unwrap();

    // ── empty result → bare hull (MVG3, no scalar bit) ───────────────
    let bare = show(&mut client, &mut sub, &svc, "").await;
    assert!(
        bare.layout.starts_with("MVG3:"),
        "bare hull is MVG3 since VB-005 promotion: {}",
        bare.layout
    );
    assert!(
        bare.scalar.is_empty(),
        "no scalar bit when no result mapped"
    );

    // ── unknown result → graceful fallback to the bare hull ──────────
    let unknown = show(&mut client, &mut sub, &svc, "no_such_svar").await;
    assert!(
        unknown.layout.starts_with("MVG3:"),
        "unknown → bare hull (MVG3)"
    );
    assert!(unknown.scalar.is_empty());

    // ── element scalar (`sand` on `brick`) → MVG3 with scalar column ─
    let sand = show(&mut client, &mut sub, &svc, "sand").await;
    assert!(sand.layout.starts_with("MVG3:"));
    assert_eq!(sand.scalar.len(), sand.verts);
    assert!(
        sand.scalar.iter().any(|v| v.is_finite()),
        "sand: brick nodes carry a value"
    );

    // ── nodal vector (`nodvel`) → MVG3, colored by component 0 ───────
    let nv1 = show(&mut client, &mut sub, &svc, "nodvel").await;
    assert!(nv1.layout.starts_with("MVG3:"));
    assert!(nv1.scalar.iter().any(|v| v.is_finite()));

    // ── scalar tracks the state (basic1 is transient) ────────────────
    set_state(&mut client, &mut sub, 101).await;
    let nv2 = show(&mut client, &mut sub, &svc, "nodvel").await;
    assert_ne!(
        nv1.scalar, nv2.scalar,
        "nodvel scalar must differ between state 1 and 101"
    );

    // ── bare hull stays MVG3 with no scalar bit (round-trip) ─────────
    let bare2 = show(&mut client, &mut sub, &svc, "").await;
    assert!(bare2.layout.starts_with("MVG3:"));
    assert!(bare2.scalar.is_empty());
}
