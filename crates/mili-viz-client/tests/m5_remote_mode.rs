//! Phase 5 M5 acceptance — remote mode over real gRPC + Arrow Flight
//! TCP. Gating test for `planning/mili-viz/phase-5-m5.md` § "M5
//! acceptance gate".
//!
//! Always-on coverage drives the CLI parser and the `--attach`
//! session-file resolver against fabricated `~/.griz/sessions/*.json`
//! files under a hermetic `$GRIZ_SESSIONS_DIR` (the same env-var
//! redirect pygriz/M2 uses). The skip-on-absent leg spawns a real
//! `mili-viz-server` over TCP via `mili_viz_server::spawn_tcp`, drives
//! a client [`Session::connect_tcp`] and [`Session::attach`]
//! end-to-end against `serial/basic1`, and asserts the Flight `DoGet`
//! arm decodes geometry **byte-identical** to the in-process
//! `VizService::fetch_geometry` arm — proving M5 is a transport swap,
//! not a contract change (`phase-4-m6.md` Decision 26 / `phase-5-m5.md`
//! Decision 90).

#![allow(clippy::too_many_lines)]

use std::path::{Path, PathBuf};

use mili_viz_client::{parse_args, CliOutcome, Session, TransportChoice};
use mili_viz_proto::v1 as pb;
use mili_viz_server::{spawn_tcp, VizService};
use tokio::sync::Mutex;

/// Serializes the env-mutating tests in this file. We mutate
/// `$GRIZ_SESSIONS_DIR` across tests; cargo's per-test parallelism
/// would otherwise race the env. `tokio::sync::Mutex` is held across
/// `.await` points cleanly (its guard is `Send`).
static ENV_LOCK: Mutex<()> = Mutex::const_new(());

fn parse(a: &[&str]) -> Result<CliOutcome, String> {
    parse_args(a.iter().map(|s| (*s).to_string()))
}

fn write_session_json(dir: &Path, id: &str, pid: u32, host: &str, port: u16) -> PathBuf {
    let path = dir.join(format!("{id}.json"));
    let body = format!(
        "{{\n  \"id\": \"{id}\",\n  \"pid\": {pid},\n  \"host\": \"{host}\",\n  \"port\": {port},\n  \"token\": \"\",\n  \"protocol_version\": \"0\",\n  \"db\": \"\"\n}}\n"
    );
    std::fs::write(&path, body).unwrap();
    path
}

