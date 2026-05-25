//! Phase 5 binary: parse the portable griz-subset argv
//! (`phase-5-m4.md` Decision 63), then open the `egui` shell over a
//! live in-process `mili-viz-server`. With a load root the session
//! `load`s it (attached idle); without one the viewport shows the
//! "attach to session" card (not attached).
//!
//! Also dispatches the `mili-viz-client snapshot` subcommand
//! (see `snapshot.rs`), which talks to a running window over a
//! filesystem trigger and prints the resolved PNG path.

use std::path::PathBuf;
use std::time::Duration;

use mili_viz_client::{parse_args, run_snapshot_cli, CliOutcome, SnapshotArgs};

fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let args = match parse_args(std::env::args().skip(1)) {
        Ok(CliOutcome::Version) => {
            println!("mili-viz-client {}", env!("CARGO_PKG_VERSION"));
            return Ok(());
        }
        Ok(CliOutcome::Snapshot(s)) => {
            run_snapshot(s);
            return Ok(());
        }
        Ok(CliOutcome::Run(a)) => a,
        Err(msg) => {
            eprintln!("mili-viz-client: {msg}");
            std::process::exit(2);
        }
    };

    if let Some(s) = &args.batch_script {
        eprintln!(
            "mili-viz-client: note: -b/-batch `{s}` accepted but not yet run \
             (the startup-script runner is Phase 6-gated)"
        );
    }
    if let Some((w, h)) = args.window_size {
        eprintln!(
            "mili-viz-client: note: -w {w} {h} accepted but not yet applied \
             (the window opens at the OS default size for now)"
        );
    }

    mili_viz_client::run(args.load_root, args.transport)
}

fn run_snapshot(s: SnapshotArgs) {
    let timeout = Duration::from_secs_f64(s.timeout_secs.unwrap_or(5.0));
    let out = s.out.map(PathBuf::from);
    match run_snapshot_cli(out, timeout) {
        Ok(path) => println!("{}", path.display()),
        Err(e) => {
            eprintln!("mili-viz-client snapshot: {e}");
            std::process::exit(1);
        }
    }
}
