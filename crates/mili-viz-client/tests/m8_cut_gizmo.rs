//! Phase 5 M8 gating test — cut-plane gizmo + Rendering → Cut UI.
//!
//! Sibling to `m7_render_modes.rs` (the M7 client gate). Always-on
//! halves pin the pure logic Decision 84/85/86 demands:
//!
//!   * gizmo state transitions: `set_cut_plane` / `preview_cut_plane` /
//!     `clear_cut` / `set_cut_gizmo_visible` mutate [`ShellState`]
//!     deterministically and return the matching `UiAction`s for the
//!     windowed app to lower;
//!   * 30 Hz wall-clock throttle: [`CutThrottle::try_preview`] passes
//!     once, blocks subsequent calls within the [`CUT_PREVIEW_INTERVAL`]
//!     window, and re-arms after the window elapses or [`CutThrottle::
//!     reset`] (the drag-end pattern);
//!   * `Cmd::Cutplane` lowering: [`cutplane_cmd`] copies the gizmo's
//!     origin / normal onto the frozen proto fields and leaves
//!     `relative` / `slice_only` at the proto3 defaults so the server's
//!     `phase-4-m8.md` `Plane::from_proto` (and the future
//!     `phase-4-m9.md` slice toggle) read them correctly;
//!   * cross-session persistence: the `interactive_clip` field
//!     round-trips through [`PersistedTweaks`], the absent-file path
//!     keeps the default `true`, and [`is_persisted_action`] now
//!     classifies the new `SetInteractiveClip` action so the windowed
//!     app re-writes `tweaks.json` on toggle.
//!
//! Skip-on-absent composite leg renders the L1 shell with the gizmo
//! turned on against `basic1.pltA` (the M8-server gating corpus) via
//! the in-process `fetch_server_mesh` seam; mirrors `phase-5-m8.md`
//! Decision 84's "the gizmo is egui shapes only" promise — the chrome
//! still composites over the byte-stable mesh pass.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use mili_viz_client::{
    build_shell_ui, cutplane_cmd, fetch_server_mesh, is_persisted_action, render_shell_to_image,
    Camera, CutPlaneState, CutThrottle, LoadedInfo, PersistedTweaks, SessionPhase, ShellState,
    UiAction, CUT_PREVIEW_INTERVAL,
};
use mili_viz_proto::v1 as pb;

fn corpus_path(rel: &[&str]) -> PathBuf {
    let mut p = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("reference")
        .join("mili-python")
        .join("tests")
        .join("data");
    for c in rel {
        p = p.join(c);
    }
    p
}

fn paint(state: &mut ShellState) -> (Vec<UiAction>, bool) {
    let ctx = egui::Context::default();
    let raw = egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(
            egui::Pos2::ZERO,
            egui::vec2(1000.0, 700.0),
        )),
        ..Default::default()
    };
    let mut actions = Vec::new();
    let out = ctx.run_ui(raw, |ui| actions = build_shell_ui(ui, state));
    (actions, !out.shapes.is_empty())
}

#[test]
fn defaults_are_the_byte_stable_m7_polish_values() {
    let s = ShellState::default();
    assert!(s.cut_plane.is_none(), "no cut active by default (VB-001)");
    assert!(
        !s.cut_gizmo_visible,
        "gizmo hidden by default — opt-in via Rendering → Cut"
    );
    assert!(
        s.interactive_clip,
        "interactive clip defaults on (griz cutpln live feel)"
    );
}

#[test]
fn set_cut_plane_mutates_and_returns_commit_action() {
    let mut s = ShellState::default();
    let plane = CutPlaneState {
        origin: [1.0, 2.0, 3.0],
        normal: [0.0, 1.0, 0.0],
    };
    let a = s.set_cut_plane(plane);
    assert_eq!(s.cut_plane, Some(plane), "state mutated");
    assert_eq!(a, UiAction::SetCutPlane(plane), "canonical commit action");
}

