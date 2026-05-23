//! AI Assistant panel state + agent-event folding (Phase 5 M6 —
//! `phase-5-m6.md` Decisions 94–99 / `client.md` §"AI Assistant
//! panel"). Pure, GPU-free, transport-free — the gating-test core
//! (the M3 / M3.5 / M5 always-on pattern). The windowed app folds the
//! broadcast `DELTA_AGENT` stream into [`AiPanelState`] and renders
//! the panel as part of [`crate::build_shell_ui`]; tool-call /
//! agent-chat actions lower to the frozen `AgentChat`/`Interrupt`/
//! `CaptureFrame` RPCs in `app.rs`.
//!
//! Decision 97 stores the pre-turn snapshot **client-side** so revert
//! can lower to typed `SetState` / `Show` / `View(SetCamera)`
//! commands without a server round-trip (every primitive is in the
//! frozen `Command` set).

use mili_viz_proto::v1 as pb;

/// Server-side pill colour mirroring `client.md` §"Session states".
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AgentStatus {
    #[default]
    Idle,
    Thinking,
    Running,
    Interrupted,
    Error,
}

impl AgentStatus {
    /// Lowercase label, matches the wireframes' status pill text.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            AgentStatus::Idle => "idle",
            AgentStatus::Thinking => "thinking",
            AgentStatus::Running => "running",
            AgentStatus::Interrupted => "interrupted",
            AgentStatus::Error => "error",
        }
    }

    /// Whether a turn is in flight (composer Send → Stop swap;
    /// wireframes §"Session states" *Agent thinking* /
    /// *Agent running tool*).
    #[must_use]
    pub fn in_flight(self) -> bool {
        matches!(self, AgentStatus::Thinking | AgentStatus::Running)
    }

    fn from_proto(kind: pb::AgentStatusKind) -> Self {
        match kind {
            pb::AgentStatusKind::AgentIdle => AgentStatus::Idle,
            pb::AgentStatusKind::AgentThinking => AgentStatus::Thinking,
            pb::AgentStatusKind::AgentRunning => AgentStatus::Running,
            pb::AgentStatusKind::AgentInterrupted => AgentStatus::Interrupted,
            pb::AgentStatusKind::AgentError => AgentStatus::Error,
        }
    }
}

/// One renderable transcript row. Mirrors `client.md` §"AI Assistant
/// panel" — messages alternate `you` / `claude` with dense one-liner
/// tool-call rows interleaved, plus an inline turn-boundary marker for
/// provenance.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TranscriptRow {
    /// `you` message (the user's text).
    User { turn_id: String, text: String },
    /// `claude` message (the assistant's text, concatenated from
    /// streamed `Token` deltas).
    Assistant { turn_id: String, text: String },
    /// One dense `▸ <verb> <summary>     → <result>` row. `result` is
    /// empty until the matching `AgentToolEnd` lands.
    Tool {
        turn_id: String,
        call_id: String,
        summary: String,
        result: String,
        ok: bool,
        delta_seq: u64,
        complete: bool,
    },
    /// Inline turn-boundary marker (`phase-5-m6.md` Decision 97
    /// primary surface): paints `state=N · result=name` plus the
    /// `↶ revert to here` link.
    TurnBoundary { turn_id: String, summary: String },
    /// `✕ interrupted by user — turn cancelled` row (wireframes §
    /// *User interrupted*).
    Interrupted { turn_id: String },
}

/// Snapshot the user-turn provenance journal restores from
/// (`phase-5-m6.md` Decision 97). Captured client-side from the live
/// `ShellState` at user-turn boundaries; revert lowers it to a typed
/// command sequence (`SetState` / `Show` / `View(SetCamera)`).
#[derive(Debug, Clone)]
pub struct TurnSnapshot {
    pub turn_id: String,
    pub state: u32,
    pub result_name: String,
    pub result_component: String,
    /// Camera azimuth / elevation / distance / focus — copied from
    /// the broadcast `CameraState`. `None` when no camera has landed
    /// yet (revert skips the `SetCamera` arm in that case).
    pub camera: Option<pb::CameraState>,
}

