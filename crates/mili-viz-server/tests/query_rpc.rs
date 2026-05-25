//! `Query` RPC gating test (`wireframe-parity.md` "What's still left"
//! #4 — server arm).
//!
//! The M1 stub returned `ok = true` with an empty `values` vector,
//! echoing only the request labels/states. This test pins the real
//! implementation: a primal svar query against a loaded `serial/basic1`
//! family returns finite, correctly-shaped values that match the
//! `Database::query_full` semantics the geometry path already uses
//! (`vertex_scalar` — same underlying gather kernel).
//!
//! Skip-on-absent per CLAUDE.md when the corpus is missing.

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

async fn load(
    client: &mut pb::mili_viz_client::MiliVizClient<tonic::transport::Channel>,
    path: &Path,
) {
    let mut req = Request::new(pb::Command {
        cmd: Some(pb::command::Cmd::Load(pb::Load {
            root: path.to_string_lossy().into_owned(),
        })),
    });
    req.metadata_mut()
        .insert(CLIENT_ID_HEADER, "q".parse().unwrap());
    let reply = client.execute(req).await.unwrap().into_inner();
    assert!(reply.ok, "load failed: {}", reply.error);
}

#[tokio::test]
async fn query_returns_real_values_for_primal_svar() {
    let path = corpus_path(&["serial", "basic1", "basic1.pltA"]);
    if !path.exists() {
        eprintln!("skip: serial/basic1 absent (run scripts/setup-parity.sh)");
        return;
    }
    let svc = VizService::builder().build();
    let (mut client, _h) = spawn_in_process(svc.clone()).await.unwrap();
    load(&mut client, &path).await;

    // `sand` is the canonical brick element scalar in basic1 — same
    // svar the M3 primal test colors the mesh with. An unfiltered
    // request reads every brick element for state 1.
    let req = Request::new(pb::QueryRequest {
        result: "sand".to_string(),
        class_name: "brick".to_string(),
        labels: vec![],
        states: vec![1],
        component: String::new(),
    });
    let reply = client.query(req).await.unwrap().into_inner();
    assert!(
        reply.ok,
        "primal query must succeed once mili-rs is wired: {}",
        reply.error
    );
    let Some(pb::query_reply::Data::Inline(t)) = reply.data else {
        panic!("inline carrier expected (no Flight ticket today)");
    };
    assert_eq!(t.states, vec![1], "echo the resolved state list");
    assert_eq!(
        t.components, 1,
        "`sand` is a scalar — one component per (state, label)"
    );
    assert!(
        !t.labels.is_empty(),
        "brick has at least one element"
    );
    assert_eq!(
        t.values.len() as i32,
        t.states.len() as i32 * t.labels.len() as i32 * t.components as i32,
        "values must match [states × labels × components] row-major shape \
         (states={} labels={} components={} values={})",
        t.states.len(),
        t.labels.len(),
        t.components,
        t.values.len()
    );
    assert!(
        t.values.iter().any(|v| v.is_finite()),
        "at least one finite value (the M1 stub returned an empty vec)"
    );
}

#[tokio::test]
async fn query_over_multiple_states_returns_per_state_rows() {
    let path = corpus_path(&["serial", "basic1", "basic1.pltA"]);
    if !path.exists() {
        eprintln!("skip: serial/basic1 absent (run scripts/setup-parity.sh)");
        return;
    }
    let svc = VizService::builder().build();
    let (mut client, _h) = spawn_in_process(svc.clone()).await.unwrap();
    load(&mut client, &path).await;

    // basic1 has multiple states — read three of them and confirm the
    // row-major `[state][label]` layout.
    let req = Request::new(pb::QueryRequest {
        result: "sand".to_string(),
        class_name: "brick".to_string(),
        labels: vec![],
        states: vec![1, 2, 3],
        component: String::new(),
    });
    let reply = client.query(req).await.unwrap().into_inner();
    assert!(reply.ok, "{}", reply.error);
    let Some(pb::query_reply::Data::Inline(t)) = reply.data else {
        panic!("inline expected");
    };
    assert_eq!(t.states, vec![1, 2, 3]);
    assert_eq!(t.components, 1);
    let n_labels = t.labels.len();
    assert!(n_labels > 0);
    assert_eq!(
        t.values.len(),
        3 * n_labels,
        "3 states × {n_labels} labels × 1 component"
    );
}

#[tokio::test]
async fn query_errors_clearly_with_no_run_loaded() {
    let svc = VizService::builder().build();
    let (mut client, _h) = spawn_in_process(svc.clone()).await.unwrap();

    // No `load` before `query` — server must return ok=false with
    // a typed error, not silently a zero-value inline table.
    let req = Request::new(pb::QueryRequest {
        result: "sand".to_string(),
        class_name: "brick".to_string(),
        labels: vec![],
        states: vec![1],
        component: String::new(),
    });
    let reply = client.query(req).await.unwrap().into_inner();
    assert!(!reply.ok, "must fail when no run is loaded");
    assert!(
        reply.error.contains("no run loaded"),
        "error message names the precondition: {}",
        reply.error
    );
}

#[tokio::test]
async fn query_rejects_derived_results_with_clear_error() {
    let path = corpus_path(&["serial", "basic1", "basic1.pltA"]);
    if !path.exists() {
        eprintln!("skip: serial/basic1 absent (run scripts/setup-parity.sh)");
        return;
    }
    let svc = VizService::builder().build();
    let (mut client, _h) = spawn_in_process(svc.clone()).await.unwrap();
    load(&mut client, &path).await;

    // `pressure` is a stress-invariant derived result — same as the
    // geometry path's `stress_invariant_spec` dispatch. Until the
    // derived routing is replicated for Query, the server returns an
    // honest "not yet supported" error.
    let req = Request::new(pb::QueryRequest {
        result: "pressure".to_string(),
        class_name: "brick".to_string(),
        labels: vec![],
        states: vec![1],
        component: String::new(),
    });
    let reply = client.query(req).await.unwrap().into_inner();
    assert!(!reply.ok, "derived `pressure` is the deferred follow-up");
    assert!(
        reply.error.contains("not yet supported"),
        "error names the missing follow-up: {}",
        reply.error
    );
}

#[tokio::test]
async fn query_rejects_out_of_range_state() {
    let path = corpus_path(&["serial", "basic1", "basic1.pltA"]);
    if !path.exists() {
        eprintln!("skip: serial/basic1 absent (run scripts/setup-parity.sh)");
        return;
    }
    let svc = VizService::builder().build();
    let (mut client, _h) = spawn_in_process(svc.clone()).await.unwrap();
    load(&mut client, &path).await;

    let req = Request::new(pb::QueryRequest {
        result: "sand".to_string(),
        class_name: "brick".to_string(),
        labels: vec![],
        // basic1 has < 100k states; 999_999 is comfortably out of range.
        states: vec![999_999],
        component: String::new(),
    });
    let reply = client.query(req).await.unwrap().into_inner();
    assert!(!reply.ok);
    assert!(
        reply.error.contains("out of range"),
        "named the range: {}",
        reply.error
    );
}
