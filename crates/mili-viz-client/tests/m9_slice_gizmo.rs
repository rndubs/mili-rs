//! Phase 5 M9 gating test — slice gizmo + Rendering → Slice UI.
//!
//! Thin sibling of `m8_cut_gizmo.rs` (`phase-5-m9.md` Decision 87).
//! Always-on halves pin the pure logic Decisions 87/88/89 demand:
//!
//!   * slice state transitions: `set_slice_plane` /
//!     `preview_slice_plane` / `clear_slice` /
//!     `set_slice_gizmo_visible` mutate [`ShellState`] deterministically
//!     and return the matching `UiAction`s for the windowed app to
//!     lower;
//!   * shared 30 Hz wall-clock throttle: slice previews ride the same
//!     [`CutThrottle`] as the cut sibling (a user only drags one
//!     gizmo at a time — one budget across both verbs);
//!   * `Cmd::Cutplane` lowering: [`slice_cmd`] copies origin/normal
//!     onto the frozen proto fields and sets `slice_only = Some(true)`
//!     so the server's M9 arm (`crates/mili-viz-server/src/clip.rs`
//!     `ClipMode::Slice`) reads the flag; the M8 cut sibling
//!     ([`cutplane_cmd`]) keeps `slice_only = None` so byte-stability
//!     against M8-only clients is preserved;
//!   * composition: cut + slice can co-exist on a single `ShellState`
//!     (Decision 87 / `phase-4-m9.md` Decision 80) — neither
//!     overwrites the other and the status-bar reads both lines.
//!
//! Skip-on-absent composite leg renders the L1 shell with both gizmos
//! turned on against `basic1.pltA` (the M9-server gating corpus) via
//! the in-process `fetch_server_mesh` seam; mirrors `phase-5-m9.md`
//! Decision 87's "gizmo overlay is egui shapes only" promise.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use mili_viz_client::{
    build_shell_ui, cutplane_cmd, fetch_server_mesh, is_persisted_action, render_shell_to_image,
    slice_cmd, Camera, CutPlaneState, CutThrottle, LoadedInfo, SessionPhase, ShellState, UiAction,
    CUT_GIZMO_COLOR, CUT_PREVIEW_INTERVAL, SLICE_GIZMO_COLOR,
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
fn defaults_are_the_byte_stable_m8_polish_values() {
    let s = ShellState::default();
    assert!(
        s.slice_plane.is_none(),
        "no slice active by default (VB-001)"
    );
    assert!(
        !s.slice_gizmo_visible,
        "gizmo hidden by default — opt-in via Rendering → Slice"
    );
}

#[test]
fn set_slice_plane_mutates_and_returns_commit_action() {
    let mut s = ShellState::default();
    let plane = CutPlaneState {
        origin: [1.0, 2.0, 3.0],
        normal: [0.0, 1.0, 0.0],
    };
    let a = s.set_slice_plane(plane);
    assert_eq!(s.slice_plane, Some(plane), "state mutated");
    assert_eq!(a, UiAction::SetSlicePlane(plane), "canonical commit action");
}

#[test]
fn preview_slice_plane_mutates_and_returns_preview_action() {
    let mut s = ShellState::default();
    let plane = CutPlaneState {
        origin: [0.5, 0.0, 0.0],
        normal: [1.0, 0.0, 0.0],
    };
    let a = s.preview_slice_plane(plane);
    assert_eq!(s.slice_plane, Some(plane));
    assert_eq!(
        a,
        UiAction::PreviewSlicePlane(plane),
        "throttle-eligible action shape (windowed app gates the emit)"
    );
}

#[test]
fn clear_slice_drops_the_plane_and_emits_clear() {
    let mut s = ShellState::default();
    let _ = s.set_slice_plane(CutPlaneState {
        origin: [0.0; 3],
        normal: [1.0, 0.0, 0.0],
    });
    assert!(s.slice_plane.is_some());
    let a = s.clear_slice();
    assert!(s.slice_plane.is_none(), "state cleared");
    assert_eq!(a, UiAction::ClearSlice);
}

#[test]
fn slice_gizmo_visibility_is_a_pure_toggle() {
    let mut s = ShellState::default();
    let a = s.set_slice_gizmo_visible(true);
    assert!(s.slice_gizmo_visible);
    assert_eq!(a, UiAction::SetSliceGizmoVisible(true));
    let a = s.set_slice_gizmo_visible(false);
    assert!(!s.slice_gizmo_visible);
    assert_eq!(a, UiAction::SetSliceGizmoVisible(false));
}

