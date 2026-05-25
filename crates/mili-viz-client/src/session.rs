//! Drive the frozen `mili-viz` contract over either the **in-process**
//! transport (M2 — `phase-5-m2.md` Decision 41) or the **remote**
//! transport (M5 — `phase-5-m5.md` Decisions 90 & 92) and decode the
//! returned geometry.
//!
//! In-process (the default): the client spawns a `mili-viz-server` in
//! the same binary (`spawn_in_process`, the M1 acceptance-gate
//! transport), subscribes, sends `load`/`show`, reads the broadcast
//! `DELTA_RESULT`'s `GeometryRef`, and resolves its `flight_ticket`
//! through the in-process `VizService::fetch_geometry` seam
//! (`phase-4-m2.md` Decision 10).
//!
//! Remote (M5): the client connects over real gRPC + Arrow Flight over
//! TCP to a `mili-viz-server` bound by [`mili_viz_server::serve_tcp`]
//! (Phase 4 M6 — `phase-4-m6.md` Decision 26). Ticket resolution rides
//! a real Flight `DoGet` whose `FlightData.data_body` chunks
//! concatenate into the **byte-identical** blob `fetch_geometry` would
//! return in-process. The `Channel` is built once and cloned for the
//! `MiliVizClient` and the `FlightServiceClient` — they share the
//! underlying HTTP/2 connection exactly as `serve_tcp` muxes them
//! (Decision 93).

use std::collections::HashMap;
use std::error::Error;
use std::path::{Path, PathBuf};
use std::time::Duration;

use mili_viz_proto::flight as fpb;
use mili_viz_proto::v1 as pb;
use mili_viz_server::{spawn_in_process, VizService, CATALOG_TICKET, CLIENT_ID_HEADER};
use tonic::transport::{Channel, Endpoint};
use tonic::Request;

use crate::catalog::{decode_catalog, ResultCatalog};
use crate::mesh::{decode_mvg, Mesh};

type BoxErr = Box<dyn Error + Send + Sync>;

type VizClient = pb::mili_viz_client::MiliVizClient<Channel>;
type FlightClient = fpb::flight_service_client::FlightServiceClient<Channel>;

async fn exec(client: &mut VizClient, cmd: pb::command::Cmd) -> Result<(), BoxErr> {
    let mut req = Request::new(pb::Command { cmd: Some(cmd) });
    req.metadata_mut()
        .insert(CLIENT_ID_HEADER, "mili-viz-client".parse()?);
    let reply = client.execute(req).await?.into_inner();
    if !reply.ok {
        return Err(format!("command failed: {}", reply.error).into());
    }
    Ok(())
}

/// Spawn an in-process server, `load <root>`, `show <result>`, and
/// return the resulting hull decoded into a [`Mesh`]. An empty
/// `result` is the no-scalar hull view (`phase-4-m2.md` Decision 12);
/// any M3 `MVG2` scalar is decoded past and ignored (M2 draws the bare
/// hull — `phase-5-m2.md` Decision 42).
///
/// # Errors
/// Returns an error if the transport fails to connect, a command is
/// rejected, the server returns no `GeometryRef` (e.g. the root did
/// not open a real database), the ticket does not resolve, or the
/// blob fails to decode.
pub async fn fetch_server_mesh(root: &str, result: &str) -> Result<Mesh, BoxErr> {
    let svc = VizService::builder().build();
    let (mut client, _server) = spawn_in_process(svc.clone()).await?;

    let mut sub = client
        .subscribe(Request::new(pb::SubscribeRequest::default()))
        .await?
        .into_inner();
    // The stream opens with a DELTA_SNAPSHOT; drain it.
    sub.message().await?;

    exec(
        &mut client,
        pb::command::Cmd::Load(pb::Load {
            root: root.to_string(),
        }),
    )
    .await?;
    exec(
        &mut client,
        pb::command::Cmd::Show(pb::Show {
            result: result.to_string(),
            component: String::new(),
            opts: HashMap::new(),
        }),
    )
    .await?;

    // Read deltas until the `show`'s DELTA_RESULT arrives.
    loop {
        let Some(delta) = sub.message().await? else {
            return Err("subscription closed before a result delta".into());
        };
        if let Some(pb::state_delta::Payload::Result(res)) = delta.payload {
            let gref = res
                .geometry
                .ok_or("show carried no GeometryRef (root did not open a real database?)")?;
            let blob = svc
                .fetch_geometry(&gref.flight_ticket)
                .ok_or("GeometryRef ticket did not resolve in the in-process store")?;
            return Ok(decode_mvg(&blob)?);
        }
    }
}

/// Which transport a [`Session`] uses to resolve a `GeometryRef`
/// (`phase-5-m5.md` Decision 90). The `MiliViz` RPC channel itself is
/// the same `MiliVizClient<Channel>` either way (tonic's in-memory
/// duplex Channel for in-process, a real TCP Channel for remote).
enum Transport {
    /// M2 in-process: direct calls into the spawned [`VizService`].
    InProcess {
        svc: VizService,
        _server: tokio::task::JoinHandle<()>,
    },
    /// M5 remote: a real Arrow Flight client over the TCP `Channel`.
    Remote { flight: FlightClient },
}

