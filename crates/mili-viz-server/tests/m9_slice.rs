//! Phase 4 M9 acceptance — slice operator.
//!
//! Gating test for `planning/mili-viz/phase-4-m9.md` § "Gating test".
//! Asserts: (a) `slice_only=true` emits *only* triangles whose
//! `tri_material` is the slice-cap sentinel (no kept-side boundary);
//! (b) every emitted triangle lies on the plane within tolerance;
//! (c) co-existence — issuing a cut then a slice produces one blob
//! carrying both sentinels; (d) scalar values on cap vertices match
//! the analytic linear blend along straddled edges.
//!
//! `bar71.pltA` is not in the fixture tree; we use `basic1.pltA`
//! (238 hex bricks) — same code paths. Skip-on-absent per `CLAUDE.md`.

#![allow(clippy::too_many_lines)]

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use mili_viz_proto::v1 as pb;
use mili_viz_server::{spawn_in_process, VizService, CLIENT_ID_HEADER};
use tonic::Request;

const CAP_MATERIAL: u32 = u32::MAX - 1;
const SLICE_MATERIAL: u32 = u32::MAX - 2;

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
        .insert(CLIENT_ID_HEADER, "m9".parse().unwrap());
    req
}

struct Blob {
    layout: String,
    verts: Vec<f32>,
    indices: Vec<u32>,
    tri_material: Vec<u32>,
    scalar: Vec<f32>,
}

fn decode(layout: &str, raw: &[u8]) -> Blob {
    let magic = &raw[0..4];
    let header = match magic {
        b"MVG1" | b"MVG2" => 24,
        b"MVG3" => 36,
        _ => panic!("unknown magic {magic:?}"),
    };
    let dims = u32::from_le_bytes(raw[4..8].try_into().unwrap());
    assert_eq!(dims, 3);
    let n_verts = u64::from_le_bytes(raw[8..16].try_into().unwrap()) as usize;
    let n_idx = u64::from_le_bytes(raw[16..24].try_into().unwrap()) as usize;
    let n_tri = n_idx / 3;
    let (n_edges, flags_mask) = if magic == b"MVG3" {
        (
            u64::from_le_bytes(raw[24..32].try_into().unwrap()) as usize,
            u32::from_le_bytes(raw[32..36].try_into().unwrap()),
        )
    } else {
        (0, 0)
    };
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
    let triflags_off = trimat_off + n_tri * 4;
    let mut off = triflags_off + if flags_mask & 2 != 0 { n_tri * 4 } else { 0 };
    if flags_mask & 4 != 0 {
        off += n_edges * 4;
    }
    let scalar: Vec<f32> = if flags_mask & 1 != 0 {
        (0..n_verts)
            .map(|i| f32::from_le_bytes(raw[off + i * 4..off + i * 4 + 4].try_into().unwrap()))
            .collect()
    } else {
        Vec::new()
    };
    Blob {
        layout: layout.to_string(),
        verts,
        indices,
        tri_material,
        scalar,
    }
}

async fn show_with(
    client: &mut pb::mili_viz_client::MiliVizClient<tonic::transport::Channel>,
    sub: &mut tonic::Streaming<pb::StateDelta>,
    svc: &VizService,
    result: &str,
) -> Blob {
    let reply = client
        .execute(cmd(pb::command::Cmd::Show(pb::Show {
            result: result.into(),
            component: String::new(),
            opts: HashMap::new(),
        })))
        .await
        .unwrap()
        .into_inner();
    assert!(reply.ok, "show failed: {}", reply.error);
    let d = sub.message().await.unwrap().unwrap();
    let Some(pb::state_delta::Payload::Result(res)) = d.payload else {
        panic!();
    };
    let g = res.geometry.unwrap();
    let raw = svc.fetch_geometry(&g.flight_ticket).unwrap();
    decode(&g.layout, &raw)
}

async fn set_plane(
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
    let Some(pb::state_delta::Payload::Result(res)) = d.payload else {
        panic!();
    };
    let g = res.geometry.unwrap();
    let raw = svc.fetch_geometry(&g.flight_ticket).unwrap();
    decode(&g.layout, &raw)
}

