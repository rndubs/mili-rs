//! Gating test for the per-element time-history series UX
//! (`wireframe-parity.md` "What's still left" #4 — the text-input
//! variant of the `Query`-fed Plot tab).
//!
//! The server arm of #4 already has its own gate
//! (`crates/mili-viz-server/tests/query_rpc.rs`); this exercises the
//! pure-client half:
//!
//!  * the Plot-tab input row consumes `class · id · svar · component`
//!    into a [`UiAction::QueryElementSeries`] and clears the buffers;
//!  * `submit_element_query` is idempotent on `(class, id, svar,
//!    component)` — re-submitting an already-listed series clears its
//!    samples in place rather than stacking a duplicate legend entry;
//!  * `push_element_series` / `drop_element_series` round-trip the
//!    placeholder the input row appended so the lowering arm in
//!    `app.rs` has a stable contract for the success / failure
//!    branches it depends on;
//!  * the L1 shell with seeded element series + the TimeHistory tab
//!    open still paints and emits nothing without pointer input —
//!    pinning the closed-by-default discipline the existing menu_bar
//!    test enforces for the populated menus (the new render arm is
//!    not allowed to fire spurious actions either).

use mili_viz_client::{
    build_shell_ui, BottomTab, ElementSeries, ElementSeriesSample, LoadedInfo, ResultInfo,
    SessionPhase, ShellState, UiAction,
};

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

fn loaded_state(num_states: u32) -> ShellState {
    ShellState {
        phase: SessionPhase::AttachedIdle,
        loaded: Some(LoadedInfo {
            db: "test.A".to_string(),
            num_states,
            state_times: (1..=num_states).map(|s| f64::from(s) * 0.1).collect(),
            class_names: vec!["brick".to_string()],
        }),
        result: Some(ResultInfo {
            name: "sx".to_string(),
            ..ResultInfo::default()
        }),
        bottom_tab: Some(BottomTab::TimeHistory),
        ..ShellState::default()
    }
}

#[test]
fn submit_empty_fields_returns_none_and_keeps_state_untouched() {
    // Empty fields must not append a placeholder. (Otherwise the user
    // would see a phantom legend entry the lowering arm has no way to
    // satisfy — the input row was blank.)
    let mut s = loaded_state(5);
    let before = s.element_series.clone();
    assert!(s.submit_element_query().is_none());
    assert_eq!(s.element_series, before);
    // A partial fill (class + svar but no id) is also rejected.
    s.plot_class_input = "brick".to_string();
    s.plot_svar_input = "sx".to_string();
    assert!(s.submit_element_query().is_none());
    assert!(s.element_series.is_empty());
}

#[test]
fn submit_full_form_emits_action_and_clears_inputs() {
    let mut s = loaded_state(5);
    s.plot_class_input = "brick".to_string();
    s.plot_label_input = "42".to_string();
    s.plot_svar_input = "sx".to_string();
    let a = s.submit_element_query().expect("full form lowers");
    let UiAction::QueryElementSeries {
        label,
        class_name,
        label_id,
        svar,
        component,
    } = a
    else {
        panic!("expected QueryElementSeries");
    };
    assert_eq!(class_name, "brick");
    assert_eq!(label_id, 42);
    assert_eq!(svar, "sx");
    assert!(component.is_empty());
    assert_eq!(label, "sx [brick 42]");
    // Inputs are drained so the next submission starts blank.
    assert!(s.plot_class_input.is_empty());
    assert!(s.plot_label_input.is_empty());
    assert!(s.plot_svar_input.is_empty());
    assert!(s.plot_component_input.is_empty());
    // A placeholder series is appended so the user sees the legend
    // entry immediately; samples are empty until the lowering arm
    // calls `push_element_series`.
    assert_eq!(s.element_series.len(), 1);
    assert!(s.element_series[0].samples.is_empty());
    assert_eq!(s.element_series[0].label, label);
}

