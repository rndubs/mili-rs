//! `Control` menu gating test (`wireframe-parity.md` "Menu bar";
//! MVP-cut item 1).
//!
//! Two halves, this crate's `vb003_render_modes.rs` /
//! `scripting_runner.rs` shape:
//!  * always-on — the pure, GPU-free wiring: [`control_menu_items`] is
//!    exactly the already-existing, already-lowered transport / animate
//!    / view `UiAction`s (no new variant, no proto change), and
//!    [`build_shell_ui`] with the menu wired still paints input-free and
//!    emits nothing (the M1-Decision-40 pattern). A no-GPU CI box
//!    hard-gates these.
//!  * `composite_render` — the byte-stable M3 seam: the `Control` menu
//!    is closed by default and emits nothing without pointer input, so
//!    the default-`ShellState` composite path is unperturbed
//!    (`bug-tracker.md` VB-001). **Skip-on-absent** when the corpus or
//!    a `wgpu` adapter is missing (CLAUDE.md convention; not a failure).
//!
//! The menu-open click path is windowed pointer input and is **not
//! headlessly verifiable in CI**; the pure item list is the contract
//! this pins, and every variant in it is already exercised end-to-end
//! by the toolbar tests + `app.rs` lowering.

use std::path::{Path, PathBuf};

use mili_viz_client::{
    build_shell_ui, control_menu_items, fetch_server_mesh, render_shell_to_image, Camera,
    LoadedInfo, SessionPhase, ShellState, UiAction,
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
fn control_items_reuse_existing_lowered_actions() {
    let items = control_menu_items();
    let actions: Vec<UiAction> = items.iter().map(|(_, a)| a.clone()).collect();

    // Exactly the session-control verbs that already have a UiAction
    // and an app.rs lowering — no new variant, no proto change.
    assert_eq!(
        actions,
        vec![
            UiAction::First,
            UiAction::Prev,
            UiAction::Next,
            UiAction::Last,
            UiAction::ToggleAnimate,
            UiAction::StopAnimate,
            UiAction::ViewReset,
            UiAction::Fit,
        ]
    );

    // Every label is non-blank and the set is unique (readable rows).
    assert!(items.iter().all(|(l, _)| !l.trim().is_empty()));
    assert_eq!(
        items
            .iter()
            .map(|(l, _)| *l)
            .collect::<std::collections::HashSet<_>>()
            .len(),
        items.len()
    );
}

#[test]
fn wired_menu_paints_input_free_in_every_phase() {
    // The Control menu is closed by default; with no pointer input the
    // L1 shell still paints and emits nothing, in all three phases
    // (the not-attached path also greys the menu body).
    for phase in [
        SessionPhase::NotAttached,
        SessionPhase::AttachedIdle,
        SessionPhase::Animating,
    ] {
        let mut s = ShellState {
            phase,
            ..ShellState::default()
        };
        let (actions, painted) = paint(&mut s);
        assert!(painted, "{phase:?}: the L1 shell must still paint");
        assert!(
            actions.is_empty(),
            "{phase:?}: no pointer input ⇒ no actions: {actions:?}"
        );
    }
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

    // Default ShellState (Control menu closed) — the byte-stable M3
    // composite seam must be unperturbed by the menu wiring: the
    // viewport centre is still the mesh, dock chrome over it.
    let mut s = ShellState {
        phase: SessionPhase::AttachedIdle,
        loaded: Some(LoadedInfo {
            db: path.to_string_lossy().into_owned(),
            num_states: 1,
            state_times: vec![0.0],
            class_names: vec!["brick".into()],
        }),
        ..ShellState::default()
    };
    let Some(px) = render_shell_to_image(w, h, &camera, &mesh, None, &mut s) else {
        eprintln!(
            "skip: no wgpu adapter (no GPU / software rasterizer) — \
             skip-on-absent per CLAUDE.md / phase-5-m3.md Decision 46"
        );
        return;
    };
    assert_eq!(px.len() as u32, w * h * 4);
    let i = (((h / 2) * w + w / 2) * 4) as usize;
    let centre = [px[i], px[i + 1], px[i + 2]];
    assert!(
        centre.iter().copied().max().unwrap() > 60,
        "viewport centre should be the mesh, got {centre:?}"
    );
}
