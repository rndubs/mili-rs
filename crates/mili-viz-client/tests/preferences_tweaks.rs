//! View / Preferences tweaks-surface gating test
//! (`wireframe-parity.md` "Menu bar" + "Tweaks / Preferences";
//! MVP-cut item 7).
//!
//! Two halves, this crate's `vb003_render_modes.rs` /
//! `control_menu.rs` shape:
//!  * always-on — the pure, GPU-free wiring: the [`ShellState`] theme /
//!    dock-collapse switches are pure + observable, the defaults are
//!    the byte-stable M3 values (`Theme::Dark`, dock expanded), and the
//!    wired shell paints input-free in every theme × collapse combo.
//!    Collapsing the dock measurably widens the published `scene_frac`
//!    (28 px rail vs. 230 px dock) — a deterministic no-GPU check that
//!    the layout actually changed. A no-GPU CI box hard-gates these.
//!  * `composite_render` — headless: the default (Dark, expanded) seam
//!    is unperturbed (`bug-tracker.md` VB-001), and a `Light` + dock-
//!    collapsed render both still composites over the unchanged mesh
//!    pass while visibly relighting the chrome. **Skip-on-absent** when
//!    the corpus or a `wgpu` adapter is missing (CLAUDE.md convention).
//!
//! The menu-open click path is windowed pointer input and is **not
//! headlessly verifiable in CI**; the pure switches + the headless
//! paint/scene-frac/chrome checks are the contract this pins.

use std::path::{Path, PathBuf};

use mili_viz_client::{
    build_shell_ui, fetch_server_mesh, render_shell_to_image, Camera, LoadedInfo, SessionPhase,
    ShellState, Theme, UiAction,
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
fn defaults_are_the_byte_stable_m3_values() {
    let s = ShellState::default();
    assert_eq!(s.theme, Theme::Dark, "default theme == egui default");
    assert!(!s.dock_collapsed, "default dock is the full L1 dock");
    assert_eq!(Theme::default(), Theme::Dark);
    assert_ne!(Theme::Dark.label(), Theme::Light.label());
}

#[test]
fn tweak_switches_are_pure_and_observable() {
    let mut s = ShellState::default();

    let a = s.set_theme(Theme::Light);
    assert_eq!(s.theme, Theme::Light, "state mutated");
    assert_eq!(a, UiAction::SetTheme(Theme::Light), "observability action");

    let a = s.set_dock_collapsed(true);
    assert!(s.dock_collapsed);
    assert_eq!(a, UiAction::SetDockCollapsed(true));
    let a = s.set_dock_collapsed(false);
    assert!(!s.dock_collapsed);
    assert_eq!(a, UiAction::SetDockCollapsed(false));
}

#[test]
fn wired_shell_paints_input_free_in_every_combo() {
    for theme in [Theme::Dark, Theme::Light] {
        for collapsed in [false, true] {
            let mut s = ShellState {
                theme,
                dock_collapsed: collapsed,
                ..ShellState::default()
            };
            let (actions, painted) = paint(&mut s);
            assert!(painted, "{theme:?}/{collapsed}: shell must paint");
            assert!(
                actions.is_empty(),
                "{theme:?}/{collapsed}: no input ⇒ no actions: {actions:?}"
            );
        }
    }
}

#[test]
fn collapsing_the_dock_widens_the_scene() {
    // The leftover central rect (`scene_frac`) is published by
    // `build_shell_ui`. Collapsing the dock (230 px → 28 px rail) must
    // move the scene's left origin leftward — a deterministic, no-GPU
    // proof the layout actually changed.
    let mut expanded = ShellState {
        phase: SessionPhase::AttachedIdle,
        ..ShellState::default()
    };
    paint(&mut expanded);
    let mut collapsed = ShellState {
        phase: SessionPhase::AttachedIdle,
        dock_collapsed: true,
        ..ShellState::default()
    };
    paint(&mut collapsed);

    let ex = expanded.scene_frac.expect("expanded scene measured");
    let co = collapsed.scene_frac.expect("collapsed scene measured");
    assert!(
        co[0] < ex[0],
        "collapsed dock should push the scene origin left: {co:?} vs {ex:?}"
    );
    assert!(
        co[2] > ex[2],
        "collapsed dock should widen the scene: {co:?} vs {ex:?}"
    );
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

    // (a) Default (Dark, expanded) — the byte-stable M3 seam: the
    // viewport centre is still the mesh, dock chrome composites over it.
    let mut dark = base.clone();
    let Some(dpx) = render_shell_to_image(w, h, &camera, &mesh, None, &mut dark) else {
        eprintln!(
            "skip: no wgpu adapter (no GPU / software rasterizer) — \
             skip-on-absent per CLAUDE.md / phase-5-m3.md Decision 46"
        );
        return;
    };
    assert_eq!(dpx.len() as u32, w * h * 4);
    let dc = at(&dpx, w / 2, h / 2);
    assert!(
        dc.iter().copied().max().unwrap() > 60,
        "dark: viewport centre should be the mesh, got {dc:?}"
    );

    // (b) Light theme + dock collapsed — still composites over the
    // unchanged mesh pass, and the left chrome is visibly relit
    // (light panel ≫ dark panel brightness).
    let mut light = base;
    light.theme = Theme::Light;
    light.dock_collapsed = true;
    let lpx = render_shell_to_image(w, h, &camera, &mesh, None, &mut light)
        .expect("adapter was present for render (a)");
    let lc = at(&lpx, w / 2, h / 2);
    assert!(
        lc.iter().copied().max().unwrap() > 60,
        "light: viewport centre should still be the mesh, got {lc:?}"
    );
    // Sample the menu-bar strip (top, away from the viewport text): the
    // light theme's panel fill is far brighter than the dark theme's.
    let dark_menu = at(&dpx, w / 2, 6);
    let light_menu = at(&lpx, w / 2, 6);
    let lum = |p: [u8; 3]| u32::from(p[0]) + u32::from(p[1]) + u32::from(p[2]);
    assert!(
        lum(light_menu) > lum(dark_menu) + 120,
        "light theme must relight the menu chrome: light {light_menu:?} vs dark {dark_menu:?}"
    );
}
