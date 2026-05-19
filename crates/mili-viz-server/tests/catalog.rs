//! Phase 5 M3 follow-up (`phase-5-m4.md` Decision 67; MVP-cut 8) —
//! the result-catalog side-channel.
//!
//! Two halves, the `m6_transport.rs` shape:
//!  * always-on — no real run loaded ⇒ `fetch_catalog` is `None`
//!    (the frozen-stub `LoadedState` is unperturbed; the client keeps
//!    its static placeholder, so the headless composite gate is
//!    byte-stable, `bug-tracker.md` VB-001).
//!  * skip-on-absent — load `serial/basic1`: the in-process
//!    `VizService::fetch_catalog` blob is well-formed (`MVCAT1\n` +
//!    `P\t<name>` primal lines from `Database::queriable_svars`), and
//!    a **real Arrow Flight `DoGet`** of the conventional
//!    `CATALOG_TICKET` returns the **byte-identical** blob (the
//!    transport-swap parity, exactly as M6 proved for geometry). No
//!    `.proto`/ticket/format change.

#![allow(clippy::too_many_lines)]

use std::path::{Path, PathBuf};

use mili_viz_proto::flight as fpb;
use mili_viz_proto::v1 as pb;
use mili_viz_server::{spawn_tcp, VizService, CATALOG_TICKET, CLIENT_ID_HEADER};
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

type VizClient = pb::mili_viz_client::MiliVizClient<tonic::transport::Channel>;
type FlightClient = fpb::flight_service_client::FlightServiceClient<tonic::transport::Channel>;

fn with_client_id(cmd: pb::command::Cmd, id: &str) -> Request<pb::Command> {
    let mut req = Request::new(pb::Command { cmd: Some(cmd) });
    req.metadata_mut()
        .insert(CLIENT_ID_HEADER, id.parse().unwrap());
    req
}

async fn exec(client: &mut VizClient, cmd: pb::command::Cmd) -> pb::CommandReply {
    client
        .execute(with_client_id(cmd, "catalog-test"))
        .await
        .unwrap()
        .into_inner()
}

async fn flight_get(flight: &mut FlightClient, ticket: &[u8]) -> Vec<u8> {
    let mut stream = flight
        .do_get(Request::new(fpb::Ticket {
            ticket: ticket.to_vec(),
        }))
        .await
        .expect("DoGet of the catalog ticket succeeds")
        .into_inner();
    let mut blob = Vec::new();
    while let Some(fd) = stream.message().await.expect("Flight stream ok") {
        blob.extend_from_slice(&fd.data_body);
    }
    blob
}

#[test]
fn no_run_loaded_yields_no_catalog() {
    // The frozen-stub path: a service that never opened a DB must not
    // synthesize a catalog (keeps the client's static placeholder and
    // the byte-stable composite gate).
    let svc = VizService::builder().build();
    assert!(
        svc.fetch_catalog().is_none(),
        "no DB loaded ⇒ no catalog blob"
    );
}

#[tokio::test]
async fn catalog_blob_is_well_formed_and_flight_byte_identical() {
    let path = corpus_path(&["serial", "basic1", "basic1.pltA"]);
    if !path.exists() {
        eprintln!("skip: serial/basic1 absent (run scripts/setup-parity.sh)");
        return;
    }

    let svc = VizService::builder().build();
    let (addr, mut viz, mut flight, _h) = spawn_tcp(svc.clone()).await.unwrap();
    assert!(addr.port() != 0);

    let r = exec(
        &mut viz,
        pb::command::Cmd::Load(pb::Load {
            root: path.to_string_lossy().into_owned(),
        }),
    )
    .await;
    assert!(r.ok, "load failed: {}", r.error);

    // In-process seam (the path the current client uses).
    let blob = svc
        .fetch_catalog()
        .expect("a loaded run yields a catalog blob");
    assert!(
        blob.starts_with(b"MVCAT1\n"),
        "self-describing magic present"
    );
    let body = std::str::from_utf8(&blob[7..]).expect("UTF-8 body");
    let primal: Vec<&str> = body.lines().filter_map(|l| l.strip_prefix("P\t")).collect();
    assert!(
        !primal.is_empty(),
        "serial/basic1 exposes queriable primal svars"
    );
    // Every non-empty line is a tagged primal entry (no stray bytes).
    assert!(
        body.lines().all(|l| l.is_empty() || l.starts_with("P\t")),
        "every catalog line is a P-tagged primal entry"
    );

    // Real Arrow Flight DoGet of the conventional ticket returns the
    // byte-identical blob — the transport swap is format-stable
    // (exactly as M6 proved for geometry).
    let over_flight = flight_get(&mut flight, CATALOG_TICKET).await;
    assert_eq!(
        over_flight, blob,
        "Flight DoGet(CATALOG_TICKET) == in-process fetch_catalog"
    );
}
