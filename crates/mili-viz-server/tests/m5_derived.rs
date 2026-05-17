//! Phase 4 M5 acceptance — derived results (scalar stress invariants).
//!
//! Gating test for `planning/mili-viz/phase-4-m5.md` § "M5 acceptance
//! gate". Skip-on-absent per CLAUDE.md.

#![allow(clippy::too_many_lines)] // one end-to-end acceptance scenario

use std::collections::HashMap;
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
    let mut off = 24 + n_verts * 3 * 4 + n_idx * 4 + (n_idx / 3) * 4;
    let scalar = if magic == b"MVG2" {
        let s: Vec<f32> = (0..n_verts)
            .map(|i| f32::from_le_bytes(blob[off + i * 4..off + i * 4 + 4].try_into().unwrap()))
            .collect();
        off += n_verts * 4;
        assert_eq!(off, blob.len(), "MVG2 blob fully consumed");
        s
    } else {
        assert_eq!(magic, b"MVG1");
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
) -> (Geom, pb::ResultState) {
    let mut req = Request::new(pb::Command {
        cmd: Some(pb::command::Cmd::Show(pb::Show {
            result: result.to_string(),
            component: String::new(),
            opts: HashMap::new(),
        })),
    });
    req.metadata_mut()
        .insert(CLIENT_ID_HEADER, "m5".parse().unwrap());
    let reply = client.execute(req).await.unwrap().into_inner();
    assert!(reply.ok, "show {result} failed: {}", reply.error);
    let d = sub.message().await.unwrap().unwrap();
    let Some(pb::state_delta::Payload::Result(res)) = d.payload else {
        panic!("show must broadcast a ResultState");
    };
    let g = res.geometry.clone().expect("show carries a GeometryRef");
    let geom = decode(&svc.fetch_geometry(&g.flight_ticket).unwrap(), &g.layout);
    (geom, res)
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
        .insert(CLIENT_ID_HEADER, "m5".parse().unwrap());
    client.execute(req).await.unwrap();
    let _ = sub.message().await.unwrap().unwrap();
}

#[tokio::test]
async fn derived_stress_invariants() {
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
        .insert(CLIENT_ID_HEADER, "m5".parse().unwrap());
    assert!(client.execute(load).await.unwrap().into_inner().ok);

    let mut sub = client
        .subscribe(Request::new(pb::SubscribeRequest::default()))
        .await
        .unwrap()
        .into_inner();
    let _snap = sub.message().await.unwrap().unwrap();

    // ── unknown derived name → graceful M3 bare-hull fallback ─────────
    let unknown = show(&mut client, &mut sub, &svc, "not_a_derived").await.0;
    assert!(
        unknown.layout.starts_with("MVG1") && unknown.scalar.is_empty(),
        "unsupported derived → bare hull, no error"
    );

    // Step to a stressed state — state 1 is the undeformed initial
    // state (all stresses zero), where the identity is trivially true.
    set_state(&mut client, &mut sub, 101).await;

    // ── linear-pressure identity (phase-4-m5.md Decision 21) ─────────
    // pressure = -1/3·(sx+sy+sz) is linear, and the M3 nodal scatter is
    // a per-node mean, so averaging commutes with the combination:
    // the served per-vertex `pressure` must equal -1/3·(Sx+Sy+Sz) where
    // S* is the M3-served per-vertex primal. This exercises the real
    // `compute_stress_invariant` kernel through the viz routing.
    let (sx, _) = show(&mut client, &mut sub, &svc, "sx").await;
    let (sy, _) = show(&mut client, &mut sub, &svc, "sy").await;
    let (sz, _) = show(&mut client, &mut sub, &svc, "sz").await;
    let (pressure, pres_res) = show(&mut client, &mut sub, &svc, "pressure").await;
    assert_eq!(
        pressure.layout,
        "MVG2:verts_f32x3+idx_u32+trimat_u32+scalar_f32"
    );
    assert_eq!(pressure.scalar.len(), pressure.verts);

    let mut compared = 0usize;
    let mut max_abs = 0.0f64;
    for i in 0..pressure.verts {
        let (p, a, b, c) = (pressure.scalar[i], sx.scalar[i], sy.scalar[i], sz.scalar[i]);
        if p.is_finite() && a.is_finite() && b.is_finite() && c.is_finite() {
            let expect = -(1.0f64 / 3.0) * (f64::from(a) + f64::from(b) + f64::from(c));
            let scale = expect.abs().max(f64::from(p).abs()).max(1.0);
            assert!(
                (f64::from(p) - expect).abs() <= 1e-3 * scale,
                "node {i}: pressure {p} != -1/3·(sx+sy+sz) ({expect})"
            );
            compared += 1;
            max_abs = max_abs.max(f64::from(p).abs());
        }
    }
    assert!(
        compared > 0 && max_abs > 0.0,
        "pressure: some finite non-zero stressed nodes were cross-checked \
         (compared={compared}, max_abs={max_abs})"
    );
    assert!(
        pres_res.min <= pres_res.max,
        "pressure: ResultState min<=max"
    );

    // ── nonlinear invariants: structural (kernel already parity-exact)
    for name in ["eff_stress", "triaxiality", "norm_press"] {
        let (g, res) = show(&mut client, &mut sub, &svc, name).await;
        assert!(g.layout.starts_with("MVG2"), "{name} → MVG2");
        assert_eq!(g.scalar.len(), g.verts, "{name} scalar is per-vertex");
        let finite: Vec<f32> = g.scalar.iter().copied().filter(|v| v.is_finite()).collect();
        assert!(!finite.is_empty(), "{name}: finite samples on elements");
        let lo = finite.iter().copied().fold(f32::INFINITY, f32::min);
        let hi = finite.iter().copied().fold(f32::NEG_INFINITY, f32::max);
        assert!(
            res.min as f32 <= lo + 1e-3 && res.max as f32 >= hi - 1e-3,
            "{name}: ResultState range brackets the scalar data"
        );
    }

    // ── derived tracks the state (basic1 is transient) ───────────────
    // Currently at state 101 (stressed). State 1 is the undeformed
    // initial state, so eff_stress must differ.
    let s101 = show(&mut client, &mut sub, &svc, "eff_stress")
        .await
        .0
        .scalar;
    set_state(&mut client, &mut sub, 1).await;
    let s1 = show(&mut client, &mut sub, &svc, "eff_stress")
        .await
        .0
        .scalar;
    assert_ne!(s1, s101, "eff_stress must differ between state 1 and 101");

    // ── primal path still byte-stable: empty result → bare hull ──────
    let bare = show(&mut client, &mut sub, &svc, "").await.0;
    assert!(bare.layout.starts_with("MVG1") && bare.scalar.is_empty());
}
