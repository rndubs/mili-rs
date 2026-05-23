//! Left-dock collapsed icon-rail (R/M/S/P glyphs) gating test
//! (`wireframe-parity.md` "Left dock" — *Per-section row-count badges;
//! Picking glyph row*; wireframes §"L3 — Focus mode").
//!
//! Two halves, this crate's `control_menu.rs` / `preferences_tweaks.rs`
//! shape:
//!  * always-on — the pure, GPU-free wiring: [`dock_rail_glyphs`] is
//!    exactly the wireframe's `R/M/S/P` set, the `P` hint tracks the
//!    live picking state, and the collapsed shell paints the rail
//!    input-free (no pointer input ⇒ no actions). The expanded default
//!    is unchanged (byte-stable). A no-GPU CI box hard-gates these.
//!  * `composite_render` — headless: the collapsed-rail render still
//!    composites over the unchanged mesh pass, and the default
//!    (expanded) seam is unperturbed (`bug-tracker.md` VB-001).
//!    **Skip-on-absent** when the corpus / a `wgpu` adapter is missing
//!    (CLAUDE.md convention; not a failure).
//!
//! The glyph-click→expand path is windowed pointer input and is **not
//! headlessly verifiable in CI**; the pure glyph list + the headless
//! paint are the contract this pins (the click just emits the same
//! `SetDockCollapsed(false)` `preferences_tweaks.rs` already pins).

use std::path::{Path, PathBuf};

use mili_viz_client::{
    build_shell_ui, dock_rail_glyphs, fetch_server_mesh, render_shell_to_image, Camera, LoadedInfo,
    SessionPhase, ShellState, UiAction,
};

mod common;

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
fn rail_glyphs_are_the_wireframe_rmsp_set() {
    let off = dock_rail_glyphs(false);
    let glyphs: Vec<&str> = off.iter().map(|(g, _)| *g).collect();
    assert_eq!(
        glyphs,
        vec!["R", "M", "S", "P"],
        "wireframes §L3: R/M/S/P for Results/Materials/Surfaces/Picking"
    );
    // Single-char glyph column; unique; non-blank hints.
    assert!(off.iter().all(|(g, _)| g.chars().count() == 1));
    assert!(off.iter().all(|(_, t)| !t.trim().is_empty()));

    // The P hint tracks the live picking state (rail doubles as a
    // status read-out); the other three are state-independent.
    let on = dock_rail_glyphs(true);
    assert_eq!(off[3].1, "Picking: off — expand dock");
    assert_eq!(on[3].1, "Picking: on — expand dock");
    for i in 0..3 {
        assert_eq!(off[i], on[i], "non-picking glyphs are state-independent");
    }
}

#[test]
fn collapsed_rail_paints_input_free_and_default_is_unchanged() {
    // Default (expanded) is the byte-stable L1 dock.
    let mut expanded = ShellState::default();
    assert!(!expanded.dock_collapsed);
    let (a, painted) = paint(&mut expanded);
    assert!(painted && a.is_empty(), "default L1 dock: {a:?}");

    // Collapsed, in every phase × picking combo, the rail paints and
    // emits nothing without pointer input.
    for phase in [
        SessionPhase::NotAttached,
        SessionPhase::AttachedIdle,
        SessionPhase::Animating,
    ] {
        for picking in [false, true] {
            let mut s = ShellState {
                phase,
                dock_collapsed: true,
                picking,
                ..ShellState::default()
            };
            let (actions, painted) = paint(&mut s);
            assert!(painted, "{phase:?}/pick={picking}: rail must paint");
            assert!(
                actions.is_empty(),
                "{phase:?}/pick={picking}: no input ⇒ no actions: {actions:?}"
            );
        }
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

    // 480×320 so a real central viewport exists once the default
    // dock (230 px) + AI rail (28 px) are subtracted (240×240 had no
    // visible viewport — the `at(w/2, h/2)` checks sampled chrome).
    let (w, h) = (480u32, 320u32);
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

    // (a) Default expanded dock — the byte-stable M3 seam.
    let mut expanded = base.clone();
    let Some(epx) = render_shell_to_image(w, h, &camera, &mesh, None, &mut expanded) else {
        eprintln!(
            "skip: no wgpu adapter (no GPU / software rasterizer) — \
             skip-on-absent per CLAUDE.md / phase-5-m3.md Decision 46"
        );
        return;
    };
    assert_eq!(epx.len() as u32, w * h * 4);
    common::assert_mesh_visible(&epx, 20, "expanded: viewport centre is the mesh");

    // (b) Collapsed icon rail — the 28 px rail still composites over
    // the unchanged mesh pass; the mesh is now visible much closer to
    // the left edge (the 230 px dock no longer occludes it).
    let mut collapsed = base;
    collapsed.dock_collapsed = true;
    let cpx = render_shell_to_image(w, h, &camera, &mesh, None, &mut collapsed)
        .expect("adapter was present for render (a)");
    common::assert_mesh_visible(&cpx, 20, "collapsed: viewport centre still the mesh");
    // Just right of the 28 px rail there is now scene, not dock chrome:
    // the column at x≈34 differs between the two layouts.
    let col = |px: &[u8]| -> u64 {
        (10..h - 10)
            .map(|y| {
                let p = at(px, 34, y);
                u64::from(p[0]) + u64::from(p[1]) + u64::from(p[2])
            })
            .sum()
    };
    assert_ne!(
        col(&epx),
        col(&cpx),
        "collapsing the dock must change what is drawn at x≈34"
    );
}
