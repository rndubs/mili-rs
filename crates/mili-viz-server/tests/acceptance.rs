//! Phase 4 M1 acceptance gate (`phase-4-m1.md` § "M1 acceptance
//! gate"). Each `#[tokio::test]` below maps 1:1 to a gate checkbox.
//! There is no upstream oracle for viz; per Decision 5's reasoning
//! generalized to the protocol, the gate is conformance +
//! internal-equivalence (Layer-0 ≡ raw, fan-out ordering).

use mili_viz_proto::v1 as pb;
use mili_viz_server::{command_delta_kind, parse_line, spawn_in_process, to_raw, VizService};
use pb::mili_viz_client::MiliVizClient;
use prost::Message;
use tonic::transport::Channel;
use tonic::Request;

type Client = MiliVizClient<Channel>;

fn with_client_id<T>(msg: T, id: &str) -> Request<T> {
    let mut req = Request::new(msg);
    req.metadata_mut()
        .insert("x-client-id", id.parse().unwrap());
    req
}

fn norm(mut d: pb::StateDelta) -> pb::StateDelta {
    d.seq = 0;
    d.origin_client_id = String::new();
    d
}

/// Subscribe, drain the opening `DELTA_SNAPSHOT`, run `cmd`, and
/// return `(CommandReply, the broadcast StateDelta)`.
async fn exec_capture(
    client: &mut Client,
    cmd: pb::command::Cmd,
) -> (pb::CommandReply, pb::StateDelta) {
    let mut sub = client
        .subscribe(Request::new(pb::SubscribeRequest::default()))
        .await
        .unwrap()
        .into_inner();
    let first = sub.message().await.unwrap().unwrap();
    assert_eq!(first.kind, pb::DeltaKind::DeltaSnapshot as i32);

    let reply = client
        .execute(Request::new(pb::Command { cmd: Some(cmd) }))
        .await
        .unwrap()
        .into_inner();
    assert!(reply.ok, "execute failed: {}", reply.error);

    let delta = sub.message().await.unwrap().unwrap();
    (reply, delta)
}