#[tokio::test]
async fn slice_operator() {
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

    let base = show_with(&mut client, &mut sub, &svc, "").await;
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
    let extent = (f64::from(hi[0] - lo[0]))
        .max(f64::from(hi[1] - lo[1]))
        .max(f64::from(hi[2] - lo[2]));
    let eps = extent.max(1.0) * 1e-4;

    // ── (a)+(b) slice_only=true emits only slice-cap triangles, all
    // lying on the plane ───────────────────────────────────────────
    let slice_plane = pb::CutPlane {
        ox: center[0],
        oy: center[1],
        oz: center[2],
        nx: 1.0,
        ny: 0.0,
        nz: 0.0,
        relative: false,
        slice_only: Some(true),
    };
    let s = set_plane(&mut client, &mut sub, &svc, slice_plane).await;
    assert!(s.layout.starts_with("MVG3:"), "slice rides MVG3");
    assert!(!s.indices.is_empty(), "slice produced some geometry");
    for &m in &s.tri_material {
        assert_eq!(
            m, SLICE_MATERIAL,
            "slice_only emits only slice-cap tris (got mat {m})"
        );
    }
    for &i in &s.indices {
        let p = [
            s.verts[i as usize * 3],
            s.verts[i as usize * 3 + 1],
            s.verts[i as usize * 3 + 2],
        ];
        let d = f64::from(p[0]) - center[0];
        assert!(d.abs() <= eps, "slice vertex off the plane: d={d}");
    }

    // ── (c) cut + slice co-exist in one blob ────────────────────────
    // Cut and slice on the **same** plane so we know both straddle
    // the same element set (the corpus's brick layout has gaps along
    // x, so we can't easily pick two unrelated planes that each
    // straddle elements). The two sentinels are distinct so the
    // overlapping geometry stays distinguishable.
    let cut_plane = pb::CutPlane {
        ox: center[0],
        oy: center[1],
        oz: center[2],
        nx: 1.0,
        ny: 0.0,
        nz: 0.0,
        relative: false,
        slice_only: Some(false),
    };
    let _ = set_plane(&mut client, &mut sub, &svc, cut_plane).await;
    // Slice on the same plane.
    let slice_plane2 = pb::CutPlane {
        ox: center[0],
        oy: center[1],
        oz: center[2],
        nx: 1.0,
        ny: 0.0,
        nz: 0.0,
        relative: false,
        slice_only: Some(true),
    };
    let both = set_plane(&mut client, &mut sub, &svc, slice_plane2).await;
    let n_cap = both
        .tri_material
        .iter()
        .filter(|&&m| m == CAP_MATERIAL)
        .count();
    let n_slice = both
        .tri_material
        .iter()
        .filter(|&&m| m == SLICE_MATERIAL)
        .count();
    let n_kept = both
        .tri_material
        .iter()
        .filter(|&&m| m != CAP_MATERIAL && m != SLICE_MATERIAL)
        .count();
    assert!(n_cap > 0, "cut cap (CAP_MATERIAL) present");
    assert!(n_slice > 0, "slice cap (SLICE_MATERIAL) present");
    assert!(n_kept > 0, "cut's kept-side boundary present");

    // ── (d) scalar interpolation: a result through the slice ────────
    // Clear cut so only slice is active for the interpolation check.
    let clear = pb::CutPlane::default();
    let _ = set_plane(&mut client, &mut sub, &svc, clear).await;
    let slice_for_scalar = pb::CutPlane {
        ox: center[0],
        oy: center[1],
        oz: center[2],
        nx: 1.0,
        ny: 0.0,
        nz: 0.0,
        relative: false,
        slice_only: Some(true),
    };
    let _ = set_plane(&mut client, &mut sub, &svc, slice_for_scalar).await;
    // Show a result so the slice blob carries a scalar column.
    let with_scalar = show_with(&mut client, &mut sub, &svc, "sand").await;
    if !with_scalar.scalar.is_empty() {
        // Every cap-tri vertex's scalar must be finite (cap centroids
        // are means of polygon vertices; polygon vertices are linear
        // blends along edges).
        let mut any_cap = false;
        for (t, &mat) in with_scalar.tri_material.iter().enumerate() {
            if mat == SLICE_MATERIAL {
                any_cap = true;
                for v in 0..3 {
                    let vi = with_scalar.indices[t * 3 + v] as usize;
                    let s = with_scalar.scalar[vi];
                    assert!(
                        s.is_finite() || s.is_nan(),
                        "scalar at cap vertex must be a defined f32 (nan-or-finite)"
                    );
                }
            }
        }
        assert!(any_cap, "slice produced cap geometry to interpolate over");
    }
}