impl TurnSnapshot {
    /// Lower this snapshot to the typed `Command` sequence the
    /// `↶ revert to here` link emits (Decision 97). Order: `SetState`
    /// first (so the geometry re-encodes), then `Show` (to re-paint
    /// the result), then `SetCamera` (so the view also reverts). Each
    /// primitive is in the frozen `Command` set — no `raw`.
    #[must_use]
    pub fn lower(&self) -> Vec<pb::command::Cmd> {
        let mut out: Vec<pb::command::Cmd> = Vec::with_capacity(3);
        if self.state > 0 {
            out.push(pb::command::Cmd::SetState(pb::SetState {
                state: self.state,
            }));
        }
        out.push(pb::command::Cmd::Show(pb::Show {
            result: self.result_name.clone(),
            component: self.result_component.clone(),
            opts: std::collections::HashMap::new(),
        }));
        if let Some(c) = self.camera {
            out.push(pb::command::Cmd::View(pb::View {
                op: Some(pb::view::Op::Set(pb::SetCamera {
                    azimuth: c.azimuth,
                    elevation: c.elevation,
                    distance: c.distance,
                    fx: Some(c.fx),
                    fy: Some(c.fy),
                    fz: Some(c.fz),
                })),
            }));
        }
        out
    }

    /// One-line summary for the inline turn-boundary marker
    /// (`client.md` §"AI Assistant panel" P1 primary surface):
    /// `state=N · result=name`. Empty result reads `(no result)`.
    #[must_use]
    pub fn summary(&self) -> String {
        let r = if self.result_name.is_empty() {
            "(no result)".to_string()
        } else if self.result_component.is_empty() {
            self.result_name.clone()
        } else {
            format!("{}.{}", self.result_name, self.result_component)
        };
        format!("state={} · result={r}", self.state)
    }
}

/// Mutable AI Assistant panel state. Owned by [`crate::ShellState`];
/// folded by [`AiPanelState::ingest_event`] and snapshotted by the
/// windowed app at user-turn boundaries (Decision 97).
#[derive(Debug, Clone, Default)]
pub struct AiPanelState {
    /// True iff the server's `HelloReply.capabilities` carried
    /// `CAP_AGENT` (`scripting.md` capability-negotiation pattern).
    /// `false` ⇒ the right-dock stays the 28 px placeholder rail.
    pub cap_agent: bool,
    /// Panel chrome: `false` = collapsed 28 px rail (default,
    /// wireframes §"L1 — Default"), `true` = expanded 340 px panel.
    pub expanded: bool,
    /// Composer input buffer.
    pub composer: String,
    /// User clicked 📷 for the next message; the windowed app calls
    /// `CaptureFrame` on Send and pins the bytes (`client.md` §
    /// "Vision is deliberate but agent-initiated").
    pub attach_frame_pending: bool,
    /// Current agent status — drives the pill colour + the
    /// Send/Stop button swap.
    pub status: AgentStatus,
    /// Status detail string (server's `AgentStatus.detail`). Used to
    /// surface error / interrupted reasons in the UI; also the
    /// carrier for `peers=N` (Decision 99) — extracted into
    /// `peer_count` for the banner.
    pub status_detail: String,
    /// The id of the in-flight turn (server's `AgentChatReply.turn_id`)
    /// — passed to `Interrupt` when the user clicks Stop.
    pub active_turn_id: Option<String>,
    /// Rendered transcript rows in display order.
    pub rows: Vec<TranscriptRow>,
    /// Per-turn snapshots for `↶ revert to here` (Decision 97);
    /// keyed by the matching `TranscriptRow::TurnBoundary.turn_id`.
    pub snapshots: Vec<TurnSnapshot>,
}

