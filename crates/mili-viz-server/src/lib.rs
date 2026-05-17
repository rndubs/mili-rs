//! Phase 4 M1 `mili-viz` server stub.
//!
//! Scope is exactly the `phase-4-m1.md` § "M1 acceptance gate":
//! the frozen wire contract + the in-process transport + the
//! dispatch/broadcast plumbing. **No `mili-rs` wiring, no real
//! geometry/colors, no renderer, no LLM backend** — those are
//! M2+/M6 (Decision 7 table). `GeometryRef` is left empty until M2.
//!
//! Per Decision 7, `Hello` / `Subscribe` / `Execute` / `Query` are
//! live (the latter as shape/plumbing); `AgentChat` / `Interrupt` /
//! `CaptureFrame` return `UNIMPLEMENTED` naming the gating milestone.
//!
//! ## M1 implementation note — `Command` → `DeltaKind` is many-to-one
//!
//! The frozen proto's `DeltaKind` enumerates *which aspect of session
//! state changed*, not one value per `Command`. Several commands fold
//! onto a shared kind by design (e.g. `show`/`contour`/`cmap`/
//! `legend`/`cutpln`/`render` → `DELTA_RESULT`; `rot`/…/named-view →
//! `DELTA_CAMERA`). The `command_delta_kind` table below is the
//! authoritative mapping the conformance test pins; see
//! `phase-4-m1.md` § "M1 implementation notes" Decision 8.

#![allow(clippy::pedantic)]

use std::collections::BTreeMap;
use std::pin::Pin;
use std::sync::{Arc, Mutex};

use mili_rs::Database;
use mili_viz_proto::v1 as pb;
use pb::mili_viz_server::{MiliViz, MiliVizServer};
use tokio_stream::wrappers::BroadcastStream;
use tokio_stream::{Stream, StreamExt};
use tonic::{Request, Response, Status};

mod geometry;
mod raw;
use geometry::MeshTopology;
pub use raw::{parse_line, parse_raw, to_raw};

/// Metadata header carrying the caller's client id. In-process
/// callers set this so broadcasts can be tagged with
/// `origin_client_id` (the proto `Command` has no client field; the
/// connection identity is the natural place for it).
pub const CLIENT_ID_HEADER: &str = "x-client-id";

const BROADCAST_CAP: usize = 256;

fn default_camera() -> pb::CameraState {
    pb::CameraState {
        azimuth: 0.0,
        elevation: 0.0,
        distance: 1.0,
        fx: 0.0,
        fy: 0.0,
        fz: 0.0,
    }
}

/// Server-authoritative session state. M1 tracks the *shape* of every
/// aspect a `StateDelta` carries; the heavy artefacts (mesh, colors,
/// derived values) are M2+ and left empty here.
#[derive(Default)]
struct Session {
    seq: u64,
    loaded: Option<pb::LoadedState>,
    state: u32,
    selection: BTreeMap<String, String>,
    result: Option<pb::ResultState>,
    isosurface: Option<pb::IsosurfaceState>,
    camera: pb::CameraState,
    materials: BTreeMap<u32, bool>,
    named_views: BTreeMap<String, pb::CameraState>,
    // M2: a `mili-rs`-backed run + its prepped topology. `None` until
    // a `load` of an openable root succeeds (a non-openable root keeps
    // the M1 stub `LoadedState`, leaving these `None`).
    db: Option<Database>,
    topo: Option<MeshTopology>,
    // In-process geometry store keyed by the frozen
    // `GeometryRef.flight_ticket` (phase-4-m2.md Decision 10). M6
    // swaps this for an Arrow-Flight `DoGet` over TCP.
    geom: BTreeMap<Vec<u8>, Vec<u8>>,
    geom_order: Vec<Vec<u8>>,
    geom_seq: u64,
}

/// Cap on retained geometry blobs (phase-4-m2.md Decision 10). Tickets
/// stay valid until evicted FIFO; the active client always holds the
/// freshest.
const GEOM_STORE_CAP: usize = 16;

