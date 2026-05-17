//! Phase 4 M4 acceptance — selection + enable/disable.
//!
//! Gating test for `planning/mili-viz/phase-4-m4.md` § "M4 acceptance
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
    blob: Vec<u8>,
    verts: usize,
    n_idx: usize,
    tri_material: Vec<u32>,
    scalar: Vec<f32>,
}

fn decode(blob: Vec<u8>, layout: &str) -> Geom {
    let magic = blob[0..4].to_vec();
    let n_verts = u64::from_le_bytes(blob[8..16].try_into().unwrap()) as usize;
    let n_idx = u64::from_le_bytes(blob[16..24].try_into().unwrap()) as usize;
    let n_tri = n_idx / 3;
    let mut off = 24 + n_verts * 3 * 4 + n_idx * 4;
    let tri_material: Vec<u32> = (0..n_tri)
        .map(|i| u32::from_le_bytes(blob[off + i * 4..off + i * 4 + 4].try_into().unwrap()))
        .collect();
    off += n_tri * 4;
    let scalar = if magic == b"MVG2" {
        let s: Vec<f32> = (0..n_verts)
            .map(|i| f32::from_le_bytes(blob[off + i * 4..off + i * 4 + 4].try_into().unwrap()))
            .collect();
        off += n_verts * 4;
        s
    } else {
        assert_eq!(magic, b"MVG1");
        Vec::new()
    };
    assert_eq!(off, blob.len(), "blob fully consumed");
    Geom {
        layout: layout.to_string(),
        blob,
        verts: n_verts,
        n_idx,
        tri_material,
        scalar,
    }
}

fn cmd(c: pb::command::Cmd) -> Request<pb::Command> {
    let mut req = Request::new(pb::Command { cmd: Some(c) });
    req.metadata_mut()
        .insert(CLIENT_ID_HEADER, "m4".parse().unwrap());
    req
}

async fn show(
    client: &mut pb::mili_viz_client::MiliVizClient<tonic::transport::Channel>,
    sub: &mut tonic::Streaming<pb::StateDelta>,
    svc: &VizService,
    result: &str,
) -> (Geom, pb::ResultState) {
    let reply = client
        .execute(cmd(pb::command::Cmd::Show(pb::Show {
            result: result.to_string(),
            component: String::new(),
            opts: HashMap::new(),
        })))
        .await
        .unwrap()
        .into_inner();
    assert!(reply.ok, "show {result} failed: {}", reply.error);
    let d = sub.message().await.unwrap().unwrap();
    assert_eq!(
        d.kind,
        pb::DeltaKind::DeltaResult as i32,
        "one DELTA_RESULT"
    );
    let Some(pb::state_delta::Payload::Result(res)) = d.payload else {
        panic!("show must broadcast a ResultState");
    };
    let g = res.geometry.clone().expect("show carries a GeometryRef");
    let geom = decode(svc.fetch_geometry(&g.flight_ticket).unwrap(), &g.layout);
    assert_eq!(
        geom.n_idx as u64, g.num_indices,
        "num_indices is post-filter"
    );
    (geom, res)
}

async fn material(
    client: &mut pb::mili_viz_client::MiliVizClient<tonic::transport::Channel>,
    sub: &mut tonic::Streaming<pb::StateDelta>,
    enable: bool,
    mat: u32,
) {
    client
        .execute(cmd(pb::command::Cmd::Material(pb::MaterialVisibility {
            enable,
            class_name: String::new(),
            material: Some(mat),
        })))
        .await
        .unwrap();
    let d = sub.message().await.unwrap().unwrap();
    assert_eq!(
        d.kind,
        pb::DeltaKind::DeltaMaterials as i32,
        "enable/disable → exactly one DELTA_MATERIALS"
    );
}

