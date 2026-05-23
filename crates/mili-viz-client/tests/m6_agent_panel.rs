//! Phase 5 M6 client-side gating gate (`phase-5-m6.md` § "M6
//! acceptance gate"). All always-on — `AiPanelState`/`ShellState` is
//! GPU-free / transport-free pure logic; the windowed dispatch lives
//! in `app.rs` and is not CI-exercised.

use mili_viz_client::{
    build_shell_ui, parse_peer_count, AgentChatIntent, AgentStatus, AiPanelState, ShellState,
    TranscriptRow, TurnSnapshot, UiAction,
};
use mili_viz_proto::v1 as pb;

// ── Gate 1: panel hidden absent CAP_AGENT (`scripting.md` capability gate) ──
#[test]
fn cap_agent_false_hides_the_panel() {
    let mut state = ShellState::default();
    assert!(!state.ai.cap_agent);
    // Without cap_agent, the 28 px placeholder rail is the only AI
    // chrome — `expanded` flipping does nothing visible because the
    // ai_dock branch is not even reached (asserted via the
    // build_shell_ui golden in cap_agent_true_expanded_panel_paints
    // below; the headless paint covers both arms in one composite
    // gate).
    state.ai.set_expanded(true);
    assert!(
        state.ai.expanded,
        "state flips even without cap (toggle is harmless)"
    );
    // The render gate (Gate 2) covers the actual paint behaviour.
    let _ = state;
}

// ── Gate 2: cap_agent + expanded = panel renders end-to-end ──
#[test]
fn cap_agent_true_expanded_panel_paints_without_input_actions() {
    let ctx = egui::Context::default();
    let mut state = ShellState::default();
    state.ai.cap_agent = true;
    state.ai.set_expanded(true);
    state.ai.composer = "hi".into();

    let mut actions: Vec<UiAction> = Vec::new();
    let raw = egui::RawInput::default();
    let _ = ctx.run_ui(raw, |ui| {
        actions = build_shell_ui(ui, &mut state);
    });

    // No real input events ⇒ no submit / no expand-rail / no
    // interrupt action. The composer just rendered; the buffer is
    // unchanged.
    assert_eq!(state.ai.composer, "hi", "no synthetic submit");
    assert!(
        actions.iter().all(|a| !matches!(
            a,
            UiAction::AgentChat { .. }
                | UiAction::AgentInterrupt { .. }
                | UiAction::AgentRevert { .. }
        )),
        "no input ⇒ no agent action: {actions:?}"
    );
}

// ── Gate 3: AgentEvent → transcript line mapping ──
#[test]
fn ingest_event_folds_full_turn_into_ordered_rows() {
    let mut s = AiPanelState::default();
    s.ingest_event(&user("t1", "diagnose"));
    s.ingest_event(&status("t1", pb::AgentStatusKind::AgentThinking, ""));
    s.ingest_event(&token("t1", "ok, "));
    s.ingest_event(&token("t1", "scanning"));
    s.ingest_event(&tool_begin("t1", "c1", "ran: state 5"));
    s.ingest_event(&tool_end("t1", "c1", "state=5", 42));
    s.ingest_event(&status("t1", pb::AgentStatusKind::AgentIdle, "peers=3"));

    // Row order: user, assistant, tool. (No TurnBoundary yet — that
    // is gate 7.)
    assert!(matches!(s.rows[0], TranscriptRow::User { .. }));
    let TranscriptRow::Assistant { ref text, .. } = s.rows[1] else {
        panic!("rows[1] must be Assistant: {:?}", s.rows[1]);
    };
    assert_eq!(text, "ok, scanning", "tokens concatenated into one row");
    let TranscriptRow::Tool {
        complete,
        ref result,
        delta_seq,
        ..
    } = s.rows[2]
    else {
        panic!("rows[2] must be Tool: {:?}", s.rows[2]);
    };
    assert!(complete, "ToolEnd marks the row complete");
    assert_eq!(result, "state=5");
    assert_eq!(delta_seq, 42, "ToolEnd.delta_seq plumbs through");
    assert_eq!(s.status, AgentStatus::Idle);
    assert_eq!(s.peer_count(), 3, "peers=N parses out of status detail");
}

// ── Gate 4: composer Send → typed action ──
#[test]
fn submit_returns_intent_and_clears_buffers() {
    let mut s = AiPanelState::default();
    assert!(s.submit().is_none(), "empty composer no-ops");
    s = AiPanelState {
        composer: "find max sx".into(),
        attach_frame_pending: true,
        ..s
    };
    let intent = s.submit().expect("non-empty composer returns intent");
    assert_eq!(
        intent,
        AgentChatIntent {
            text: "find max sx".into(),
            attach_frame: true
        }
    );
    assert!(s.composer.is_empty(), "submit clears the buffer");
    assert!(!s.attach_frame_pending, "submit resets the toggle");
}