impl Session {
    /// 1-based state-count bound, 0 when no real run is loaded (the M1
    /// stub `LoadedState` carries `num_states == 0`).
    fn num_states(&self) -> u32 {
        self.loaded.as_ref().map_or(0, |l| l.num_states)
    }

    /// Clamp the cursor to `[1, num_states]` once a real run is loaded;
    /// leave it untouched otherwise (phase-4-m2.md Decision 12 — the
    /// frozen M1 tests never open a database and must be unaffected).
    fn clamp_state(&mut self) {
        let n = self.num_states();
        if n > 0 {
            self.state = self.state.clamp(1, n);
        }
    }

    /// Encode the current-state hull, file it under a fresh ticket,
    /// and return the `GeometryRef` plus the scalar `(min, max)`
    /// range. A non-empty `svar` that resolves adds the per-vertex
    /// scalar field (`MVG2`, phase-4-m3.md Decisions 13–15);
    /// otherwise the M2 bare hull (`MVG1`, range `(0, 0)`). `None`
    /// when no real mesh is loaded (M1 behavior — `GeometryRef` stays
    /// empty, frozen tests green).
    fn geometry_ref(&mut self, svar: &str) -> Option<(pb::GeometryRef, f64, f64)> {
        let topo = self.topo.as_ref()?;
        let db = self.db.as_ref()?;
        let scalar = topo.vertex_scalar(db, svar, self.state);
        let materials = &self.materials;
        let ((blob, num_indices), layout, min, max) = match &scalar {
            Some((s, lo, hi)) => (
                topo.encode(db, self.state, Some(s), materials),
                geometry::LAYOUT_SCALAR,
                *lo,
                *hi,
            ),
            None => (
                topo.encode(db, self.state, None, materials),
                geometry::LAYOUT,
                0.0,
                0.0,
            ),
        };
        let num_vertices = topo.num_vertices();
        self.geom_seq += 1;
        let ticket = format!("geom:{}", self.geom_seq).into_bytes();
        self.geom.insert(ticket.clone(), blob);
        self.geom_order.push(ticket.clone());
        if self.geom_order.len() > GEOM_STORE_CAP {
            let old = self.geom_order.remove(0);
            self.geom.remove(&old);
        }
        Some((
            pb::GeometryRef {
                flight_ticket: ticket,
                layout: layout.to_string(),
                num_vertices,
                num_indices,
            },
            min,
            max,
        ))
    }
}

impl Session {
    fn new() -> Self {
        Session {
            camera: default_camera(),
            ..Session::default()
        }
    }

    fn snapshot(&self) -> pb::Snapshot {
        pb::Snapshot {
            loaded: self.loaded.clone(),
            state: self.state,
            selection: Some(pb::SelectionState {
                by_class: self.selection.clone().into_iter().collect(),
            }),
            result: self.result.clone(),
            camera: Some(self.camera),
            materials: Some(pb::MaterialsState {
                visible: self.materials.clone().into_iter().collect(),
            }),
            // Δ8: late joiner gets the running transcript. Empty in
            // M1 — the agent loop is M6 (Decision 6).
            agent: Some(pb::AgentTranscript::default()),
        }
    }
}

struct Inner {
    session: Mutex<Session>,
    tx: tokio::sync::broadcast::Sender<pb::StateDelta>,
    agent: bool,
    expected_token: Option<String>,
}

/// The `MiliViz` service. Construct with [`VizService::builder`].
#[derive(Clone)]
pub struct VizService {
    inner: Arc<Inner>,
}

/// Builder for [`VizService`]. `agent(true)` advertises the `agent`
/// capability (a real deployment sets this iff an LLM backend is
/// configured — Decision 6); the implementation is still M6.
pub struct VizServiceBuilder {
    agent: bool,
    expected_token: Option<String>,
}

impl VizServiceBuilder {
    #[must_use]
    pub fn agent(mut self, on: bool) -> Self {
        self.agent = on;
        self
    }

