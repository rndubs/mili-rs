//! Server-hosted agent loop (Phase 5 M6 — `phase-5-m6.md` Decisions
//! 94–99). The agent surface in the frozen proto (`AgentChat`,
//! `Interrupt`, `CaptureFrame`, `DELTA_AGENT`, `AgentEvent`,
//! `AgentTranscript`, `Snapshot.agent`) has existed since Phase 4 M1
//! Decision 1 Δ4–Δ9; this module is the implementation that turns it
//! on without a `.proto` change.
//!
//! The design principle ([`client.md`](../planning/mili-viz/client.md)
//! §"Design principle") is that **the agent is a peer of the command
//! vocabulary**: any command it issues flows through the same
//! [`crate::VizService::dispatch`] seam that typed `Execute` /
//! `Command{raw}` already use, so it broadcasts as an ordinary
//! `StateDelta` tagged with the agent's `origin_client_id`. M6 lights
//! up the loop **wiring**; the always-on [`MockAgent`] is the
//! gating-test backend. A real LLM-backed implementation is a separate
//! follow-up (Decision 94 — gated behind a future Cargo feature with
//! its own dep tree and config contract).

use std::pin::Pin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use mili_viz_proto::v1 as pb;

/// Stable `origin_client_id` for any `StateDelta` the mock agent
/// dispatches. The provenance journal correlates on this — every
/// real LLM backend should pick its own stable id (e.g. `agent:claude`)
/// so the journal can attribute commands to the right peer.
pub const AGENT_MOCK_ORIGIN: &str = "agent:mock";

/// A pluggable backend that drives one agent turn. Object-safe by
/// construction (manual `Pin<Box<dyn Future>>` return — no
/// `async-trait` macro dep). The always-on [`MockAgent`] implements it
/// deterministically for the gating test; a real LLM backend (e.g.
/// `ClaudeAgent` behind a future Cargo feature) is a separate
/// follow-up.
pub trait AgentBackend: Send + Sync + 'static {
    /// Run one user turn against `ctx`. Called from a freshly-spawned
    /// tokio task on the server's runtime — the impl may `.await`
    /// freely. Must observe `ctx.cancelled()` between emits so
    /// `Interrupt` (M6e) can stop the turn promptly. The closing
    /// `Status(idle)` / `Status(interrupted)` is emitted by the
    /// caller, **not** the backend — the backend just returns when
    /// done.
    fn run_turn<'a>(
        &'a self,
        ctx: AgentTurnCtx,
    ) -> Pin<Box<dyn std::future::Future<Output = ()> + Send + 'a>>;
}

/// Allocates the next `StateDelta.seq`.
pub(crate) type SeqAllocator = Arc<dyn Fn() -> u64 + Send + Sync>;
/// Dispatches a typed command through `VizService::dispatch`.
pub(crate) type Dispatcher = Arc<dyn Fn(pb::command::Cmd, &str) -> u64 + Send + Sync>;
/// Folds an emitted agent event into the running per-turn transcript.
pub(crate) type EventRecorder = Arc<dyn Fn(&str, &pb::agent_event::Ev) + Send + Sync>;

/// The handle the backend uses to talk to the server. Owns the
/// broadcast bus, the dispatch closure, the cancel flag, and the
/// user's request. Constructed by `MiliViz::agent_chat`; consumed by
/// [`AgentBackend::run_turn`].
pub struct AgentTurnCtx {
    pub turn_id: String,
    pub request: pb::AgentChatRequest,
    /// Stable id the backend tags its dispatched commands with —
    /// reused as the `origin_client_id` on the broadcast `StateDelta`
    /// so the provenance journal correlates (`client.md` §"Design
    /// principle"). [`MockAgent`] uses [`AGENT_MOCK_ORIGIN`].
    pub origin_client_id: String,
    /// Broadcast bus for `DELTA_AGENT` events.
    pub(crate) tx: tokio::sync::broadcast::Sender<pb::StateDelta>,
    /// Allocates the next `StateDelta.seq` (single source of truth
    /// shared with `VizService::dispatch`).
    pub(crate) next_seq: SeqAllocator,
    /// Dispatch a typed command through the same seam typed `Execute`
    /// uses — returns the broadcast `StateDelta.seq` so the matching
    /// `AgentToolEnd` can carry it (`client.md` Δ5 / Decision 95).
    pub(crate) dispatcher: Dispatcher,
    /// Fold this event into the running per-turn transcript
    /// (`phase-5-m6.md` Decision 97). Late joiners see the assistant
    /// text + tool-line summaries via the opening `DELTA_SNAPSHOT`.
    pub(crate) recorder: EventRecorder,
    /// `Interrupt` (M6e) flips this; the backend must observe it.
    pub(crate) cancel: Arc<AtomicBool>,
}

impl AgentTurnCtx {
    /// Has [`crate::MiliViz::interrupt`] been called for this turn?
    /// Backends should check this between emits so a barge-in lands
    /// promptly.
    #[must_use]
    pub fn cancelled(&self) -> bool {
        self.cancel.load(Ordering::SeqCst)
    }