#[test]
fn preview_cut_plane_mutates_and_returns_preview_action() {
    let mut s = ShellState::default();
    let plane = CutPlaneState {
        origin: [0.5, 0.0, 0.0],
        normal: [1.0, 0.0, 0.0],
    };
    let a = s.preview_cut_plane(plane);
    assert_eq!(s.cut_plane, Some(plane));
    assert_eq!(
        a,
        UiAction::PreviewCutPlane(plane),
        "throttle-eligible action shape (windowed app gates the emit)"
    );
}

#[test]
fn clear_cut_drops_the_plane_and_emits_clear() {
    let mut s = ShellState::default();
    let _ = s.set_cut_plane(CutPlaneState {
        origin: [0.0; 3],
        normal: [1.0, 0.0, 0.0],
    });
    assert!(s.cut_plane.is_some());
    let a = s.clear_cut();
    assert!(s.cut_plane.is_none(), "state cleared");
    assert_eq!(a, UiAction::ClearCut);
}

#[test]
fn gizmo_visibility_and_interactive_clip_are_pure_toggles() {
    let mut s = ShellState::default();
    let a = s.set_cut_gizmo_visible(true);
    assert!(s.cut_gizmo_visible);
    assert_eq!(a, UiAction::SetCutGizmoVisible(true));

    let a = s.set_interactive_clip(false);
    assert!(!s.interactive_clip);
    assert_eq!(a, UiAction::SetInteractiveClip(false));
}

#[test]
fn seed_from_aabb_centres_origin_and_uses_view_normal() {
    // AABB `[0, 0, 0] .. [2, 4, 6]` → centre `[1, 2, 3]`.
    let aabb = ([0.0, 0.0, 0.0], [2.0, 4.0, 6.0]);
    let camera = Camera::looking_at(glam::Vec3::new(1.0, 2.0, 3.0), 4.0);
    let plane = CutPlaneState::from_aabb_and_camera(aabb, &camera);
    assert!(
        (plane.origin[0] - 1.0).abs() < 1e-6
            && (plane.origin[1] - 2.0).abs() < 1e-6
            && (plane.origin[2] - 3.0).abs() < 1e-6,
        "centre of the AABB: {:?}",
        plane.origin
    );

    // The default-orbit camera looks down -Z (azimuth 0, elevation 0)
    // so its view normal is the -Z axis. `from_aabb_and_camera` uses
    // `basis().forward`, which is the eye→focus direction — also -Z
    // for the default orbit. Either way, the chosen normal should be
    // non-zero and primarily along Z.
    let n = plane.normal;
    let mag = (n[0].powi(2) + n[1].powi(2) + n[2].powi(2)).sqrt();
    assert!(mag > 0.5, "normal must be non-trivial: {n:?}");
    assert!(
        n[2].abs() > n[0].abs() && n[2].abs() > n[1].abs(),
        "default orbit's view normal lies along Z: {n:?}"
    );
}

#[test]
fn throttle_first_call_passes_and_subsequent_within_window_blocks() {
    let mut th = CutThrottle::new();
    let t0 = Instant::now();
    assert!(th.try_preview(t0), "first preview must fire");
    // Same tick — blocked.
    assert!(!th.try_preview(t0), "back-to-back within 0 ms blocked");
    // 10 ms later — still inside the 33 ms window.
    assert!(
        !th.try_preview(t0 + Duration::from_millis(10)),
        "+10 ms still inside the window"
    );
    // 33 ms later — boundary opens (the constant is the contract).
    assert!(
        th.try_preview(t0 + CUT_PREVIEW_INTERVAL),
        "at +33 ms the window re-opens"
    );
}