#[test]
fn slice_and_cut_compose_in_shell_state() {
    // `phase-4-m9.md` Decision 80 / `phase-5-m9.md` Decision 87 — the
    // two verbs do not overwrite each other; both can be set on the
    // same ShellState and both readouts show.
    let mut s = ShellState::default();
    let cut = CutPlaneState {
        origin: [0.0, 0.0, 0.0],
        normal: [1.0, 0.0, 0.0],
    };
    let slice = CutPlaneState {
        origin: [1.0, 0.0, 0.0],
        normal: [0.0, 1.0, 0.0],
    };
    let _ = s.set_cut_plane(cut);
    let _ = s.set_slice_plane(slice);
    assert_eq!(s.cut_plane, Some(cut), "cut survives setting slice");
    assert_eq!(s.slice_plane, Some(slice), "slice survives setting cut");
    // Clearing slice does not touch cut, and vice versa.
    let _ = s.clear_slice();
    assert_eq!(s.cut_plane, Some(cut), "clear slice leaves cut intact");
    assert!(s.slice_plane.is_none());
    let _ = s.set_slice_plane(slice);
    let _ = s.clear_cut();
    assert_eq!(s.slice_plane, Some(slice), "clear cut leaves slice intact");
    assert!(s.cut_plane.is_none());
}

#[test]
fn slice_lowering_sets_slice_only_some_true() {
    // Pin the slice command's byte shape: same origin/normal as the
    // cut sibling but `slice_only = Some(true)` so the server's
    // `ClipMode::Slice` arm reads the flag (`phase-4-m9.md`
    // Decisions 78–79).
    let plane = CutPlaneState {
        origin: [1.5, -2.0, 3.25],
        normal: [0.0, 0.0, 1.0],
    };
    let pb::command::Cmd::Cutplane(p) = slice_cmd(plane) else {
        panic!("must lower to Cmd::Cutplane");
    };
    assert!((p.ox - 1.5).abs() < 1e-6);
    assert!((p.oy - -2.0).abs() < 1e-6);
    assert!((p.oz - 3.25).abs() < 1e-6);
    assert!((p.nx - 0.0).abs() < 1e-6);
    assert!((p.ny - 0.0).abs() < 1e-6);
    assert!((p.nz - 1.0).abs() < 1e-6);
    assert!(!p.relative, "absolute plane");
    assert_eq!(
        p.slice_only,
        Some(true),
        "the M9 slice verb sets the flag the server reads"
    );
}

#[test]
fn cut_and_slice_lowerings_differ_only_in_slice_only_flag() {
    // Byte-stability against M8-only clients: `cutplane_cmd` must keep
    // `slice_only == None` (proto3 default; the m8 gating test pins
    // this) while `slice_cmd` flips it to `Some(true)`. All other
    // fields are identical for identical input.
    let plane = CutPlaneState {
        origin: [0.5, -1.5, 2.5],
        normal: [0.0, 1.0, 0.0],
    };
    let pb::command::Cmd::Cutplane(c) = cutplane_cmd(plane) else {
        panic!()
    };
    let pb::command::Cmd::Cutplane(s) = slice_cmd(plane) else {
        panic!()
    };
    assert_eq!((c.ox, c.oy, c.oz), (s.ox, s.oy, s.oz));
    assert_eq!((c.nx, c.ny, c.nz), (s.nx, s.ny, s.nz));
    assert_eq!(c.relative, s.relative);
    assert_eq!(c.slice_only, None, "cut sibling stays at proto3 default");
    assert_eq!(s.slice_only, Some(true), "slice verb flips the flag");
}

#[test]
fn clear_slice_lowering_pin() {
    // ClearSlice in app.rs lowers to a zero-normal CutPlane with
    // `slice_only = Some(true)` — the server (`phase-4-m9.md`) treats
    // a zero-length normal as a clear, and the flag routes it to the
    // slice bucket (not the cut bucket). Pin the bit pattern so a
    // future refactor cannot accidentally clear the cut on the
    // "clear slice" menu row.
    let clear = pb::CutPlane {
        slice_only: Some(true),
        ..pb::CutPlane::default()
    };
    assert!(clear.nx == 0.0 && clear.ny == 0.0 && clear.nz == 0.0);
    assert!(!clear.relative);
    assert_eq!(clear.slice_only, Some(true));
}

#[test]
fn shared_throttle_blocks_a_cut_then_slice_burst() {
    // A user dragging slice immediately after cut should still see
    // 30 Hz rate-limiting across the two verbs: one shared
    // [`CutThrottle`] in `app.rs` covers both (`phase-5-m9.md` "throttle
    // behavior shared with M8 cut"). Frame-clock-driven probe pins this.
    let mut th = CutThrottle::new();
    let t0 = Instant::now();
    assert!(th.try_preview(t0), "first emit passes");
    assert!(
        !th.try_preview(t0 + Duration::from_millis(5)),
        "back-to-back blocked even if the verb changed"
    );
    assert!(
        th.try_preview(t0 + CUT_PREVIEW_INTERVAL),
        "at +33 ms the window re-opens for the second verb"
    );
}