    /// Dispatch a command through the server's existing
    /// `VizService::dispatch` seam (`client.md` §"Design principle"
    /// — every agent action is an ordinary `Command` producing an
    /// ordinary `StateDelta` tagged with the agent's
    /// `origin_client_id`). Returns the broadcast `StateDelta.seq`
    /// so the matching `AgentToolEnd` can carry it.
    pub fn dispatch(&self, cmd: pb::command::Cmd) -> u64 {
        (self.dispatcher)(cmd, &self.origin_client_id)
    }

    /// Broadcast an `AgentStatus` for this turn. Empty `detail` is
    /// fine; status broadcasts also carry the live peer count via
    /// [`broadcast_agent_event_with_peers`] when emitted from
    /// `MiliViz::agent_chat`.
    pub fn emit_status(&self, kind: pb::AgentStatusKind, detail: impl Into<String>) {
        self.emit(pb::agent_event::Ev::Status(pb::AgentStatus {
            kind: kind as i32,
            detail: detail.into(),
        }));
    }

    /// Stream one assistant token / token-delta.
    pub fn emit_token(&self, text: impl Into<String>) {
        self.emit(pb::agent_event::Ev::Token(pb::AgentToken {
            text: text.into(),
        }));
    }

    /// Emit the opening one-liner of a tool call (`client.md` §"AI
    /// Assistant panel" — `▸ ran      state 47; show sx`). `summary`
    /// is the dense one-line text; `detail` is the optional
    /// click-to-expand `Command`/`Query` payload (a `to_raw` render is
    /// the conventional choice).
    pub fn emit_tool_begin(
        &self,
        call_id: impl Into<String>,
        summary: impl Into<String>,
        detail: impl Into<String>,
    ) {
        self.emit(pb::agent_event::Ev::ToolBegin(pb::AgentToolBegin {
            call_id: call_id.into(),
            summary: summary.into(),
            detail: detail.into(),
        }));
    }

    /// Emit the closing one-liner of a tool call. `delta_seq` is the
    /// broadcast `StateDelta.seq` of the command this tool dispatched
    /// (0 if the tool was a pure read like `Query` — the conventional
    /// `client.md` semantics).
    pub fn emit_tool_end(
        &self,
        call_id: impl Into<String>,
        ok: bool,
        result_summary: impl Into<String>,
        delta_seq: u64,
    ) {
        self.emit(pb::agent_event::Ev::ToolEnd(pb::AgentToolEnd {
            call_id: call_id.into(),
            ok,
            result_summary: result_summary.into(),
            delta_seq,
        }));
    }

    fn emit(&self, ev: pb::agent_event::Ev) {
        (self.recorder)(&self.turn_id, &ev);
        let seq = (self.next_seq)();
        let _ = self.tx.send(pb::StateDelta {
            seq,
            origin_client_id: self.origin_client_id.clone(),
            kind: pb::DeltaKind::DeltaAgent as i32,
            payload: Some(pb::state_delta::Payload::Agent(pb::AgentEvent {
                turn_id: self.turn_id.clone(),
                ev: Some(ev),
            })),
        });
    }
}

/// Deterministic always-on backend. The gating-test driver
/// (`phase-5-m6.md` §"M6 acceptance gate" tests 2/3); also the
/// default-when-`.agent(true)`-but-no-real-backend so the panel
/// renders end-to-end in a vanilla `cargo run`.
///
/// The turn shape is:
/// 1. `Status(thinking)` (with the peer-count detail piggyback —
///    Decision 99 — handled by `MiliViz::agent_chat`'s framing).
/// 2. A short stream of `Token`s ("acknowledged …").
/// 3. One `ToolBegin`/`ToolEnd` pair around a benign `Cmd::SetState`
///    so the [`AgentToolEnd.delta_seq`] gets a real broadcast seq to
///    correlate (the M6 gating test 2 pins this).
/// 4. Return — the caller emits the closing `Status(idle)`.
///
/// The point is **the wiring**, not the chosen command; a real LLM
/// backend's turn shape is its own concern.
#[derive(Default)]
pub struct MockAgent;

