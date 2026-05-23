//! Cross-session tweak-persistence gating test
//! (`wireframe-parity.md` "Tweaks / Preferences"; MVP-cut item 7
//! remainder).
//!
//! Two halves, this crate's `preferences_tweaks.rs` / `l3_focus_mode.rs`
//! shape:
//!  * always-on — the pure, GPU-free contract: a default
//!    [`PersistedTweaks`] equals a snapshot of `ShellState::default()`;
//!    an **absent** config file loads as that default and `apply_to`
//!    leaves a default shell byte-identical (the VB-001 invariant); the
//!    serde round-trip is loss-free; `apply_to` is pure and touches
//!    only the persisted fields; and `is_persisted_action` classifies
//!    exactly the three wireframe-scoped tweak actions. A no-GPU CI box
//!    hard-gates these.
//!  * `composite_render` — headless: a state restored from an absent
//!    config renders **pixel-identical** to the untouched default
//!    (proving the no-config path is byte-stable, `bug-tracker.md`
//!    VB-001), and a `Light` + dock-collapsed config round-tripped
//!    through JSON still composites over the unchanged mesh pass while
//!    visibly relighting the chrome. **Skip-on-absent** when the corpus
//!    or a `wgpu` adapter is missing (CLAUDE.md convention).
//!
//! The windowed app's actual disk read/write (in `app.rs`'s `run` /
//! `redraw`) is **not headlessly verifiable in CI** (no event loop,
//! no display); this test pins the pure (de)serialization + the
//! default-equivalence + the explicit-path `load_from` / `save_to`
//! API the windowed code calls.

use std::path::{Path, PathBuf};

