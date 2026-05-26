//! Round-trip test for the snapshot trigger-file IPC.
//!
//! `mili-viz-client snapshot` and the windowed app communicate via a
//! file at `$GRIZ_SNAPSHOTS_DIR/.capture-request` — the CLI drops it,
//! the app picks it up on the next redraw and writes a PNG to the
//! requested path. This exercises both halves without needing a real
//! window: a worker thread simulates the redraw-loop side
//! (`try_consume_request_file` + `write_png`) while the main thread
//! runs the CLI helper end-to-end.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use mili_viz_client::{
    run_snapshot_cli, snapshot_request_file, snapshots_dir, try_consume_request_file,
    write_snapshot_png,
};

/// Both tests mutate the process-global `GRIZ_SNAPSHOTS_DIR` env var.
/// Serialise them so cargo's default parallel runner can't interleave
/// reads and writes.
static ENV_LOCK: Mutex<()> = Mutex::new(());

fn fresh_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("snapshot-trigger-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn cli_drops_request_and_simulated_app_writes_png() {
    let _g = ENV_LOCK.lock().unwrap();
    let dir = fresh_dir("happy");
    // SAFETY: this test binary is a single process; setting the env
    // here means every subsequent `snapshots_dir()` call (CLI helper
    // and worker thread) resolves to `dir`.
    unsafe {
        std::env::set_var("GRIZ_SNAPSHOTS_DIR", &dir);
    }

    let stop = Arc::new(AtomicBool::new(false));
    let stop_flag = stop.clone();
    let worker = std::thread::spawn(move || {
        // Stand in for the app's redraw loop: poll the trigger file
        // until one appears, then write a tiny PNG to the requested
        // path.
        while !stop_flag.load(Ordering::Relaxed) {
            if let Some(req) = try_consume_request_file() {
                let rgba = vec![
                    0u8, 200, 0, 255, 0, 200, 0, 255, 0, 200, 0, 255, 0, 200, 0, 255,
                ];
                write_snapshot_png(&req.out_path, 2, 2, &rgba).expect("png write");
                return;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
    });

    let out = dir.join("frame.png");
    let path = run_snapshot_cli(Some(out.clone()), Duration::from_secs(2))
        .expect("CLI helper returns a path");
    stop.store(true, Ordering::Relaxed);
    let _ = worker.join();
    assert_eq!(path, out);
    assert!(path.exists(), "PNG must exist at the returned path");
    let img = image::open(&path).unwrap().to_rgba8();
    assert_eq!(img.dimensions(), (2, 2));
    assert!(
        !snapshot_request_file().exists(),
        "trigger file must be drained"
    );
    assert!(
        snapshots_dir().join("latest.png").exists(),
        "latest.png must be updated alongside the timestamped write"
    );
}

#[test]
fn cli_times_out_when_no_app_is_listening() {
    let _g = ENV_LOCK.lock().unwrap();
    let dir = fresh_dir("timeout");
    unsafe {
        std::env::set_var("GRIZ_SNAPSHOTS_DIR", &dir);
    }
    let out = dir.join("never.png");
    let err = run_snapshot_cli(Some(out.clone()), Duration::from_millis(200))
        .expect_err("no worker → timeout");
    assert!(err.to_string().contains("no running"));
    // The stale trigger is cleaned up so the next attempt is fresh.
    assert!(
        !snapshot_request_file().exists(),
        "trigger must be removed on timeout"
    );
    assert!(!out.exists());
}