impl AgentBackend for MockAgent {
    fn run_turn<'a>(
        &'a self,
        ctx: AgentTurnCtx,
    ) -> Pin<Box<dyn std::future::Future<Output = ()> + Send + 'a>> {
        Box::pin(async move {
            ctx.emit_status(pb::AgentStatusKind::AgentThinking, "");
            // Echo a short canned response so the transcript has
            // something to render past the user message. Tokens are
            // independent broadcast events so the client transcript
            // can stream incrementally (the wireframes' streaming-text
            // affordance).
            for tok in ["acknowledged: ", &ctx.request.text, ""] {
                if ctx.cancelled() {
                    return;
                }
                ctx.emit_token(tok);
                tokio::time::sleep(Duration::from_millis(1)).await;
            }
            if ctx.cancelled() {
                return;
            }
            ctx.emit_status(pb::AgentStatusKind::AgentRunning, "");
            // One tool call. The dispatched `SetState` flows through
            // `VizService::dispatch` and broadcasts as an ordinary
            // `DELTA_STATE` tagged with the agent's `origin_client_id`
            // (`client.md` §"Design principle" — gating test 2 pins
            // the tag + the matching delta_seq round-trip).
            let call_id = format!("{}-call-1", ctx.turn_id);
            ctx.emit_tool_begin(&call_id, "ran: state 1", "state 1");
            let seq = ctx.dispatch(pb::command::Cmd::SetState(pb::SetState { state: 1 }));
            ctx.emit_tool_end(&call_id, true, "state=1", seq);
        })
    }
}

/// Encode a `(width, height, format)` request to a deterministic
/// placeholder image (Decision 96). A midtone-grey fill — the
/// CaptureFrame contract surface lights up without a server-side
/// wgpu adapter; the production swap is a separate milestone.
/// `format` is case-insensitive ("png" | "jpeg"/"jpg"); unknown
/// formats fall back to PNG.
///
/// # Errors
/// Returns an error string on zero-sized requests or encoder failure.
pub fn encode_placeholder_frame(
    width: u32,
    height: u32,
    format: &str,
) -> Result<(Vec<u8>, String), String> {
    if width == 0 || height == 0 {
        return Err(format!(
            "CaptureFrame request has zero extent: {width}x{height}"
        ));
    }
    let mut img = image::RgbImage::new(width, height);
    for pixel in img.pixels_mut() {
        *pixel = image::Rgb([0x40, 0x44, 0x48]); // midtone, matches the
                                                 // wireframe panel-2 colour
    }
    let fmt = format.trim().to_ascii_lowercase();
    let (out_fmt, image_fmt) = match fmt.as_str() {
        "jpeg" | "jpg" => ("jpeg", image::ImageFormat::Jpeg),
        _ => ("png", image::ImageFormat::Png),
    };
    let mut buf: Vec<u8> = Vec::new();
    img.write_to(&mut std::io::Cursor::new(&mut buf), image_fmt)
        .map_err(|e| format!("encode {out_fmt} failed: {e}"))?;
    Ok((buf, out_fmt.to_string()))
}

/// Render the "ran: …" one-liner the dense-tool-call row prefers
/// (`client.md` §"AI Assistant panel"). Tools beyond `ran` follow the
/// same pattern (`queried`, `captured`, etc.); this helper keeps the
/// format string in one place.
#[must_use]
pub fn ran_summary(raw: &str) -> String {
    format!("ran: {raw}")
}

/// A user-turn snapshot the provenance journal restores from
/// (`phase-5-m6.md` Decision 97). A snapshot is keyed by
/// `AgentMessage.turn_id`; the client transcript renders an inline
/// `↶ revert to here` row against it. The actual session-state
/// `Snapshot` (the existing M1 `pb::Snapshot`) is what gets stashed —
/// revert lowers it to typed `SetState` / `Show` / `SetCamera`
/// commands client-side.
#[derive(Debug, Clone)]
pub struct TurnSnapshot {
    pub turn_id: String,
    pub snapshot: pb::Snapshot,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn placeholder_png_round_trips_to_requested_extent() {
        let (bytes, fmt) = encode_placeholder_frame(16, 12, "png").unwrap();
        assert_eq!(fmt, "png");
        let img = image::load_from_memory(&bytes).expect("decode placeholder PNG");
        assert_eq!(img.width(), 16);
        assert_eq!(img.height(), 12);
    }

    #[test]
    fn placeholder_jpeg_is_non_empty_and_decodes() {
        let (bytes, fmt) = encode_placeholder_frame(32, 24, "JPEG").unwrap();
        assert_eq!(fmt, "jpeg");
        assert!(!bytes.is_empty());
        let img = image::load_from_memory(&bytes).expect("decode placeholder JPEG");
        // JPEG is lossy but the dimensions are exact.
        assert_eq!(img.width(), 32);
        assert_eq!(img.height(), 24);
    }

    #[test]
    fn placeholder_unknown_format_falls_back_to_png() {
        let (_bytes, fmt) = encode_placeholder_frame(4, 4, "bmp").unwrap();
        assert_eq!(fmt, "png");
    }

    #[test]
    fn placeholder_rejects_zero_extent() {
        assert!(encode_placeholder_frame(0, 16, "png").is_err());
        assert!(encode_placeholder_frame(16, 0, "png").is_err());
    }

    #[test]
    fn ran_summary_format_matches_wireframe() {
        // wireframes/README.md: `▸ ran      state 47; show sx`
        // The `▸ ran      ` prefix is the client renderer's; this
        // helper supplies the right half ("ran: <raw>") so the prefix
        // can render uniformly across "ran"/"queried"/"captured".
        assert_eq!(ran_summary("state 47"), "ran: state 47");
    }
}