/// Every typed `Command` variant we exercise. Covers all 16 typed
/// `oneof` arms (the 17th, `raw`, is covered explicitly below) plus
/// the `View`/`Step`/`NamedView` sub-variants.
#[allow(clippy::too_many_lines)]
fn sample_commands() -> Vec<(&'static str, pb::command::Cmd)> {
    use pb::command::Cmd;
    use pb::view::Op;
    let mut v = vec![
        (
            "load",
            Cmd::Load(pb::Load {
                root: "/runs/demo".into(),
            }),
        ),
        ("close", Cmd::Close(pb::Close {})),
        ("set_state", Cmd::SetState(pb::SetState { state: 7 })),
        (
            "step_next",
            Cmd::Step(pb::Step {
                dir: pb::step::Dir::Next as i32,
            }),
        ),
        (
            "step_prev",
            Cmd::Step(pb::Step {
                dir: pb::step::Dir::Prev as i32,
            }),
        ),
        (
            "step_first",
            Cmd::Step(pb::Step {
                dir: pb::step::Dir::First as i32,
            }),
        ),
        (
            "step_last",
            Cmd::Step(pb::Step {
                dir: pb::step::Dir::Last as i32,
            }),
        ),
        (
            "select",
            Cmd::Select(pb::Select {
                class_name: "brick".into(),
                range: "1-100,150".into(),
            }),
        ),
        (
            "clrsel",
            Cmd::Clrsel(pb::ClearSelection {
                class_name: "brick".into(),
            }),
        ),
        ("show", {
            let mut opts = std::collections::HashMap::new();
            opts.insert("scale".to_string(), "log".to_string());
            Cmd::Show(pb::Show {
                result: "sx".into(),
                component: "eff".into(),
                opts,
            })
        }),
        (
            "show_no_comp",
            Cmd::Show(pb::Show {
                result: "vmag".into(),
                component: String::new(),
                opts: std::collections::HashMap::default(),
            }),
        ),
        (
            "rot",
            Cmd::View(pb::View {
                op: Some(Op::Rotate(pb::Rotate {
                    x: 30.0,
                    y: -15.5,
                    z: 0.0,
                })),
            }),
        ),
        (
            "translate",
            Cmd::View(pb::View {
                op: Some(Op::Translate(pb::Translate {
                    dx: 1.0,
                    dy: 2.5,
                    dz: -3.0,
                })),
            }),
        ),
        (
            "scale",
            Cmd::View(pb::View {
                op: Some(Op::Scale(pb::Scale { factor: 2.0 })),
            }),
        ),
        (
            "zoom",
            Cmd::View(pb::View {
                op: Some(Op::Zoom(pb::Zoom { factor: 1.25 })),
            }),
        ),
        (
            "view_set",
            Cmd::View(pb::View {
                op: Some(Op::Set(pb::SetCamera {
                    azimuth: 45.0,
                    elevation: 20.0,
                    distance: 5.0,
                    fx: Some(1.0),
                    fy: Some(2.0),
                    fz: Some(3.0),
                })),
            }),
        ),
        (
            "view_set_no_focal",
            Cmd::View(pb::View {
                op: Some(Op::Set(pb::SetCamera {
                    azimuth: 10.0,
                    elevation: 0.0,
                    distance: 9.0,
                    fx: None,
                    fy: None,
                    fz: None,
                })),
            }),
        ),
        (
            "view_reset",
            Cmd::View(pb::View {
                op: Some(Op::Reset(true)),
            }),
        ),
        (
            "iso_levels",
            Cmd::Iso(pb::Isosurface {
                result: "sx".into(),
                on: true,
                levels: vec![1.0, 2.5, 3.0],
                count: 0,
                min: None,
                max: None,
            }),
        ),
        (
            "iso_count",
            Cmd::Iso(pb::Isosurface {
                result: "sy".into(),
                on: false,
                levels: vec![],
                count: 3,
                min: Some(0.0),
                max: Some(10.0),
            }),
        ),
        (
            "contour",
            Cmd::Contour(pb::Contour {
                result: "sz".into(),
                count: 8,
            }),
        ),
        (
            "material_on",
            Cmd::Material(pb::MaterialVisibility {
                enable: true,
                class_name: "brick".into(),
                material: Some(3),
            }),
        ),
        (
            "material_all",
            Cmd::Material(pb::MaterialVisibility {
                enable: false,
                class_name: String::new(),
                material: None,
            }),
        ),
        (
            "cutpln",
            Cmd::Cutplane(pb::CutPlane {
                ox: 0.0,
                oy: 1.0,
                oz: 2.0,
                nx: 1.0,
                ny: 0.0,
                nz: 0.0,
                relative: false,
                slice_only: None,
            }),
        ),
        (
            "cutrpln",
            Cmd::Cutplane(pb::CutPlane {
                ox: 0.0,
                oy: 0.0,
                oz: 0.0,
                nx: 0.0,
                ny: 0.0,
                nz: 1.0,
                relative: true,
                slice_only: None,
            }),
        ),
        ("cmap", Cmd::Colormap(pb::Colormap { name: "jet".into() })),
        (
            "legend",
            Cmd::Legend(pb::LegendLimits {
                min: Some(-1.0),
                max: Some(5.0),
            }),
        ),
        (
            "legend_auto",
            Cmd::Legend(pb::LegendLimits {
                min: None,
                max: None,
            }),
        ),
        (
            "nv_save",
            Cmd::NamedView(pb::NamedView {
                op: pb::named_view::Op::Save as i32,
                name: "front".into(),
            }),
        ),
        (
            "nv_restore",
            Cmd::NamedView(pb::NamedView {
                op: pb::named_view::Op::Restore as i32,
                name: "front".into(),
            }),
        ),
        (
            "nv_list",
            Cmd::NamedView(pb::NamedView {
                op: pb::named_view::Op::List as i32,
                name: String::new(),
            }),
        ),
        (
            "render",
            Cmd::Render(pb::Render {
                path: "/tmp/o.png".into(),
                width: 800,
                height: 600,
                states: vec![1, 2, 3],
                format: "mp4".into(),
            }),
        ),
    ];
    v.shrink_to_fit();
    v
}

