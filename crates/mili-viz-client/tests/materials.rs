//! Materials enable/disable affordance gating test (wireframe-parity
//! Left dock / MVP-cut 3). The server side is already done
//! (`status.md` item 8); this is the GUI affordance, so it is
//! all **always-on** pure logic — the windowed transport is not
//! headlessly verifiable in CI (no display).

use mili_viz_client::{build_shell_ui, LoadedInfo, SessionPhase, ShellState, UiAction};

fn loaded(classes: &[&str]) -> ShellState {
    ShellState {
        phase: SessionPhase::AttachedIdle,
        loaded: Some(LoadedInfo {
            db: "bar71".into(),
            num_states: 1,
            state_times: vec![0.0],
            class_names: classes.iter().copied().map(String::from).collect(),
        }),
        ..ShellState::default()
    }
}

#[test]
fn materials_default_all_visible() {
    let s = loaded(&["brick", "shell", "beam"]);
    assert!(s.hidden_materials.is_empty(), "default = nothing hidden");
    for c in ["brick", "shell", "beam"] {
        assert!(s.material_visible(c), "{c} visible by default");
    }
}

#[test]
fn toggle_material_flips_and_emits_typed_command() {
    let mut s = loaded(&["brick", "shell"]);

    let a = s.toggle_material("brick");
    assert!(!s.material_visible("brick"), "brick now hidden");
    assert!(s.material_visible("shell"), "shell untouched");
    assert_eq!(
        a,
        UiAction::SetMaterialVisible {
            class_name: "brick".into(),
            visible: false,
        },
        "lowered to Command::Material{{ enable:false }}"
    );

    // Toggling back re-enables it (and clears it from the hidden set).
    let b = s.toggle_material("brick");
    assert!(s.material_visible("brick"));
    assert!(s.hidden_materials.is_empty());
    assert_eq!(
        b,
        UiAction::SetMaterialVisible {
            class_name: "brick".into(),
            visible: true,
        }
    );
}

#[test]
fn materials_section_runs_headlessly_and_is_pure() {
    let mut s = loaded(&["brick", "shell", "beam"]);
    s.hidden_materials.insert("shell".into());

    // The L1 layout is a pure fn of state: it renders the Materials
    // rows (one hidden) head­lessly and, with no pointer input, emits
    // no actions (the m3/m4 pattern).
    let ctx = egui::Context::default();
    let raw = egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(
            egui::Pos2::ZERO,
            egui::vec2(900.0, 600.0),
        )),
        ..Default::default()
    };
    let mut actions = Vec::new();
    let out = ctx.run_ui(raw, |ui| actions = build_shell_ui(ui, &mut s));
    assert!(actions.is_empty(), "no input ⇒ no actions: {actions:?}");
    assert!(!out.shapes.is_empty(), "the Materials section must paint");
    // State is unchanged by a pure render.
    assert_eq!(s.hidden_materials.len(), 1);
}