/// A live session for the M3 windowed shell (`phase-5-m3.md`
/// Decision 46) backed by either the in-process or the remote
/// transport (`phase-5-m5.md` Decision 90).
pub struct Session {
    client: VizClient,
    transport: Transport,
    deltas: std::sync::mpsc::Receiver<pb::StateDelta>,
    /// Server's advertised capabilities (`Hello` reply). Phase 5 M6
    /// (`phase-5-m6.md` Decision 94): the client keys the AI Assistant
    /// panel's existence off `CAP_AGENT`. The in-process
    /// `connect_in_process` path also runs the handshake so the same
    /// gate works without a TCP hop.
    capabilities: Vec<String>,
}

/// Tune one tonic [`Endpoint`] for an HPC-latency hop
/// (`phase-5-m5.md` Decision 93). `tcp_nodelay` so sub-MTU `Execute`
/// commands don't sit a Nagle window; TCP + HTTP/2 keep-alives so an
/// idle `Subscribe` stream survives stateful NAT/firewall drops; an
/// explicit connect timeout so a misconfigured `-r host:port` fails
/// loud instead of waiting forever.
fn tune(ep: Endpoint) -> Endpoint {
    ep.tcp_nodelay(true)
        .tcp_keepalive(Some(Duration::from_secs(30)))
        .http2_keep_alive_interval(Duration::from_secs(20))
        .keep_alive_timeout(Duration::from_secs(10))
        .connect_timeout(Duration::from_secs(10))
}

async fn run_initial_load(s: &mut Session, root: Option<&str>) -> Result<(), BoxErr> {
    if let Some(root) = root {
        s.execute(pb::command::Cmd::Load(pb::Load {
            root: root.to_string(),
        }))
        .await?;
        s.execute(pb::command::Cmd::Show(pb::Show {
            result: String::new(),
            component: String::new(),
            opts: HashMap::new(),
        }))
        .await?;
    }
    Ok(())
}

fn spawn_delta_pump(
    mut sub: tonic::Streaming<pb::StateDelta>,
) -> std::sync::mpsc::Receiver<pb::StateDelta> {
    let (tx, rx) = std::sync::mpsc::channel();
    tokio::spawn(async move {
        while let Ok(Some(delta)) = sub.message().await {
            if tx.send(delta).is_err() {
                break;
            }
        }
    });
    rx
}

impl Session {
    /// Spawn an in-process server, subscribe, and (optionally)
    /// `load <root>`. The returned session is *attached idle* if a
    /// root was given and opened, else *not attached*.
    ///
    /// # Errors
    /// Returns an error if the transport fails to connect, the
    /// subscription cannot open, or an initial `load` is rejected.
    pub async fn connect_in_process(root: Option<&str>) -> Result<Self, BoxErr> {
        Self::connect_in_process_with(VizService::builder().build(), root).await
    }

    /// Spawn an in-process server backed by a caller-built
    /// [`VizService`] (Phase 5 M6 — lets the windowed app plug in a
    /// `MockAgent` via `agent_backend(...)` so the in-process arm
    /// advertises `CAP_AGENT` and the AI Assistant panel lights up).
    /// The transport / subscription / `load` flow is identical to
    /// [`Session::connect_in_process`].
    ///
    /// # Errors
    /// Returns an error if the transport fails to connect, the
    /// subscription cannot open, or an initial `load` is rejected.
    pub async fn connect_in_process_with(
        svc: VizService,
        root: Option<&str>,
    ) -> Result<Self, BoxErr> {
        let (mut client, server) = spawn_in_process(svc.clone()).await?;

        // Phase 5 M6: run Hello on the in-process arm too so the
        // capability gate (CAP_AGENT) is detected consistently across
        // transports.
        let hello = client
            .hello(Request::new(pb::HelloRequest {
                protocol_version: pb::PROTOCOL_VERSION.to_string(),
                client_id: "mili-viz-client".to_string(),
                ..Default::default()
            }))
            .await?
            .into_inner();

        let sub = client
            .subscribe(Request::new(pb::SubscribeRequest::default()))
            .await?
            .into_inner();
        let rx = spawn_delta_pump(sub);

        let mut s = Self {
            client,
            transport: Transport::InProcess {
                svc,
                _server: server,
            },
            deltas: rx,
            capabilities: hello.capabilities,
        };
        run_initial_load(&mut s, root).await?;
        Ok(s)
    }

