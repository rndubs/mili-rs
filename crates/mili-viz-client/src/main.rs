//! Phase 5 binary: parse the portable griz-subset argv
//! (`phase-5-m4.md` Decision 63), then open the `egui` shell over a
//! live in-process `mili-viz-server`. With a load root the session
//! `load`s it (attached idle); without one the viewport shows the
//! "attach to session" card (not attached).

use mili_viz_client::{parse_args, CliOutcome};

fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let args = match parse_args(std::env::args().skip(1)) {
        Ok(CliOutcome::Version) => {
            println!("mili-viz-client {}", env!("CARGO_PKG_VERSION"));
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

    mili_viz_client::run(args.load_root)
}