// ── Handshake (match + deliberate version-bump mismatch, no panic) ──
#[tokio::test]
async fn handshake_match_and_mismatch() {
    let (mut client, _h) = spawn_in_process(VizService::builder().build())
        .await
        .unwrap();

    let ok = client
        .hello(Request::new(pb::HelloRequest {
            protocol_version: pb::PROTOCOL_VERSION.to_string(),
            session_token: String::new(),
            client_id: "test".into(),
            capabilities: vec![],
        }))
        .await
        .unwrap()
        .into_inner();
    assert!(ok.compatible);
    assert!(ok.mismatch_detail.is_empty());
    assert_eq!(ok.server_protocol_version, pb::PROTOCOL_VERSION);
    assert!(ok.session.is_some());

    // Deliberately bumped major: reported, never a panic, never Err.
    let bad = client
        .hello(Request::new(pb::HelloRequest {
            protocol_version: "99.0.0".into(),
            session_token: String::new(),
            client_id: "test".into(),
            capabilities: vec![],
        }))
        .await
        .unwrap()
        .into_inner();
    assert!(!bad.compatible);
    assert!(!bad.mismatch_detail.is_empty());

    // Empty version is also reported, not fatal.
    let empty = client
        .hello(Request::new(pb::HelloRequest::default()))
        .await
        .unwrap()
        .into_inner();
    assert!(!empty.compatible);
    assert!(!empty.mismatch_detail.is_empty());
}

// ── Capability (`agent` present/absent) ────────────────────────────
#[tokio::test]
async fn capability_agent_present_absent() {
    let (mut no_agent, _h1) = spawn_in_process(VizService::builder().build())
        .await
        .unwrap();
    let r = no_agent
        .hello(Request::new(pb::HelloRequest {
            protocol_version: pb::PROTOCOL_VERSION.into(),
            ..Default::default()
        }))
        .await
        .unwrap()
        .into_inner();
    assert!(!r.capabilities.iter().any(|c| c == pb::CAP_AGENT));

    let (mut with_agent, _h2) = spawn_in_process(VizService::builder().agent(true).build())
        .await
        .unwrap();
    let r = with_agent
        .hello(Request::new(pb::HelloRequest {
            protocol_version: pb::PROTOCOL_VERSION.into(),
            ..Default::default()
        }))
        .await
        .unwrap()
        .into_inner();
    assert!(r.capabilities.iter().any(|c| c == pb::CAP_AGENT));
}

// ── Layer-0 ≡ raw equivalence (the M1 form of a parity test) ───────
#[tokio::test]
async fn layer0_equals_raw() {
    for (name, cmd) in sample_commands() {
        // 1. The canonical line round-trips through the parser.
        let line = to_raw(&cmd);
        let reparsed =
            parse_line(&line).unwrap_or_else(|e| panic!("{name}: parse {line:?} failed: {e}"));
        assert_eq!(reparsed, cmd, "{name}: round-trip mismatch ({line:?})");

        // 2. Typed dispatch and `raw` dispatch of the equivalent
        //    line produce an identical StateDelta.
        let (mut c1, _h1) = spawn_in_process(VizService::builder().build())
            .await
            .unwrap();
        let (r_typed, d_typed) = exec_capture(&mut c1, cmd.clone()).await;

        let (mut c2, _h2) = spawn_in_process(VizService::builder().build())
            .await
            .unwrap();
        let (r_raw, d_raw) = exec_capture(&mut c2, pb::command::Cmd::Raw(line.clone())).await;

        assert_eq!(
            norm(d_typed),
            norm(d_raw),
            "{name}: delta differs typed vs raw"
        );
        assert_eq!(r_typed.delta_seq, r_raw.delta_seq, "{name}: seq differs");
    }
}

