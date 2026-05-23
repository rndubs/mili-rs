//! Phase 4 M5 follow-up acceptance — eigenvalue-based derived families
//! (principal stress/strain, deviatoric, max-shear, volumetric strain).
//!
//! Gating test for `planning/mili-viz/phase-4-m5b.md` § "M5b
//! acceptance gate". The eigensolver itself is already bit-exact vs
//! the `mili` Python package in the `mili-rs` core parity suite
//! (phase-4-m5.md Decision 19); this test validates the **viz
//! routing** via algebraic invariants that ride a single shared
//! derived primal gather — eigenvalue ordering, relative deviatoric
//! tracelessness, the max-shear relation (phase-4-m5b.md Decision 24;
//! cross-cardinality "trace" checks are deliberately avoided — they
//! trip the IP-sampling skew on the IP-inconsistent corpus).
//! Skip-on-absent per CLAUDE.md.

#![allow(clippy::too_many_lines)] // one end-to-end acceptance scenario
#![allow(clippy::many_single_char_names)] // x/y/z = eigenvalues, a/b/c = the three served fields

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
) -> (Geom, pb::ResultState) {
    let mut req = Request::new(pb::Command {
        cmd: Some(pb::command::Cmd::Show(pb::Show {
            result: result.to_string(),
            component: String::new(),
            opts: HashMap::new(),
        })),
    });
    req.metadata_mut()
        .insert(CLIENT_ID_HEADER, "m5b".parse().unwrap());
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
        .insert(CLIENT_ID_HEADER, "m5b".parse().unwrap());
    client.execute(req).await.unwrap();
    let _ = sub.message().await.unwrap().unwrap();
}

/// Per-node `|a - b|` check over finite samples on both fields, with a
/// relative-to-scale f32 tolerance. Returns the count of compared nodes
/// and the max magnitude seen so the caller can assert non-triviality.
fn assert_identity(a: &[f32], b: &[f32], label: &str) -> (usize, f64) {
    assert_eq!(a.len(), b.len(), "{label}: per-vertex length mismatch");
    let mut compared = 0usize;
    let mut max_abs = 0.0f64;
    for i in 0..a.len() {
        let (x, y) = (a[i], b[i]);
        if x.is_finite() && y.is_finite() {
            let scale = f64::from(x).abs().max(f64::from(y).abs()).max(1.0);
            assert!(
                (f64::from(x) - f64::from(y)).abs() <= 1e-3 * scale,
                "{label}: node {i}: {x} != {y}"
            );
            compared += 1;
            max_abs = max_abs.max(f64::from(x).abs());
        }
    }
    (compared, max_abs)
}

/// Per-node `lo .. hi` are descending (`a ≥ b ≥ c`) over finite
/// samples, within an f32 slack. The M3 nodal average is a per-node
/// mean and a mean preserves order, so the served eigenvalue fields
/// must stay ordered node-by-node. Returns the count of compared
/// nodes. This is skew-free: all three fields ride the *same* derived
/// primal gather, so it never trips the integration-point sampling
/// difference a different-cardinality cross-field check would.
fn assert_descending(a: &[f32], b: &[f32], c: &[f32], label: &str) -> usize {
    assert_eq!(a.len(), b.len(), "{label}: length mismatch");
    assert_eq!(b.len(), c.len(), "{label}: length mismatch");
    let mut n = 0usize;
    for i in 0..a.len() {
        let (x, y, z) = (a[i], b[i], c[i]);
        if x.is_finite() && y.is_finite() && z.is_finite() {
            let s1 = f64::from(x).abs().max(f64::from(y).abs()).max(1.0);
            let s2 = f64::from(y).abs().max(f64::from(z).abs()).max(1.0);
            assert!(
                f64::from(x) >= f64::from(y) - 1e-3 * s1
                    && f64::from(y) >= f64::from(z) - 1e-3 * s2,
                "{label}: node {i} not descending: {x} {y} {z}"
            );
            n += 1;
        }
    }
    n
}