    #[must_use]
    pub fn expected_token(mut self, token: impl Into<String>) -> Self {
        self.expected_token = Some(token.into());
        self
    }

    #[must_use]
    pub fn build(self) -> VizService {
        let (tx, _) = tokio::sync::broadcast::channel(BROADCAST_CAP);
        VizService {
            inner: Arc::new(Inner {
                session: Mutex::new(Session::new()),
                tx,
                agent: self.agent,
                expected_token: self.expected_token,
            }),
        }
    }
}

impl VizService {
    #[must_use]
    pub fn builder() -> VizServiceBuilder {
        VizServiceBuilder {
            agent: false,
            expected_token: None,
        }
    }

    /// Resolve a `GeometryRef.flight_ticket` to its encoded `MVG1`
    /// blob (phase-4-m2.md Decisions 10 & 11). In M2 the in-process
    /// client calls this directly; M6 fronts the same store with an
    /// Arrow-Flight `DoGet` over TCP (the ticket and blob are
    /// unchanged across that swap).
    #[must_use]
    pub fn fetch_geometry(&self, ticket: &[u8]) -> Option<Vec<u8>> {
        self.inner.session.lock().unwrap().geom.get(ticket).cloned()
    }

    /// Dispatch one parsed command: mutate state, assign a seq, build
    /// the `StateDelta`, broadcast it to every subscriber, return the
    /// seq. This is the single internal entry point shared by typed
    /// `Execute`, the `raw` escape hatch, and (at M6) the agent loop,
    /// which is what makes Layer-0 ≡ raw hold by construction.
    fn dispatch(&self, cmd: pb::command::Cmd, origin: &str) -> u64 {
        let mut s = self.inner.session.lock().unwrap();
        s.seq += 1;
        let seq = s.seq;
        let (kind, payload) = apply(&mut s, cmd);
        let delta = pb::StateDelta {
            seq,
            origin_client_id: origin.to_string(),
            kind: kind as i32,
            payload: Some(payload),
        };
        // `send` errs only when there are no receivers; that is fine.
        let _ = self.inner.tx.send(delta);
        seq
    }
}

/// Authoritative `Command` → `DeltaKind` mapping (M1 impl note —
/// many-to-one by design; pinned by the conformance test).
#[must_use]
pub fn command_delta_kind(cmd: &pb::command::Cmd) -> pb::DeltaKind {
    use pb::command::Cmd;
    use pb::DeltaKind as D;
    match cmd {
        Cmd::Raw(_) => D::DeltaUnspecified, // resolved per parsed line
        Cmd::Load(_) => D::DeltaLoaded,
        Cmd::Close(_) => D::DeltaClosed,
        Cmd::SetState(_) | Cmd::Step(_) => D::DeltaState,
        Cmd::Select(_) | Cmd::Clrsel(_) => D::DeltaSelection,
        Cmd::Show(_)
        | Cmd::Contour(_)
        | Cmd::Colormap(_)
        | Cmd::Legend(_)
        | Cmd::Cutplane(_)
        | Cmd::Render(_) => D::DeltaResult,
        Cmd::View(_) | Cmd::NamedView(_) => D::DeltaCamera,
        Cmd::Iso(_) => D::DeltaIsosurface,
        Cmd::Material(_) => D::DeltaMaterials,
    }
}

