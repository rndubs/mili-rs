//! `wireframe-parity-5.md` Decision 109 — UDS transport.
//!
//! Round-trips a `Hello` over a Unix domain socket: binds
//! `mili_viz_server::serve_uds` on a tmpdir path, dials the same path
//! with a tonic channel backed by `tokio::net::UnixStream`, and asserts
//! the server's `Hello` reply comes back protocol-compatible. Proves
//! the UDS arm is a transport swap — the same MiliViz router, the same
//! frozen wire bytes.
//!
//! Unix-only (`#[cfg(unix)]`). Hermetic — no fixture, no corpus.

#![cfg(unix)]

use hyper_util::rt::TokioIo;
use mili_viz_proto::v1 as pb;
use mili_viz_server::VizService;
use tonic::transport::{Endpoint, Uri};
use tonic::Request;
use tower::service_fn;

fn uds_path(tag: &str) -> std::path::PathBuf {
    let dir = if std::path::Path::new("/tmp").is_dir() {
        std::path::PathBuf::from("/tmp")
    } else {
        std::env::temp_dir()
    };
    dir.join(format!("mili-viz-uds-test-{}-{tag}.sock", std::process::id()))
}

async fn dial(
    path: &std::path::Path,
) -> pb::mili_viz_client::MiliVizClient<tonic::transport::Channel> {
    let owned = path.to_path_buf();
    let channel = Endpoint::try_from("http://uds.invalid")
        .unwrap()
        .connect_with_connector(service_fn(move |_: Uri| {
            let path = owned.clone();
            async move {
                let io = tokio::net::UnixStream::connect(&path).await?;
                Ok::<_, std::io::Error>(TokioIo::new(io))
            }
        }))
        .await
        .expect("uds dial");
    pb::mili_viz_client::MiliVizClient::new(channel)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn serve_uds_round_trips_a_hello() {
    let svc = VizService::builder().build();
    let path = uds_path("hello");
    let _ = std::fs::remove_file(&path);

    let (bound, _handle) = mili_viz_server::serve_uds(svc, &path)
        .await
        .expect("serve_uds");
    assert_eq!(bound, path, "bound path round-trips");
    assert!(
        std::fs::metadata(&path).is_ok(),
        "serve_uds created the socket file"
    );

    let mut client = dial(&path).await;
    let reply = client
        .hello(Request::new(pb::HelloRequest {
            protocol_version: pb::PROTOCOL_VERSION.to_string(),
            client_id: "uds-test".to_string(),
            ..Default::default()
        }))
        .await
        .expect("hello rpc")
        .into_inner();

    assert!(reply.compatible, "matching protocol version");
    assert_eq!(reply.server_protocol_version, pb::PROTOCOL_VERSION);
    // Clean up — Drop on the listener task happens on process exit
    // anyway, but tmp hygiene is cheap.
    let _ = std::fs::remove_file(&path);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn serve_uds_clears_a_stale_socket_file() {
    // A previous crashed instance left a file behind; serve_uds must
    // remove it before binding so the listener does not see EADDRINUSE.
    let path = uds_path("stale");
    let _ = std::fs::remove_file(&path);
    std::fs::write(&path, b"stale").expect("create stale file");

    let svc = VizService::builder().build();
    let (bound, _handle) = mili_viz_server::serve_uds(svc, &path)
        .await
        .expect("serve_uds removes stale and binds");
    assert_eq!(bound, path);

    // Dial it — proves the listener is live, not just the file present.
    let mut client = dial(&path).await;
    let reply = client
        .hello(Request::new(pb::HelloRequest {
            protocol_version: pb::PROTOCOL_VERSION.to_string(),
            client_id: "uds-stale-test".to_string(),
            ..Default::default()
        }))
        .await
        .expect("hello rpc")
        .into_inner();
    assert!(reply.compatible);

    let _ = std::fs::remove_file(&path);
}