/// Per-node `a+b+c ≈ 0` *relative to the eigenvalue magnitudes*
/// (`|a|+|b|+|c|`). Deviatoric eigenvalues sum to the (zero) trace, but
/// each is O(stress) and the cancellation is in f32 over a nodal
/// average, so the meaningful, skew-free statement is "the residual is
/// negligible against the magnitudes", not "< an absolute epsilon".
/// Returns the count of compared nodes.
fn assert_traceless(a: &[f32], b: &[f32], c: &[f32], label: &str) -> usize {
    assert_eq!(a.len(), b.len(), "{label}: length mismatch");
    assert_eq!(b.len(), c.len(), "{label}: length mismatch");
    let mut n = 0usize;
    for i in 0..a.len() {
        let (x, y, z) = (f64::from(a[i]), f64::from(b[i]), f64::from(c[i]));
        if x.is_finite() && y.is_finite() && z.is_finite() {
            let mag = x.abs() + y.abs() + z.abs();
            assert!(
                (x + y + z).abs() <= 1e-3 * mag.max(1.0),
                "{label}: node {i} not traceless: {x}+{y}+{z}"
            );
            n += 1;
        }
    }
    n
}

fn structural(g: &Geom, res: &pb::ResultState, name: &str) {
    assert!(g.layout.starts_with("MVG3:"), "{name} → MVG3");
    assert_eq!(g.scalar.len(), g.verts, "{name} scalar is per-vertex");
    let finite: Vec<f32> = g.scalar.iter().copied().filter(|v| v.is_finite()).collect();
    assert!(!finite.is_empty(), "{name}: finite samples on elements");
    let lo = finite.iter().copied().fold(f32::INFINITY, f32::min);
    let hi = finite.iter().copied().fold(f32::NEG_INFINITY, f32::max);
    assert!(
        res.min as f32 <= lo + 1e-3 && res.max as f32 >= hi - 1e-3,
        "{name}: ResultState range brackets the scalar data"
    );
    assert!(res.min <= res.max, "{name}: ResultState min<=max");
}

