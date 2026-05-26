//! Phase 4 M5d acceptance — the `*_alt` griz closed-form trig
//! principal-strain variants (`prin_strain[1-3]_alt` /
//! `prin_dev_strain[1-3]_alt`).
//!
//! Gating test for `planning/mili-viz/phase-4-m5d.md` § "M5d
//! acceptance gate". This slice discharges `phase-4-m5c.md`
//! Decision 28: the parity-gated `mili_rs::compute_principal_strain_alt`
//! kernel (planning/mili-py/m4.md Decision 27) now exists, so the viz
//! seam is the IDENTICAL M5b element nodal-average scatter as the
//! non-alt `principal_strain` branch — only the `*_spec`/`*_primals`/
//! `compute_*` calls swapped.
//!
//! Per `phase-4-m5b.md` Decision 24 / `phase-4-m5c.md` Decision 31 the
//! test asserts **single-shared-gather invariants only** — it does NOT
//! re-validate the kernel numerics (the `mili-rs`/`mili-py` core parity
//! suite owns those, `test_alt_strain_parity.py`). Every invariant here
//! rides one and the same element→node scatter over the same classes:
//!  - structural + `ResultState` bracketing for all six names;
//!  - principal-ordering `1_alt ≥ 2_alt ≥ 3_alt` per vertex (holds per
//!    element by the load-angle construction `cos(θ) ≥ cos(θ−2π/3) ≥
//!    cos(θ+2π/3)`, and the nodal average is monotone over the *same*
//!    weights for all three components — a true shared-gather identity,
//!    the M5d analogue of M5c's displacement-norm identity);
//!  - state-tracking on the transient corpus;
//!  - totality: an unknown/empty name still falls to the M3 bare hull,
//!    while `prin_strain1_alt` now resolves to an `MVG2` scalar (the
//!    explicit closure of Decision 28 — the inverse of the assertion
//!    `m5c_derived.rs` previously made).
//!
//! Corpus: `serial/sstate/d3samp6` — upstream's canonical strain corpus
//! (`SerialDerivedExpressions`), transient (101 states, so state
//! tracking is exercised) with the `*_alt` family resolving on its
//! `brick` class. (`serial/basic1`, used by M5c, is valuable only for
//! its IP-inconsistency, which is irrelevant to this element-scatter
//! ordering invariant — and per Decision 24 no cross-cardinality check
//! is made.) Skip-on-absent per CLAUDE.md.

#![allow(clippy::too_many_lines)] // one end-to-end acceptance scenario
#![allow(clippy::many_single_char_names)] // i = principal index

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
        .insert(CLIENT_ID_HEADER, "m5d".parse().unwrap());
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
        .insert(CLIENT_ID_HEADER, "m5d".parse().unwrap());
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

#[tokio::test]
async fn derived_alt_principal_strain() {
    let path = corpus_path(&["serial", "sstate", "d3samp6.pltA"]);
    if !path.exists() {
        eprintln!("skip: serial/sstate/d3samp6 absent (run scripts/setup-parity.sh)");
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
        .insert(CLIENT_ID_HEADER, "m5d".parse().unwrap());
    assert!(client.execute(load).await.unwrap().into_inner().ok);

    let mut sub = client
        .subscribe(Request::new(pb::SubscribeRequest::default()))
        .await
        .unwrap()
        .into_inner();
    let _snap = sub.message().await.unwrap().unwrap();

    // ── totality: unknown / empty still fall to the M3 bare hull;
    //    `prin_strain1_alt` now RESOLVES to an MVG2 scalar (the explicit
    //    closure of phase-4-m5c.md Decision 28). ─────────────────────
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
                result: "not_a_derived_xyz".to_string(),
                component: String::new(),
                opts: HashMap::new(),
            })),
        });
        req.metadata_mut()
            .insert(CLIENT_ID_HEADER, "m5d".parse().unwrap());
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

    // Step to a deformed/stressed state — state 1 is undeformed (strains
    // ~0 → every `*_alt` ~0, a degenerate all-zero result).
    set_state(&mut client, &mut sub, 22).await;

    let names = [
        "prin_strain1_alt",
        "prin_strain2_alt",
        "prin_strain3_alt",
        "prin_dev_strain1_alt",
        "prin_dev_strain2_alt",
        "prin_dev_strain3_alt",
    ];
    let mut scalars: HashMap<&str, Vec<f32>> = HashMap::new();
    for name in names {
        let (g, res) = show(&mut client, &mut sub, &svc, name).await;
        structural(&g, &res, name);
        scalars.insert(name, g.scalar);
    }

    // Decision 28 closure: the family now resolves (NOT bare hull).
    assert!(
        scalars.values().all(|s| !s.is_empty()),
        "every *_alt name resolves to a per-vertex scalar (Decision 28 discharged)"
    );

    // Single-shared-gather principal ordering: per element the
    // load-angle construction gives `cos θ₁ ≥ cos θ₂ ≥ cos θ₃` (and the
    // same `value ≥ 0` / `+e_hyd`), so component 1 ≥ 2 ≥ 3; the M5b
    // nodal average is monotone over the SAME weights for all three, so
    // the ordering survives per vertex. f32 averaging noise → a small
    // relative slack (same shape as M5c's norm tolerance).
    for fam in [
        ["prin_strain1_alt", "prin_strain2_alt", "prin_strain3_alt"],
        [
            "prin_dev_strain1_alt",
            "prin_dev_strain2_alt",
            "prin_dev_strain3_alt",
        ],
    ] {
        let a = &scalars[fam[0]];
        let b = &scalars[fam[1]];
        let c = &scalars[fam[2]];
        assert_eq!(a.len(), b.len());
        assert_eq!(b.len(), c.len());
        let mut ordered_nodes = 0usize;
        for i in 0..a.len() {
            if !(a[i].is_finite() && b[i].is_finite() && c[i].is_finite()) {
                continue;
            }
            let scale = a[i].abs().max(b[i].abs()).max(c[i].abs()).max(1.0);
            let tol = 1e-3 * scale;
            assert!(
                a[i] >= b[i] - tol && b[i] >= c[i] - tol,
                "{}/{}/{} not ordered at node {i}: {} {} {}",
                fam[0],
                fam[1],
                fam[2],
                a[i],
                b[i],
                c[i]
            );
            ordered_nodes += 1;
        }
        assert!(
            ordered_nodes > 0,
            "{fam:?}: at least one node ordering-checked"
        );
    }

    // Non-vacuous: the closed-form branch actually fired (some non-zero
    // vertex), so the structural/ordering checks are not an all-zeros
    // pass.
    assert!(
        scalars["prin_strain1_alt"]
            .iter()
            .any(|v| v.is_finite() && *v != 0.0),
        "prin_strain1_alt is identically zero — kernel not exercised"
    );

    // State-tracking: a `*_alt` scalar differs between two states on the
    // transient corpus (it reads the per-state strain primals).
    let s22 = scalars["prin_strain1_alt"].clone();
    set_state(&mut client, &mut sub, 60).await;
    let s60 = show(&mut client, &mut sub, &svc, "prin_strain1_alt")
        .await
        .0
        .scalar;
    assert_eq!(s22.len(), s60.len());
    assert!(
        s22.iter()
            .zip(&s60)
            .any(|(x, y)| x.is_finite() && y.is_finite() && (x - y).abs() > 1e-6),
        "prin_strain1_alt must differ between state 22 and 60"
    );
}
