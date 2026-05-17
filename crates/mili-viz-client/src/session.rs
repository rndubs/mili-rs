//! Drive the frozen `mili-viz` contract over the **in-process**
//! transport and decode the returned geometry (`phase-5-m2.md`
//! Decision 41).
//!
//! This is the README run-mode-1 path: the client spawns a
//! `mili-viz-server` in the same binary (`spawn_in_process`, the
//! M1 acceptance-gate transport), subscribes, sends `load`/`show`,
//! reads the broadcast `DELTA_RESULT`'s `GeometryRef`, and resolves
//! its `flight_ticket` through the in-process
//! `VizService::fetch_geometry` seam (`phase-4-m2.md` Decision 10).
//! Remote mode over gRPC + Flight TCP is Phase 5 M5.

use std::collections::HashMap;
use std::error::Error;

use mili_viz_proto::v1 as pb;
use mili_viz_server::{spawn_in_process, VizService, CLIENT_ID_HEADER};
use tonic::Request;

use crate::mesh::{decode_mvg, Mesh};

type BoxErr = Box<dyn Error + Send + Sync>;

async fn exec(
    client: &mut pb::mili_viz_client::MiliVizClient<tonic::transport::Channel>,
    cmd: pb::command::Cmd,
) -> Result<(), BoxErr> {
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

/// A live in-process session for the M3 windowed shell
/// (`phase-5-m3.md` Decision 46): an owned `Execute` client, the
/// in-process geometry seam, and a background task forwarding the
/// `Subscribe` broadcast through a channel the UI drains every frame.
/// `fetch_server_mesh` above is the M2 one-shot and is kept unchanged.
pub struct Session {
    client: pb::mili_viz_client::MiliVizClient<tonic::transport::Channel>,
    svc: VizService,
    deltas: std::sync::mpsc::Receiver<pb::StateDelta>,
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
        let svc = VizService::builder().build();
        let (mut client, _server) = spawn_in_process(svc.clone()).await?;

        let mut sub = client
            .subscribe(Request::new(pb::SubscribeRequest::default()))
            .await?
            .into_inner();

        let (tx, rx) = std::sync::mpsc::channel();
        tokio::spawn(async move {
            while let Ok(Some(delta)) = sub.message().await {
                if tx.send(delta).is_err() {
                    break;
                }
            }
        });

        let mut s = Self {
            client,
            svc,
            deltas: rx,
        };
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
        Ok(s)
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

    /// Resolve a broadcast `GeometryRef` through the in-process
    /// geometry seam and decode it.
    ///
    /// # Errors
    /// Returns an error if the ticket does not resolve or the blob
    /// fails to decode.
    pub fn resolve_geometry(&self, gref: &pb::GeometryRef) -> Result<Mesh, BoxErr> {
        let blob = self
            .svc
            .fetch_geometry(&gref.flight_ticket)
            .ok_or("GeometryRef ticket did not resolve in the in-process store")?;
        Ok(decode_mvg(&blob)?)
    }
}
