//! `mili-viz-server` entry point (Phase 4 M6 — remote transport;
//! Phase 6 M2 — session/connection file).
//!
//! Serves the frozen `MiliViz` contract **and** the Arrow Flight
//! bulk-geometry transport over a real TCP socket
//! (phase-4-m6.md Decisions 26 & 27). The bind address is `argv[1]`,
//! defaulting to `127.0.0.1:50051`. The in-process transport
//! (`mili_viz_server::spawn_in_process`) is kept as the default
//! local-workstation / embedding seam (README.md run mode 1).
//!
//! Phase 6 M2 (phase-6-m2.md Decision 56): after the listener binds,
//! the **binary** (never the frozen library transport, never the
//! frozen proto) writes the Jupyter-style session/connection file
//! `<GRIZ_SESSIONS_DIR|~/.griz/sessions>/<id>.json` so a `pygriz`
//! script can `griz.attach()` to this running server. This is the
//! only Phase 6 server-side change; `lib.rs`/`mili_viz.proto` are
//! untouched.

use std::fmt::Write as _;
use std::io::Write as _;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let mut addr_arg = "127.0.0.1:50051".to_string();
    let mut agent_kind: Option<String> = None;
    let mut agent_url = "http://localhost:8080".to_string();

    let mut args = std::env::args().skip(1).peekable();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--agent" => {
                agent_kind = args.next();
            }
            "--agent-url" => {
                agent_url = args.next().unwrap_or_default();
            }
            arg if !arg.starts_with("--") => {
                addr_arg = arg.to_string();
            }
            _ => {}
        }
    }

    let addr: std::net::SocketAddr = addr_arg.parse()?;

    let svc = if agent_kind.as_deref() == Some("llamacpp") {
        println!("mili-viz-server: FunctionGemma agent enabled ({agent_url})");
        mili_viz_server::VizService::builder()
            .agent_backend(mili_viz_server::LlamaCppAgent::with_url(&agent_url))
            .build()
    } else {
        mili_viz_server::VizService::builder().build()
    };
    let (local, handle) = mili_viz_server::serve_tcp(svc, addr).await?;
    println!("mili-viz-server (Phase 4 M6): MiliViz + Arrow Flight on tcp://{local}");

    // Phase 6 M2 / Decision 56: emit the session/connection file from
    // the binary, best-effort. A write failure must not take down the
    // server (the wire transport is the contract; the file is an
    // additive attach side-channel) — it is logged to stderr only.
    if let Err(e) = write_session_file(&local) {
        eprintln!("mili-viz-server: could not write session file: {e}");
    }

    handle.await?;
    Ok(())
}

/// Write `<sessions_dir>/<id>.json` for `griz.attach()` (phase-6-m2.md
/// Decision 56). `id`/`token` are short lowercase-hex strings mixed
/// from the pid and startup time (no new crate — the server's
/// dependency surface is frozen; the JSON is hand-formatted with a
/// minimal escaper). `db` is empty (a fresh binary has nothing
/// loaded; the live db is the `Hello`/`Subscribe` wire, not the
/// file). The token is written for the Jupyter-file contract /
/// forward-compat but is **not** enforced here — `main` does not call
/// the opt-in `.expected_token(...)` builder, so the frozen tokenless
/// M1 gate keeps passing (Decision 56).
fn write_session_file(local: &std::net::SocketAddr) -> std::io::Result<()> {
    let pid = std::process::id();
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);

    let id = format!("{:08x}", mix(u128::from(pid), nanos));
    let token = format!(
        "{:016x}",
        mix(nanos, u128::from(pid).wrapping_mul(0x9E37_79B9))
    );

    let dir = sessions_dir();
    std::fs::create_dir_all(&dir)?;
    let path = dir.join(format!("{id}.json"));

    let mut json = String::new();
    json.push_str("{\n");
    let _ = writeln!(json, "  \"id\": \"{}\",", escape(&id));
    let _ = writeln!(json, "  \"pid\": {pid},");
    let _ = writeln!(json, "  \"host\": \"{}\",", escape(&local.ip().to_string()));
    let _ = writeln!(json, "  \"port\": {},", local.port());
    let _ = writeln!(json, "  \"token\": \"{}\",", escape(&token));
    let _ = writeln!(
        json,
        "  \"protocol_version\": \"{}\",",
        escape(mili_viz_proto::v1::PROTOCOL_VERSION)
    );
    let _ = writeln!(json, "  \"db\": \"\"");
    json.push_str("}\n");

    // Write to a temp sibling then rename so a concurrent reader never
    // sees a half-written file (atomic-on-rename, the Jupyter pattern).
    let tmp = dir.join(format!(".{id}.json.{pid}.tmp"));
    {
        let mut f = std::fs::File::create(&tmp)?;
        f.write_all(json.as_bytes())?;
        f.flush()?;
    }
    std::fs::rename(&tmp, &path)?;
    println!("mili-viz-server: session file {}", path.display());
    Ok(())
}

/// `$GRIZ_SESSIONS_DIR` if set (hermetic tests / redirection), else
/// `~/.griz/sessions` — the scripting.md / Jupyter-pattern default.
fn sessions_dir() -> std::path::PathBuf {
    if let Some(d) = std::env::var_os("GRIZ_SESSIONS_DIR") {
        return std::path::PathBuf::from(d);
    }
    let home = std::env::var_os("HOME")
        .map_or_else(|| std::path::PathBuf::from("."), std::path::PathBuf::from);
    home.join(".griz").join("sessions")
}

/// A tiny non-cryptographic mix → stable per-process hex (id/token).
/// Not a security boundary (Decision 56: the token is unenforced); it
/// only needs to avoid collisions between concurrently running
/// servers, which pid+nanos already gives.
fn mix(a: u128, b: u128) -> u64 {
    let mut x = a
        .wrapping_mul(0x2545_F491_4F6C_DD1D)
        .wrapping_add(b.wrapping_mul(0x9E37_79B9_7F4A_7C15));
    x ^= x >> 33;
    x = x.wrapping_mul(0xFF51_AFD7_ED55_8CCD);
    x ^= x >> 33;
    (x & u128::from(u64::MAX)) as u64
}

/// Minimal JSON string escaper for the handful of fields above
/// (the only realistic specials are `\` and `"` in a path/host).
fn escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => {
                let _ = write!(out, "\\u{:04x}", c as u32);
            }
            c => out.push(c),
        }
    }
    out
}
