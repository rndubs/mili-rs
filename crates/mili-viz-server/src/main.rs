//! `mili-viz-server` entry point (Phase 4 M6 — remote transport).
//!
//! Serves the frozen `MiliViz` contract **and** the Arrow Flight
//! bulk-geometry transport over a real TCP socket
//! (phase-4-m6.md Decisions 26 & 27). The bind address is `argv[1]`,
//! defaulting to `127.0.0.1:50051`. The in-process transport
//! (`mili_viz_server::spawn_in_process`) is kept as the default
//! local-workstation / embedding seam (README.md run mode 1).

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let addr: std::net::SocketAddr = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "127.0.0.1:50051".to_string())
        .parse()?;

    let svc = mili_viz_server::VizService::builder().build();
    let (local, handle) = mili_viz_server::serve_tcp(svc, addr).await?;
    println!("mili-viz-server (Phase 4 M6): MiliViz + Arrow Flight on tcp://{local}");
    handle.await?;
    Ok(())
}