// ── Conformance: every Command arm → correct DeltaKind, geom empty ──
#[tokio::test]
async fn conformance_all_command_arms() {
    for (name, cmd) in sample_commands() {
        let (mut client, _h) = spawn_in_process(VizService::builder().build())
            .await
            .unwrap();
        let (reply, delta) = exec_capture(&mut client, cmd.clone()).await;

        let expect = command_delta_kind(&cmd);
        assert_eq!(delta.kind, expect as i32, "{name}: wrong DeltaKind");
        assert_eq!(delta.seq, reply.delta_seq, "{name}: seq != reply.delta_seq");

        // Geometry effects are stubbed until M2: GeometryRef empty.
        match delta.payload {
            Some(pb::state_delta::Payload::Result(r)) => assert!(r.geometry.is_none()),
            Some(pb::state_delta::Payload::Isosurface(i)) => assert!(i.geometry.is_none()),
            _ => {}
        }
    }

    // The 17th arm: `raw`. A raw line dispatches as its parsed
    // command's kind.
    let (mut client, _h) = spawn_in_process(VizService::builder().build())
        .await
        .unwrap();
    let (_r, d) = exec_capture(&mut client, pb::command::Cmd::Raw("state 12".into())).await;
    assert_eq!(d.kind, pb::DeltaKind::DeltaState as i32);
    assert!(matches!(
        d.payload,
        Some(pb::state_delta::Payload::State(12))
    ));
}

// ── Subscription fan-out (two clients, ordered, seq, late snapshot) ─
#[tokio::test]
async fn subscription_fanout() {
    let (mut client, _h) = spawn_in_process(VizService::builder().build())
        .await
        .unwrap();

    // Two in-process subscribers (distinct logical clients).
    let mut a = client
        .subscribe(Request::new(pb::SubscribeRequest::default()))
        .await
        .unwrap()
        .into_inner();
    let mut b = client
        .subscribe(Request::new(pb::SubscribeRequest::default()))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(
        a.message().await.unwrap().unwrap().kind,
        pb::DeltaKind::DeltaSnapshot as i32
    );
    assert_eq!(
        b.message().await.unwrap().unwrap().kind,
        pb::DeltaKind::DeltaSnapshot as i32
    );

    // A mutation from "alice" is observed by BOTH, ordered, with the
    // origin tagged and seq == CommandReply.delta_seq.
    let reply = client
        .execute(with_client_id(
            pb::Command {
                cmd: Some(pb::command::Cmd::SetState(pb::SetState { state: 5 })),
            },
            "alice",
        ))
        .await
        .unwrap()
        .into_inner();

    for sub in [&mut a, &mut b] {
        let d = sub.message().await.unwrap().unwrap();
        assert_eq!(d.kind, pb::DeltaKind::DeltaState as i32);
        assert_eq!(d.seq, reply.delta_seq);
        assert_eq!(d.origin_client_id, "alice");
        assert!(matches!(
            d.payload,
            Some(pb::state_delta::Payload::State(5))
        ));
    }

    // A late subscriber's stream opens with a DELTA_SNAPSHOT
    // reflecting the prior mutation, incl. the (empty in M1)
    // AgentTranscript field.
    let mut late = client
        .subscribe(Request::new(pb::SubscribeRequest::default()))
        .await
        .unwrap()
        .into_inner();
    let snap = late.message().await.unwrap().unwrap();
    assert_eq!(snap.kind, pb::DeltaKind::DeltaSnapshot as i32);
    let Some(pb::state_delta::Payload::Snapshot(s)) = snap.payload else {
        panic!("late subscriber did not open with a Snapshot");
    };
    assert_eq!(s.state, 5, "snapshot must reflect prior mutation");
    assert!(
        s.agent.is_some(),
        "Snapshot.agent (Δ8) present, empty in M1"
    );
    assert!(s.agent.unwrap().messages.is_empty());
}