    /// Connect to a running `mili-viz-server` over real gRPC + Arrow
    /// Flight TCP (Phase 5 M5 — `phase-5-m5.md` Decision 90), subscribe,
    /// and (optionally) `load <root>`. `endpoint` is the bare
    /// `host:port` (e.g. `127.0.0.1:50051`); the function prepends
    /// `http://` for tonic's [`Endpoint`]. The Hello handshake is sent
    /// to surface a major-version mismatch as a clear `Err` rather
    /// than a silent runtime regression.
    ///
    /// # Errors
    /// Returns an error if the endpoint URL is malformed, the TCP
    /// connect fails or times out (Decision 93 — 10 s), the `Hello`
    /// reports `compatible == false`, the subscription cannot open,
    /// or an initial `load` is rejected.
    pub async fn connect_tcp(endpoint: &str, root: Option<&str>) -> Result<Self, BoxErr> {
        let url = if endpoint.starts_with("http://") || endpoint.starts_with("https://") {
            endpoint.to_string()
        } else {
            format!("http://{endpoint}")
        };
        let channel = tune(Endpoint::try_from(url)?).connect().await?;
        let mut client = pb::mili_viz_client::MiliVizClient::new(channel.clone());
        let flight = fpb::flight_service_client::FlightServiceClient::new(channel);

        let hello = client
            .hello(Request::new(pb::HelloRequest {
                protocol_version: pb::PROTOCOL_VERSION.to_string(),
                client_id: "mili-viz-client".to_string(),
                ..Default::default()
            }))
            .await?
            .into_inner();
        if !hello.compatible {
            return Err(format!(
                "server protocol mismatch (client {}, server {}): {}",
                pb::PROTOCOL_VERSION,
                hello.server_protocol_version,
                hello.mismatch_detail
            )
            .into());
        }

        let sub = client
            .subscribe(Request::new(pb::SubscribeRequest::default()))
            .await?
            .into_inner();
        let rx = spawn_delta_pump(sub);

        let mut s = Self {
            client,
            transport: Transport::Remote { flight },
            deltas: rx,
            capabilities: hello.capabilities,
        };
        run_initial_load(&mut s, root).await?;
        Ok(s)
    }

    /// Resolve a `~/.griz/sessions/<id>.json` entry (the file the
    /// server binary's `main` writes — `phase-6-m2.md` Decision 56)
    /// and connect to the host/port it advertises. With `id = None`,
    /// pick the **newest live** session (mtime-sorted, pid liveness
    /// via `kill(pid, 0)`); the same algorithm pygriz uses
    /// ([`phase-6-m2.md`](phase-6-m2.md) Decision 57).
    ///
    /// `$GRIZ_SESSIONS_DIR` overrides the default `~/.griz/sessions`
    /// dir; required for the hermetic gate and matched by the server
    /// writer.
    ///
    /// # Errors
    /// Returns an error if the sessions directory is missing or empty,
    /// no live session is found, the explicit `id` file is missing or
    /// malformed, or the resolved `connect_tcp` fails.
    pub async fn attach(id: Option<&str>, root: Option<&str>) -> Result<Self, BoxErr> {
        let info = resolve_session(id)?;
        Self::connect_tcp(&format!("{}:{}", info.host, info.port), root).await
    }

    /// Send one command over the frozen `Execute` RPC.
    ///
    /// # Errors
    /// Returns an error if the transport fails or the server rejects
    /// the command.
    pub async fn execute(&mut self, cmd: pb::command::Cmd) -> Result<(), BoxErr> {
        exec(&mut self.client, cmd).await
    }

    /// Drain every `StateDelta` the background task has buffered.
    #[must_use]
    pub fn poll_deltas(&self) -> Vec<pb::StateDelta> {
        self.deltas.try_iter().collect()
    }

    /// Resolve a broadcast `GeometryRef` to a decoded [`Mesh`].
    /// In-process arm calls [`VizService::fetch_geometry`] directly;
    /// remote arm pulls the blob over Arrow Flight `DoGet` and
    /// concatenates `FlightData.data_body` (`phase-5-m5.md`
    /// Decision 92) — the bytes are identical to what the in-process
    /// arm would return for the same ticket (M6 guarantee).
    ///
    /// # Errors
    /// Returns an error if the ticket does not resolve, the Flight
    /// RPC fails, or the blob does not decode.
    pub async fn resolve_geometry(&mut self, gref: &pb::GeometryRef) -> Result<Mesh, BoxErr> {
        let blob = self.fetch_blob(&gref.flight_ticket).await?;
        Ok(decode_mvg(&blob)?)
    }

    /// Fetch + decode the result catalog (`phase-5-m3.md`
    /// Decision 67). In-process calls [`VizService::fetch_catalog`]
    /// directly; remote fronts the same byte-stable blob via Flight
    /// `DoGet` against the reserved [`CATALOG_TICKET`].
    ///
    /// Returns `None` when no real run is loaded (or the remote
    /// catalog returns `NotFound`) — the caller keeps the static
    /// placeholder so the headless composite gate stays byte-stable
    /// (`bug-tracker.md` VB-001).
    pub async fn fetch_catalog(&mut self) -> Option<ResultCatalog> {
        let blob = match &mut self.transport {
            Transport::InProcess { svc, .. } => svc.fetch_catalog()?,
            Transport::Remote { flight } => flight_get(flight, CATALOG_TICKET).await.ok()?,
        };
        decode_catalog(&blob)
    }

    /// Whether this session connects over the remote transport
    /// (the `-r`/`--attach` paths). Inspectable so the status bar /
    /// the catalog window can label the seam (`phase-5-m5.md`).
    #[must_use]
    pub fn is_remote(&self) -> bool {
        matches!(self.transport, Transport::Remote { .. })
    }

