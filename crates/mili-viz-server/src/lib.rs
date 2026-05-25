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

use mili_rs::{Database, QueryArgs, StateValues};
use mili_viz_proto::v1 as pb;
use pb::mili_viz_server::{MiliViz, MiliVizServer};
use tokio_stream::wrappers::BroadcastStream;
use tokio_stream::{Stream, StreamExt};
use tonic::{Request, Response, Status};

mod agent;
mod clip;
mod flight;
mod geometry;
mod llamacpp_agent;
mod raw;
pub use agent::{
    encode_placeholder_frame, ran_summary, AgentBackend, AgentTurnCtx, DispatchOutcome, MockAgent,
    TurnSnapshot, AGENT_MOCK_ORIGIN,
};
pub use flight::FlightGeometryService;
use geometry::MeshTopology;
pub use llamacpp_agent::LlamaCppAgent;
pub use raw::{parse_line, parse_raw, to_raw};

/// Conventional Flight ticket for the result catalog
/// (`planning/mili-viz/phase-5-m3.md` Decision 67). The frozen proto
/// carries **no svar catalog** anywhere, so — unlike geometry, whose
/// ticket rides the `GeometryRef` broadcast — the catalog is fetched
/// by a *well-known* ticket the client constructs. No `.proto` change:
/// this reuses the existing Flight bulk-data boundary (`DoGet`) plus
/// the in-process [`VizService::fetch_catalog`] seam, exactly mirroring
/// the geometry transport.
pub const CATALOG_TICKET: &[u8] = b"catalog:current";