// ── Agent surface — M1 frozen stubs lit up in M6 + message round-trip ─
//
// Originally `frozen_stubs_unimplemented` (Decision 7); Phase 5 M6
// (phase-5-m6.md Decisions 94–96) lights up the wire so the panel
// renders end-to-end. A deployment without a backend still rejects
// `agent_chat` cleanly (now `ok=false` with a clear error, not
// `Status::unimplemented`; the wire surface is implemented). The
// message-round-trip leg of the gate is unchanged.
#[tokio::test]
async fn frozen_stubs_unimplemented() {
    // Phase 5 M6 Decision 94 — agent(true) without a backend means
    // the capability advertises but the loop is not wired. The RPC
    // returns ok=false with a clear "no backend configured" error
    // instead of being silently unimplemented.
    let (mut no_backend, _h0) = spawn_in_process(VizService::builder().agent(true).build())
        .await
        .unwrap();
    let reply = no_backend
        .agent_chat(Request::new(pb::AgentChatRequest {
            text: "hi".into(),
            ..Default::default()
        }))
        .await
        .expect("agent_chat is implemented; the deployment policy is the failure")
        .into_inner();
    assert!(!reply.ok);
    assert!(
        reply.error.to_lowercase().contains("backend"),
        "no-backend error names the missing piece: {}",
        reply.error
    );

    // Phase 5 M6 Decision 94 — wire in MockAgent and the same call
    // succeeds with a turn_id the panel can later cancel.
    let (mut client, _h) = spawn_in_process(
        VizService::builder()
            .agent_backend(mili_viz_server::MockAgent)
            .build(),
    )
    .await
    .unwrap();
    let reply = client
        .agent_chat(Request::new(pb::AgentChatRequest {
            text: "hi".into(),
            ..Default::default()
        }))
        .await
        .unwrap()
        .into_inner();
    assert!(reply.ok, "MockAgent ⇒ AgentChat lit up: {}", reply.error);
    assert!(!reply.turn_id.is_empty(), "turn_id allocated for Interrupt");

    // Phase 5 M6 Decision 98 — Interrupt is always ok (server treats
    // "stop whatever is happening" as a never-fail intent). Empty id =
    // current turn per the frozen-proto convention.
    let reply = client
        .interrupt(Request::new(pb::InterruptRequest::default()))
        .await
        .unwrap()
        .into_inner();
    assert!(reply.ok);

    // Phase 5 M6 Decision 96 — CaptureFrame returns deterministic
    // placeholder bytes of the requested extent / format. The
    // production server-side wgpu offscreen swap is a separate
    // milestone.
    let reply = client
        .capture_frame(Request::new(pb::FrameRequest {
            width: 16,
            height: 16,
            format: "png".into(),
        }))
        .await
        .unwrap()
        .into_inner();
    assert!(reply.ok, "CaptureFrame lit up");
    assert_eq!(reply.width, 16);
    assert_eq!(reply.height, 16);
    assert_eq!(reply.format, "png");
    assert!(!reply.image.is_empty(), "placeholder PNG is non-empty");

    // The frozen messages compile and round-trip on the wire.
    let ev = pb::StateDelta {
        seq: 9,
        origin_client_id: "agent".into(),
        kind: pb::DeltaKind::DeltaAgent as i32,
        payload: Some(pb::state_delta::Payload::Agent(pb::AgentEvent {
            turn_id: "t1".into(),
            ev: Some(pb::agent_event::Ev::Token(pb::AgentToken {
                text: "hello".into(),
            })),
        })),
    };
    let bytes = ev.encode_to_vec();
    assert_eq!(pb::StateDelta::decode(&bytes[..]).unwrap(), ev);
}