    /// Run one frozen `Query` RPC (`wireframe-parity.md` "What's
    /// still left" #4 — server arm). The server is now real for
    /// primal svars (it dispatches to `Database::query_full` and
    /// inlines the values); derived results return `ok = false` with
    /// a typed error until the geometry-path derived routing is
    /// replicated for `Query`. Callers should check
    /// `QueryReply.ok` and the `data` oneof — the inline carrier is
    /// the only path implemented today (the proto's `flight_ticket`
    /// variant is reserved for large/typed payloads via Arrow
    /// Flight, future work).
    ///
    /// # Errors
    /// Returns a transport-level error if the RPC fails outright. A
    /// server-rejected query (unknown svar, no run loaded, etc.)
    /// surfaces as `Ok(reply)` with `reply.ok == false`; the caller
    /// inspects `reply.error`.
    pub async fn query(
        &mut self,
        req: pb::QueryRequest,
    ) -> Result<pb::QueryReply, BoxErr> {
        let reply = self.client.query(tonic::Request::new(req)).await?.into_inner();
        Ok(reply)
    }

    /// The server's advertised capabilities from `Hello`. Phase 5 M6
    /// — the AI Assistant panel keys off `CAP_AGENT` here.
    #[must_use]
    pub fn capabilities(&self) -> &[String] {
        &self.capabilities
    }

    /// Whether the server advertised the `CAP_AGENT` capability.
    #[must_use]
    pub fn has_cap_agent(&self) -> bool {
        self.capabilities
            .iter()
            .any(|c| c == mili_viz_proto::v1::CAP_AGENT)
    }

    /// Phase 5 M6 — start one agent turn via the frozen `AgentChat`
    /// RPC. `attached_frame` is the optional pre-encoded framebuffer
    /// the client pinned via [`Session::capture_frame`]. Returns the
    /// server-allocated `turn_id` (passed to [`Session::interrupt`]
    /// for barge-in) on success.
    ///
    /// # Errors
    /// Returns an error if the transport fails or the server's
    /// `AgentChatReply.ok == false` (no backend configured, etc.).
    pub async fn agent_chat(
        &mut self,
        text: String,
        attached_frame: Vec<u8>,
        format: String,
    ) -> Result<String, BoxErr> {
        let attach_frame = !attached_frame.is_empty();
        let req = pb::AgentChatRequest {
            text,
            attach_frame,
            attached_frame,
            attached_frame_format: format,
        };
        let mut r = Request::new(req);
        r.metadata_mut()
            .insert(CLIENT_ID_HEADER, "mili-viz-client".parse()?);
        let reply = self.client.agent_chat(r).await?.into_inner();
        if !reply.ok {
            return Err(format!("agent_chat failed: {}", reply.error).into());
        }
        Ok(reply.turn_id)
    }

    /// Phase 5 M6 — barge-in. Empty `turn_id` cancels the current
    /// turn (the frozen-proto convention — see
    /// `proto/mili_viz.proto:437`).
    ///
    /// # Errors
    /// Returns an error if the transport fails. Server replies with
    /// `ok=true` even when there is no active turn (the call's
    /// semantics are "stop whatever is happening"); only an Err here
    /// is a real failure.
    pub async fn interrupt(&mut self, turn_id: String) -> Result<(), BoxErr> {
        let mut r = Request::new(pb::InterruptRequest { turn_id });
        r.metadata_mut()
            .insert(CLIENT_ID_HEADER, "mili-viz-client".parse()?);
        let _ = self.client.interrupt(r).await?;
        Ok(())
    }

    /// Phase 5 M6 — request a server-side framebuffer encode
    /// (`CaptureFrame`). Decision 96: M6's server returns a
    /// deterministic placeholder PNG/JPEG; production-grade
    /// offscreen-render is a follow-up. Returns `(bytes, format)`.
    ///
    /// # Errors
    /// Returns an error if the transport fails or the server reply's
    /// `ok == false` (e.g. zero-extent request).
    pub async fn capture_frame(
        &mut self,
        width: u32,
        height: u32,
        format: String,
    ) -> Result<(Vec<u8>, String), BoxErr> {
        let req = pb::FrameRequest {
            width,
            height,
            format,
        };
        let mut r = Request::new(req);
        r.metadata_mut()
            .insert(CLIENT_ID_HEADER, "mili-viz-client".parse()?);
        let reply = self.client.capture_frame(r).await?.into_inner();
        if !reply.ok {
            return Err(format!("capture_frame failed: {}", reply.error).into());
        }
        Ok((reply.image, reply.format))
    }

    async fn fetch_blob(&mut self, ticket: &[u8]) -> Result<Vec<u8>, BoxErr> {
        match &mut self.transport {
            Transport::InProcess { svc, .. } => svc
                .fetch_geometry(ticket)
                .ok_or_else(|| "GeometryRef ticket did not resolve in the in-process store".into()),
            Transport::Remote { flight } => flight_get(flight, ticket).await,
        }
    }
}

/// Pull a Flight `DoGet` ticket back as a single `Vec<u8>`,
/// concatenating `FlightData.data_body` across the server stream
/// (`phase-5-m5.md` Decision 92; mirrors the `m6_transport.rs` test).
async fn flight_get(flight: &mut FlightClient, ticket: &[u8]) -> Result<Vec<u8>, BoxErr> {
    let mut stream = flight
        .do_get(Request::new(fpb::Ticket {
            ticket: ticket.to_vec(),
        }))
        .await?
        .into_inner();
    let mut out = Vec::new();
    while let Some(fd) = stream.message().await? {
        out.extend_from_slice(&fd.data_body);
    }
    Ok(out)
}