#[tokio::test]
async fn derived_principal_families() {
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
        .insert(CLIENT_ID_HEADER, "m5b".parse().unwrap());
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
        unknown.layout.starts_with("MVG3:") && unknown.scalar.is_empty(),
        "unsupported derived → bare hull, no error"
    );

    // Step to a stressed state — state 1 is the undeformed initial
    // state (all stresses/strains zero), where the identities are
    // trivially true.
    set_state(&mut client, &mut sub, 101).await;

    // The validation here is deliberately restricted to algebraic
    // invariants that ride a *single shared derived primal gather*
    // (phase-4-m5b.md Decision 24): ordering, deviatoric tracelessness,
    // and the max-shear relation. Cross-field checks against a
    // different-cardinality derived (e.g. trace vs `-3·pressure`, which
    // gathers 6 vs 3 primals) are *not* used — on the IP-inconsistent
    // `basic1` corpus the 3- and 6-primal `query_full`s select
    // different integration-point representations, an O(1e-3) skew that
    // is real and expected (it is exactly the M5 Decision 21
    // derived-vs-primal skew, not a routing defect). Ordering +
    // tracelessness + the max-shear relation jointly pin the whole
    // eigenvalue-family routing (right primals → eigensolver → correct
    // λ-to-`prin{1,2,3}` mapping → M3 scatter) with zero skew and zero
    // external oracle; the eigenvalues' numeric correctness is already
    // owned by the `mili-rs` core parity suite (M5 Decision 19).

    // ── prin_stress: descending order + max-shear relation ───────────
    let (p1, p1r) = show(&mut client, &mut sub, &svc, "prin_stress1").await;
    let (p2, p2r) = show(&mut client, &mut sub, &svc, "prin_stress2").await;
    let (p3, _) = show(&mut client, &mut sub, &svc, "prin_stress3").await;
    assert!(p1.layout.starts_with("MVG3:"));
    structural(&p1, &p1r, "prin_stress1");
    structural(&p2, &p2r, "prin_stress2");
    let n = assert_descending(&p1.scalar, &p2.scalar, &p3.scalar, "prin_stress order");
    assert!(n > 0, "prin_stress order: nodes cross-checked");

    // ── deviatoric traceless: prin_dev_stress1+2+3 ≈ 0 ───────────────
    let (d1, d1r) = show(&mut client, &mut sub, &svc, "prin_dev_stress1").await;
    let (d2, _) = show(&mut client, &mut sub, &svc, "prin_dev_stress2").await;
    let (d3, _) = show(&mut client, &mut sub, &svc, "prin_dev_stress3").await;
    structural(&d1, &d1r, "prin_dev_stress1");
    let c = assert_traceless(
        &d1.scalar,
        &d2.scalar,
        &d3.scalar,
        "prin_dev_stress traceless",
    );
    assert!(c > 0, "prin_dev_stress traceless: nodes cross-checked");
    let n = assert_descending(&d1.scalar, &d2.scalar, &d3.scalar, "prin_dev_stress order");
    assert!(n > 0, "prin_dev_stress order: nodes cross-checked");

    // ── max-shear relation: ½·(prin_stress1 − prin_stress3) ──────────
    // All three ride the same 6-stress-primal eigensolver gather, so
    // this is exact to f32 (skew-free).
    let (ms, msr) = show(&mut client, &mut sub, &svc, "max_shear_stress").await;
    structural(&ms, &msr, "max_shear_stress");
    let half_span: Vec<f32> = (0..p1.verts)
        .map(|i| 0.5 * (p1.scalar[i] - p3.scalar[i]))
        .collect();
    let (c, m) = assert_identity(&ms.scalar, &half_span, "max_shear");
    assert!(c > 0 && m > 0.0, "max_shear: non-trivial cross-check");

    // ── strain families: structural + ordering + traceless ──────────
    // vol_strain is the trivial linear strain trace — the same kernel
    // family M5 already validated via the pressure identity; here it
    // gets structural + state-tracking coverage (its numeric
    // correctness is core-parity-owned, M5 Decision 19).
    let (vol, volr) = show(&mut client, &mut sub, &svc, "vol_strain").await;
    structural(&vol, &volr, "vol_strain");
    let (e1, e1r) = show(&mut client, &mut sub, &svc, "prin_strain1").await;
    let (e2, _) = show(&mut client, &mut sub, &svc, "prin_strain2").await;
    let (e3, _) = show(&mut client, &mut sub, &svc, "prin_strain3").await;
    structural(&e1, &e1r, "prin_strain1");
    let n = assert_descending(&e1.scalar, &e2.scalar, &e3.scalar, "prin_strain order");
    assert!(n > 0, "prin_strain order: nodes cross-checked");

    let (pe1, pe1r) = show(&mut client, &mut sub, &svc, "prin_dev_strain1").await;
    let (pe2, _) = show(&mut client, &mut sub, &svc, "prin_dev_strain2").await;
    let (pe3, _) = show(&mut client, &mut sub, &svc, "prin_dev_strain3").await;
    structural(&pe1, &pe1r, "prin_dev_strain1");
    let c = assert_traceless(
        &pe1.scalar,
        &pe2.scalar,
        &pe3.scalar,
        "prin_dev_strain traceless",
    );
    assert!(c > 0, "prin_dev_strain traceless: nodes cross-checked");
    let n = assert_descending(
        &pe1.scalar,
        &pe2.scalar,
        &pe3.scalar,
        "prin_dev_strain order",
    );
    assert!(n > 0, "prin_dev_strain order: nodes cross-checked");

    // ── derived tracks the state (basic1 is transient) ───────────────
    let s101 = show(&mut client, &mut sub, &svc, "prin_stress1")
        .await
        .0
        .scalar;
    set_state(&mut client, &mut sub, 1).await;
    let s1 = show(&mut client, &mut sub, &svc, "prin_stress1")
        .await
        .0
        .scalar;
    assert_ne!(s1, s101, "prin_stress1 must differ between state 1 and 101");

    // ── primal path still byte-stable: empty result → bare hull ──────
    let bare = show(&mut client, &mut sub, &svc, "").await.0;
    assert!(bare.layout.starts_with("MVG3:") && bare.scalar.is_empty());
}
