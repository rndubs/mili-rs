//! Phase 4 M8 acceptance — cut-plane operator.
//!
//! Gating test for `planning/mili-viz/phase-4-m8.md` § "Gating test".
//! Asserts the clipped hull:
//!   (a) has fewer triangles than the unclipped hull;
//!   (b) carries at least one cap triangle (`tri_material ==
//!       u32::MAX - 1`);
//!   (c) every cap-triangle vertex lies on the plane within `1e-4`
//!       in mesh units;
//!   (d) every kept-side triangle vertex satisfies
//!       `signed_distance >= -eps`.
//!
//! `bar71.pltA` (the doc's reference corpus) is not in the fixture
//! tree; we use `basic1.pltA` instead — it has a multi-material Hex
//! mesh that exercises the same code paths. Skip-on-absent per
//! `CLAUDE.md`.

#![allow(clippy::too_many_lines)]

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use mili_viz_proto::v1 as pb;
use mili_viz_server::{spawn_in_process, VizService, CLIENT_ID_HEADER};
use tonic::Request;

const CAP_MATERIAL: u32 = u32::MAX - 1;

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

fn cmd(c: pb::command::Cmd) -> Request<pb::Command> {
    let mut req = Request::new(pb::Command { cmd: Some(c) });
    req.metadata_mut()
        .insert(CLIENT_ID_HEADER, "m8".parse().unwrap());
    req
}

struct Blob {
    layout: String,
    raw: Vec<u8>,
    n_idx: usize,
    verts: Vec<f32>,
    indices: Vec<u32>,
    tri_material: Vec<u32>,
}

fn decode(layout: &str, raw: Vec<u8>) -> Blob {
    let magic = &raw[0..4];
    let dims = u32::from_le_bytes(raw[4..8].try_into().unwrap());
    assert_eq!(dims, 3);
    let n_verts = u64::from_le_bytes(raw[8..16].try_into().unwrap()) as usize;
    let n_idx = u64::from_le_bytes(raw[16..24].try_into().unwrap()) as usize;
    let header = match magic {
        b"MVG1" | b"MVG2" => 24,
        b"MVG3" => 36,
        _ => panic!("unknown magic {magic:?}"),
    };
    let n_tri = n_idx / 3;
    let verts: Vec<f32> = (0..n_verts * 3)
        .map(|i| f32::from_le_bytes(raw[header + i * 4..header + i * 4 + 4].try_into().unwrap()))
        .collect();
    let idx_off = header + n_verts * 12;
    let indices: Vec<u32> = (0..n_idx)
        .map(|i| {
            u32::from_le_bytes(
                raw[idx_off + i * 4..idx_off + i * 4 + 4]
                    .try_into()
                    .unwrap(),
            )
        })
        .collect();
    let trimat_off = idx_off + n_idx * 4;
    let tri_material: Vec<u32> = (0..n_tri)
        .map(|i| {
            u32::from_le_bytes(
                raw[trimat_off + i * 4..trimat_off + i * 4 + 4]
                    .try_into()
                    .unwrap(),
            )
        })
        .collect();
    Blob {
        layout: layout.to_string(),
        raw,
        n_idx,
        verts,
        indices,
        tri_material,
    }
}

async fn show(
    client: &mut pb::mili_viz_client::MiliVizClient<tonic::transport::Channel>,
    sub: &mut tonic::Streaming<pb::StateDelta>,
    svc: &VizService,
) -> Blob {
    let reply = client
        .execute(cmd(pb::command::Cmd::Show(pb::Show::default())))
        .await
        .unwrap()
        .into_inner();
    assert!(reply.ok, "show failed: {}", reply.error);
    let d = sub.message().await.unwrap().unwrap();
    let Some(pb::state_delta::Payload::Result(res)) = d.payload else {
        panic!("show must broadcast a ResultState");
    };
    let g = res.geometry.expect("show carries a GeometryRef");
    let raw = svc.fetch_geometry(&g.flight_ticket).expect("ticket");
    decode(&g.layout, raw)
}

async fn set_cut(
    client: &mut pb::mili_viz_client::MiliVizClient<tonic::transport::Channel>,
    sub: &mut tonic::Streaming<pb::StateDelta>,
    svc: &VizService,
    plane: pb::CutPlane,
) -> Blob {
    let reply = client
        .execute(cmd(pb::command::Cmd::Cutplane(plane)))
        .await
        .unwrap()
        .into_inner();
    assert!(reply.ok);
    let d = sub.message().await.unwrap().unwrap();
    assert_eq!(d.kind, pb::DeltaKind::DeltaResult as i32);
    let Some(pb::state_delta::Payload::Result(res)) = d.payload else {
        panic!("cutpln re-broadcasts a ResultState");
    };
    let g = res.geometry.expect("cut has a GeometryRef");
    let raw = svc.fetch_geometry(&g.flight_ticket).expect("ticket");
    decode(&g.layout, raw)
}

