//! Phase 4 M5 follow-up acceptance (third slice) — surfstrain + nodal
//! time-derived families.
//!
//! Gating test for `planning/mili-viz/phase-4-m5c.md` § "M5c
//! acceptance gate". The `mili-rs` kernels (`compute_node_*`,
//! `surface_strain_query`) are already bit-exact vs the `mili` Python
//! package in the `mili-rs` core parity suite (phase-4-m5.md
//! Decision 19); this test validates the **viz routing** via
//! single-shared-gather invariants only (phase-4-m5c.md Decision 31):
//! the exact displacement-magnitude norm identity, structural +
//! state-tracking for `surfstrain*`/kinematics, and the
//! `vel_*`-at-state-1-is-zero kernel fact. No cross-cardinality checks
//! (per phase-4-m5b.md Decision 24 — the IP-sampling skew is real and
//! expected). Skip-on-absent per CLAUDE.md.

#![allow(clippy::too_many_lines)] // one end-to-end acceptance scenario
#![allow(clippy::many_single_char_names)] // x/y/z = displacement components

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
        off += n_tri * 4;
    }
    if magic == b"MVG3" && flags_mask & 4 != 0 {
        off += n_edges * 4;
    }
    let scalar = if flags_mask & 1 != 0 {
        let s: Vec<f32> = (0..n_verts)
            .map(|i| f32::from_le_bytes(blob[off + i * 4..off + i * 4 + 4].try_into().unwrap()))
            .collect();
        off += n_verts * 4;
        s
    } else {
        Vec::new()
    };
    if magic == b"MVG3" && flags_mask & 16 != 0 {
        off += n_tri * 4; // tri_member_id (wireframe-parity #6 path (a))
    }
    assert_eq!(off, blob.len(), "blob fully consumed");
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
        .insert(CLIENT_ID_HEADER, "m5c".parse().unwrap());
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
        .insert(CLIENT_ID_HEADER, "m5c".parse().unwrap());
    client.execute(req).await.unwrap();
    let _ = sub.message().await.unwrap().unwrap();
}

/// `MVG2`, per-vertex length, finite samples present, and the
/// `ResultState` range brackets the finite scalar data.
fn structural(g: &Geom, res: &pb::ResultState, name: &str) {
    assert!(g.layout.starts_with("MVG3:"), "{name} → MVG3");
    assert_eq!(g.scalar.len(), g.verts, "{name} scalar is per-vertex");
    let finite: Vec<f32> = g.scalar.iter().copied().filter(|v| v.is_finite()).collect();
    assert!(!finite.is_empty(), "{name}: finite samples present");
    let lo = finite.iter().copied().fold(f32::INFINITY, f32::min);
    let hi = finite.iter().copied().fold(f32::NEG_INFINITY, f32::max);
    assert!(
        res.min as f32 <= lo + 1e-3 && res.max as f32 >= hi - 1e-3,
        "{name}: ResultState range brackets the scalar data"
    );
    assert!(res.min <= res.max, "{name}: ResultState min<=max");
}

/// `mag ≈ sqrt(Σ cᵢ²)` per node over finite samples (same node-direct
/// gather, exact to f32). Returns the compared-node count and the max
/// magnitude so the caller can assert non-triviality.
fn assert_norm(mag: &[f32], comps: &[&[f32]], label: &str) -> (usize, f64) {
    for c in comps {
        assert_eq!(mag.len(), c.len(), "{label}: per-vertex length mismatch");
    }
    let mut compared = 0usize;
    let mut max_abs = 0.0f64;
    for i in 0..mag.len() {
        if !mag[i].is_finite() || comps.iter().any(|c| !c[i].is_finite()) {
            continue;
        }
        let norm = comps
            .iter()
            .map(|c| f64::from(c[i]) * f64::from(c[i]))
            .sum::<f64>()
            .sqrt();
        let scale = f64::from(mag[i]).abs().max(norm).max(1.0);
        assert!(
            (f64::from(mag[i]) - norm).abs() <= 1e-3 * scale,
            "{label}: node {i}: {} != {norm}",
            mag[i]
        );
        compared += 1;
        max_abs = max_abs.max(f64::from(mag[i]).abs());
    }
    (compared, max_abs)
}