#[test]
fn resubmitting_same_series_does_not_duplicate_legend() {
    let mut s = loaded_state(5);
    s.plot_class_input = "brick".to_string();
    s.plot_label_input = "42".to_string();
    s.plot_svar_input = "sx".to_string();
    let _ = s.submit_element_query().unwrap();
    // Seed some samples to confirm a re-submit clears them.
    s.push_element_series(
        "sx [brick 42]",
        vec![ElementSeriesSample {
            state: 1,
            t: 0.0,
            value: 1.5,
        }],
    );
    assert_eq!(s.element_series[0].samples.len(), 1);
    s.plot_class_input = "brick".to_string();
    s.plot_label_input = "42".to_string();
    s.plot_svar_input = "sx".to_string();
    let _ = s.submit_element_query().unwrap();
    assert_eq!(s.element_series.len(), 1, "no duplicate legend row");
    assert!(s.element_series[0].samples.is_empty(), "samples reset");
}

#[test]
fn component_distinguishes_two_otherwise_identical_series() {
    let mut s = loaded_state(5);
    s.plot_class_input = "brick".to_string();
    s.plot_label_input = "42".to_string();
    s.plot_svar_input = "stress".to_string();
    s.plot_component_input = "1".to_string();
    let _ = s.submit_element_query().unwrap();
    s.plot_class_input = "brick".to_string();
    s.plot_label_input = "42".to_string();
    s.plot_svar_input = "stress".to_string();
    s.plot_component_input = "2".to_string();
    let _ = s.submit_element_query().unwrap();
    assert_eq!(s.element_series.len(), 2);
    assert_ne!(s.element_series[0].label, s.element_series[1].label);
}

#[test]
fn push_and_drop_round_trip_the_placeholder() {
    let mut s = loaded_state(3);
    s.element_series.push(ElementSeries {
        label: "sx [brick 7]".to_string(),
        class_name: "brick".to_string(),
        label_id: 7,
        svar: "sx".to_string(),
        component: String::new(),
        samples: Vec::new(),
    });
    // Unknown label is a no-op (lowering arm dropped the placeholder
    // already; we must not resurrect it under the user).
    s.push_element_series(
        "sx [brick 999]",
        vec![ElementSeriesSample {
            state: 1,
            t: 0.0,
            value: 0.0,
        }],
    );
    assert_eq!(s.element_series.len(), 1);
    assert!(s.element_series[0].samples.is_empty());
    // Push to the matching label fills the samples in place.
    let payload = vec![
        ElementSeriesSample {
            state: 1,
            t: 0.0,
            value: 1.0,
        },
        ElementSeriesSample {
            state: 2,
            t: 0.1,
            value: 2.0,
        },
    ];
    s.push_element_series("sx [brick 7]", payload.clone());
    assert_eq!(s.element_series[0].samples, payload);
    // Drop removes the entry; a second drop is a tolerated no-op.
    s.drop_element_series("sx [brick 7]");
    assert!(s.element_series.is_empty());
    s.drop_element_series("sx [brick 7]");
    assert!(s.element_series.is_empty());
}

#[test]
fn plot_tab_with_seeded_series_paints_and_emits_nothing() {
    // Even with element series open in the TimeHistory body, no
    // pointer input ⇒ no actions. Mirrors the menu_bar test's
    // closed-by-default discipline for the newly-rendered legend
    // entries (the input row also stays inert without clicks).
    let mut s = loaded_state(3);
    s.element_series.push(ElementSeries {
        label: "sx [brick 7]".to_string(),
        class_name: "brick".to_string(),
        label_id: 7,
        svar: "sx".to_string(),
        component: String::new(),
        samples: vec![
            ElementSeriesSample {
                state: 1,
                t: 0.0,
                value: 1.0,
            },
            ElementSeriesSample {
                state: 2,
                t: 0.1,
                value: 2.0,
            },
            ElementSeriesSample {
                state: 3,
                t: 0.2,
                value: 3.0,
            },
        ],
    });
    let (actions, painted) = paint(&mut s);
    assert!(painted, "plot body must paint with a seeded series");
    assert!(
        actions.is_empty(),
        "no pointer input ⇒ no actions: {actions:?}"
    );
}

#[test]
fn empty_plot_tab_still_paints_with_only_the_hint() {
    // The empty-state hint stays painted without samples; no actions
    // emit just from showing the placeholder text.
    let mut s = loaded_state(3);
    let (actions, painted) = paint(&mut s);
    assert!(painted, "empty plot body still paints the hint");
    assert!(actions.is_empty(), "{actions:?}");
}
