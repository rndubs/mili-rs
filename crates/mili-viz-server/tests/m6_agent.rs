//! Phase 5 M6 server-side gating gate (`phase-5-m6.md` § "M6
//! acceptance gate"). Every test below is always-on — the always-on
//! `MockAgent` is self-contained (no fixture corpus needed).

use std::time::Duration;

use mili_viz_proto::v1 as pb;
use mili_viz_server::{spawn_in_process, MockAgent, VizService, AGENT_MOCK_ORIGIN};
use tonic::Request;

async fn next_delta(sub: &mut tonic::Streaming<pb::StateDelta>) -> pb::StateDelta {
    tokio::time::timeout(Duration::from_secs(2), sub.message())
        .await
        .expect("delta did not arrive within 2s")
        .expect("subscription stream error")
        .expect("subscription closed early")
}

async fn drain_until_agent_status(
    sub: &mut tonic::Streaming<pb::StateDelta>,
    target_kind: pb::AgentStatusKind,
) -> pb::AgentEvent {
    loop {
        let d = next_delta(sub).await;
        if d.kind != pb::DeltaKind::DeltaAgent as i32 {
            continue;
        }
        let Some(pb::state_delta::Payload::Agent(ev)) = d.payload else {
            continue;
        };
        let Some(pb::agent_event::Ev::Status(ref s)) = ev.ev else {
            continue;
        };
        if s.kind == target_kind as i32 {
            return ev;
        }
    }
}

// ── Gate 1: capability gate ties to backend presence (Decisions 94 + 7) ──
#[tokio::test]
async fn capability_advertised_without_backend_returns_clear_error() {
    let (mut client, _h) = spawn_in_process(VizService::builder().agent(true).build())
        .await
        .unwrap();
    let r = client
        .agent_chat(Request::new(pb::AgentChatRequest {
            text: "hi".into(),
            ..Default::default()
        }))
        .await
        .unwrap()
        .into_inner();
    assert!(!r.ok);
    assert!(
        r.error.to_lowercase().contains("backend"),
        "no-backend message names what is missing: {}",
        r.error
    );
}

#[tokio::test]
async fn backend_present_lights_up_agent_chat() {
    let (mut client, _h) = spawn_in_process(VizService::builder().agent_backend(MockAgent).build())
        .await
        .unwrap();
    let r = client
        .agent_chat(Request::new(pb::AgentChatRequest {
            text: "hello".into(),
            ..Default::default()
        }))
        .await
        .unwrap()
        .into_inner();
    assert!(r.ok, "MockAgent backend ⇒ ok: {}", r.error);
    assert!(!r.turn_id.is_empty(), "server allocates a turn_id");
}

// ── Gate 2: full agent_chat broadcast in order (Decisions 94/95/98) ──
#[tokio::test]
async fn agent_chat_broadcasts_user_status_token_tool_pair_and_dispatched_delta() {
    let (mut client, _h) = spawn_in_process(VizService::builder().agent_backend(MockAgent).build())
        .await
        .unwrap();
    let mut sub = client
        .subscribe(Request::new(pb::SubscribeRequest::default()))
        .await
        .unwrap()
        .into_inner();
    // Drain the opening snapshot + the peers=N status the subscribe
    // triggers (Decision 99).
    let snap = next_delta(&mut sub).await;
    assert_eq!(snap.kind, pb::DeltaKind::DeltaSnapshot as i32);
    let _peers = next_delta(&mut sub).await; // the post-subscribe peer status

    let reply = client
        .agent_chat(Request::new(pb::AgentChatRequest {
            text: "diagnose the spike".into(),
            ..Default::default()
        }))
        .await
        .unwrap()
        .into_inner();
    assert!(reply.ok);
    let turn_id = reply.turn_id;

    // Collect deltas until we have observed:
    //  - UserTurn
    //  - at least one Token
    //  - ToolBegin / ToolEnd pair
    //  - the dispatched StateDelta (DELTA_STATE) tagged with the
    //    agent's origin_client_id
    //  - the closing Status(idle)
    let mut saw_user = false;
    let mut tokens = String::new();
    let mut tool_begin: Option<pb::AgentToolBegin> = None;
    let mut tool_end: Option<pb::AgentToolEnd> = None;
    let mut dispatched_seq: Option<u64> = None;
    let mut closing_status: Option<pb::AgentStatus> = None;
    let deadline = std::time::Instant::now() + Duration::from_secs(2);
    while std::time::Instant::now() < deadline && closing_status.is_none() {
        let d = next_delta(&mut sub).await;
        match d.payload {
            Some(pb::state_delta::Payload::Agent(ev)) => {
                assert_eq!(ev.turn_id, turn_id);
                match ev.ev {
                    Some(pb::agent_event::Ev::UserTurn(u)) => {
                        saw_user = true;
                        assert_eq!(u.text, "diagnose the spike");
                    }
                    Some(pb::agent_event::Ev::Token(t)) => tokens.push_str(&t.text),
                    Some(pb::agent_event::Ev::ToolBegin(b)) => tool_begin = Some(b),
                    Some(pb::agent_event::Ev::ToolEnd(e)) => tool_end = Some(e),
                    Some(pb::agent_event::Ev::Status(s)) => {
                        if s.kind == pb::AgentStatusKind::AgentIdle as i32 {
                            closing_status = Some(s);
                        }
                    }
                    None => {}
                }
            }
            Some(pb::state_delta::Payload::State(_)) => {
                // The dispatched tool-call command (SetState(1) per
                // MockAgent) lands here as an ordinary StateDelta
                // tagged with the agent's origin_client_id.
                assert_eq!(
                    d.origin_client_id, AGENT_MOCK_ORIGIN,
                    "dispatched StateDelta is tagged with the agent's origin"
                );
                dispatched_seq = Some(d.seq);
            }
            _ => {}
        }
    }
    assert!(saw_user, "UserTurn event must broadcast");
    assert!(!tokens.is_empty(), "at least one Token streamed");
    let tool_begin = tool_begin.expect("ToolBegin broadcast");
    let tool_end = tool_end.expect("ToolEnd broadcast");
    assert_eq!(tool_begin.call_id, tool_end.call_id, "matching call_id");
    let dispatched_seq = dispatched_seq.expect("dispatched StateDelta observed");
    assert_eq!(
        tool_end.delta_seq, dispatched_seq,
        "ToolEnd.delta_seq round-trips the broadcast StateDelta.seq"
    );
    let s = closing_status.expect("closing Status(idle)");
    assert!(
        s.detail.contains("peers="),
        "Status.detail piggybacks the peer count (Decision 99): {}",
        s.detail
    );
}

