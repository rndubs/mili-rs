//! Query RPC: server's typed `Query` round-trips a real `mili-rs`
//! `Database::query` (Stage 3 SFT read-path bring-up). The stub used to
//! return empty `values` regardless of the loaded DB; this gate pins
//! the wire-level response against the direct mili-rs answer for one
//! representative `(svar, class, label, state)` tuple on d3samp6.
//!
//! Skip-on-absent per `CLAUDE.md` so a local `cargo test` without the
//! parity corpus stays green.

use std::path::{Path, PathBuf};

use mili_rs::{Database as RsDb, QueryArgs, StateValues};
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

#[tokio::test]
async fn query_rpc_matches_mili_rs_direct_call() {
    let path = corpus_path(&["v3", "serial_t", "d3samp6.pltA"]);
    if !path.exists() {
        eprintln!("skip: v3/serial_t/d3samp6.pltA absent (run scripts/setup-parity.sh)");
        return;
    }

    // Direct mili-rs oracle: `sx` on brick label 1 at state index 0.
    let oracle = RsDb::open(&path).expect("d3samp6 opens");
    let args = QueryArgs {
        svar: "sx",
        class: "brick",
        labels: Some(&[1]),
        states: &[0],
        materials: None,
        ips: None,
        subrec: None,
    };
    let (vals, ret_labels) = oracle.query_with_labels(&args).expect("sx@brick[1]");
    let oracle_f64: Vec<f64> = match vals {
        StateValues::F32(v) => v.into_iter().map(f64::from).collect(),
        StateValues::F64(v) => v,
        other => panic!("unexpected svar numeric type for sx: {other:?}"),
    };
    assert_eq!(ret_labels, vec![1]);
    assert!(
        !oracle_f64.is_empty(),
        "oracle returned no values — fixture changed?"
    );

    // Server: same query through the wire.
    let svc = VizService::builder().build();
    let (mut client, _h) = spawn_in_process(svc.clone()).await.unwrap();

    let mut load = Request::new(pb::Command {
        cmd: Some(pb::command::Cmd::Load(pb::Load {
            root: path.to_string_lossy().into_owned(),
        })),
    });
    load.metadata_mut()
        .insert(CLIENT_ID_HEADER, "qtest".parse().unwrap());
    assert!(client.execute(load).await.unwrap().into_inner().ok);

    let mut req = Request::new(pb::QueryRequest {
        result: "sx".to_string(),
        class_name: "brick".to_string(),
        labels: vec![1],
        states: vec![1],
        component: String::new(),
    });
    req.metadata_mut()
        .insert(CLIENT_ID_HEADER, "qtest".parse().unwrap());
    let reply = client.query(req).await.unwrap().into_inner();
    assert!(reply.ok, "query RPC failed: {}", reply.error);
    let Some(pb::query_reply::Data::Inline(inline)) = reply.data else {
        panic!("expected InlineTable payload");
    };
    assert_eq!(inline.labels, vec![1]);
    assert_eq!(inline.states, vec![1]);
    assert_eq!(
        inline.values, oracle_f64,
        "wire values must equal mili-rs Database::query"
    );
    assert!(inline.components >= 1, "components > 0 for a populated svar");
}

#[tokio::test]
async fn query_rpc_without_load_reports_error() {
    let svc = VizService::builder().build();
    let (mut client, _h) = spawn_in_process(svc.clone()).await.unwrap();

    let mut req = Request::new(pb::QueryRequest {
        result: "sx".to_string(),
        class_name: "brick".to_string(),
        labels: vec![1],
        states: vec![1],
        component: String::new(),
    });
    req.metadata_mut()
        .insert(CLIENT_ID_HEADER, "qtest".parse().unwrap());
    let reply = client.query(req).await.unwrap().into_inner();
    assert!(!reply.ok, "no-DB query should report ok=false");
    assert!(
        reply.error.contains("no database loaded"),
        "error names the missing precondition: got {:?}",
        reply.error,
    );
    assert!(reply.data.is_none(), "no payload on error");
}