/// Apply a command to the session and return the broadcast payload.
/// Geometry/colours/derived values are deliberately empty (M2+).
fn apply(s: &mut Session, cmd: pb::command::Cmd) -> (pb::DeltaKind, pb::state_delta::Payload) {
    use pb::command::Cmd;
    use pb::state_delta::Payload as P;
    use pb::DeltaKind as D;

    match cmd {
        Cmd::Raw(_) => unreachable!("raw is split into typed cmds before dispatch"),

        Cmd::Load(l) => {
            // Try to open a real run. A non-openable root falls back
            // to the M1 stub LoadedState so the frozen M1 acceptance
            // tests (which never point at a real corpus) stay green
            // (phase-4-m2.md Decision 12).
            let loaded = match Database::open(&l.root) {
                Ok(db) => {
                    let num_states = db.state_count() as u32;
                    let state_times = db.times().into_iter().map(f64::from).collect::<Vec<_>>();
                    let topo = MeshTopology::build(&db);
                    let class_names = topo
                        .as_ref()
                        .and_then(|t| {
                            db.meshes()
                                .meshes()
                                .find(|m| m.id == t.mesh_id())
                                .map(|m| m.classes().map(|c| c.short_name.clone()).collect())
                        })
                        .unwrap_or_default();
                    let loaded = pb::LoadedState {
                        db: l.root,
                        num_states,
                        state_times,
                        class_names,
                    };
                    s.db = Some(db);
                    s.topo = topo;
                    s.state = if num_states == 0 { 0 } else { 1 };
                    loaded
                }
                Err(_) => {
                    let loaded = pb::LoadedState {
                        db: l.root,
                        num_states: 0,
                        state_times: vec![],
                        class_names: vec![],
                    };
                    s.db = None;
                    s.topo = None;
                    s.state = 0;
                    loaded
                }
            };
            s.geom.clear();
            s.geom_order.clear();
            s.loaded = Some(loaded.clone());
            (D::DeltaLoaded, P::Loaded(loaded))
        }
        Cmd::Close(_) => {
            // Reset session aspects but preserve the monotonic seq —
            // subscribers correlate on it across a close/reload.
            let seq = s.seq;
            *s = Session::new();
            s.seq = seq;
            (D::DeltaClosed, P::Closed(true))
        }
        Cmd::SetState(st) => {
            s.state = st.state;
            // griz clamps an over-range `state` to the run bounds
            // rather than erroring (phase-4-m2.md Decision 12);
            // no-op when nothing is loaded (M1 behavior preserved).
            s.clamp_state();
            (D::DeltaState, P::State(s.state))
        }
        Cmd::Step(step) => {
            let n = s.num_states();
            s.state = match pb::step::Dir::try_from(step.dir).unwrap_or(pb::step::Dir::Next) {
                pb::step::Dir::Next => s.state.saturating_add(1),
                pb::step::Dir::Prev => s.state.saturating_sub(1).max(1),
                pb::step::Dir::First => 1,
                pb::step::Dir::Last => {
                    if n == 0 {
                        s.state
                    } else {
                        n
                    }
                }
            };
            s.clamp_state();
            (D::DeltaState, P::State(s.state))
        }
        Cmd::Select(sel) => {
            s.selection.insert(sel.class_name, sel.range);
            (D::DeltaSelection, P::Selection(selection_state(s)))
        }
        Cmd::Clrsel(c) => {
            // griz `clrsel`/`poof` with no class clears the whole
            // selection; a named class clears just that class
            // (phase-4-m4.md Decision 17;
            // reference/griz/Src/interpret.c:1450).
            if c.class_name.is_empty() {
                s.selection.clear();
            } else {
                s.selection.remove(&c.class_name);
            }
            (D::DeltaSelection, P::Selection(selection_state(s)))
        }
        Cmd::Show(show) => {
            // M3: the queried svar is `component` if set, else
            // `result` (griz leaf-scalar semantics, phase-4-m3.md
            // Decision 13). An unresolvable result falls back to the
            // M2 bare hull; an empty result is the no-scalar mesh view.
            let svar = if show.component.is_empty() {
                show.result.clone()
            } else {
                show.component.clone()
            };
            let (geometry, min, max) = match s.geometry_ref(&svar) {
                Some((g, lo, hi)) => (Some(g), lo, hi),
                None => (None, 0.0, 0.0),
            };
            let r = pb::ResultState {
                result: show.result,
                component: show.component,
                min,
                max,
                geometry,
            };
            s.result = Some(r.clone());
            (D::DeltaResult, P::Result(r))
        }
        Cmd::Contour(c) => {
            let (geometry, min, max) = match s.geometry_ref(&c.result) {
                Some((g, lo, hi)) => (Some(g), lo, hi),
                None => (None, 0.0, 0.0),
            };
            let r = pb::ResultState {
                result: c.result,
                component: String::new(),
                min,
                max,
                geometry,
            };
            s.result = Some(r.clone());
            (D::DeltaResult, P::Result(r))
        }
        Cmd::Colormap(_) | Cmd::Legend(_) | Cmd::Cutplane(_) | Cmd::Render(_) => {
            // Recolor / rescale / cut / offscreen-render of the
            // *current* result. The visual effect is M3+/M6; M1
            // re-broadcasts the (geometry-stubbed) result state.
            let r = s.result.clone().unwrap_or_default();
            (D::DeltaResult, P::Result(r))
        }
        Cmd::View(v) => {
            apply_view(s, v);
            (D::DeltaCamera, P::Camera(s.camera))
        }
        Cmd::NamedView(nv) => {
            match pb::named_view::Op::try_from(nv.op).unwrap_or(pb::named_view::Op::List) {
                pb::named_view::Op::Save => {
                    s.named_views.insert(nv.name, s.camera);
                }
                pb::named_view::Op::Restore => {
                    if let Some(c) = s.named_views.get(&nv.name) {
                        s.camera = *c;
                    }
                }
                pb::named_view::Op::List => {}
            }
            (D::DeltaCamera, P::Camera(s.camera))
        }
        Cmd::Iso(iso) => {
            let st = pb::IsosurfaceState {
                result: iso.result,
                levels: if iso.levels.is_empty() {
                    iso_levels(iso.count, iso.min, iso.max)
                } else {
                    iso.levels
                },
                geometry: None,
            };
            s.isosurface = if iso.on { Some(st.clone()) } else { None };
            (D::DeltaIsosurface, P::Isosurface(st))
        }
        Cmd::Material(m) => {
            // Track per-material visibility; the geometry filter runs
            // on the next `show` (phase-4-m4.md Decision 16). A `None`
            // material and `class_name` scoping stay no-ops for the
            // filter (Decision 16 trade-off).
            if let Some(mat) = m.material {
                s.materials.insert(mat, m.enable);
            }
            (
                D::DeltaMaterials,
                P::Materials(pb::MaterialsState {
                    visible: s.materials.clone().into_iter().collect(),
                }),
            )
        }
    }
}