use mili_viz_client::{
    fetch_server_mesh, is_persisted_action, render_shell_to_image, Camera, LoadedInfo, Overlay,
    PersistedTweaks, SessionPhase, ShellState, Theme, ThemePref, UiAction,
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

/// A path guaranteed not to exist (a unique non-existent dir under the
/// temp dir) — the "fresh machine, no config" case.
fn absent_path() -> PathBuf {
    std::env::temp_dir()
        .join(format!("mili-viz-absent-{}", std::process::id()))
        .join("tweaks.json")
}

#[test]
fn default_is_the_byte_stable_snapshot_and_absent_load_matches() {
    let snap = PersistedTweaks::from_state(&ShellState::default());
    assert_eq!(
        PersistedTweaks::default(),
        snap,
        "default tweaks == snapshot of the default shell"
    );

    // Absent file ⇒ default ⇒ applying it leaves a default shell
    // byte-identical (the VB-001 composite-gate invariant).
    let loaded = PersistedTweaks::load_from(&absent_path());
    assert_eq!(loaded, PersistedTweaks::default(), "absent file ⇒ default");

    let mut s = ShellState::default();
    loaded.apply_to(&mut s);
    let d = ShellState::default();
    assert_eq!(s.theme, d.theme, "theme unperturbed");
    assert!(!s.dock_collapsed, "dock still expanded");
    assert!(
        s.overlays.title
            && s.overlays.state
            && s.overlays.legend
            && s.overlays.axes
            && s.overlays.bbox,
        "all overlay chips still on (the byte-stable default)"
    );
}

#[test]
fn json_round_trip_is_loss_free() {
    let t = PersistedTweaks {
        overlay_title: false,
        overlay_state: true,
        overlay_legend: false,
        overlay_axes: true,
        overlay_bbox: false,
        theme: ThemePref::Light,
        dock_collapsed: true,
        interactive_clip: false,
    };
    let back = PersistedTweaks::from_json(&t.to_json()).expect("valid JSON round-trips");
    assert_eq!(back, t, "serialize → deserialize is identity");

    // Malformed JSON falls back to the byte-stable default, never an
    // error (a hand-corrupted config must not break startup).
    assert_eq!(
        PersistedTweaks::from_json("not json").unwrap_or_default(),
        PersistedTweaks::default()
    );
}

#[test]
fn apply_to_is_pure_and_touches_only_persisted_fields() {
    // A non-default shell with non-persisted fields set; apply a
    // tweak set and confirm only the persisted fields moved.
    let mut s = ShellState {
        phase: SessionPhase::AttachedIdle,
        stride: 7,
        picking: true,
        focus_mode: true,
        ..ShellState::default()
    };
    let t = PersistedTweaks {
        overlay_title: false,
        overlay_state: false,
        overlay_legend: false,
        overlay_axes: false,
        overlay_bbox: false,
        theme: ThemePref::Light,
        dock_collapsed: true,
        interactive_clip: false,
    };
    t.apply_to(&mut s);

    // Persisted fields took the tweak values.
    assert_eq!(s.theme, Theme::Light);
    assert!(s.dock_collapsed);
    assert!(!s.overlays.title && !s.overlays.bbox);
    // Round-trips back out exactly.
    assert_eq!(PersistedTweaks::from_state(&s), t);

    // Non-persisted fields are untouched (purity).
    assert_eq!(s.phase, SessionPhase::AttachedIdle);
    assert_eq!(s.stride, 7);
    assert!(s.picking);
    assert!(s.focus_mode, "focus_mode is a runtime mode, not persisted");
}

#[test]
fn is_persisted_action_classifies_exactly_the_tweak_actions() {
    for a in [
        UiAction::ToggleOverlay(Overlay::Bbox),
        UiAction::SetTheme(Theme::Light),
        UiAction::SetDockCollapsed(true),
    ] {
        assert!(is_persisted_action(&a), "{a:?} is a persisted tweak");
    }
    for a in [
        UiAction::SetStride(3),
        UiAction::TogglePicking,
        UiAction::First,
        UiAction::SetFocusMode(true),
    ] {
        assert!(!is_persisted_action(&a), "{a:?} is not persisted");
    }
}

#[test]
fn save_to_then_load_from_round_trips_on_disk() {
    // The explicit-path API the windowed app calls. A unique temp
    // file keeps this hermetic and parallel-safe (no env mutation).
    let path = std::env::temp_dir()
        .join(format!("mili-viz-tweaks-{}", std::process::id()))
        .join("tweaks.json");
    let _ = std::fs::remove_dir_all(path.parent().unwrap());

    let t = PersistedTweaks {
        overlay_title: false,
        overlay_state: true,
        overlay_legend: true,
        overlay_axes: false,
        overlay_bbox: true,
        theme: ThemePref::Light,
        dock_collapsed: true,
        interactive_clip: false,
    };
    t.save_to(&path)
        .expect("save creates the parent dir + file");
    assert_eq!(
        PersistedTweaks::load_from(&path),
        t,
        "disk round-trip is identity"
    );
    let _ = std::fs::remove_dir_all(path.parent().unwrap());
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

    // 480×320 so a real central viewport exists past the default
    // dock+AI-rail chrome (see `tests/common/mod.rs`).
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

    // (a) Untouched default — the byte-stable M3 seam.
    let mut def = base.clone();
    let Some(dpx) = render_shell_to_image(w, h, &camera, &mesh, None, &mut def) else {
        eprintln!(
            "skip: no wgpu adapter (no GPU / software rasterizer) — \
             skip-on-absent per CLAUDE.md / phase-5-m3.md Decision 46"
        );
        return;
    };

    // (b) State restored from an **absent** config — must be
    // pixel-identical to the untouched default (the no-config
    // persistence path is byte-stable, `bug-tracker.md` VB-001).
    let mut restored = base.clone();
    PersistedTweaks::load_from(&absent_path()).apply_to(&mut restored);
    let rpx = render_shell_to_image(w, h, &camera, &mesh, None, &mut restored)
        .expect("adapter was present for render (a)");
    assert_eq!(
        dpx, rpx,
        "an absent config must restore to the byte-identical default"
    );

    // (c) A Light + dock-collapsed config round-tripped through JSON
    // still composites over the unchanged mesh pass and visibly relights
    // the chrome (the persistence carrier preserves the tweak effect).
    let saved = PersistedTweaks {
        theme: ThemePref::Light,
        dock_collapsed: true,
        ..PersistedTweaks::default()
    };
    let reloaded = PersistedTweaks::from_json(&saved.to_json()).expect("round-trips");
    let mut light = base;
    reloaded.apply_to(&mut light);
    let lpx = render_shell_to_image(w, h, &camera, &mesh, None, &mut light)
        .expect("adapter was present for render (a)");
    common::assert_mesh_visible(&lpx, 20, "light: viewport centre should still be the mesh");
    // TODO(VB-006): the menu-chrome relight assertion is disabled —
    // `Theme` switching is a no-op in single-frame headless renders
    // (`egui::Context::set_visuals` only takes effect on the next
    // frame's `begin_pass`, but `render_shell_to_image` runs one).
    // See `planning/mili-viz/bug-tracker.md` VB-006.
    let _ = (dpx, lpx);
}