/// Magic + version of the self-describing catalog blob. Like the
/// `MVG1`/`MVG2` geometry blob (phase-4-m2.md Decision 11) this is an
/// opaque buffer, **never** an Arrow `RecordBatch`, so it rides
/// verbatim in `FlightData.data_body`. Body: UTF-8 lines after the
/// header, each `TAG\tNAME` (`P` = primal queriable svar). Future
/// kinds (time-indep, derived) add tags without a format break.
const CATALOG_MAGIC: &[u8] = b"MVCAT1\n";

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
    // M8 cut-plane operator (phase-4-m8.md Decision 77): a session-
    // level plane that composes with every subsequent `show` / step /
    // material toggle. Cleared by a `cutpln` with a zero-length
    // normal (the doc's "clear" sentinel).
    cut: Option<clip::Plane>,
    // M9 slice operator (phase-4-m9.md Decision 80): a *second*
    // session-level plane, independent of `cut`. Both compose into
    // one MVG3 blob.
    slice: Option<clip::Plane>,
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
        let (min, max) = scalar.as_ref().map_or((0.0, 0.0), |(_, lo, hi)| (*lo, *hi));
        let materials = &self.materials;
        let (blob, layout, num_indices, num_vertices) = if self.cut.is_some()
            || self.slice.is_some()
        {
            // M8/M9: cut/slice operators (phase-4-m8.md, phase-4-m9.md).
            // Per-vertex scalar at existing nodes feeds linear edge
            // blends along the straddled edges (Decision 79).
            let coords = topo.coords_at(db, self.state);
            let base_n_verts = coords.len() / 3;
            let scalar_in: Option<Vec<f32>> = scalar.as_ref().map(|(s, _, _)| s.clone());
            let scalar_ref: Option<&[f32]> = scalar_in.as_deref();

            // Always start from a Cut pass (kept hull + cut cap) when
            // a cut is active; otherwise an empty hull from a Slice
            // pass.
            let cb = if let Some(plane) = &self.cut {
                clip::clip_topology(topo, &coords, scalar_ref, plane, clip::ClipMode::Cut)
            } else {
                // No cut plane → start from an empty buffer with the
                // base coords/scalar.
                clip::ClipBuffers {
                    verts: coords.clone(),
                    indices: Vec::new(),
                    tri_material: Vec::new(),
                    tri_flags: Vec::new(),
                    edges: Vec::new(),
                    scalar: scalar_in.clone(),
                    tri_member_id: Vec::new(),
                }
            };
            let cb = if let Some(plane) = &self.slice {
                let slice_buf =
                    clip::clip_topology(topo, &coords, scalar_ref, plane, clip::ClipMode::Slice);
                clip::append_clip(cb, slice_buf, base_n_verts)
            } else {
                cb
            };

            let nv = (cb.verts.len() / 3) as u64;
            let n_idx = cb.indices.len() as u64;
            let buf = geometry::MeshTopology::pack_mvg3_buffers(
                &cb.verts,
                &cb.indices,
                &cb.tri_material,
                &cb.tri_flags,
                &cb.edges,
                cb.scalar.as_deref(),
                Some(&cb.tri_member_id),
            );
            (buf, geometry::LAYOUT_VOL, n_idx, nv)
        } else {
            let (blob, layout, num_indices) = match &scalar {
                Some((s, _, _)) => topo.encode(db, self.state, Some(s), materials),
                None => topo.encode(db, self.state, None, materials),
            };
            (blob, layout, num_indices, topo.num_vertices())
        };
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

    /// Build the self-describing result-catalog blob from the loaded
    /// `mili-rs` run (`phase-5-m3.md` Decision 67). The primal section
    /// is `Database::queriable_svars(false, false)` — a *reshape* of
    /// the parsed svar table, never a re-port (the M5 "reuse, don't
    /// re-port" boundary). `None` when no real run is loaded, exactly
    /// like [`Session::geometry_ref`]: the client then keeps its
    /// static placeholder and the headless composite gate is
    /// unperturbed (`bug-tracker.md` VB-001).
    ///
    /// The `D` section is the loaded run's *computable* derived
    /// results — the union over the mesh's element classes of
    /// `Database::derived_variables_of_class` (the oracle-gated
    /// enumeration milestone; `phase-5-m4.md` Decision 71), deduped
    /// first-seen in (class order × registry order). This is the
    /// faithful DB-filtered analog of griz's `analy->derived_results`,
    /// and still a *reshape* — no new file parsing, no derived math
    /// (that stays the `derived.rs` compute path). Time-independent
    /// variables remain unenumerated (no TI accessor — Decision 69;
    /// the client labels that sub-tree accordingly).
    fn catalog_blob(&self) -> Option<Vec<u8>> {
        let db = self.db.as_ref()?;
        let mut blob = CATALOG_MAGIC.to_vec();
        for name in db.queriable_svars(false, false) {
            // svar names are file identifiers (no tab/newline); the
            // tab-delimited line stays unambiguous.
            blob.extend_from_slice(b"P\t");
            blob.extend_from_slice(name.as_bytes());
            blob.push(b'\n');
        }
        if let Some(mesh_id) = self.topo.as_ref().map(MeshTopology::mesh_id) {
            let mut seen: Vec<String> = Vec::new();
            for class in db.class_names(mesh_id) {
                for d in db.derived_variables_of_class(mesh_id, &class) {
                    if !seen.iter().any(|s| s == &d) {
                        blob.extend_from_slice(b"D\t");
                        blob.extend_from_slice(d.as_bytes());
                        blob.push(b'\n');
                        seen.push(d);
                    }
                }
            }
        }
        // Wireframe-parity #6 path (a): emit per-class membership rows
        // so a `Pick::member_id` lifted off the geometry blob's bit-4
        // column resolves locally to (class_name, label) without a
        // Query round-trip. `class_idx` matches the high 8 bits of the
        // `tri_member_id` packing because both walks iterate
        // `MeshTopology::elem_classes` in the same order. Unknown to
        // older clients (skipped by the existing `decode_catalog`
        // tag-tolerance loop).
        if let Some(topo) = self.topo.as_ref() {
            for (ci, summary) in topo.elem_class_summary().iter().enumerate() {
                if summary.elements == 0 {
                    continue;
                }
                let ec = topo.elem_class_at(ci);
                blob.extend_from_slice(b"M\t");
                blob.extend_from_slice(ci.to_string().as_bytes());
                blob.push(b'\t');
                blob.extend_from_slice(ec.name.as_bytes());
                blob.push(b'\t');
                let mut first = true;
                for label in &ec.labels {
                    if !first {
                        blob.push(b',');
                    }
                    blob.extend_from_slice(label.to_string().as_bytes());
                    first = false;
                }
                blob.push(b'\n');
            }
        }
        Some(blob)
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
            // Δ8 carrier; populated by [`VizService::snapshot_full`]
            // in M6 (phase-5-m6.md Decision 97) — left
            // [`pb::AgentTranscript::default`] here so the M1/M2/M3
            // session-state path stays single-purpose.
            agent: Some(pb::AgentTranscript::default()),
        }
    }
}