fn selection_state(s: &Session) -> pb::SelectionState {
    pb::SelectionState {
        by_class: s.selection.clone().into_iter().collect(),
    }
}

fn iso_levels(count: u32, min: Option<f64>, max: Option<f64>) -> Vec<f64> {
    let (lo, hi) = (min.unwrap_or(0.0), max.unwrap_or(1.0));
    if count == 0 {
        return vec![];
    }
    (1..=count)
        .map(|i| lo + (hi - lo) * f64::from(i) / f64::from(count + 1))
        .collect()
}

fn apply_view(s: &mut Session, v: pb::View) {
    use pb::view::Op;
    let Some(op) = v.op else { return };
    match op {
        Op::Rotate(r) => {
            s.camera.azimuth += r.x;
            s.camera.elevation += r.y;
        }
        Op::Translate(t) => {
            s.camera.fx += t.dx;
            s.camera.fy += t.dy;
            s.camera.fz += t.dz;
        }
        Op::Scale(sc) => {
            if sc.factor != 0.0 {
                s.camera.distance /= sc.factor;
            }
        }
        Op::Zoom(z) => {
            if z.factor != 0.0 {
                s.camera.distance /= z.factor;
            }
        }
        Op::Set(c) => {
            s.camera.azimuth = c.azimuth;
            s.camera.elevation = c.elevation;
            s.camera.distance = c.distance;
            if let Some(x) = c.fx {
                s.camera.fx = x;
            }
            if let Some(y) = c.fy {
                s.camera.fy = y;
            }
            if let Some(z) = c.fz {
                s.camera.fz = z;
            }
        }
        Op::Reset(_) => s.camera = default_camera(),
    }
}