/// Bound on the rendered transcript to keep the panel responsive on a
/// long session (mirrors the server-side AGENT_TRANSCRIPT_CAP cap on
/// the replay carrier). When exceeded, oldest rows drop FIFO; the
/// matching snapshot drops with the turn-boundary row.
const TRANSCRIPT_CAP: usize = 256;

impl AiPanelState {
    /// Fold one broadcast `AgentEvent` into the transcript +
    /// status (`phase-5-m6.md` Decision 99 / `client.md` §"AI
    /// Assistant panel"). Idempotent on repeat events with the same
    /// `call_id`: a duplicate `ToolBegin` is ignored; `ToolEnd`
    /// completes the matching row in place.
    pub fn ingest_event(&mut self, ev: &pb::AgentEvent) {
        use pb::agent_event::Ev;
        let Some(payload) = &ev.ev else { return };
        match payload {
            Ev::UserTurn(u) => {
                self.rows.push(TranscriptRow::User {
                    turn_id: ev.turn_id.clone(),
                    text: u.text.clone(),
                });
                // Phase 5 M6 Decision 97 — if a pre-turn snapshot has
                // already been stashed for this turn (the windowed
                // app calls `stash_snapshot` synchronously off its
                // own `agent_chat` call before the broadcast lands),
                // append the inline turn-boundary row right after the
                // user row so the wireframes §P1 ordering (user →
                // boundary → assistant) holds.
                if let Some(snap) = self.snapshots.iter().find(|s| s.turn_id == ev.turn_id) {
                    let summary = snap.summary();
                    self.rows.push(TranscriptRow::TurnBoundary {
                        turn_id: ev.turn_id.clone(),
                        summary,
                    });
                }
                // Assistant row is appended lazily on first Token so
                // a tool-only turn doesn't render an empty bubble.
                self.active_turn_id = Some(ev.turn_id.clone());
            }
            Ev::Token(t) => {
                if let Some(TranscriptRow::Assistant { turn_id, text }) = self
                    .rows
                    .iter_mut()
                    .rev()
                    .find(|r| matches!(r, TranscriptRow::Assistant { .. }))
                {
                    if turn_id == &ev.turn_id {
                        text.push_str(&t.text);
                        self.cap();
                        return;
                    }
                }
                self.rows.push(TranscriptRow::Assistant {
                    turn_id: ev.turn_id.clone(),
                    text: t.text.clone(),
                });
            }
            Ev::ToolBegin(b) => {
                if self.rows.iter().any(
                    |r| matches!(r, TranscriptRow::Tool { call_id, .. } if call_id == &b.call_id),
                ) {
                    return;
                }
                self.rows.push(TranscriptRow::Tool {
                    turn_id: ev.turn_id.clone(),
                    call_id: b.call_id.clone(),
                    summary: b.summary.clone(),
                    result: String::new(),
                    ok: true,
                    delta_seq: 0,
                    complete: false,
                });
            }
            Ev::ToolEnd(e) => {
                for row in self.rows.iter_mut().rev() {
                    if let TranscriptRow::Tool {
                        call_id,
                        result,
                        ok,
                        delta_seq,
                        complete,
                        ..
                    } = row
                    {
                        if call_id == &e.call_id {
                            *result = e.result_summary.clone();
                            *ok = e.ok;
                            *delta_seq = e.delta_seq;
                            *complete = true;
                            break;
                        }
                    }
                }
            }
            Ev::Status(s) => {
                let kind =
                    pb::AgentStatusKind::try_from(s.kind).unwrap_or(pb::AgentStatusKind::AgentIdle);
                self.status = AgentStatus::from_proto(kind);
                self.status_detail = s.detail.clone();
                if matches!(self.status, AgentStatus::Interrupted) {
                    // Wireframes §"User interrupted": tail row, the
                    // composer placeholder reflects the cancel. The
                    // active turn id stays around until the next
                    // user message so the user can see what stopped.
                    self.rows.push(TranscriptRow::Interrupted {
                        turn_id: ev.turn_id.clone(),
                    });
                }
                if !self.status.in_flight() {
                    self.active_turn_id = None;
                }
            }
        }
        self.cap();
    }