struct Inner {
    session: Mutex<Session>,
    tx: tokio::sync::broadcast::Sender<pb::StateDelta>,
    agent: bool,
    expected_token: Option<String>,
    /// Phase 5 M6 Decision 94. `None` ⇒ no LLM wired in; `agent_chat`
    /// returns a clear `ok=false` error and `CAP_AGENT` advertises
    /// only if `agent == true` (the M1 builder contract is preserved
    /// — a deployment can advertise the capability without a backend
    /// for testing).
    backend: Option<Arc<dyn AgentBackend>>,
    /// Phase 5 M6 Decision 98. The in-flight turn's id + cancel flag.
    /// `Interrupt` looks the id up here and flips the flag; the
    /// backend observes it via `AgentTurnCtx::cancelled`.
    active_turn: Mutex<Option<ActiveTurn>>,
    /// Phase 5 M6 Decision 97. One entry per landed user turn; the
    /// opening `DELTA_SNAPSHOT` for any new subscriber carries
    /// these as `AgentTranscript.messages` so a late joiner sees the
    /// running conversation. Caps at [`AGENT_TRANSCRIPT_CAP`] to keep
    /// the snapshot small.
    transcript: Mutex<Vec<TurnRecord>>,
    /// Phase 5 M6 Decision 99. Live peer count broadcast detail; read
    /// by `subscribe` after the new receiver lands, included in the
    /// next `Status` event.
    peer_count: std::sync::atomic::AtomicU64,
}

#[derive(Clone)]
struct ActiveTurn {
    turn_id: String,
    cancel: Arc<std::sync::atomic::AtomicBool>,
}

/// One landed user turn — the user's message, the assistant's emitted
/// tokens concatenated into one body string, and the dense one-liner
/// tool-call summaries the client renders inline (`client.md` §"AI
/// Assistant panel"). The pre-turn snapshot for revert (Decision 97)
/// lives **client-side** in the windowed app's `ShellState` so a peer
/// that observed the `UserTurn` delta can revert from its own
/// observed state; the server-side carrier here is only for the
/// late-joiner transcript replay.
#[derive(Clone, Debug)]
struct TurnRecord {
    turn_id: String,
    user_text: String,
    assistant_text: String,
    tool_lines: Vec<String>,
    final_status: pb::AgentStatusKind,
    final_detail: String,
}

/// Bound on the running transcript carried in `Snapshot.agent`
/// (`phase-5-m6.md` Decision 97). Late joiners get this many trailing
/// turns; older turns are dropped so the snapshot payload stays
/// small.
const AGENT_TRANSCRIPT_CAP: usize = 64;

/// The `MiliViz` service. Construct with [`VizService::builder`].
#[derive(Clone)]
pub struct VizService {
    inner: Arc<Inner>,
}

/// Builder for [`VizService`]. `agent(true)` advertises the `agent`
/// capability (a real deployment sets this iff an LLM backend is
/// configured — Decision 6). `agent_backend(MockAgent)` (Phase 5 M6
/// Decision 94) plugs in the loop that actually runs turns and
/// implicitly sets `agent` to `true` if not yet set.
pub struct VizServiceBuilder {
    agent: bool,
    expected_token: Option<String>,
    backend: Option<Arc<dyn AgentBackend>>,
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