// ──────────────────────────────────────────────────────────────────────
// In-process session/connection-file publisher
// (`wireframe-parity-5.md` Decisions 109–111). The default windowed
// GUI runs an in-process server (no TCP socket) — without this writer
// no `~/.griz/sessions/<id>.json` exists for a sibling `pygriz` script
// to `attach()` against. We extend the Decision-56 session file with
// two new optional fields (`transport` + `socket_path`) and publish a
// UDS the in-process server side-binds; pygriz reads the discriminator
// and connects over `unix:<socket_path>` instead of TCP. The frozen
// wire contract is untouched — the bytes on the UDS are the same
// MiliViz/Flight HTTP/2 frames a TCP client sees.
// ──────────────────────────────────────────────────────────────────────

/// Guards the lifetime of an in-process session file + UDS socket
/// file: on Drop, removes both best-effort so a normal GUI exit leaves
/// `~/.griz/sessions/` clean. A force-killed GUI leaves the JSON
/// behind; the read side filters by pid-liveness (Decision 57) so a
/// stale entry does not poison `attach()`.
#[cfg(unix)]
pub(crate) struct InProcessSessionGuard {
    json_path: PathBuf,
    socket_path: PathBuf,
    /// Kept alive so the UDS listener task runs as long as the guard.
    /// Dropped on shutdown — tokio detaches the task, which then exits
    /// when its listener (held by the spawned future) goes away with
    /// the runtime. We never join it: the GUI's tokio runtime is
    /// torn down before Drop runs anyway.
    _server: tokio::task::JoinHandle<()>,
}

#[cfg(unix)]
impl Drop for InProcessSessionGuard {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.json_path);
        let _ = std::fs::remove_file(&self.socket_path);
    }
}

/// Pick the UDS path the in-process server will publish at.
/// Constraints (per Decision 110):
///   * absolute & short — macOS `sun_path` caps at 104 bytes,
///   * unique-per-process — id encodes pid+nanos so concurrent GUIs
///     never collide,
///   * tmpdir-resident so the OS reclaims it on reboot if a force-kill
///     skipped Drop.
/// `/tmp` is preferred over `$TMPDIR` (the macOS `/var/folders/...`
/// path is often >80 bytes and crowds the sun_path budget).
#[cfg(unix)]
fn pick_uds_path(id: &str) -> PathBuf {
    let dir = if std::path::Path::new("/tmp").is_dir() {
        PathBuf::from("/tmp")
    } else {
        std::env::temp_dir()
    };
    dir.join(format!("griz-{id}.sock"))
}

/// Generate a session id and a token — same `{:08x}` / `{:016x}` shape
/// the server binary's `main.rs` uses (Decision 56) so a session file
/// the GUI writes is byte-shape-identical to one the binary writes,
/// modulo the two new optional `transport`/`socket_path` fields.
#[cfg(unix)]
fn fresh_id_and_token() -> (String, String) {
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
    (id, token)
}

#[cfg(unix)]
fn mix(a: u128, b: u128) -> u64 {
    let mut x = a
        .wrapping_mul(0x2545_F491_4F6C_DD1D)
        .wrapping_add(b.wrapping_mul(0x9E37_79B9_7F4A_7C15));
    x ^= x >> 33;
    x = x.wrapping_mul(0xFF51_AFD7_ED55_8CCD);
    x ^= x >> 33;
    (x & u128::from(u64::MAX)) as u64
}

/// Write a session JSON (atomic temp+rename, the Jupyter pattern the
/// server binary already uses) with the new in-process discriminator.
/// `socket_path` is the UDS already bound by `serve_uds`.
#[cfg(unix)]
fn write_in_process_session_file(
    id: &str,
    token: &str,
    socket_path: &std::path::Path,
) -> std::io::Result<PathBuf> {
    use std::fmt::Write as _;
    use std::io::Write as _;

    let dir = sessions_dir();
    std::fs::create_dir_all(&dir)?;
    let path = dir.join(format!("{id}.json"));

    let pid = std::process::id();
    let mut json = String::new();
    json.push_str("{\n");
    let _ = writeln!(json, "  \"id\": \"{}\",", json_escape(id));
    let _ = writeln!(json, "  \"pid\": {pid},");
    // `host`/`port` are kept (existing field shape — pygriz/Rust
    // readers may parse them eagerly) but set to the in-process
    // sentinels so an older client that ignores `transport` and tries
    // to TCP-connect fails loud (`connect 127.0.0.1:0` → error) rather
    // than silently landing on the wrong server.
    let _ = writeln!(json, "  \"host\": \"127.0.0.1\",");
    let _ = writeln!(json, "  \"port\": 0,");
    let _ = writeln!(json, "  \"token\": \"{}\",", json_escape(token));
    let _ = writeln!(
        json,
        "  \"protocol_version\": \"{}\",",
        json_escape(mili_viz_proto::v1::PROTOCOL_VERSION)
    );
    let _ = writeln!(json, "  \"transport\": \"in-process\",");
    let _ = writeln!(
        json,
        "  \"socket_path\": \"{}\",",
        json_escape(&socket_path.display().to_string())
    );
    let _ = writeln!(json, "  \"db\": \"\"");
    json.push_str("}\n");

    let tmp = dir.join(format!(".{id}.json.{pid}.tmp"));
    {
        let mut f = std::fs::File::create(&tmp)?;
        f.write_all(json.as_bytes())?;
        f.flush()?;
    }
    std::fs::rename(&tmp, &path)?;
    Ok(path)
}