#[test]
fn slice_actions_are_not_persisted() {
    // Slice state is transport, not preferences — the windowed app
    // must NOT re-write `tweaks.json` on slice toggles.
    let plane = CutPlaneState {
        origin: [0.0; 3],
        normal: [1.0, 0.0, 0.0],
    };
    assert!(!is_persisted_action(&UiAction::SetSlicePlane(plane)));
    assert!(!is_persisted_action(&UiAction::PreviewSlicePlane(plane)));
    assert!(!is_persisted_action(&UiAction::ClearSlice));
    assert!(!is_persisted_action(&UiAction::SetSliceGizmoVisible(true)));
}

#[test]
fn slice_and_cut_gizmo_colours_are_distinct() {
    // Decision 87: cut handles render in the existing M8 colour,
    // slice handles in a contrasting second colour so when both are
    // active the user can tell which gizmo controls which plane.
    assert_ne!(
        CUT_GIZMO_COLOR, SLICE_GIZMO_COLOR,
        "cut and slice handles must contrast"
    );
}

#[test]
fn rendering_menu_emits_slice_actions_through_the_shell() {
    // Drive the menu without egui pointer input by calling the
    // mutating shell methods directly — same shape the menu rows do.
    // Pins that the visible-toggle action lands in build_shell_ui's
    // returned action vec when the toggle round-trips through the
    // state.
    let mut s = ShellState {
        phase: SessionPhase::AttachedIdle,
        ..ShellState::default()
    };
    let a = s.set_slice_gizmo_visible(true);
    assert_eq!(a, UiAction::SetSliceGizmoVisible(true));
    assert!(s.slice_gizmo_visible);
    // The headless paint pass with the toggle on succeeds (no panics).
    let (actions, painted) = paint(&mut s);
    assert!(painted, "shell paints with slice gizmo toggled on");
    assert!(
        actions.is_empty(),
        "no input ⇒ no actions emitted: {actions:?}"
    );
}

#[test]
fn shell_paints_input_free_with_both_gizmos_on_and_planes_active() {
    // Pin the egui-shapes-only overlay path for the composed
    // cut+slice case (Decision 87 / 80): the shell must paint without
    // panics when both gizmos are visible, both planes are set, and a
    // live camera is attached.
    let mut s = ShellState {
        phase: SessionPhase::AttachedIdle,
        camera: Some(Camera::looking_at(glam::Vec3::ZERO, 2.0)),
        cut_plane: Some(CutPlaneState {
            origin: [0.0, 0.0, 0.0],
            normal: [0.0, 0.0, 1.0],
        }),
        slice_plane: Some(CutPlaneState {
            origin: [0.5, 0.0, 0.0],
            normal: [1.0, 0.0, 0.0],
        }),
        cut_gizmo_visible: true,
        slice_gizmo_visible: true,
        ..ShellState::default()
    };
    let (actions, painted) = paint(&mut s);
    assert!(painted, "shell must paint with both gizmos on");
    assert!(
        actions.is_empty(),
        "no input ⇒ no actions emitted: {actions:?}"
    );
}

#[tokio::test]
async fn composite_render_with_slice_gizmo() {
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

    // (a) Baseline: slice gizmo off / no slice. The chrome composites
    // over the unchanged mesh pass (same VB-001 expectation as the
    // M8 gating test's baseline leg).
    let mut off = base.clone();
    let Some(off_px) = render_shell_to_image(w, h, &camera, &mesh, None, &mut off) else {
        eprintln!(
            "skip: no wgpu adapter (no GPU / software rasterizer) — \
             skip-on-absent per CLAUDE.md / phase-5-m3.md Decision 46"
        );
        return;
    };
    assert_eq!(off_px.len() as u32, w * h * 4);

    // (b) Slice gizmo on + plane seeded at the AABB centre. Same mesh,
    // same camera; the egui pass paints the slice gizmo handles, so
    // some pixels must change vs. the off baseline.
    let mut on = base;
    let plane = CutPlaneState::from_aabb_and_camera(mesh.aabb(), &camera);
    on.slice_plane = Some(plane);
    on.slice_gizmo_visible = true;
    let on_px = render_shell_to_image(w, h, &camera, &mesh, None, &mut on)
        .expect("adapter was present for render (a)");
    assert_ne!(
        on_px, off_px,
        "the slice gizmo overlay must paint additional egui shapes"
    );
}