    /// Stash a captured pre-turn snapshot (Decision 97). The
    /// windowed app calls this synchronously off its own
    /// `agent_chat` (it knows the current state — every peer that
    /// observes the broadcast `UserTurn` already has the matching
    /// `ShellState`, so the snapshot is captured locally without a
    /// server round-trip). The inline `TurnBoundary` row is inserted
    /// by [`AiPanelState::ingest_event`] when it sees the matching
    /// `UserTurn` event — either insertion order (stash-then-event or
    /// event-then-stash) results in the wireframes §P1
    /// user → boundary → assistant order.
    pub fn stash_snapshot(&mut self, snap: TurnSnapshot) {
        let turn_id = snap.turn_id.clone();
        self.snapshots.push(snap);
        // Event-then-stash: the user row already landed via
        // `ingest_event` before we got here; insert the boundary now.
        if let Some(idx) = self
            .rows
            .iter()
            .rposition(|r| matches!(r, TranscriptRow::User { turn_id: t, .. } if t == &turn_id))
        {
            // Already have a boundary? (Replay path may have inserted
            // one synthetically.) Don't double-insert.
            let already = self.rows.iter().skip(idx + 1).take(2).any(
                |r| matches!(r, TranscriptRow::TurnBoundary { turn_id: t, .. } if t == &turn_id),
            );
            if !already {
                let summary = self
                    .snapshots
                    .last()
                    .map(TurnSnapshot::summary)
                    .unwrap_or_default();
                self.rows
                    .insert(idx + 1, TranscriptRow::TurnBoundary { turn_id, summary });
            }
        }
        self.cap();
    }

    /// Look up the captured snapshot for `turn_id` (the `↶ revert to
    /// here` link target). Returns `None` if the turn predates this
    /// peer's join time or its snapshot was pruned by [`TRANSCRIPT_CAP`].
    #[must_use]
    pub fn snapshot_for(&self, turn_id: &str) -> Option<&TurnSnapshot> {
        self.snapshots.iter().find(|s| s.turn_id == turn_id)
    }

    /// Submit the composer buffer. Returns `Some(UiAction::AgentChat)`
    /// for a non-empty buffer and clears the composer + pending
    /// attach flag; `None` for an empty buffer (matches the M3.5
    /// command-line submit semantics).
    pub fn submit(&mut self) -> Option<AgentChatIntent> {
        let text = self.composer.trim().to_string();
        if text.is_empty() {
            return None;
        }
        self.composer.clear();
        let attach_frame = self.attach_frame_pending;
        self.attach_frame_pending = false;
        Some(AgentChatIntent { text, attach_frame })
    }

    /// Toggle the panel chrome between collapsed rail and expanded
    /// panel (wireframes §"L1 — Default" / §"L2 — AI expanded").
    pub fn set_expanded(&mut self, on: bool) {
        self.expanded = on;
    }

    /// Toggle the 📷 attach-frame pending flag — pre-Send affordance,
    /// the windowed app calls `CaptureFrame` on Send.
    pub fn toggle_attach_frame(&mut self) {
        self.attach_frame_pending = !self.attach_frame_pending;
    }

    /// Decode the peer count piggybacked in [`Self::status_detail`]
    /// (Decision 99 — `peers=N` in the free-form detail string). 1
    /// when the field is missing / unparseable (a session must have
    /// at least the one self-subscriber, so a peer-count of 0 is
    /// meaningless to surface).
    #[must_use]
    pub fn peer_count(&self) -> u32 {
        parse_peer_count(&self.status_detail).unwrap_or(1)
    }

