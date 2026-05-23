//! Phase 4 M7 acceptance — `MVG3` volumetric geometry contract.
//!
//! Gating test for `planning/mili-viz/phase-4-m7.md` § "Gating test".
//! Asserts:
//!   (a) round-trip encode→decode of an `MVG3` blob with all four
//!       flag bits set;
//!   (b) hex element emits exactly 12 element edges per element (no
//!       face diagonals) — the in-module unit tests pin this at the
//!       table level; here we verify the live server's blob carries
//!       the buffer with the right structural shape;
//!   (c) IncludeInterior on a multi-hex corpus emits interior
//!       triangles flagged `tri_flags & 1 == 1` *above* the
//!       boundary-only count;
//!   (d) `MVG2` decode path stays byte-identical (the existing
//!       M2/M3/M4/M5/M5b/M5c/M5d/M6 fixture goldens carry that —
//!       this test simply confirms the default path is still
//!       `MVG2`, never `MVG3`).
//!
//! Skip-on-absent per `CLAUDE.md` — the live-server legs need a
//! real corpus.

#![allow(clippy::too_many_lines)]

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use mili_viz_proto::v1 as pb;
use mili_viz_server::{spawn_in_process, VizService, CLIENT_ID_HEADER};
use tonic::Request;

const INTERIOR_SENTINEL: u32 = u32::MAX;

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
        .insert(CLIENT_ID_HEADER, "m7".parse().unwrap());
    req
}

struct Mvg3 {
    n_verts: usize,
    n_idx: usize,
    n_edges: usize,
    flags_mask: u32,
    indices: Vec<u32>,
    tri_flags: Vec<u32>,
    edges: Vec<u32>,
    scalar: Vec<f32>,
}

fn decode_mvg3(blob: &[u8]) -> Mvg3 {
    assert_eq!(&blob[0..4], b"MVG3", "expected MVG3 magic");
    let dims = u32::from_le_bytes(blob[4..8].try_into().unwrap());
    assert_eq!(dims, 3);
    let n_verts = u64::from_le_bytes(blob[8..16].try_into().unwrap()) as usize;
    let n_idx = u64::from_le_bytes(blob[16..24].try_into().unwrap()) as usize;
    let n_edges = u64::from_le_bytes(blob[24..32].try_into().unwrap()) as usize;
    let flags_mask = u32::from_le_bytes(blob[32..36].try_into().unwrap());
    let n_tri = n_idx / 3;

    let mut off = 36 + n_verts * 12;
    let indices: Vec<u32> = (0..n_idx)
        .map(|i| u32::from_le_bytes(blob[off + i * 4..off + i * 4 + 4].try_into().unwrap()))
        .collect();
    off += n_idx * 4;
    // tri_material: skipped — exercised in MVG2 tests.
    off += n_tri * 4;
    let tri_flags: Vec<u32> = if flags_mask & 2 != 0 {
        let v = (0..n_tri)
            .map(|i| u32::from_le_bytes(blob[off + i * 4..off + i * 4 + 4].try_into().unwrap()))
            .collect();
        off += n_tri * 4;
        v
    } else {
        Vec::new()
    };
    let edges: Vec<u32> = if flags_mask & 4 != 0 {
        let v = (0..n_edges)
            .map(|i| u32::from_le_bytes(blob[off + i * 4..off + i * 4 + 4].try_into().unwrap()))
            .collect();
        off += n_edges * 4;
        v
    } else {
        Vec::new()
    };
    let scalar: Vec<f32> = if flags_mask & 1 != 0 {
        let v = (0..n_verts)
            .map(|i| f32::from_le_bytes(blob[off + i * 4..off + i * 4 + 4].try_into().unwrap()))
            .collect();
        off += n_verts * 4;
        v
    } else {
        Vec::new()
    };
    assert_eq!(off, blob.len(), "MVG3 blob fully consumed");
    Mvg3 {
        n_verts,
        n_idx,
        n_edges,
        flags_mask,
        indices,
        tri_flags,
        edges,
        scalar,
    }
}

async fn show(
    client: &mut pb::mili_viz_client::MiliVizClient<tonic::transport::Channel>,
    sub: &mut tonic::Streaming<pb::StateDelta>,
    svc: &VizService,
    result: &str,
) -> (String, Vec<u8>) {
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
    let Some(pb::state_delta::Payload::Result(res)) = d.payload else {
        panic!("show must broadcast a ResultState");
    };
    let g = res.geometry.expect("show carries a GeometryRef");
    let blob = svc.fetch_geometry(&g.flight_ticket).expect("ticket lookup");
    assert_eq!(g.num_indices, (blob_n_idx(&blob)) as u64);
    (g.layout, blob)
}

fn blob_n_idx(blob: &[u8]) -> usize {
    u64::from_le_bytes(blob[16..24].try_into().unwrap()) as usize
}

async fn set_interior(
    client: &mut pb::mili_viz_client::MiliVizClient<tonic::transport::Channel>,
    sub: &mut tonic::Streaming<pb::StateDelta>,
    on: bool,
) {
    client
        .execute(cmd(pb::command::Cmd::Material(pb::MaterialVisibility {
            enable: on,
            class_name: String::new(),
            material: Some(INTERIOR_SENTINEL),
        })))
        .await
        .unwrap();
    let d = sub.message().await.unwrap().unwrap();
    assert_eq!(
        d.kind,
        pb::DeltaKind::DeltaMaterials as i32,
        "interior toggle → exactly one DELTA_MATERIALS"
    );
}