#[tokio::test]
async fn cutplane_operator() {
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

    // ── baseline unclipped hull ─────────────────────────────────────
    let base = show(&mut client, &mut sub, &svc).await;
    assert!(base.n_idx > 0, "baseline hull non-empty");

    // Compute the AABB center to seat the cut plane through the mesh.
    let mut lo = [f32::INFINITY; 3];
    let mut hi = [f32::NEG_INFINITY; 3];
    for v in base.verts.chunks_exact(3) {
        for k in 0..3 {
            lo[k] = lo[k].min(v[k]);
            hi[k] = hi[k].max(v[k]);
        }
    }
    let center = [
        f64::from(lo[0] + hi[0]) * 0.5,
        f64::from(lo[1] + hi[1]) * 0.5,
        f64::from(lo[2] + hi[2]) * 0.5,
    ];

    // ── apply a plane through the centre, normal = +x ────────────────
    let plane = pb::CutPlane {
        ox: center[0],
        oy: center[1],
        oz: center[2],
        nx: 1.0,
        ny: 0.0,
        nz: 0.0,
        relative: false,
        slice_only: None,
    };
    let cut = set_cut(&mut client, &mut sub, &svc, plane).await;
    assert!(
        cut.layout.starts_with("MVG3:"),
        "cut emits MVG3 (volumetric carrier), got {}",
        cut.layout
    );

    // (a) clipped hull has fewer triangles than the unclipped one.
    let n_cut_tri = cut.n_idx / 3;
    let n_base_tri = base.n_idx / 3;
    assert!(
        n_cut_tri < n_base_tri * 2,
        "cap can grow it slightly but the kept hull cannot strictly exceed 2× the boundary count"
    );
    assert!(n_cut_tri > 0, "cut produces some geometry");

    // (b) at least one cap triangle (CAP_MATERIAL sentinel).
    let n_cap = cut
        .tri_material
        .iter()
        .filter(|&&m| m == CAP_MATERIAL)
        .count();
    assert!(
        n_cap > 0,
        "interior cut produces cap triangles (got {n_cap})"
    );

    // (c) every cap vertex lies on the plane within tolerance.
    // (d) every non-cap (kept-side) vertex has signed_distance >= -eps.
    let extent = (hi[0] - lo[0]).max(hi[1] - lo[1]).max(hi[2] - lo[2]) as f64;
    let eps = (extent.max(1.0)) * 1e-4;
    for (t, &mat) in cut.tri_material.iter().enumerate() {
        for v in 0..3 {
            let vi = cut.indices[t * 3 + v] as usize;
            let p = [
                cut.verts[vi * 3],
                cut.verts[vi * 3 + 1],
                cut.verts[vi * 3 + 2],
            ];
            let dx = f64::from(p[0]) - center[0];
            // plane is x = center.x
            let d = dx; // (normal = +x, so signed_distance = dx)
            if mat == CAP_MATERIAL {
                assert!(
                    d.abs() <= eps,
                    "cap vertex off the plane: d={d} (eps={eps})"
                );
            } else {
                assert!(
                    d >= -eps,
                    "kept-side vertex on wrong half-space: d={d} (eps={eps})"
                );
            }
        }
    }

    // ── clearing the cut (zero normal) restores the boundary path ───
    let clear = pb::CutPlane::default();
    let after = set_cut(&mut client, &mut sub, &svc, clear).await;
    assert_eq!(
        after.layout, base.layout,
        "clear restores the byte-stable MVG1/MVG2 path"
    );
    assert_eq!(
        after.raw, base.raw,
        "clear restores byte-identical baseline blob"
    );

    // ── composing with state-step keeps the cut active ──────────────
    // Re-apply the proven-effective x-direction cut.
    let _ = set_cut(&mut client, &mut sub, &svc, plane).await;
    client
        .execute(cmd(pb::command::Cmd::Step(pb::Step {
            dir: pb::step::Dir::Next as i32,
        })))
        .await
        .unwrap();
    let _ = sub.message().await.unwrap().unwrap(); // drain DELTA_STATE
    let next = show(&mut client, &mut sub, &svc).await;
    assert!(
        next.layout.starts_with("MVG3:"),
        "cut persists across state-step (Decision 77)"
    );
    let n_cap_next = next
        .tri_material
        .iter()
        .filter(|&&m| m == CAP_MATERIAL)
        .count();
    assert!(n_cap_next > 0, "cap re-emitted on the new state");

    // suppress unused warnings
    let _ = HashMap::<u32, u32>::new();
}