#[test]
fn throttle_blocks_60hz_into_30hz() {
    // A 60 Hz frame loop (~16.7 ms) must not emit twice per ~33 ms
    // window — exactly one of every two attempts should pass.
    let mut th = CutThrottle::new();
    let t0 = Instant::now();
    let frame = Duration::from_micros(16_667);
    let mut passes = 0;
    for i in 0..10 {
        if th.try_preview(t0 + frame * i) {
            passes += 1;
        }
    }
    // 10 frames at 16.67 ms = 166.7 ms wall-clock → 5 windows of 33 ms.
    // The first call always passes, then every second frame thereafter:
    // expect ~5 emits.
    assert!(
        (4..=6).contains(&passes),
        "60 Hz over 10 frames must yield ~5 emits (got {passes})"
    );
}

#[test]
fn throttle_reset_re_arms_for_drag_end_commit() {
    let mut th = CutThrottle::new();
    let t0 = Instant::now();
    assert!(th.try_preview(t0));
    assert!(!th.try_preview(t0), "blocked");
    th.reset();
    assert!(
        th.try_preview(t0),
        "drag-end / clear / explicit commit re-arms the throttle"
    );
}

#[test]
fn lowering_copies_origin_normal_and_keeps_proto3_defaults() {
    let plane = CutPlaneState {
        origin: [1.5, -2.0, 3.25],
        normal: [0.0, 0.0, 1.0],
    };
    let pb::command::Cmd::Cutplane(p) = cutplane_cmd(plane) else {
        panic!("must lower to Cmd::Cutplane");
    };
    assert!((p.ox - 1.5).abs() < 1e-6);
    assert!((p.oy - -2.0).abs() < 1e-6);
    assert!((p.oz - 3.25).abs() < 1e-6);
    assert!((p.nx - 0.0).abs() < 1e-6);
    assert!((p.ny - 0.0).abs() < 1e-6);
    assert!((p.nz - 1.0).abs() < 1e-6);
    // Proto3 defaults for `relative` (false) and `slice_only` (unset)
    // — the server's `Plane::from_proto` reads `relative` and the
    // future M9 reads `slice_only` (none of M8's job is to set them).
    assert!(!p.relative, "absolute plane");
    assert!(p.slice_only.is_none(), "M9 territory, untouched at M8");
}

#[test]
fn clear_lowering_is_a_zero_normal_default_cutplane() {
    // ClearCut in app.rs lowers to `pb::CutPlane::default()` — a zero
    // normal that the server (`phase-4-m8.md`) treats as a clear. Pin
    // the bit pattern so a future refactor cannot accidentally emit a
    // non-clearing plane on the "Clear cut" menu row.
    let clear = pb::CutPlane::default();
    assert!(clear.nx == 0.0 && clear.ny == 0.0 && clear.nz == 0.0);
    assert!(!clear.relative);
    assert!(clear.slice_only.is_none());
}

#[test]
fn interactive_clip_persists_through_tweaks_round_trip() {
    let mut s = ShellState::default();
    let _ = s.set_interactive_clip(false);
    let snap = PersistedTweaks::from_state(&s);
    let json = snap.to_json();
    let back = PersistedTweaks::from_json(&json).expect("round-trips");
    assert_eq!(back, snap, "interactive_clip survives the JSON trip");

    let mut s2 = ShellState::default();
    assert!(s2.interactive_clip, "default before apply");
    back.apply_to(&mut s2);
    assert!(
        !s2.interactive_clip,
        "apply_to restores the persisted value"
    );
}

#[test]
fn absent_tweaks_file_keeps_interactive_clip_default_on() {
    // Fresh-machine path: no config file → default → applying it to a
    // default shell leaves `interactive_clip == true` (the byte-stable
    // VB-001 default; griz cutpln live feel).
    let p = std::env::temp_dir()
        .join(format!("mili-viz-absent-m8-{}", std::process::id()))
        .join("tweaks.json");
    let loaded = PersistedTweaks::load_from(&p);
    assert_eq!(loaded, PersistedTweaks::default());
    let mut s = ShellState::default();
    loaded.apply_to(&mut s);
    assert!(s.interactive_clip, "default is on after applying absent");
}