    /// Plug an [`AgentBackend`] (Phase 5 M6 Decision 94). Implicitly
    /// flips on the `agent` capability — a deployment with a wired
    /// backend should always advertise the capability so the client
    /// renders the panel; a deployment without one stays panel-less.
    #[must_use]
    pub fn agent_backend<B: AgentBackend>(mut self, backend: B) -> Self {
        self.backend = Some(Arc::new(backend));
        self.agent = true;
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
                backend: self.backend,
                active_turn: Mutex::new(None),
                transcript: Mutex::new(Vec::new()),
                peer_count: std::sync::atomic::AtomicU64::new(0),
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
            backend: None,
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

    /// Resolve the conventional [`CATALOG_TICKET`] to the
    /// self-describing result-catalog blob (`phase-5-m3.md`
    /// Decision 67). The in-process client calls this directly (the
    /// `fetch_geometry` pattern); the Flight `do_get` fronts the same
    /// seam for the deferred remote mode. `None` ⇒ no real run loaded
    /// (the client keeps its static placeholder).
    #[must_use]
    pub fn fetch_catalog(&self) -> Option<Vec<u8>> {
        self.inner.session.lock().unwrap().catalog_blob()
    }

    /// The Arrow Flight adapter over this service's geometry store
    /// (phase-4-m6.md Decision 26). `serve_tcp` co-serves it next to
    /// `MiliVizServer`; a Flight `DoGet` of a frozen ticket streams
    /// the byte-identical blob `fetch_geometry` would return.
    #[must_use]
    pub fn flight_service(&self) -> FlightGeometryService {
        FlightGeometryService::new(Arc::clone(&self.inner))
    }

    /// Phase 5 M6 Decision 95 — `pub(crate)` so [`agent::AgentTurnCtx`]
    /// can allocate a `StateDelta.seq` for its bare `DELTA_AGENT`
    /// events through the same monotonic counter the dispatch path
    /// uses. The lock is brief; callers do not hold it across awaits.
    pub(crate) fn next_seq_value(&self) -> u64 {
        let mut s = self.inner.session.lock().unwrap();
        s.seq += 1;
        s.seq
    }

    /// Build the running `AgentTranscript` for the opening
    /// `DELTA_SNAPSHOT` (Decision 97). Late joiners see the same
    /// conversation as every other peer.
    fn build_transcript(&self) -> pb::AgentTranscript {
        let turns = self.inner.transcript.lock().unwrap();
        let mut messages = Vec::with_capacity(turns.len() * 2);
        for t in turns.iter() {
            messages.push(pb::AgentMessage {
                role: "user".to_string(),
                text: t.user_text.clone(),
                tool_lines: Vec::new(),
                turn_id: t.turn_id.clone(),
            });
            messages.push(pb::AgentMessage {
                role: "assistant".to_string(),
                text: t.assistant_text.clone(),
                tool_lines: t.tool_lines.clone(),
                turn_id: t.turn_id.clone(),
            });
        }
        let last_status = turns.last().map_or(
            pb::AgentStatus {
                kind: pb::AgentStatusKind::AgentIdle as i32,
                detail: format!(
                    "peers={}",
                    self.inner
                        .peer_count
                        .load(std::sync::atomic::Ordering::SeqCst)
                ),
            },
            |t| pb::AgentStatus {
                kind: t.final_status as i32,
                detail: if t.final_detail.is_empty() {
                    format!(
                        "peers={}",
                        self.inner
                            .peer_count
                            .load(std::sync::atomic::Ordering::SeqCst)
                    )
                } else {
                    t.final_detail.clone()
                },
            },
        );
        pb::AgentTranscript {
            messages,
            status: Some(last_status),
        }
    }

    /// Allocate a delta seq and broadcast a `DELTA_AGENT` carrying
    /// `event`. Used by `agent_chat` for the `UserTurn` echo and by
    /// `close_turn` for the closing `Status`.
    fn broadcast_agent_event(&self, event: pb::AgentEvent) {
        let seq = self.next_seq_value();
        let _ = self.inner.tx.send(pb::StateDelta {
            seq,
            origin_client_id: AGENT_MOCK_ORIGIN.to_string(),
            kind: pb::DeltaKind::DeltaAgent as i32,
            payload: Some(pb::state_delta::Payload::Agent(event)),
        });
    }

    /// Phase 5 M6 Decision 99 — broadcast the live peer count as a
    /// `DELTA_AGENT` `Status` event with `detail = "peers=N"`. The
    /// client parses this out of the detail field to drive the
    /// wireframes §"Session states" peer banner / status-bar cell.
    fn broadcast_peer_status(&self, peers: u64) {
        let ev = pb::AgentEvent {
            turn_id: String::new(),
            ev: Some(pb::agent_event::Ev::Status(pb::AgentStatus {
                kind: pb::AgentStatusKind::AgentIdle as i32,
                detail: format!("peers={peers}"),
            })),
        };
        self.broadcast_agent_event(ev);
    }

    /// Finalize a turn record (Decision 97) and broadcast the closing
    /// `Status` (Decision 98). Drops the active-turn slot so the next
    /// `agent_chat` opens fresh.
    fn close_turn(&self, turn_id: &str, kind: pb::AgentStatusKind, detail: String) {
        let peers = self.inner.tx.receiver_count() as u64;
        self.inner
            .peer_count
            .store(peers, std::sync::atomic::Ordering::SeqCst);
        let pieces = if detail.is_empty() {
            format!("peers={peers}")
        } else {
            format!("{detail}; peers={peers}")
        };
        {
            let mut active = self.inner.active_turn.lock().unwrap();
            if active.as_ref().is_some_and(|a| a.turn_id == turn_id) {
                *active = None;
            }
        }
        {
            let mut turns = self.inner.transcript.lock().unwrap();
            if let Some(t) = turns.iter_mut().find(|t| t.turn_id == turn_id) {
                t.final_status = kind;
                t.final_detail = pieces.clone();
            }
            if turns.len() > AGENT_TRANSCRIPT_CAP {
                let excess = turns.len() - AGENT_TRANSCRIPT_CAP;
                turns.drain(0..excess);
            }
        }
        let ev = pb::AgentEvent {
            turn_id: turn_id.to_string(),
            ev: Some(pb::agent_event::Ev::Status(pb::AgentStatus {
                kind: kind as i32,
                detail: pieces,
            })),
        };
        self.broadcast_agent_event(ev);
    }

    /// Append the assistant token / tool-line into the turn record so
    /// late joiners see the running transcript. The broadcast itself
    /// is the backend's job (via `AgentTurnCtx`); this is a parallel
    /// fold for the journal carried in `Snapshot.agent`.
    pub(crate) fn record_turn_event(&self, turn_id: &str, ev: &pb::agent_event::Ev) {
        use pb::agent_event::Ev;
        let mut turns = self.inner.transcript.lock().unwrap();
        let Some(t) = turns.iter_mut().find(|t| t.turn_id == turn_id) else {
            return;
        };
        match ev {
            Ev::Token(tok) => t.assistant_text.push_str(&tok.text),
            Ev::ToolBegin(b) => t.tool_lines.push(b.summary.clone()),
            // ToolEnd / Status / UserTurn don't extend the transcript
            // text — UserTurn was already folded at turn-open time,
            // and status / tool-end are presentational.
            _ => {}
        }
    }

    /// Dispatch one parsed command: mutate state, assign a seq, build
    /// the `StateDelta`, broadcast it to every subscriber, return the
    /// seq. This is the single internal entry point shared by typed
    /// `Execute`, the `raw` escape hatch, and (at M6) the agent loop,
    /// which is what makes Layer-0 ≡ raw hold by construction.
    fn dispatch(&self, cmd: pb::command::Cmd, origin: &str) -> agent::DispatchOutcome {
        let mut s = self.inner.session.lock().unwrap();
        s.seq += 1;
        let seq = s.seq;
        let (kind, payload) = apply(&mut s, cmd);
        let delta = pb::StateDelta {
            seq,
            origin_client_id: origin.to_string(),
            kind: kind as i32,
            payload: Some(payload.clone()),
        };
        // `send` errs only when there are no receivers; that is fine.
        let _ = self.inner.tx.send(delta);
        agent::DispatchOutcome { seq, payload }
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
        Cmd::Cutplane(cp) => {
            // M8/M9 cut-plane + slice operators (phase-4-m8.md,
            // phase-4-m9.md). `slice_only=true` lands the plane in
            // `Session.slice`; `slice_only=false` lands it in
            // `Session.cut`. The two compose (Decision 80). A
            // zero-length normal clears whichever bucket the call
            // addresses.
            let slice_only = cp.slice_only.unwrap_or(false);
            let new_plane = clip::Plane::from_proto(&cp);
            if slice_only {
                s.slice = new_plane;
            } else {
                s.cut = new_plane;
            }
            // Preserve the existing result (result/component/min/max
            // stay byte-stable across cut on/off — phase-4-m8.md
            // Decision 75); only the geometry blob changes.
            let prior = s.result.clone().unwrap_or_default();
            let svar = if prior.component.is_empty() {
                prior.result.clone()
            } else {
                prior.component.clone()
            };
            let (geometry, min, max) = match s.geometry_ref(&svar) {
                Some((g, lo, hi)) => (Some(g), lo, hi),
                None => (None, prior.min, prior.max),
            };
            let r = pb::ResultState {
                result: prior.result,
                component: prior.component,
                min,
                max,
                geometry,
            };
            s.result = Some(r.clone());
            (D::DeltaResult, P::Result(r))
        }
        Cmd::Colormap(_) | Cmd::Legend(_) | Cmd::Render(_) => {
            // Recolor / rescale / offscreen-render of the *current*
            // result. The visual effect is M3+/M6; M1 re-broadcasts
            // the (geometry-stubbed) result state.
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
            last = self.dispatch(c, &origin).seq;
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
        // ordering guarantee). The opening snapshot now carries the
        // populated `AgentTranscript` for the late-joiner replay
        // (phase-5-m6.md Decision 97).
        let (snapshot_delta, rx) = {
            let s = self.inner.session.lock().unwrap();
            let mut snap = s.snapshot();
            snap.agent = Some(self.build_transcript());
            let delta = pb::StateDelta {
                seq: s.seq,
                origin_client_id: String::new(),
                kind: pb::DeltaKind::DeltaSnapshot as i32,
                payload: Some(pb::state_delta::Payload::Snapshot(snap)),
            };
            (delta, self.inner.tx.subscribe())
        };

        // Phase 5 M6 Decision 99: broadcast the new peer count to
        // every prior subscriber so the banner / status-bar peer cell
        // updates. The fresh subscriber already gets the count from
        // the opening snapshot's transcript status. Gated on the
        // agent capability so the M1 acceptance gate (which uses a
        // vanilla `.agent(false)` server, the byte-stable default)
        // sees no extra `DELTA_AGENT` traffic.
        let peers = self.inner.tx.receiver_count() as u64;
        self.inner
            .peer_count
            .store(peers, std::sync::atomic::Ordering::SeqCst);
        if self.inner.agent {
            self.broadcast_peer_status(peers);
        }

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
        // Real `mili-rs`-backed read (`wireframe-parity.md` "What's
        // still left" #4 — the time-history-Query forward path):
        // dispatch to `Database::query_full` for primal svars, return
        // the values inline. Derived results (stress invariants,
        // principal stress/strain) need the same routing the geometry
        // path uses — that's a follow-up; reject them here with a
        // clear error so callers know to query the primals instead.
        // Returns `ok=false` with a typed error on any read failure
        // rather than `Status::*` so the wire surface stays the same
        // shape the M1 stub already had (the client treats `ok` as
        // the success flag).
        let q = request.into_inner();
        let err = |msg: String| {
            Ok(Response::new(pb::QueryReply {
                ok: false,
                error: msg,
                data: None,
            }))
        };

        if q.result.is_empty() {
            return err("query: empty `result`".to_string());
        }
        if q.class_name.is_empty() {
            return err("query: empty `class_name`".to_string());
        }
        // Derived results route through stress_invariant_spec /
        // principal_stress_spec / principal_strain_spec in
        // `geometry.rs::vertex_scalar`. A future cut can replicate
        // that here; for now an honest "not yet supported" beats a
        // silent zero.
        if mili_rs::stress_invariant_spec(&q.result).is_some()
            || mili_rs::principal_stress_spec(&q.result).is_some()
            || mili_rs::principal_strain_spec(&q.result).is_some()
        {
            return err(format!(
                "query: derived result `{}` not yet supported over the \
                 Query RPC (query the primals instead — \
                 wireframe-parity.md #4 follow-up)",
                q.result
            ));
        }

        let svar = if q.component.is_empty() {
            q.result.clone()
        } else {
            // mili-rs accepts the `vec[component]` / `array[idx]` shape
            // in its parser — same convention `mili.query()` uses.
            format!("{}[{}]", q.result, q.component)
        };

        // Hold the session lock across the gather. `Database::query_full`
        // is sync (no `.await` while the lock is held — std Mutex
        // tolerates this), and `vertex_scalar` already calls
        // `db.query_full` from inside the same lock during `show`, so
        // this matches the existing single-mutex discipline.
        let labels_i32: Vec<i32> = q.labels.iter().map(|l| *l as i32).collect();
        let labels_opt: Option<&[i32]> = if labels_i32.is_empty() {
            None
        } else {
            Some(&labels_i32)
        };
        let s = self.inner.session.lock().unwrap();
        let Some(db) = s.db.as_ref() else {
            return err("query: no run loaded".to_string());
        };
        let n = db.state_count();
        if n == 0 {
            return err("query: loaded database has no states".to_string());
        }
        // Empty `states` ⇒ current cursor (1-based) per the proto.
        let states_1: Vec<u32> = if q.states.is_empty() {
            vec![s.state.max(1)]
        } else {
            q.states.clone()
        };
        let mut states_0 = Vec::with_capacity(states_1.len());
        for st in &states_1 {
            if *st == 0 || (*st as usize) > n {
                return err(format!(
                    "query: state {st} out of range (1..={n})"
                ));
            }
            states_0.push(*st as usize - 1);
        }

        let qr = match db.query_full(&QueryArgs {
            svar: &svar,
            class: &q.class_name,
            labels: labels_opt,
            states: &states_0,
            materials: None,
            ips: None,
            subrec: None,
        }) {
            Ok(qr) => qr,
            Err(e) => {
                return err(format!("query: `{svar}` on class `{}`: {e}", q.class_name))
            }
        };
        drop(s);

        let values_f64: Vec<f64> = match qr.values {
            StateValues::F32(v) => v.into_iter().map(f64::from).collect(),
            StateValues::F64(v) => v,
            StateValues::I32(v) => v.into_iter().map(f64::from).collect(),
            StateValues::I64(v) => v.into_iter().map(|x| x as f64).collect(),
        };

        // Echo the resolved state list back so the client doesn't have
        // to remember which states the server filled in for an empty
        // request.
        let states_out: Vec<u32> = states_0.iter().map(|s| (*s as u32) + 1).collect();
        let labels_out: Vec<i64> = qr.labels.iter().map(|l| i64::from(*l)).collect();

        Ok(Response::new(pb::QueryReply {
            ok: true,
            error: String::new(),
            data: Some(pb::query_reply::Data::Inline(pb::InlineTable {
                labels: labels_out,
                states: states_out,
                values: values_f64,
                components: qr.components.len() as u32,
            })),
        }))
    }

    async fn agent_chat(
        &self,
        request: Request<pb::AgentChatRequest>,
    ) -> Result<Response<pb::AgentChatReply>, Status> {
        // Phase 5 M6 (phase-5-m6.md Decisions 94–98) — the M1 frozen
        // stub is gone. A server with no backend configured returns a
        // clear ok=false error (not Status::unimplemented; the wire
        // surface is implemented, the deployment just hasn't wired a
        // backend). With a backend, dispatch one turn.
        let Some(backend) = self.inner.backend.clone() else {
            return Ok(Response::new(pb::AgentChatReply {
                ok: false,
                error: "no agent backend configured (build the server \
                        with VizService::builder().agent_backend(...))"
                    .to_string(),
                turn_id: String::new(),
            }));
        };
        let req = request.into_inner();
        let turn_id = format!("turn-{}", self.next_seq_value());
        let cancel = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let origin = AGENT_MOCK_ORIGIN.to_string();

        // Register the active turn (Decision 98). The pre-turn
        // snapshot for revert (Decision 97) is the client's job —
        // every peer that observed this `UserTurn` delta has the
        // matching session state to snapshot for itself.
        *self.inner.active_turn.lock().unwrap() = Some(ActiveTurn {
            turn_id: turn_id.clone(),
            cancel: cancel.clone(),
        });

        // Broadcast the user-turn echo so all peers see the message
        // (`client.md` §"Design principle" — shared transcript).
        let user_text = req.text.clone();
        let had_frame = req.attach_frame;
        let ev = pb::AgentEvent {
            turn_id: turn_id.clone(),
            ev: Some(pb::agent_event::Ev::UserTurn(pb::AgentUserTurn {
                text: user_text.clone(),
                had_frame,
            })),
        };
        self.broadcast_agent_event(ev);

        // Open the assistant transcript row for this turn. Tokens fold
        // into `assistant_text`; tool-call summaries fold into
        // `tool_lines`. Completed at turn end (Decision 97).
        self.inner.transcript.lock().unwrap().push(TurnRecord {
            turn_id: turn_id.clone(),
            user_text,
            assistant_text: String::new(),
            tool_lines: Vec::new(),
            final_status: pb::AgentStatusKind::AgentRunning,
            final_detail: String::new(),
        });

        // Run the turn on the current tokio runtime. The dispatcher
        // closure proxies VizService::dispatch — `client.md` §"Design
        // principle": every agent action flows through the same seam
        // typed Execute uses, broadcasting as an ordinary StateDelta
        // tagged with the agent's origin_client_id.
        let svc = self.clone();
        let dispatcher: agent::Dispatcher = {
            let svc = svc.clone();
            Arc::new(move |cmd, origin| svc.dispatch(cmd, origin))
        };
        let next_seq: agent::SeqAllocator = {
            let svc = svc.clone();
            Arc::new(move || svc.next_seq_value())
        };
        let recorder: agent::EventRecorder = {
            let svc = svc.clone();
            Arc::new(move |id: &str, ev: &pb::agent_event::Ev| svc.record_turn_event(id, ev))
        };
        let ctx = AgentTurnCtx {
            turn_id: turn_id.clone(),
            request: req,
            origin_client_id: origin,
            tx: self.inner.tx.clone(),
            next_seq,
            dispatcher,
            recorder,
            cancel: cancel.clone(),
        };
        let turn_id_for_task = turn_id.clone();
        let svc_for_task = svc.clone();
        tokio::spawn(async move {
            backend.run_turn(ctx).await;
            // Closing status: interrupted iff the cancel flag was
            // flipped during the turn, idle otherwise. The peer-count
            // detail piggyback (Decision 99) rides on every status so
            // the banner stays live.
            let (kind, detail) = if cancel.load(std::sync::atomic::Ordering::SeqCst) {
                (pb::AgentStatusKind::AgentInterrupted, String::new())
            } else {
                (pb::AgentStatusKind::AgentIdle, String::new())
            };
            svc_for_task.close_turn(&turn_id_for_task, kind, detail);
        });

        Ok(Response::new(pb::AgentChatReply {
            ok: true,
            error: String::new(),
            turn_id,
        }))
    }

    async fn interrupt(
        &self,
        request: Request<pb::InterruptRequest>,
    ) -> Result<Response<pb::InterruptReply>, Status> {
        // Phase 5 M6 Decision 98. Empty `turn_id` is "cancel whatever
        // is in flight" (the frozen-proto convention — see
        // proto/mili_viz.proto:437). An out-of-date turn_id (no
        // active turn or stale id) is a no-op success — the user's
        // intent was "stop", and there is nothing to stop.
        let req = request.into_inner();
        let active = self.inner.active_turn.lock().unwrap().clone();
        match active {
            Some(at) if req.turn_id.is_empty() || at.turn_id == req.turn_id => {
                at.cancel.store(true, std::sync::atomic::Ordering::SeqCst);
                Ok(Response::new(pb::InterruptReply {
                    ok: true,
                    error: String::new(),
                }))
            }
            _ => Ok(Response::new(pb::InterruptReply {
                ok: true,
                error: String::new(),
            })),
        }
    }

    async fn capture_frame(
        &self,
        request: Request<pb::FrameRequest>,
    ) -> Result<Response<pb::FrameReply>, Status> {
        // Phase 5 M6 Decision 96 — the RPC is no longer
        // Status::unimplemented; a deterministic placeholder PNG/JPEG
        // satisfies the contract surface. Production server-side
        // wgpu offscreen rendering is a separate milestone.
        let req = request.into_inner();
        match encode_placeholder_frame(req.width, req.height, &req.format) {
            Ok((bytes, fmt)) => Ok(Response::new(pb::FrameReply {
                ok: true,
                error: String::new(),
                image: bytes,
                format: fmt,
                width: req.width,
                height: req.height,
            })),
            Err(e) => Ok(Response::new(pb::FrameReply {
                ok: false,
                error: e,
                image: vec![],
                format: req.format,
                width: req.width,
                height: req.height,
            })),
        }
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

/// Serve `MiliViz` **and** the Arrow Flight `FlightService` over a
/// real TCP socket (Phase 4 M6 — phase-4-m6.md Decisions 26 & 27).
/// Both services share one `Arc<Inner>` (one session, one geometry
/// store, one broadcast bus) and are multiplexed on the one HTTP/2
/// port by tonic's router. The listener is bound *before* serving so
/// an ephemeral `addr` port (`127.0.0.1:0`) resolves to a concrete
/// `SocketAddr` returned to the caller (no TOCTOU).
///
/// # Errors
/// Returns an error if the TCP listener cannot bind `addr`.
pub async fn serve_tcp(
    svc: VizService,
    addr: std::net::SocketAddr,
) -> Result<
    (std::net::SocketAddr, tokio::task::JoinHandle<()>),
    Box<dyn std::error::Error + Send + Sync>,
> {
    use mili_viz_proto::flight::flight_service_server::FlightServiceServer;
    use tokio_stream::wrappers::TcpListenerStream;
    use tonic::transport::Server;

    let listener = tokio::net::TcpListener::bind(addr).await?;
    let local = listener.local_addr()?;
    let flight = svc.flight_service();

    let handle = tokio::spawn(async move {
        Server::builder()
            .add_service(MiliVizServer::new(svc))
            .add_service(FlightServiceServer::new(flight))
            .serve_with_incoming(TcpListenerStream::new(listener))
            .await
            .expect("tcp server terminated with error");
    });

    Ok((local, handle))
}

/// Bind a server on an ephemeral `127.0.0.1` TCP port and return
/// connected real `MiliViz` **and** Flight clients over that TCP
/// transport, plus the bound address and the server task handle. The
/// remote-transport analogue of [`spawn_in_process`]; the M6
/// acceptance-gate transport.
///
/// # Errors
/// Returns a transport error if the listener cannot bind or a client
/// channel fails to connect.
pub async fn spawn_tcp(
    svc: VizService,
) -> Result<
    (
        std::net::SocketAddr,
        pb::mili_viz_client::MiliVizClient<tonic::transport::Channel>,
        mili_viz_proto::flight::flight_service_client::FlightServiceClient<
            tonic::transport::Channel,
        >,
        tokio::task::JoinHandle<()>,
    ),
    Box<dyn std::error::Error + Send + Sync>,
> {
    let addr = std::net::SocketAddr::from(([127, 0, 0, 1], 0));
    let (local, handle) = serve_tcp(svc, addr).await?;

    let url = format!("http://{local}");
    let viz = pb::mili_viz_client::MiliVizClient::connect(url.clone()).await?;
    let flight =
        mili_viz_proto::flight::flight_service_client::FlightServiceClient::connect(url).await?;

    Ok((local, viz, flight, handle))
}
