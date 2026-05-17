//! Phase 5 M3 binary: open the `egui` shell over a live in-process
//! `mili-viz-server`. With an optional `argv[1]` database root the
//! session `load`s it (attached idle); with no argument the viewport
//! shows the "attach to session" card (not attached).

fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    mili_viz_client::run(std::env::args().nth(1))
}