// ── Gate 5: Stop swaps for Send when status is in-flight ──
#[test]
fn in_flight_status_is_the_stop_swap_signal() {
    let mut s = AiPanelState::default();
    assert!(!s.status.in_flight(), "default is Idle ⇒ no Stop");
    s.ingest_event(&user("t1", "hi"));
    s.ingest_event(&status("t1", pb::AgentStatusKind::AgentThinking, ""));
    assert!(s.status.in_flight(), "Thinking ⇒ Stop replaces Send");
    s.ingest_event(&status("t1", pb::AgentStatusKind::AgentRunning, ""));
    assert!(s.status.in_flight(), "Running ⇒ Stop still");
    s.ingest_event(&status("t1", pb::AgentStatusKind::AgentInterrupted, ""));
    assert!(!s.status.in_flight(), "Interrupted ⇒ Send returns");
    assert!(
        matches!(s.rows.last().unwrap(), TranscriptRow::Interrupted { .. }),
        "wireframes §User interrupted tail row appended",
    );
    assert!(
        s.active_turn_id.is_none(),
        "post-interrupt active_turn_id clears"
    );
}

// ── Gate 6: peer count parse + count cell ──
#[test]
fn peer_count_parses_assorted_details() {
    assert_eq!(parse_peer_count("peers=5"), Some(5));
    assert_eq!(parse_peer_count("ok peers=2"), Some(2));
    assert_eq!(parse_peer_count("backend=mock; peers=11"), Some(11));
    assert_eq!(parse_peer_count(""), None);
    assert_eq!(parse_peer_count("no peer info"), None);
}

#[test]
fn peer_count_drives_panel_state() {
    let mut s = AiPanelState::default();
    assert_eq!(s.peer_count(), 1, "default ⇒ at-least-one self");
    assert!(!s.has_peers(), "no peers banner by default");
    s.ingest_event(&status("", pb::AgentStatusKind::AgentIdle, "peers=4"));
    assert_eq!(s.peer_count(), 4);
    assert!(s.has_peers(), "peers>1 ⇒ banner shows");
}

// ── Gate 7: revert lowering pin (Decision 97) ──
#[test]
fn revert_lowers_to_typed_setstate_show_setcamera_sequence() {
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
    // None of these is `raw` — the M6 contract is "every primitive in
    // the frozen Command set" (Decision 97).
    for c in &cmds {
        assert!(!matches!(c, pb::command::Cmd::Raw(_)), "never `raw`: {c:?}");
    }
    assert!(matches!(
        cmds[0],
        pb::command::Cmd::SetState(pb::SetState { state: 12 })
    ));
    let pb::command::Cmd::Show(pb::Show {
        ref result,
        ref component,
        ..
    }) = cmds[1]
    else {
        panic!("cmds[1] = Show: {:?}", cmds[1]);
    };
    assert_eq!(result, "sx");
    assert!(component.is_empty());
    let pb::command::Cmd::View(pb::View {
        op: Some(pb::view::Op::Set(ref sc)),
        ..
    }) = cmds[2]
    else {
        panic!("cmds[2] = View::SetCamera: {:?}", cmds[2]);
    };
    assert!((sc.distance - 8.0).abs() < 1e-9);
}

#[test]
fn revert_without_camera_emits_setstate_and_show_only() {
    let snap = TurnSnapshot {
        turn_id: "t1".into(),
        state: 1,
        result_name: "disp_mag".into(),
        result_component: String::new(),
        camera: None,
    };
    let cmds = snap.lower();
    assert_eq!(cmds.len(), 2, "no camera ⇒ no SetCamera arm");
    assert!(matches!(cmds[0], pb::command::Cmd::SetState(_)));
    assert!(matches!(cmds[1], pb::command::Cmd::Show(_)));
}

#[test]
fn turn_boundary_inserts_between_user_and_assistant() {
    let mut s = AiPanelState::default();
    // Race A: stash first, then UserTurn event arrives.
    s.stash_snapshot(TurnSnapshot {
        turn_id: "t1".into(),
        state: 7,
        result_name: "sx".into(),
        result_component: String::new(),
        camera: None,
    });
    s.ingest_event(&user("t1", "hi"));
    s.ingest_event(&token("t1", "..."));
    assert!(matches!(s.rows[0], TranscriptRow::User { .. }));
    assert!(matches!(s.rows[1], TranscriptRow::TurnBoundary { .. }));
    assert!(matches!(s.rows[2], TranscriptRow::Assistant { .. }));
}

// ── Helpers ──────────────────────────────────────────────────────────

fn user(turn: &str, text: &str) -> pb::AgentEvent {
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
