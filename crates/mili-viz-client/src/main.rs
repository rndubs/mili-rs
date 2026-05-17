//! Phase 5 M2 binary: open the renderer window. With an optional
//! `argv[1]` database root, spawn an in-process `mili-viz-server`,
//! `load`/`show` it, and draw the returned mesh; with no argument,
//! open the empty (clear) window.

fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let mesh = match std::env::args().nth(1) {
        Some(root) => {
            let rt = tokio::runtime::Runtime::new()?;
            Some(rt.block_on(mili_viz_client::fetch_server_mesh(&root, ""))?)
        }
        None => None,
    };
    mili_viz_client::run(mesh)?;
    Ok(())
}