#[tokio::test]
async fn material_visibility_and_selection() {
    let path = corpus_path(&["serial", "basic1", "basic1.pltA"]);
    if !path.exists() {
        eprintln!("skip: serial/basic1 absent (run scripts/setup-parity.sh)");
        return;
    }
    let svc = VizService::builder().build();
    let (mut client, _h) = spawn_in_process(svc.clone()).await.unwrap();

    assert!(
        client
            .execute(cmd(pb::command::Cmd::Load(pb::Load {
                root: path.to_string_lossy().into_owned(),
            })))
            .await
            .unwrap()
            .into_inner()
            .ok
    );

    let mut sub = client
        .subscribe(Request::new(pb::SubscribeRequest::default()))
        .await
        .unwrap()
        .into_inner();
    let _snap = sub.message().await.unwrap().unwrap();

    // ── all-visible baseline hull ────────────────────────────────────
    let (base, _) = show(&mut client, &mut sub, &svc, "").await;
    assert_eq!(base.layout, "MVG1:verts_f32x3+idx_u32+trimat_u32");
    assert!(base.n_idx > 0 && base.n_idx % 3 == 0);
    // Pick the most-frequent material to maximize the filtered delta.
    let mut freq: HashMap<u32, usize> = HashMap::new();
    for &m in &base.tri_material {
        *freq.entry(m).or_default() += 1;
    }
    let (&victim, _) = freq.iter().max_by_key(|(_, &c)| c).unwrap();
    let distinct_mats = freq.len();

    // ── disable a material → its triangles leave the blob ────────────
    material(&mut client, &mut sub, false, victim).await;
    let (off, _) = show(&mut client, &mut sub, &svc, "").await;
    assert!(
        off.n_idx < base.n_idx,
        "disabling material {victim} must shrink num_indices ({} !< {})",
        off.n_idx,
        base.n_idx
    );
    assert!(
        !off.tri_material.contains(&victim),
        "no surviving triangle keeps the disabled material"
    );
    assert_eq!(off.verts, base.verts, "num_vertices unchanged by filter");
    if distinct_mats == 1 {
        assert_eq!(off.n_idx, 0, "sole material disabled → empty hull");
    }

    // ── re-enable → byte-identical to the pre-disable hull ───────────
    material(&mut client, &mut sub, true, victim).await;
    let (on, _) = show(&mut client, &mut sub, &svc, "").await;
    assert_eq!(
        on.blob, base.blob,
        "re-enable at the same state restores a byte-identical blob"
    );

    // ── filter composes with MVG2 (scalar untouched) ─────────────────
    let (mvg2_all, res_all) = show(&mut client, &mut sub, &svc, "sand").await;
    if mvg2_all.layout.starts_with("MVG2") {
        material(&mut client, &mut sub, false, victim).await;
        let (mvg2_off, res_off) = show(&mut client, &mut sub, &svc, "sand").await;
        assert_eq!(
            mvg2_off.layout,
            "MVG2:verts_f32x3+idx_u32+trimat_u32+scalar_f32"
        );
        assert_eq!(
            mvg2_off.scalar.len(),
            mvg2_off.verts,
            "per-vertex scalar array length is unchanged by the filter"
        );
        assert_eq!(
            mvg2_off.scalar, mvg2_all.scalar,
            "the scalar field is byte-stable under material filtering"
        );
        assert!(
            res_off.min.to_bits() == res_all.min.to_bits()
                && res_off.max.to_bits() == res_all.max.to_bits(),
            "ResultState range is the M3 data range (unchanged by filter)"
        );
        assert!(mvg2_off.n_idx < mvg2_all.n_idx, "MVG2 triangle list shrank");
        material(&mut client, &mut sub, true, victim).await;
    }

    // ── select / clrsel: metadata-only, one DELTA_SELECTION each ─────
    let sel_reply = client
        .execute(cmd(pb::command::Cmd::Select(pb::Select {
            class_name: "brick".into(),
            range: "1-10,20".into(),
        })))
        .await
        .unwrap()
        .into_inner();
    assert!(sel_reply.ok);
    let d = sub.message().await.unwrap().unwrap();
    assert_eq!(d.kind, pb::DeltaKind::DeltaSelection as i32);
    let Some(pb::state_delta::Payload::Selection(sel)) = d.payload else {
        panic!("select broadcasts a SelectionState");
    };
    assert_eq!(
        sel.by_class.get("brick").map(String::as_str),
        Some("1-10,20")
    );

    // A fresh subscriber's opening Snapshot carries the live selection.
    let mut sub2 = client
        .subscribe(Request::new(pb::SubscribeRequest::default()))
        .await
        .unwrap()
        .into_inner();
    let snap = sub2.message().await.unwrap().unwrap();
    let Some(pb::state_delta::Payload::Snapshot(s)) = snap.payload else {
        panic!("first message is the DELTA_SNAPSHOT");
    };
    assert_eq!(
        s.selection
            .and_then(|sl| sl.by_class.get("brick").cloned())
            .as_deref(),
        Some("1-10,20"),
        "late joiner sees the selection (metadata-only contract)"
    );

    // clrsel with an empty class clears the whole selection (griz).
    assert!(
        client
            .execute(cmd(pb::command::Cmd::Clrsel(pb::ClearSelection {
                class_name: String::new(),
            })))
            .await
            .unwrap()
            .into_inner()
            .ok
    );
    let d = sub.message().await.unwrap().unwrap();
    assert_eq!(d.kind, pb::DeltaKind::DeltaSelection as i32);
    let Some(pb::state_delta::Payload::Selection(sel)) = d.payload else {
        panic!("clrsel broadcasts a SelectionState");
    };
    assert!(sel.by_class.is_empty(), "clrsel (no class) clears all");

    // ── selection never edited the geometry blob ─────────────────────
    let (after_sel, _) = show(&mut client, &mut sub, &svc, "").await;
    assert_eq!(
        after_sel.blob, base.blob,
        "selection is metadata-only — the hull is unchanged"
    );
}