fn client_id<T>(req: &Request<T>) -> String {
    req.metadata()
        .get(CLIENT_ID_HEADER)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string()
}

fn major(v: &str) -> Option<&str> {
    let v = v.trim();
    if v.is_empty() {
        return None;
    }
    Some(v.split('.').next().unwrap_or(v))
}

#[tonic::async_trait]
impl MiliViz for VizService {
    async fn hello(
        &self,
        request: Request<pb::HelloRequest>,
    ) -> Result<Response<pb::HelloReply>, Status> {
        let req = request.into_inner();

        if let Some(expected) = &self.inner.expected_token {
            if &req.session_token != expected {
                return Err(Status::unauthenticated("session token mismatch"));
            }
        }

        // Version negotiation never panics and never errors — a
        // mismatch is a *reported* state so a pip-upgraded client
        // warns instead of segfaulting (scripting.md guarantee).
        let server_v = pb::PROTOCOL_VERSION;
        let (compatible, mismatch_detail) = match major(&req.protocol_version) {
            None => (false, "client sent an empty protocol_version".to_string()),
            Some(cm) if Some(cm) == major(server_v) => (true, String::new()),
            Some(cm) => (
                false,
                format!(
                    "protocol major mismatch: client {cm}.x vs server {} ({server_v})",
                    major(server_v).unwrap_or("?")
                ),
            ),
        };

        let mut capabilities = Vec::new();
        if self.inner.agent {
            capabilities.push(pb::CAP_AGENT.to_string());
        }

        let db = self
            .inner
            .session
            .lock()
            .unwrap()
            .loaded
            .as_ref()
            .map(|l| l.db.clone())
            .unwrap_or_default();

        Ok(Response::new(pb::HelloReply {
            server_protocol_version: server_v.to_string(),
            compatible,
            mismatch_detail,
            capabilities,
            session: Some(pb::SessionInfo {
                id: "in-process".to_string(),
                pid: std::process::id(),
                host: "localhost".to_string(),
                port: 0,
                db,
            }),
        }))
    }

    async fn execute(
        &self,
        request: Request<pb::Command>,
    ) -> Result<Response<pb::CommandReply>, Status> {
        let origin = client_id(&request);
        let Some(cmd) = request.into_inner().cmd else {
            return Ok(Response::new(pb::CommandReply {
                ok: false,
                error: "empty command".to_string(),
                delta_seq: 0,
            }));
        };

        let cmds = match cmd {
            pb::command::Cmd::Raw(line) => match parse_raw(&line) {
                Ok(c) => c,
                Err(e) => {
                    return Ok(Response::new(pb::CommandReply {
                        ok: false,
                        error: format!("parse error: {e}"),
                        delta_seq: 0,
                    }))
                }
            },
            typed => vec![typed],
        };

        if cmds.is_empty() {
            return Ok(Response::new(pb::CommandReply {
                ok: false,
                error: "no command".to_string(),
                delta_seq: 0,
            }));
        }

        let mut last = 0;
        for c in cmds {
            last = self.dispatch(c, &origin);
        }
        Ok(Response::new(pb::CommandReply {
            ok: true,
            error: String::new(),
            delta_seq: last,
        }))
    }

    type SubscribeStream =
        Pin<Box<dyn Stream<Item = Result<pb::StateDelta, Status>> + Send + 'static>>;