#[test]
fn is_persisted_action_classifies_interactive_clip_toggle() {
    assert!(
        is_persisted_action(&UiAction::SetInteractiveClip(true)),
        "the windowed app must rewrite tweaks.json on this toggle"
    );
    assert!(
        is_persisted_action(&UiAction::SetInteractiveClip(false)),
        "off-toggle also persists"
    );
    // Negative: the cut-plane commits / previews are NOT persisted.
    // They are transport events, not preferences.
    let plane = CutPlaneState {
        origin: [0.0; 3],
        normal: [1.0, 0.0, 0.0],
    };
    assert!(!is_persisted_action(&UiAction::SetCutPlane(plane)));
    assert!(!is_persisted_action(&UiAction::PreviewCutPlane(plane)));
    assert!(!is_persisted_action(&UiAction::ClearCut));
    assert!(!is_persisted_action(&UiAction::SetCutGizmoVisible(true)));
}

#[test]
fn shell_paints_input_free_with_gizmo_on_and_cut_active() {
    // The pure paint must succeed (no panics, shapes emitted) when the
    // gizmo is visible and a plane is set — pins the egui-shapes-only
    // overlay path (Decision 84) is reachable through `build_shell_ui`.
    let mut s = ShellState {
        phase: SessionPhase::AttachedIdle,
        camera: Some(Camera::looking_at(glam::Vec3::ZERO, 2.0)),
        cut_plane: Some(CutPlaneState {
            origin: [0.0, 0.0, 0.0],
            normal: [0.0, 0.0, 1.0],
        }),
        cut_gizmo_visible: true,
        ..ShellState::default()
    };
    let (actions, painted) = paint(&mut s);
    assert!(painted, "shell must paint with the gizmo on");
    assert!(
        actions.is_empty(),
        "no input ⇒ no actions emitted: {actions:?}"
    );
}

#[tokio::test]
async fn composite_render_with_gizmo() {
    let path = corpus_path(&["serial", "basic1", "basic1.pltA"]);
    if !path.exists() {
        eprintln!("skip: serial/basic1 absent (run scripts/setup-parity.sh)");
        return;
    }

    let mesh = fetch_server_mesh(&path.to_string_lossy(), "")
        .await
        .expect("in-process load/show yields a decoded hull");

    let (w, h) = (240u32, 240u32);
    let (center, radius) = mesh.bounds();
    let camera = Camera::looking_at(center, radius);

    let base = ShellState {
        phase: SessionPhase::AttachedIdle,
        loaded: Some(LoadedInfo {
            db: path.to_string_lossy().into_owned(),
            num_states: 1,
            state_times: vec![0.0],
            class_names: vec!["brick".into()],
        }),
        camera: Some(camera),
        ..ShellState::default()
    };

    // (a) Baseline: gizmo off / no cut. The chrome composites over the
    // unchanged mesh pass (VB-001 — same expectation as
    // `preferences_tweaks.rs`'s dark default leg).
    let mut off = base.clone();
    let Some(off_px) = render_shell_to_image(w, h, &camera, &mesh, None, &mut off) else {
        eprintln!(
            "skip: no wgpu adapter (no GPU / software rasterizer) — \
             skip-on-absent per CLAUDE.md / phase-5-m3.md Decision 46"
        );
        return;
    };
    assert_eq!(off_px.len() as u32, w * h * 4);

    // (b) Gizmo on + plane seeded at the AABB centre. Same mesh, same
    // camera; the egui pass paints the gizmo handles, so some pixels
    // must change vs. the off baseline.
    let mut on = base;
    let plane = CutPlaneState::from_aabb_and_camera(mesh.aabb(), &camera);
    on.cut_plane = Some(plane);
    on.cut_gizmo_visible = true;
    let on_px = render_shell_to_image(w, h, &camera, &mesh, None, &mut on)
        .expect("adapter was present for render (a)");
    assert_ne!(
        on_px, off_px,
        "the gizmo overlay must paint additional egui shapes"
    );
}