/// Minimal JSON string escaper for the handful of fields above.
/// Mirrors the server binary's `escape` (Decision 56) so the two
/// writers produce byte-compatible output for the shared fields.
#[cfg(unix)]
fn json_escape(s: &str) -> String {
    use std::fmt::Write as _;
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

/// Publish the in-process session: bind a UDS on the same `VizService`
/// the in-process client speaks to, then write `<sessions_dir>/<id>.json`
/// pointing at it. Called from the windowed shell's default-transport
/// path (the only arm that has no other session file source).
///
/// Returns a guard whose `Drop` removes both files on a clean GUI
/// exit.
///
/// # Errors
/// Returns an error if the UDS cannot bind (path-length collision /
/// permission) or the session file cannot be written.
#[cfg(unix)]
pub(crate) async fn publish_in_process_session(
    svc: mili_viz_server::VizService,
) -> Result<InProcessSessionGuard, BoxErr> {
    let (id, token) = fresh_id_and_token();
    let socket_path = pick_uds_path(&id);
    let (bound, server) = mili_viz_server::serve_uds(svc, &socket_path).await?;
    let json_path = write_in_process_session_file(&id, &token, &bound)?;
    eprintln!(
        "mili-viz-client: in-process session file {} (uds {})",
        json_path.display(),
        bound.display()
    );
    Ok(InProcessSessionGuard {
        json_path,
        socket_path: bound,
        _server: server,
    })
}

#[cfg(not(unix))]
pub(crate) struct InProcessSessionGuard;

// ──────────────────────────────────────────────────────────────────────
// `--attach` session-file resolver (mirrors python/pygriz/.../attach()).
// ──────────────────────────────────────────────────────────────────────

/// One parsed `~/.griz/sessions/<id>.json` entry (the same record the
/// server binary's `main` writes — `phase-6-m2.md` Decision 56;
/// `python/pygriz/src/griz/__init__.py:_parse_session_file`).
#[derive(Debug, Clone)]
#[allow(dead_code)] // `id`/`path` are kept for diagnostics / future surface.
pub(crate) struct SessionInfo {
    pub(crate) id: String,
    pub(crate) pid: u32,
    pub(crate) host: String,
    pub(crate) port: u16,
    pub(crate) path: PathBuf,
    pub(crate) mtime: std::time::SystemTime,
}

/// `$GRIZ_SESSIONS_DIR` (hermetic tests / redirection) else
/// `~/.griz/sessions`. Must match the server writer and the pygriz
/// reader.
pub(crate) fn sessions_dir() -> PathBuf {
    if let Some(d) = std::env::var_os("GRIZ_SESSIONS_DIR") {
        return PathBuf::from(d);
    }
    let home = std::env::var_os("HOME").map_or_else(|| PathBuf::from("."), PathBuf::from);
    home.join(".griz").join("sessions")
}

/// Parse one session file. Returns `None` (never raises) for a
/// missing/partial/malformed file so a stale or half-written sibling
/// can never break `attach()`/`list_sessions()` (the Decision-56
/// staleness-handled-read-side discipline).
fn parse_session_file(path: &Path) -> Option<SessionInfo> {
    let text = std::fs::read_to_string(path).ok()?;
    // Avoid a serde_json dep growth on the client: the JSON is a
    // small flat object the server writes ourselves; a tiny scan is
    // enough. Strings: "key": "value"; numbers: "key": 12345.
    let id = json_str(&text, "id")?;
    let pid: u32 = json_num(&text, "pid")?.parse().ok()?;
    let host = json_str(&text, "host")?;
    let port: u16 = json_num(&text, "port")?.parse().ok()?;
    let mtime = path.metadata().ok()?.modified().ok()?;
    Some(SessionInfo {
        id,
        pid,
        host,
        port,
        path: path.to_path_buf(),
        mtime,
    })
}

/// Tiny tolerant extractor for `"key": "value"`. Not a general JSON
/// parser — only the four fields above ever leave the server writer's
/// hand-formatted JSON (see `crates/mili-viz-server/src/main.rs`'s
/// `write_session_file`), so a focused scanner is correct and saves a
/// new dep on the client.
fn json_str(text: &str, key: &str) -> Option<String> {
    let needle = format!("\"{key}\":");
    let start = text.find(&needle)? + needle.len();
    let after = &text[start..];
    let q = after.find('"')? + 1;
    let rest = &after[q..];
    let end = rest.find('"')?;
    Some(rest[..end].to_string())
}

fn json_num(text: &str, key: &str) -> Option<String> {
    let needle = format!("\"{key}\":");
    let start = text.find(&needle)? + needle.len();
    let rest = &text[start..];
    // Skip whitespace, then take digits.
    let trimmed = rest.trim_start();
    let end = trimmed
        .find(|c: char| !c.is_ascii_digit())
        .unwrap_or(trimmed.len());
    if end == 0 {
        None
    } else {
        Some(trimmed[..end].to_string())
    }
}

#[cfg(unix)]
fn pid_alive(pid: u32) -> bool {
    if pid == 0 {
        return false;
    }
    // SAFETY: `kill(pid, 0)` is a query — sends no signal. ESRCH ⇒ no
    // such process; EPERM (permission denied) ⇒ the process exists.
    let r = unsafe { libc_kill(pid as i32, 0) };
    if r == 0 {
        return true;
    }
    // errno = EPERM (1) ⇒ alive (Decision-57 generosity); ESRCH (3)
    // ⇒ dead. The errno value is read via the libc-free path below.
    let errno = std::io::Error::last_os_error().raw_os_error().unwrap_or(0);
    errno == 1 // EPERM: process exists but we cannot signal it.
}

#[cfg(not(unix))]
fn pid_alive(_pid: u32) -> bool {
    // Best-effort on non-Unix: treat as alive (never hide a session we
    // can't disprove — mirror pygriz's tolerance).
    true
}

#[cfg(unix)]
extern "C" {
    /// Stripped-down `kill(2)` binding — avoids pulling in the `libc`
    /// crate just for one extern (the client's dep surface stays
    /// minimal). `pid_t` is `i32` on every Unix Rust supports.
    #[link_name = "kill"]
    fn libc_kill(pid: i32, sig: i32) -> i32;
}

fn list_sessions_in(dir: &Path) -> Vec<SessionInfo> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut out: Vec<SessionInfo> = entries
        .filter_map(Result::ok)
        .filter(|e| {
            e.path()
                .extension()
                .is_some_and(|x| x.eq_ignore_ascii_case("json"))
        })
        .filter_map(|e| parse_session_file(&e.path()))
        .collect();
    out.sort_by(|a, b| b.mtime.cmp(&a.mtime));
    out
}

