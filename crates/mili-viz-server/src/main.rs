//! M1 `mili-viz-server` entry point.
//!
//! M1's transport is **in-process only** (no TCP — that is M6). This
//! binary is a thin scaffold: it constructs the service and reports
//! that the live transport for M1 is the in-memory channel exercised
//! by the acceptance-gate tests / consumed in-process by the client.
//! TCP/remote serving lands at Phase 4 M6 (Decision: out of scope).

fn main() {
    let _svc = mili_viz_server::VizService::builder().build();
    println!(
        "mili-viz-server (Phase 4 M1): in-process transport only. \
         Use `mili_viz_server::spawn_in_process` to obtain a connected \
         client; TCP/remote is Phase 4 M6."
    );
}