// ── Gate 3: Interrupt cancels mid-turn (Decision 98) ──
#[tokio::test]
async fn interrupt_causes_an_interrupted_status_to_broadcast() {
    let (mut client, _h) = spawn_in_process(VizService::builder().agent_backend(MockAgent).build())
        .await
        .unwrap();
    let mut sub = client
        .subscribe(Request::new(pb::SubscribeRequest::default()))
        .await
        .unwrap()
        .into_inner();
    next_delta(&mut sub).await; // opening snapshot
    next_delta(&mut sub).await; // peers=N

    let reply = client
        .agent_chat(Request::new(pb::AgentChatRequest {
            text: "long task".into(),
            ..Default::default()
        }))
        .await
        .unwrap()
        .into_inner();
    assert!(reply.ok);
    // Immediately interrupt — by `turn_id` to verify lookup also
    // works (not just the empty-id "cancel current turn" path).
    let r = client
        .interrupt(Request::new(pb::InterruptRequest {
            turn_id: reply.turn_id.clone(),
        }))
        .await
        .unwrap()
        .into_inner();
    assert!(r.ok);

    // The closing status must be Interrupted, not Idle. The mock
    // backend yields between every emit so the cancel flag flips in
    // time for the next observation.
    let ev = drain_until_agent_status(&mut sub, pb::AgentStatusKind::AgentInterrupted).await;
    assert_eq!(ev.turn_id, reply.turn_id);
}

#[tokio::test]
async fn interrupt_with_no_active_turn_is_a_clean_noop_success() {
    let (mut client, _h) = spawn_in_process(VizService::builder().agent_backend(MockAgent).build())
        .await
        .unwrap();
    let r = client
        .interrupt(Request::new(pb::InterruptRequest::default()))
        .await
        .unwrap()
        .into_inner();
    assert!(r.ok, "Interrupt on idle never errors (intent = stop)");
}

// ── Gate 4: CaptureFrame returns deterministic placeholder (Decision 96) ──
#[tokio::test]
async fn capture_frame_returns_a_png_of_requested_extent() {
    let (mut client, _h) = spawn_in_process(VizService::builder().agent_backend(MockAgent).build())
        .await
        .unwrap();
    let r = client
        .capture_frame(Request::new(pb::FrameRequest {
            width: 24,
            height: 16,
            format: "png".into(),
        }))
        .await
        .unwrap()
        .into_inner();
    assert!(r.ok, "{}", r.error);
    assert_eq!(r.format, "png");
    assert_eq!(r.width, 24);
    assert_eq!(r.height, 16);
    let img = image::load_from_memory(&r.image).expect("server returned a decodable PNG");
    assert_eq!(img.width(), 24);
    assert_eq!(img.height(), 16);
}