    /// Whether the wireframes §"Multi-client (peer attached)" banner
    /// should be drawn. True iff more than one peer is attached.
    #[must_use]
    pub fn has_peers(&self) -> bool {
        self.peer_count() > 1
    }

    /// Reset the panel back to its post-`Close` state (wireframes:
    /// the transcript is per-session, so a `Close` clears it).
    pub fn clear(&mut self) {
        self.rows.clear();
        self.snapshots.clear();
        self.active_turn_id = None;
        self.status = AgentStatus::Idle;
        self.status_detail.clear();
        self.composer.clear();
        self.attach_frame_pending = false;
    }

    fn cap(&mut self) {
        if self.rows.len() > TRANSCRIPT_CAP {
            let excess = self.rows.len() - TRANSCRIPT_CAP;
            let dropped: Vec<String> = self
                .rows
                .drain(0..excess)
                .filter_map(|r| match r {
                    TranscriptRow::TurnBoundary { turn_id, .. } => Some(turn_id),
                    _ => None,
                })
                .collect();
            self.snapshots.retain(|s| !dropped.contains(&s.turn_id));
        }
    }
}

/// The submit-time payload — the windowed app calls
/// `Session::agent_chat(text, attach_frame)` against it (and
/// optionally pre-encodes the frame via `Session::capture_frame`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentChatIntent {
    pub text: String,
    pub attach_frame: bool,
}

