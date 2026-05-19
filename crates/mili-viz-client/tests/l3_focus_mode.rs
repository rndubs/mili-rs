//! L3 focus-mode (`Ctrl+\`) gating test (`wireframe-parity.md`
//! "Window shape & layout" L3 row; wireframes §"L3 — Focus mode").
//!
//! Two halves, this crate's `dock_rail.rs` / `preferences_tweaks.rs`
//! shape:
//!  * always-on — the pure, GPU-free wiring: `set_focus_mode` is a
//!    pure, observable switch that round-trips the dock-collapse with
//!    it; a real `Ctrl+\` key event toggles it (and *only* a key event
//!    does — the "no input ⇒ no actions" invariant still holds without
//!    it); entering focus hides the AI rail and bottom tabs, measurably
//!    enlarging the published `scene_frac` (a deterministic no-GPU
//!    check). The default (focus off) is the byte-stable L1 chrome.
//!  * `composite_render` — headless: the default seam is unperturbed
//!    (`bug-tracker.md` VB-001) and a focus-mode render still
//!    composites the mesh while the AI-rail and bottom-strip chrome is
//!    gone. **Skip-on-absent** when the corpus / a `wgpu` adapter is
//!    missing (CLAUDE.md convention; not a failure).

use std::path::{Path, PathBuf};

use mili_viz_client::{
    build_shell_ui, fetch_server_mesh, render_shell_to_image, Camera, LoadedInfo, SessionPhase,
    ShellState, UiAction,
};

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

fn raw(events: Vec<egui::Event>) -> egui::RawInput {
    // egui's `InputState.modifiers` comes from `RawInput.modifiers`
    // (egui-winit keeps it in sync in the windowed app), not from the
    // event itself — mirror that so the `Ctrl+\` shortcut resolves.
    let modifiers = events
        .iter()
        .find_map(|e| match e {
            egui::Event::Key { modifiers, .. } => Some(*modifiers),
            _ => None,
        })
        .unwrap_or_default();
    egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(
            egui::Pos2::ZERO,
            egui::vec2(1000.0, 700.0),
        )),
        modifiers,
        events,
        ..Default::default()
    }
}

fn ctrl_backslash() -> egui::Event {
    egui::Event::Key {
        key: egui::Key::Backslash,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: egui::Modifiers {
            ctrl: true,
            ..Default::default()
        },
    }
}

fn run(state: &mut ShellState, events: Vec<egui::Event>) -> (Vec<UiAction>, bool) {
    let ctx = egui::Context::default();
    let mut actions = Vec::new();
    let out = ctx.run_ui(raw(events), |ui| actions = build_shell_ui(ui, state));
    (actions, !out.shapes.is_empty())
}

#[test]
fn set_focus_mode_is_pure_and_round_trips_the_dock() {
    let mut s = ShellState::default();
    assert!(!s.focus_mode && !s.dock_collapsed, "default is full L1");

    let a = s.set_focus_mode(true);
    assert_eq!(a, UiAction::SetFocusMode(true));
    assert!(s.focus_mode, "focus on");
    assert!(s.dock_collapsed, "entering focus collapses the dock");

    let a = s.set_focus_mode(false);
    assert_eq!(a, UiAction::SetFocusMode(false));
    assert!(!s.focus_mode && !s.dock_collapsed, "exit restores full L1");
}

#[test]
fn ctrl_backslash_toggles_focus_and_no_key_is_inert() {
    let mut s = ShellState {
        phase: SessionPhase::AttachedIdle,
        ..ShellState::default()
    };

    // No key ⇒ no actions (the pure-shell invariant is preserved).
    let (a, painted) = run(&mut s, vec![]);
    assert!(painted && a.is_empty(), "no input ⇒ no actions: {a:?}");
    assert!(!s.focus_mode);

    // Ctrl+\ enters focus mode.
    let (a, _) = run(&mut s, vec![ctrl_backslash()]);
    assert_eq!(a, vec![UiAction::SetFocusMode(true)]);
    assert!(s.focus_mode && s.dock_collapsed);

    // Ctrl+\ again exits it (full L1 restored).
    let (a, _) = run(&mut s, vec![ctrl_backslash()]);
    assert_eq!(a, vec![UiAction::SetFocusMode(false)]);
    assert!(!s.focus_mode && !s.dock_collapsed);
}

#[test]
fn focus_mode_hides_ai_and_tabs_and_enlarges_the_scene() {
    let mut l1 = ShellState {
        phase: SessionPhase::AttachedIdle,
        ..ShellState::default()
    };
    run(&mut l1, vec![]);

    let mut l3 = ShellState {
        phase: SessionPhase::AttachedIdle,
        focus_mode: true,
        dock_collapsed: true,
        ..ShellState::default()
    };
    let (a, painted) = run(&mut l3, vec![]);
    assert!(painted, "the stripped viewport must still paint");
    assert!(a.is_empty(), "no pointer input ⇒ no actions: {a:?}");

    let s1 = l1.scene_frac.expect("L1 scene measured");
    let s3 = l3.scene_frac.expect("L3 scene measured");
    // AI rail (28 px) + bottom strip (22 px) reclaimed ⇒ a wider and
    // taller central viewport fraction.
    assert!(s3[2] > s1[2], "focus widens the scene: {s3:?} vs {s1:?}");
    assert!(s3[3] > s1[3], "focus heightens the scene: {s3:?} vs {s1:?}");
}

#[tokio::test]
async fn composite_render() {
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
        ..ShellState::default()
    };

    let at = |px: &[u8], x: u32, y: u32| {
        let i = ((y * w + x) * 4) as usize;
        [px[i], px[i + 1], px[i + 2]]
    };

    // (a) Default L1 — the byte-stable M3 seam.
    let mut l1 = base.clone();
    let Some(p1) = render_shell_to_image(w, h, &camera, &mesh, None, &mut l1) else {
        eprintln!(
            "skip: no wgpu adapter (no GPU / software rasterizer) — \
             skip-on-absent per CLAUDE.md / phase-5-m3.md Decision 46"
        );
        return;
    };
    assert_eq!(p1.len() as u32, w * h * 4);
    let c1 = at(&p1, w / 2, h / 2);
    assert!(
        c1.iter().copied().max().unwrap() > 60,
        "L1: viewport centre is the mesh, got {c1:?}"
    );

    // (b) Focus mode — still composites the mesh; the rightmost column
    // (the 28 px AI rail in L1) is now scene, not rail chrome, so the
    // right-edge content differs between the two layouts.
    let mut l3 = base;
    l3.focus_mode = true;
    l3.dock_collapsed = true;
    let p3 = render_shell_to_image(w, h, &camera, &mesh, None, &mut l3)
        .expect("adapter was present for render (a)");
    let c3 = at(&p3, w / 2, h / 2);
    assert!(
        c3.iter().copied().max().unwrap() > 60,
        "L3: viewport centre still the mesh, got {c3:?}"
    );
    let col = |px: &[u8], x: u32| -> u64 {
        (10..h - 10)
            .map(|y| {
                let p = at(px, x, y);
                u64::from(p[0]) + u64::from(p[1]) + u64::from(p[2])
            })
            .sum()
    };
    assert_ne!(
        col(&p1, w - 4),
        col(&p3, w - 4),
        "hiding the AI rail must change the right-edge column"
    );
}