#[tokio::test]
async fn derived_surfstrain_and_nodal_time() {
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
        .insert(CLIENT_ID_HEADER, "m5c".parse().unwrap());
    assert!(client.execute(load).await.unwrap().into_inner().ok);

    let mut sub = client
        .subscribe(Request::new(pb::SubscribeRequest::default()))
        .await
        .unwrap()
        .into_inner();
    let _snap = sub.message().await.unwrap().unwrap();

    // ── totality: an unknown name and the empty result fall back to
    //    the M3 bare hull (`show` never errors). NOTE: `*_alt` names
    //    (`prin_strain1_alt`, …) USED to be re-deferred-and-bare-hull
    //    here under phase-4-m5c.md Decision 28; that decision is now
    //    discharged — the parity-gated `mili-rs` kernel + viz seam
    //    landed (phase-4-m5d.md Decisions 32–34), so `*_alt` now
    //    resolves to an `MVG2` scalar and its routing coverage moved to
    //    `m5d_alt_strain.rs`. They are deliberately no longer in this
    //    bare-hull list. ──────────────────────────────────────────────
    // Empty svar still intentionally renders the bare hull (the
    // "unmap result" affordance is preserved across M7 Delta 4).
    {
        let g = show(&mut client, &mut sub, &svc, "").await.0;
        assert!(
            g.layout.starts_with("MVG3:") && g.scalar.is_empty(),
            "empty svar → bare hull, no error"
        );
    }
    // Unknown svar is now an M7 Delta 4 no-op (broadcast carries
    // geometry: None; prior result preserved). See
    // `m7-bench-live-parity.md`.
    {
        let mut req = Request::new(pb::Command {
            cmd: Some(pb::command::Cmd::Show(pb::Show {
                result: "not_a_derived".to_string(),
                component: String::new(),
                opts: HashMap::new(),
            })),
        });
        req.metadata_mut()
            .insert(CLIENT_ID_HEADER, "m5c".parse().unwrap());
        assert!(client.execute(req).await.unwrap().into_inner().ok);
        let d = sub.message().await.unwrap().unwrap();
        let Some(pb::state_delta::Payload::Result(r)) = d.payload else {
            panic!("show must broadcast a ResultState");
        };
        assert!(
            r.geometry.is_none(),
            "M7 Delta 4: unresolved → geometry None"
        );
    }

    // Step to a stressed/deformed state — state 1 is undeformed (all
    // displacements/strains zero), where the identities are trivial.
    set_state(&mut client, &mut sub, 101).await;

    // ── nodal displacement: the exact same-gather norm identities ───
    let (dx, dxr) = show(&mut client, &mut sub, &svc, "disp_x").await;
    let (dy, _) = show(&mut client, &mut sub, &svc, "disp_y").await;
    let (dz, _) = show(&mut client, &mut sub, &svc, "disp_z").await;
    let (dm, dmr) = show(&mut client, &mut sub, &svc, "disp_mag").await;
    let (drm, _) = show(&mut client, &mut sub, &svc, "disp_rad_mag_xy").await;
    assert!(dx.layout.starts_with("MVG3:"));
    structural(&dx, &dxr, "disp_x");
    structural(&dm, &dmr, "disp_mag");
    let (c, m) = assert_norm(
        &dm.scalar,
        &[&dx.scalar, &dy.scalar, &dz.scalar],
        "disp_mag norm",
    );
    assert!(c > 0 && m > 0.0, "disp_mag: non-trivial cross-check");
    let (c, _) = assert_norm(
        &drm.scalar,
        &[&dx.scalar, &dy.scalar],
        "disp_rad_mag_xy norm",
    );
    assert!(c > 0, "disp_rad_mag_xy: nodes cross-checked");

    // ── velocity / acceleration: structural ─────────────────────────
    for name in ["vel_x", "vel_y", "vel_z", "acc_x", "acc_y", "acc_z"] {
        let (g, r) = show(&mut client, &mut sub, &svc, name).await;
        structural(&g, &r, name);
    }

    // ── surfstrain: structural (its tensor numerics are owned by the
    //    core parity suite, M5 Decision 19) ───────────────────────────
    for name in [
        "surfstrainx",
        "surfstrainy",
        "surfstrainz",
        "surfstrainxy",
        "surfstrainyz",
        "surfstrainzx",
    ] {
        let (g, r) = show(&mut client, &mut sub, &svc, name).await;
        structural(&g, &r, name);
    }

    // ── nodal family tracks the state (basic1 is transient) ─────────
    let d101 = show(&mut client, &mut sub, &svc, "disp_mag").await.0.scalar;
    set_state(&mut client, &mut sub, 1).await;
    let d1 = show(&mut client, &mut sub, &svc, "disp_mag").await.0.scalar;
    assert_ne!(d1, d101, "disp_mag must differ between state 1 and 101");

    // ── `vel_x` at state 1 is identically zero (a kernel-defined
    //    same-gather fact, derived.py:1062) ───────────────────────────
    let v1 = show(&mut client, &mut sub, &svc, "vel_x").await.0;
    assert!(v1.layout.starts_with("MVG3:"), "vel_x at state 1 → MVG3");
    assert!(
        v1.scalar.iter().all(|v| !v.is_finite() || v.abs() < 1e-6),
        "vel_x at state 1 is identically zero"
    );

    // ── primal path still byte-stable: empty result → bare hull ─────
    let bare = show(&mut client, &mut sub, &svc, "").await.0;
    assert!(bare.layout.starts_with("MVG3:") && bare.scalar.is_empty());
}