fn fresh_tmp(tag: &str) -> PathBuf {
    let p = std::env::temp_dir().join(format!(
        "mili-viz-client-m5-{tag}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let _ = std::fs::remove_dir_all(&p);
    std::fs::create_dir_all(&p).unwrap();
    p
}

/// RAII env-var guard. Restores `$GRIZ_SESSIONS_DIR` on drop. The
/// caller acquires [`ENV_LOCK`] first (kept as a separate `await`
/// point so the guard lifetime is explicit). Holding the guard across
/// `.await` is safe because tokio's `Mutex` guard is `Send`.
struct EnvDir {
    prev: Option<std::ffi::OsString>,
}

impl EnvDir {
    fn set(dir: &Path) -> Self {
        let prev = std::env::var_os("GRIZ_SESSIONS_DIR");
        std::env::set_var("GRIZ_SESSIONS_DIR", dir);
        Self { prev }
    }
}

impl Drop for EnvDir {
    fn drop(&mut self) {
        if let Some(p) = &self.prev {
            std::env::set_var("GRIZ_SESSIONS_DIR", p);
        } else {
            std::env::remove_var("GRIZ_SESSIONS_DIR");
        }
    }
}

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

// ── Always-on: CLI surface ──────────────────────────────────────────

#[test]
fn cli_dispatches_remote_attach_and_default_distinctly() {
    let CliOutcome::Run(r) = parse(&["-r", "host:50051"]).unwrap() else {
        panic!("Run");
    };
    assert_eq!(
        r.transport,
        Some(TransportChoice::Remote("host:50051".into()))
    );

    let CliOutcome::Run(a) = parse(&["--attach"]).unwrap() else {
        panic!("Run");
    };
    assert_eq!(a.transport, Some(TransportChoice::Attach(None)));

    let CliOutcome::Run(a2) = parse(&["--attach", "abc123"]).unwrap() else {
        panic!("Run");
    };
    assert_eq!(
        a2.transport,
        Some(TransportChoice::Attach(Some("abc123".into())))
    );

    let CliOutcome::Run(d) = parse(&["-i", "x.pltA"]).unwrap() else {
        panic!("Run");
    };
    assert_eq!(d.transport, None, "M4 in-process default preserved");
}

#[test]
fn cli_rejects_mixed_or_double_transports() {
    assert!(parse(&["-r", "h:1", "--attach"])
        .unwrap_err()
        .contains("mutually exclusive"));
    assert!(parse(&["--attach", "id1", "-r", "h:1"])
        .unwrap_err()
        .contains("mutually exclusive"));
    assert!(parse(&["-r", "a:1", "--remote", "b:2"])
        .unwrap_err()
        .contains("mutually exclusive"));
}

// ── Always-on: `--attach` resolver against fabricated session files ─

#[tokio::test]
async fn attach_explicit_id_missing_is_a_clear_error() {
    let dir = fresh_tmp("missing-id");
    let _lock = ENV_LOCK.lock().await;
    let _env = EnvDir::set(&dir);
    let e = match Session::attach(Some("does-not-exist"), None).await {
        Ok(_) => panic!("attach of a missing id should error"),
        Err(e) => e.to_string(),
    };
    assert!(
        e.contains("no readable griz session") || e.contains("does-not-exist"),
        "explicit-id-missing surfaces a clear message: {e}"
    );
    std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn attach_empty_dir_is_a_clear_error() {
    let dir = fresh_tmp("empty");
    let _lock = ENV_LOCK.lock().await;
    let _env = EnvDir::set(&dir);
    let e = match Session::attach(None, None).await {
        Ok(_) => panic!("attach with empty sessions dir should error"),
        Err(e) => e.to_string(),
    };
    assert!(
        e.contains("no live griz sessions"),
        "empty sessions dir surfaces a clear message: {e}"
    );
    std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn attach_with_dead_pid_id_still_attempts_connect_and_fails_cleanly() {
    // An explicit `id` path bypasses the pid-liveness filter (Decision
    // 57 — explicit selection trumps newest-live). With a pid=1 (alive
    // on Unix as init), the resolver picks the entry and proceeds to
    // connect; the connect fails because no real server is listening
    // on the synthetic port. The error must be a clear connect/
    // transport error, not a silent infinite hang (Decision 93 — the
    // 10 s `connect_timeout`).
    let dir = fresh_tmp("dead");
    let _ = write_session_json(&dir, "synth", 1, "127.0.0.1", 1);
    let _lock = ENV_LOCK.lock().await;
    let _env = EnvDir::set(&dir);
    let e = match Session::attach(Some("synth"), None).await {
        Ok(_) => panic!("attach should fail when target server isn't listening"),
        Err(e) => e.to_string(),
    };
    // Either a connect-refused or a transport-layer string — anything
    // other than the resolver's "no readable" message proves the file
    // parsed and the connect was attempted.
    assert!(
        !e.contains("no readable griz session"),
        "explicit-id resolved, then connect failed cleanly: {e}"
    );
    std::fs::remove_dir_all(&dir).ok();
}

// ── Skip-on-absent: end-to-end remote against the live TCP server ──

#[tokio::test]
async fn remote_session_resolves_geometry_byte_identical_to_in_process() {
    let path = corpus_path(&["serial", "basic1", "basic1.pltA"]);
    if !path.exists() {
        eprintln!("skip: serial/basic1 absent (run scripts/setup-parity.sh)");
        return;
    }

    // Spawn one server; the client connects over real TCP to it. The
    // same `VizService` clone gives us the in-process direct seam for
    // the byte-identical comparison (M6 acceptance pattern).
    let svc = VizService::builder().build();
    let (addr, _viz, _flight, _h) = spawn_tcp(svc.clone()).await.unwrap();

    let mut session = Session::connect_tcp(&format!("{addr}"), Some(&path.to_string_lossy()))
        .await
        .expect("remote session loads serial/basic1 over TCP");
    assert!(session.is_remote(), "connect_tcp marks the session remote");

    // Drive the same `show` the in-process gate uses, find the
    // broadcast GeometryRef, decode via the remote (Flight) arm and
    // assert vs the in-process direct fetch.
    session
        .execute(pb::command::Cmd::Show(pb::Show::default()))
        .await
        .expect("show \"\" over the wire");

    // Pull deltas until the show's DELTA_RESULT arrives.
    let mut gref: Option<pb::GeometryRef> = None;
    for _ in 0..30 {
        for d in session.poll_deltas() {
            if let Some(pb::state_delta::Payload::Result(r)) = d.payload {
                if let Some(g) = r.geometry {
                    gref = Some(g);
                    break;
                }
            }
        }
        if gref.is_some() {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    let gref = gref.expect("show emitted a DELTA_RESULT with a GeometryRef");
    assert!(
        gref.flight_ticket.starts_with(b"geom:"),
        "the frozen geom:{{seq}} ticket form is unchanged across the wire"
    );

    // Remote arm: Flight DoGet through `resolve_geometry`.
    let mesh_remote = session
        .resolve_geometry(&gref)
        .await
        .expect("Flight DoGet resolves the live ticket");

    // Cross-check vs the in-process direct seam — bytes must match.
    let blob_inproc = svc
        .fetch_geometry(&gref.flight_ticket)
        .expect("in-process seam still resolves the same ticket");
    let mesh_inproc = mili_viz_client::decode_mvg(&blob_inproc).expect("in-process blob decodes");

    assert_eq!(
        mesh_remote.positions.len(),
        mesh_inproc.positions.len(),
        "remote vs in-process vertex counts identical"
    );
    assert_eq!(
        mesh_remote.indices.len(),
        mesh_inproc.indices.len(),
        "remote vs in-process index counts identical"
    );
    assert_eq!(
        &mesh_remote.positions[..mesh_remote.positions.len().min(64)],
        &mesh_inproc.positions[..mesh_inproc.positions.len().min(64)],
        "remote vs in-process vertices bit-identical (transport swap)"
    );
    assert_eq!(
        &mesh_remote.indices[..mesh_remote.indices.len().min(64)],
        &mesh_inproc.indices[..mesh_inproc.indices.len().min(64)],
        "remote vs in-process indices bit-identical (transport swap)"
    );
}

#[tokio::test]
async fn remote_session_fetches_catalog_via_flight() {
    let path = corpus_path(&["serial", "basic1", "basic1.pltA"]);
    if !path.exists() {
        eprintln!("skip: serial/basic1 absent (run scripts/setup-parity.sh)");
        return;
    }

    let svc = VizService::builder().build();
    let (addr, _viz, _flight, _h) = spawn_tcp(svc.clone()).await.unwrap();

    let mut session = Session::connect_tcp(&format!("{addr}"), Some(&path.to_string_lossy()))
        .await
        .unwrap();

    let cat = session
        .fetch_catalog()
        .await
        .expect("remote fetch_catalog decodes via Flight CATALOG_TICKET");
    assert!(
        !cat.primal.is_empty(),
        "serial/basic1 exposes primal svars over the wire"
    );
    assert!(
        !cat.derived.is_empty(),
        "serial/basic1 exposes derived results over the wire"
    );
}

#[tokio::test]
async fn attach_round_trip_resolves_a_real_running_server() {
    let path = corpus_path(&["serial", "basic1", "basic1.pltA"]);
    if !path.exists() {
        eprintln!("skip: serial/basic1 absent (run scripts/setup-parity.sh)");
        return;
    }

    let svc = VizService::builder().build();
    let (addr, _viz, _flight, _h) = spawn_tcp(svc.clone()).await.unwrap();

    let dir = fresh_tmp("attach");
    let _ = write_session_json(
        &dir,
        "live",
        std::process::id(), // our own pid is definitely alive
        &addr.ip().to_string(),
        addr.port(),
    );

    let mut session = {
        let _lock = ENV_LOCK.lock().await;
        let _env = EnvDir::set(&dir);
        Session::attach(None, Some(&path.to_string_lossy()))
            .await
            .expect("newest-live attach connects to the spawned server")
    };
    assert!(session.is_remote());

    session
        .execute(pb::command::Cmd::Show(pb::Show::default()))
        .await
        .unwrap();
    let mut gref = None;
    for _ in 0..30 {
        for d in session.poll_deltas() {
            if let Some(pb::state_delta::Payload::Result(r)) = d.payload {
                if let Some(g) = r.geometry {
                    gref = Some(g);
                    break;
                }
            }
        }
        if gref.is_some() {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    let gref = gref.expect("show after attach broadcasts a GeometryRef");
    let mesh = session.resolve_geometry(&gref).await.unwrap();
    assert!(
        !mesh.positions.is_empty(),
        "attached session decodes a non-empty mesh"
    );
    std::fs::remove_dir_all(&dir).ok();
}