fn resolve_session(id: Option<&str>) -> Result<SessionInfo, BoxErr> {
    let dir = sessions_dir();
    if let Some(id) = id {
        let path = dir.join(format!("{id}.json"));
        return parse_session_file(&path).ok_or_else(|| {
            format!(
                "no readable griz session {id:?} in {} \
                 (is the server running and is GRIZ_SESSIONS_DIR set \
                 the same as the server's?)",
                dir.display()
            )
            .into()
        });
    }
    let live: Vec<SessionInfo> = list_sessions_in(&dir)
        .into_iter()
        .filter(|s| pid_alive(s.pid))
        .collect();
    live.into_iter().next().ok_or_else(|| {
        format!(
            "no live griz sessions in {}. Start one with \
             `mili-viz-server` or pass `-r <host:port>` explicitly.",
            dir.display()
        )
        .into()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Serialise the cases that mutate `GRIZ_SESSIONS_DIR`. Cargo runs
    /// tests in this binary in parallel; without this lock, the env
    /// var two cases set independently race and the wrong tmp dir is
    /// observed (the bug surfaced when combining the client and
    /// server test runs increased the cargo-wide parallelism).
    static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    fn write_session_json(
        dir: &Path,
        id: &str,
        pid: u32,
        host: &str,
        port: u16,
    ) -> std::path::PathBuf {
        let path = dir.join(format!("{id}.json"));
        let body = format!(
            "{{\n  \"id\": \"{id}\",\n  \"pid\": {pid},\n  \"host\": \"{host}\",\n  \"port\": {port},\n  \"token\": \"\",\n  \"protocol_version\": \"0\",\n  \"db\": \"\"\n}}\n"
        );
        std::fs::write(&path, body).unwrap();
        path
    }

    #[test]
    fn json_str_and_num_round_trip_the_server_format() {
        let text = "{ \"id\": \"abc\", \"pid\": 4242, \"host\": \"1.2.3.4\", \"port\": 50051 }";
        assert_eq!(json_str(text, "id").as_deref(), Some("abc"));
        assert_eq!(json_str(text, "host").as_deref(), Some("1.2.3.4"));
        assert_eq!(json_num(text, "pid").as_deref(), Some("4242"));
        assert_eq!(json_num(text, "port").as_deref(), Some("50051"));
        assert_eq!(json_str(text, "missing"), None);
    }

    #[test]
    fn parse_session_file_round_trips() {
        let tmp =
            std::env::temp_dir().join(format!("mili-viz-client-attach-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();
        let path = write_session_json(&tmp, "deadbeef", 1, "127.0.0.1", 7777);
        let info = parse_session_file(&path).unwrap();
        assert_eq!(info.id, "deadbeef");
        assert_eq!(info.pid, 1);
        assert_eq!(info.host, "127.0.0.1");
        assert_eq!(info.port, 7777);
        std::fs::remove_dir_all(&tmp).ok();
    }

    #[test]
    fn list_sessions_in_sorts_newest_first_and_skips_malformed() {
        let tmp = std::env::temp_dir().join(format!("mili-viz-client-list-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();
        let _a = write_session_json(&tmp, "aaaa", 1, "h1", 1);
        std::thread::sleep(std::time::Duration::from_millis(20));
        let _b = write_session_json(&tmp, "bbbb", 2, "h2", 2);
        std::fs::write(tmp.join("not-json.json"), "this is not json").unwrap();
        let listed = list_sessions_in(&tmp);
        assert_eq!(listed.len(), 2, "malformed skipped, two parse");
        assert_eq!(listed[0].id, "bbbb", "newest mtime first");
        assert_eq!(listed[1].id, "aaaa");
        std::fs::remove_dir_all(&tmp).ok();
    }

    #[test]
    fn resolve_session_missing_id_is_a_clear_error() {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let tmp =
            std::env::temp_dir().join(format!("mili-viz-client-resolve-id-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();
        let prev = std::env::var_os("GRIZ_SESSIONS_DIR");
        std::env::set_var("GRIZ_SESSIONS_DIR", &tmp);
        let e = resolve_session(Some("nope")).unwrap_err().to_string();
        if let Some(p) = prev {
            std::env::set_var("GRIZ_SESSIONS_DIR", p);
        } else {
            std::env::remove_var("GRIZ_SESSIONS_DIR");
        }
        assert!(
            e.contains("no readable griz session"),
            "explicit-id-missing surfaces clearly: {e}"
        );
        std::fs::remove_dir_all(&tmp).ok();
    }

    #[test]
    fn resolve_session_empty_dir_errors() {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let tmp = std::env::temp_dir().join(format!(
            "mili-viz-client-resolve-empty-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();
        let prev = std::env::var_os("GRIZ_SESSIONS_DIR");
        std::env::set_var("GRIZ_SESSIONS_DIR", &tmp);
        let e = resolve_session(None).unwrap_err().to_string();
        if let Some(p) = prev {
            std::env::set_var("GRIZ_SESSIONS_DIR", p);
        } else {
            std::env::remove_var("GRIZ_SESSIONS_DIR");
        }
        assert!(
            e.contains("no live griz sessions"),
            "empty sessions dir surfaces clearly: {e}"
        );
        std::fs::remove_dir_all(&tmp).ok();
    }

    // ─────────────────────────────────────────────────────────────
    // In-process publisher (`wireframe-parity-5.md` Decisions 109–111).
    // ─────────────────────────────────────────────────────────────

    /// One combined test for the publisher: file shape + live UDS +
    /// Drop cleanup. Combined because `cargo test` runs cases in
    /// parallel and they share the `GRIZ_SESSIONS_DIR` env var — a
    /// split case would race the env stomping of the other.
    #[cfg(unix)]
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn publish_in_process_session_round_trip() {
        use hyper_util::rt::TokioIo;
        use tonic::transport::{Endpoint, Uri};
        use tower::service_fn;

        let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let tmp = std::env::temp_dir().join(format!(
            "mili-viz-client-publish-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();
        let prev = std::env::var_os("GRIZ_SESSIONS_DIR");
        std::env::set_var("GRIZ_SESSIONS_DIR", &tmp);

        let svc = mili_viz_server::VizService::builder().build();
        let guard = publish_in_process_session(svc).await.unwrap();

        // Exactly one *.json with the new discriminator + legacy
        // fields. host/port are written as `127.0.0.1`/`0` so a
        // pre-Decision-109 reader that ignores `transport` fails loud
        // on TCP-connect (Decision 110) rather than landing somewhere
        // wrong by silent mis-route.
        let jsons: Vec<_> = std::fs::read_dir(&tmp)
            .unwrap()
            .filter_map(Result::ok)
            .filter(|e| {
                e.path()
                    .extension()
                    .is_some_and(|x| x.eq_ignore_ascii_case("json"))
            })
            .collect();
        assert_eq!(jsons.len(), 1, "exactly one session file published");
        let json_path = jsons[0].path();
        let text = std::fs::read_to_string(&json_path).unwrap();
        assert!(
            text.contains("\"transport\": \"in-process\""),
            "missing transport discriminator:\n{text}"
        );
        assert!(text.contains("\"socket_path\":"));
        assert!(text.contains("\"host\": \"127.0.0.1\""));
        assert!(text.contains("\"port\": 0"));
        assert_eq!(json_str(&text, "transport").as_deref(), Some("in-process"));

        // The advertised UDS exists and the router is alive on it.
        let socket = json_str(&text, "socket_path").unwrap();
        assert!(
            std::fs::metadata(&socket).is_ok(),
            "uds socket_path {socket} not present on disk"
        );
        let socket_owned = std::path::PathBuf::from(&socket);
        let channel = Endpoint::try_from("http://uds.invalid")
            .unwrap()
            .connect_with_connector(service_fn(move |_: Uri| {
                let path = socket_owned.clone();
                async move {
                    let io = tokio::net::UnixStream::connect(&path).await?;
                    Ok::<_, std::io::Error>(TokioIo::new(io))
                }
            }))
            .await
            .expect("uds dial");
        let mut client = pb::mili_viz_client::MiliVizClient::new(channel);
        let reply = client
            .hello(tonic::Request::new(pb::HelloRequest {
                protocol_version: pb::PROTOCOL_VERSION.to_string(),
                client_id: "publish-test".into(),
                ..Default::default()
            }))
            .await
            .expect("hello rpc")
            .into_inner();
        assert!(reply.compatible, "published UDS speaks the same router");

        // Drop → both files cleaned (Decision 111: clean exit leaves
        // nothing behind; a force-killed GUI leaves the JSON which
        // the read-side pid-liveness filters out).
        drop(guard);
        assert!(!json_path.exists(), "Drop removes the session JSON");
        assert!(
            !std::path::Path::new(&socket).exists(),
            "Drop removes the UDS socket file ({socket})"
        );

        if let Some(p) = prev {
            std::env::set_var("GRIZ_SESSIONS_DIR", p);
        } else {
            std::env::remove_var("GRIZ_SESSIONS_DIR");
        }
        std::fs::remove_dir_all(&tmp).ok();
    }
}