#[tokio::test]
async fn capture_frame_returns_non_empty_jpeg() {
    let (mut client, _h) = spawn_in_process(VizService::builder().agent_backend(MockAgent).build())
        .await
        .unwrap();
    let r = client
        .capture_frame(Request::new(pb::FrameRequest {
            width: 32,
            height: 24,
            format: "jpeg".into(),
        }))
        .await
        .unwrap()
        .into_inner();
    assert!(r.ok);
    assert_eq!(r.format, "jpeg");
    assert!(!r.image.is_empty());
}

#[tokio::test]
async fn capture_frame_rejects_zero_extent_cleanly() {
    let (mut client, _h) = spawn_in_process(VizService::builder().agent_backend(MockAgent).build())
        .await
        .unwrap();
    let r = client
        .capture_frame(Request::new(pb::FrameRequest {
            width: 0,
            height: 16,
            format: "png".into(),
        }))
        .await
        .unwrap()
        .into_inner();
    assert!(!r.ok, "zero extent ⇒ ok=false with a clear error");
    assert!(r.image.is_empty());
}

// ── Gate 5: Snapshot.agent populated after a turn lands (Decision 97) ──
#[tokio::test]
async fn late_subscriber_sees_populated_transcript_in_opening_snapshot() {
    let svc = VizService::builder().agent_backend(MockAgent).build();
    let (mut client, _h) = spawn_in_process(svc.clone()).await.unwrap();

    // Drain the first peer's subscription through one full turn so
    // the server-side `TurnRecord` is populated.
    let mut first = client
        .subscribe(Request::new(pb::SubscribeRequest::default()))
        .await
        .unwrap()
        .into_inner();
    next_delta(&mut first).await; // opening snapshot
    next_delta(&mut first).await; // peers=1 status

    let reply = client
        .agent_chat(Request::new(pb::AgentChatRequest {
            text: "fixture turn".into(),
            ..Default::default()
        }))
        .await
        .unwrap()
        .into_inner();
    assert!(reply.ok);
    drain_until_agent_status(&mut first, pb::AgentStatusKind::AgentIdle).await;

    // Now a late subscriber: their opening DELTA_SNAPSHOT carries the
    // populated AgentTranscript per Decision 97.
    let mut late = client
        .subscribe(Request::new(pb::SubscribeRequest::default()))
        .await
        .unwrap()
        .into_inner();
    let d = next_delta(&mut late).await;
    assert_eq!(d.kind, pb::DeltaKind::DeltaSnapshot as i32);
    let Some(pb::state_delta::Payload::Snapshot(s)) = d.payload else {
        panic!("late opening was not a Snapshot");
    };
    let t = s.agent.expect("Snapshot.agent populated post-M6");
    assert!(
        t.messages.len() >= 2,
        "user + assistant rows present: {} messages",
        t.messages.len()
    );
    let has_user = t
        .messages
        .iter()
        .any(|m| m.role == "user" && m.text == "fixture turn");
    assert!(has_user, "user row of the prior turn is replayed");
    let has_assistant = t
        .messages
        .iter()
        .any(|m| m.role == "assistant" && !m.text.is_empty());
    assert!(has_assistant, "assistant row carries the streamed tokens");
    let status = t.status.expect("transcript carries a current status");
    assert!(
        status.detail.contains("peers="),
        "peers=N rides the transcript status too: {}",
        status.detail
    );
}

// ── Gate 6: peer count broadcast on subscribe (Decision 99) ──
#[tokio::test]
async fn second_subscriber_triggers_peer_count_broadcast_to_first() {
    let (mut client, _h) = spawn_in_process(VizService::builder().agent_backend(MockAgent).build())
        .await
        .unwrap();
    let mut first = client
        .subscribe(Request::new(pb::SubscribeRequest::default()))
        .await
        .unwrap()
        .into_inner();
    next_delta(&mut first).await; // opening snapshot
    let _peers1 = next_delta(&mut first).await; // peers=1 status

    // A second subscribe must broadcast peers=2 to every prior peer.
    let _second = client
        .subscribe(Request::new(pb::SubscribeRequest::default()))
        .await
        .unwrap()
        .into_inner();
    let d = next_delta(&mut first).await;
    assert_eq!(d.kind, pb::DeltaKind::DeltaAgent as i32);
    let Some(pb::state_delta::Payload::Agent(ev)) = d.payload else {
        panic!("expected AgentEvent");
    };
    let Some(pb::agent_event::Ev::Status(s)) = ev.ev else {
        panic!("expected Status");
    };
    assert_eq!(s.kind, pb::AgentStatusKind::AgentIdle as i32);
    assert!(
        s.detail.contains("peers=2"),
        "second subscribe ⇒ peers=2: {}",
        s.detail
    );
}