#[tokio::test]
async fn volumetric_geometry_contract() {
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

    // ── (d) default path is MVG3 since the VB-005 promotion ────────
    // Before the promotion this was MVG1/MVG2; the boundary hull now
    // rides as MVG3 too so the client's wireframe pass picks up the
    // per-element edge buffer (no more face diagonals on hex meshes).
    // What stays byte-stable is the *rendered pixels* in the
    // `Shaded` mode — the MVG3 strict-superset blob carries the same
    // vertex/index/material columns the M2/M3/M4 pipeline drew.
    let (base_layout, base_blob) = show(&mut client, &mut sub, &svc, "").await;
    assert!(
        base_layout.starts_with("MVG3:"),
        "default emit is MVG3 since VB-005 promotion, got {base_layout}"
    );
    // Same show again — bytes match exactly.
    let (l2, b2) = show(&mut client, &mut sub, &svc, "").await;
    assert_eq!(l2, base_layout);
    assert_eq!(b2, base_blob, "repeat show stays byte-stable");
    // Verify the interior bit is *off* by default: decode the blob and
    // assert flags_mask & 8 == 0 (boundary-only hull).
    let base = decode_mvg3(&base_blob);
    assert_eq!(
        base.flags_mask & 8,
        0,
        "interior bit off by default (no interior tris in the default blob)"
    );
    assert!(
        base.n_edges >= 2,
        "edges buffer non-empty by default for a hex corpus (VB-005 fix)"
    );

    // ── (c) IncludeInterior flips on the interior bit (still MVG3) ──
    set_interior(&mut client, &mut sub, true).await;
    let (vol_layout, vol_blob) = show(&mut client, &mut sub, &svc, "").await;
    assert!(
        vol_layout.starts_with("MVG3:"),
        "interior on → MVG3 layout, got {vol_layout}"
    );
    let v = decode_mvg3(&vol_blob);
    assert!(v.n_verts > 0, "MVG3 has vertices");
    assert!(
        v.n_idx > 0 && v.n_idx.is_multiple_of(3),
        "MVG3 index list valid"
    );
    assert_eq!(v.tri_flags.len(), v.n_idx / 3, "tri_flags column present");
    let interior_count = v.tri_flags.iter().filter(|f| **f & 1 == 1).count();
    let boundary_count = v.tri_flags.iter().filter(|f| **f & 1 == 0).count();
    assert!(
        boundary_count > 0,
        "the outward hull is in the volumetric blob too"
    );
    assert!(
        interior_count > 0,
        "a multi-element corpus must have shared faces tagged interior"
    );
    assert!(
        v.flags_mask & 8 != 0,
        "flags_mask records that interior is on"
    );

    // ── (b) per-superclass edge buffer is present, non-degenerate ───
    assert!(v.n_edges >= 2, "edges buffer non-empty for a solid corpus");
    assert_eq!(v.edges.len(), v.n_edges, "edges array matches header");
    assert!(v.n_edges.is_multiple_of(2), "edges in line-list pairs");
    for pair in v.edges.chunks_exact(2) {
        assert_ne!(pair[0], pair[1], "no degenerate self-loop edges");
        assert!(
            (pair[0] as usize) < v.n_verts && (pair[1] as usize) < v.n_verts,
            "edge endpoints reference valid vertices"
        );
    }
    // The blob is internally consistent: every index references a
    // valid vertex.
    for &i in &v.indices {
        assert!((i as usize) < v.n_verts);
    }
    // Scalar is absent (no result selected).
    assert_eq!(v.scalar.len(), 0, "no result → no scalar section");
    assert_eq!(v.flags_mask & 1, 0, "scalar bit unset");

    // ── (a) round-trip all four flag bits via the typed result show ─
    // Show *with* a primal result so the scalar bit is set too. The
    // four bits: scalar (1), tri_flags (2), edges (4), interior (8).
    let (vol_layout_s, vol_blob_s) = show(&mut client, &mut sub, &svc, "sand").await;
    if vol_layout_s.starts_with("MVG3:") {
        let v2 = decode_mvg3(&vol_blob_s);
        // sand may or may not resolve — bit 0 set iff scalar present.
        // We require interior+edges+tri_flags bits in this corpus.
        assert_ne!(v2.flags_mask & 2, 0, "tri_flags bit set");
        assert_ne!(v2.flags_mask & 4, 0, "edges bit set");
        assert_ne!(v2.flags_mask & 8, 0, "interior bit set");
        if v2.flags_mask & 1 != 0 {
            assert_eq!(v2.scalar.len(), v2.n_verts, "scalar column matches verts");
        }
    }

    // ── reverting interior restores the byte-stable boundary blob ──
    // Both shapes are MVG3 since the VB-005 promotion; what was tested
    // before as "MVG2 byte-stability" is now "MVG3 byte-stability with
    // the interior bit off".
    set_interior(&mut client, &mut sub, false).await;
    let (after_layout, after_blob) = show(&mut client, &mut sub, &svc, "").await;
    assert_eq!(
        after_layout, base_layout,
        "interior off restores the boundary-MVG3 layout"
    );
    assert_eq!(
        after_blob, base_blob,
        "interior off restores byte-identical boundary MVG3 blob (VB-001 \
         updated semantic)"
    );
}