    async fn subscribe(
        &self,
        request: Request<pb::SubscribeRequest>,
    ) -> Result<Response<Self::SubscribeStream>, Status> {
        let kinds = request.into_inner().kinds;
        let want = move |k: i32| kinds.is_empty() || kinds.contains(&k);

        // Take the opening snapshot and subscribe under the same lock
        // so a concurrent Execute cannot slip a delta between the
        // snapshot and the first streamed item (late-subscriber
        // ordering guarantee).
        let (snapshot_delta, rx) = {
            let s = self.inner.session.lock().unwrap();
            let delta = pb::StateDelta {
                seq: s.seq,
                origin_client_id: String::new(),
                kind: pb::DeltaKind::DeltaSnapshot as i32,
                payload: Some(pb::state_delta::Payload::Snapshot(s.snapshot())),
            };
            (delta, self.inner.tx.subscribe())
        };

        let want2 = want.clone();
        let tail = BroadcastStream::new(rx).filter_map(move |item| match item {
            Ok(d) if want2(d.kind) => Some(Ok(d)),
            Ok(_) => None,
            // A lagged receiver drops messages rather than crashing.
            Err(_) => None,
        });

        let head = tokio_stream::iter(
            std::iter::once(snapshot_delta)
                .filter(move |d| want(d.kind))
                .map(Ok),
        );

        Ok(Response::new(Box::pin(head.chain(tail))))
    }

    async fn query(
        &self,
        request: Request<pb::QueryRequest>,
    ) -> Result<Response<pb::QueryReply>, Status> {
        // Shape/plumbing only in M1; real values need `mili-rs`
        // wired in at M2/M3 (Decision 7 table).
        let q = request.into_inner();
        Ok(Response::new(pb::QueryReply {
            ok: true,
            error: String::new(),
            data: Some(pb::query_reply::Data::Inline(pb::InlineTable {
                labels: q.labels.clone(),
                states: q.states.clone(),
                values: vec![],
                components: 0,
            })),
        }))
    }

    async fn agent_chat(
        &self,
        _request: Request<pb::AgentChatRequest>,
    ) -> Result<Response<pb::AgentChatReply>, Status> {
        Err(Status::unimplemented(
            "AgentChat is frozen in M1; the agent loop is Phase 4/5 M6 \
             (phase-4-m1.md Decisions 6 & 7)",
        ))
    }

    async fn interrupt(
        &self,
        _request: Request<pb::InterruptRequest>,
    ) -> Result<Response<pb::InterruptReply>, Status> {
        Err(Status::unimplemented(
            "Interrupt is frozen in M1; the agent loop is Phase 4/5 M6 \
             (phase-4-m1.md Decisions 6 & 7)",
        ))
    }

    async fn capture_frame(
        &self,
        _request: Request<pb::FrameRequest>,
    ) -> Result<Response<pb::FrameReply>, Status> {
        Err(Status::unimplemented(
            "CaptureFrame is frozen in M1; it needs the offscreen \
             renderer (Phase 4 M6 / Phase 5)",
        ))
    }
}

/// Spawn the M1 server on an **in-process** (in-memory duplex)
/// transport — no TCP — and return a connected client channel plus
/// the server task handle. This is the M1 acceptance-gate transport.
///
/// # Errors
/// Returns a transport error if the in-memory channel fails to
/// connect (it should not, in practice).
pub async fn spawn_in_process(
    svc: VizService,
) -> Result<
    (
        pb::mili_viz_client::MiliVizClient<tonic::transport::Channel>,
        tokio::task::JoinHandle<()>,
    ),
    Box<dyn std::error::Error + Send + Sync>,
> {
    use hyper_util::rt::TokioIo;
    use tonic::transport::{Endpoint, Server, Uri};
    use tower::service_fn;

    let (client_io, server_io) = tokio::io::duplex(1024 * 1024);

    let handle = tokio::spawn(async move {
        Server::builder()
            .add_service(MiliVizServer::new(svc))
            .serve_with_incoming(tokio_stream::once(Ok::<_, std::io::Error>(server_io)))
            .await
            .expect("in-process server terminated with error");
    });

    let mut client_io = Some(client_io);
    let channel = Endpoint::try_from("http://in-process.invalid")?
        .connect_with_connector(service_fn(move |_: Uri| {
            let io = client_io
                .take()
                .expect("in-process connector invoked more than once");
            async move { Ok::<_, std::io::Error>(TokioIo::new(io)) }
        }))
        .await?;

    Ok((pb::mili_viz_client::MiliVizClient::new(channel), handle))
}