/// Decode `peers=N` out of a free-form `AgentStatus.detail` (Decision
/// 99). Robust against extra `;`-separated fields a future server
/// might append (e.g. `peers=3; backend=mock`).
#[must_use]
pub fn parse_peer_count(detail: &str) -> Option<u32> {
    for part in detail.split(|c: char| c == ';' || c == ',' || c.is_whitespace()) {
        if let Some(rest) = part.trim().strip_prefix("peers=") {
            return rest.parse().ok();
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn user_turn(turn: &str, text: &str) -> pb::AgentEvent {
        pb::AgentEvent {
            turn_id: turn.into(),
            ev: Some(pb::agent_event::Ev::UserTurn(pb::AgentUserTurn {
                text: text.into(),
                had_frame: false,
            })),
        }
    }

    fn token(turn: &str, text: &str) -> pb::AgentEvent {
        pb::AgentEvent {
            turn_id: turn.into(),
            ev: Some(pb::agent_event::Ev::Token(pb::AgentToken {
                text: text.into(),
            })),
        }
    }

    fn status(turn: &str, kind: pb::AgentStatusKind, detail: &str) -> pb::AgentEvent {
        pb::AgentEvent {
            turn_id: turn.into(),
            ev: Some(pb::agent_event::Ev::Status(pb::AgentStatus {
                kind: kind as i32,
                detail: detail.into(),
            })),
        }
    }

    fn tool_begin(turn: &str, call: &str, summary: &str) -> pb::AgentEvent {
        pb::AgentEvent {
            turn_id: turn.into(),
            ev: Some(pb::agent_event::Ev::ToolBegin(pb::AgentToolBegin {
                call_id: call.into(),
                summary: summary.into(),
                detail: String::new(),
            })),
        }
    }

    fn tool_end(turn: &str, call: &str, summary: &str, seq: u64) -> pb::AgentEvent {
        pb::AgentEvent {
            turn_id: turn.into(),
            ev: Some(pb::agent_event::Ev::ToolEnd(pb::AgentToolEnd {
                call_id: call.into(),
                ok: true,
                result_summary: summary.into(),
                delta_seq: seq,
            })),
        }
    }

    #[test]
    fn parses_peer_count_in_assorted_details() {
        assert_eq!(parse_peer_count("peers=3"), Some(3));
        assert_eq!(parse_peer_count("backend=mock; peers=7"), Some(7));
        assert_eq!(parse_peer_count("ok peers=2 extra"), Some(2));
        assert_eq!(parse_peer_count(""), None);
        assert_eq!(parse_peer_count("no peer info"), None);
    }

    #[test]
    fn agent_status_in_flight_swaps_send_for_stop() {
        assert!(AgentStatus::Thinking.in_flight());
        assert!(AgentStatus::Running.in_flight());
        assert!(!AgentStatus::Idle.in_flight());
        assert!(!AgentStatus::Interrupted.in_flight());
        assert!(!AgentStatus::Error.in_flight());
    }

    #[test]
    fn ingest_event_folds_a_full_turn_into_rows_in_order() {
        let mut s = AiPanelState::default();
        s.ingest_event(&user_turn("t1", "diagnose the spike"));
        s.ingest_event(&status("t1", pb::AgentStatusKind::AgentThinking, ""));
        s.ingest_event(&token("t1", "okay, "));
        s.ingest_event(&token("t1", "let me check"));
        s.ingest_event(&tool_begin("t1", "c1", "ran: state 5"));
        s.ingest_event(&tool_end("t1", "c1", "state=5", 42));
        s.ingest_event(&status("t1", pb::AgentStatusKind::AgentIdle, "peers=2"));

        assert!(matches!(s.rows[0], TranscriptRow::User { .. }));
        let TranscriptRow::Assistant { ref text, .. } = s.rows[1] else {
            panic!("row[1] expected Assistant: {:?}", s.rows[1]);
        };
        assert_eq!(
            text, "okay, let me check",
            "tokens concatenate into one assistant row"
        );
        let TranscriptRow::Tool {
            complete,
            ref result,
            delta_seq,
            ..
        } = s.rows[2]
        else {
            panic!("row[2] expected Tool: {:?}", s.rows[2]);
        };
        assert!(complete, "ToolEnd marks the row complete");
        assert_eq!(result, "state=5");
        assert_eq!(delta_seq, 42, "ToolEnd.delta_seq plumbs through");
        assert_eq!(s.status, AgentStatus::Idle);
        assert_eq!(s.peer_count(), 2, "Status.detail peers=N parses through");
        assert!(s.active_turn_id.is_none(), "idle clears active_turn_id");
    }

    #[test]
    fn interrupt_status_appends_interrupted_row() {
        let mut s = AiPanelState::default();
        s.ingest_event(&user_turn("t1", "hi"));
        s.ingest_event(&status("t1", pb::AgentStatusKind::AgentThinking, ""));
        s.ingest_event(&status("t1", pb::AgentStatusKind::AgentInterrupted, ""));
        assert_eq!(s.status, AgentStatus::Interrupted);
        let last = s.rows.last().expect("nonempty");
        assert!(
            matches!(last, TranscriptRow::Interrupted { .. }),
            "interrupted row appended: {last:?}"
        );
    }

    #[test]
    fn submit_clears_buffers_and_returns_intent() {
        let mut s = AiPanelState {
            composer: "find max von mises".into(),
            attach_frame_pending: true,
            ..AiPanelState::default()
        };
        let i = s.submit().expect("non-empty composer submits");
        assert_eq!(i.text, "find max von mises");
        assert!(i.attach_frame);
        assert!(s.composer.is_empty());
        assert!(!s.attach_frame_pending, "submit resets the toggle");
        assert!(s.submit().is_none(), "empty composer no-ops");
    }

    #[test]
    fn stash_after_user_event_inserts_turn_boundary_in_correct_position() {
        let mut s = AiPanelState::default();
        s.ingest_event(&user_turn("t1", "hi"));
        s.ingest_event(&token("t1", "..."));
        s.stash_snapshot(TurnSnapshot {
            turn_id: "t1".into(),
            state: 47,
            result_name: "sx".into(),
            result_component: String::new(),
            camera: None,
        });
        // Order: User, TurnBoundary, Assistant
        assert!(matches!(s.rows[0], TranscriptRow::User { .. }));
        assert!(matches!(s.rows[1], TranscriptRow::TurnBoundary { .. }));
        assert!(matches!(s.rows[2], TranscriptRow::Assistant { .. }));
        assert_eq!(s.snapshot_for("t1").unwrap().state, 47);
    }

    #[test]
    fn stash_before_user_event_still_inserts_turn_boundary_after_user() {
        // Race: the app stashes the snapshot synchronously off its
        // agent_chat reply, but the broadcast UserTurn may arrive
        // through the delta-pump later. The boundary still has to
        // land after the user row.
        let mut s = AiPanelState::default();
        s.stash_snapshot(TurnSnapshot {
            turn_id: "t1".into(),
            state: 47,
            result_name: "sx".into(),
            result_component: String::new(),
            camera: None,
        });
        s.ingest_event(&user_turn("t1", "hi"));
        s.ingest_event(&token("t1", "..."));
        assert!(matches!(s.rows[0], TranscriptRow::User { .. }));
        assert!(matches!(s.rows[1], TranscriptRow::TurnBoundary { .. }));
        assert!(matches!(s.rows[2], TranscriptRow::Assistant { .. }));
        // Exactly one boundary regardless of order.
        let boundaries = s
            .rows
            .iter()
            .filter(|r| matches!(r, TranscriptRow::TurnBoundary { .. }))
            .count();
        assert_eq!(boundaries, 1, "no double-insert under either order");
    }

    #[test]
    fn snapshot_lower_emits_typed_command_sequence() {
        let snap = TurnSnapshot {
            turn_id: "t1".into(),
            state: 12,
            result_name: "sx".into(),
            result_component: String::new(),
            camera: Some(pb::CameraState {
                azimuth: 1.0,
                elevation: 0.5,
                distance: 8.0,
                fx: 0.1,
                fy: 0.2,
                fz: 0.3,
            }),
        };
        let cmds = snap.lower();
        assert_eq!(cmds.len(), 3, "SetState + Show + SetCamera");
        assert!(matches!(
            cmds[0],
            pb::command::Cmd::SetState(pb::SetState { state: 12 })
        ));
        assert!(matches!(
            cmds[1],
            pb::command::Cmd::Show(pb::Show { ref result, .. }) if result == "sx"
        ));
        let pb::command::Cmd::View(pb::View {
            op: Some(pb::view::Op::Set(ref sc)),
            ..
        }) = cmds[2]
        else {
            panic!("cmds[2] expected View::SetCamera: {:?}", cmds[2]);
        };
        assert!((sc.azimuth - 1.0).abs() < 1e-9);
        assert_eq!(sc.fx, Some(0.1));
    }

    #[test]
    fn snapshot_summary_handles_missing_result() {
        let snap = TurnSnapshot {
            turn_id: "t1".into(),
            state: 3,
            result_name: String::new(),
            result_component: String::new(),
            camera: None,
        };
        assert_eq!(snap.summary(), "state=3 · result=(no result)");
    }

    #[test]
    fn snapshot_summary_includes_component_when_set() {
        let snap = TurnSnapshot {
            turn_id: "t1".into(),
            state: 9,
            result_name: "sx".into(),
            result_component: "eff".into(),
            camera: None,
        };
        assert_eq!(snap.summary(), "state=9 · result=sx.eff");
    }

    #[test]
    fn clear_resets_panel_to_post_close_state() {
        let mut s = AiPanelState {
            composer: "draft".into(),
            attach_frame_pending: true,
            ..AiPanelState::default()
        };
        s.ingest_event(&user_turn("t1", "hi"));
        s.clear();
        assert!(s.rows.is_empty());
        assert!(s.snapshots.is_empty());
        assert_eq!(s.status, AgentStatus::Idle);
        assert!(s.composer.is_empty());
        assert!(!s.attach_frame_pending);
    }
}
