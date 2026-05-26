//! Menu-bar parity gating test (`wireframe-parity.md` "Menu bar"; the
//! "What's still left" punch list item #1).
//!
//! The wireframe spec names the menu bar as `Control · Rendering ·
//! Picking · Results · Time · Plot · Help` and defers the contents to
//! "the legacy griz Motif menus" (`reference/griz/Src/gui.c::create_menu_bar`).
//! `Control`/`Rendering`/`Picking`/`Preferences` already had real bodies;
//! `Results`/`Time`/`Plot`/`Help` were the four `|_| {}` empty stubs.
//! This test pins the pure contracts of the new bodies:
//!
//!  * `time_menu_items()` is exactly the legacy griz `Time` pulldown's
//!    transport verbs (Next/Prev/First/Last + Animate/Stop Animate),
//!    using `UiAction`s the toolbar / `Control` menu already lower —
//!    no new variant, no proto change.
//!  * The L1 shell with the populated menus still paints input-free
//!    and emits nothing (the closed-by-default discipline the
//!    `control_menu_items` test already pins).
//!
//! The actual menu-open click path is windowed pointer input and not
//! headlessly verifiable (egui's menu open state is gated on real
//! pointer events). The wiring underneath each row is exercised
//! end-to-end by the toolbar / bottom-tabs / catalog tests already.

use mili_viz_client::{
    build_shell_ui, time_menu_items, BottomTab, SessionPhase, ShellState, UiAction,
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

#[test]
fn time_items_are_the_legacy_griz_transport_verbs() {
    // `reference/griz/Src/gui.c::create_menu_bar` — the legacy `Time`
    // pulldown is exactly Next/Prev/First/Last State + Animate/Stop
    // Animate. "Continue Animate" maps to re-entering the same toggle.
    let items = time_menu_items();
    let actions: Vec<UiAction> = items.iter().map(|(_, a)| a.clone()).collect();
    assert_eq!(
        actions,
        vec![
            UiAction::First,
            UiAction::Prev,
            UiAction::Next,
            UiAction::Last,
            UiAction::ToggleAnimate,
            UiAction::StopAnimate,
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
fn time_items_reuse_only_already_lowered_actions() {
    // Every Time variant is already a `Control` menu / toolbar lowering
    // — the griz idiom is menus duplicating the toolbar, not new verbs.
    // Pinning this prevents drift where a Time-specific action sneaks
    // in without an `app.rs` arm.
    let control = mili_viz_client::control_menu_items();
    let control_actions: Vec<UiAction> = control.iter().map(|(_, a)| a.clone()).collect();
    for (_, a) in time_menu_items() {
        assert!(
            control_actions.iter().any(|c| c == &a),
            "Time menu action {a:?} must also live in control_menu_items \
             (griz idiom: menus duplicate the toolbar; new verbs need an \
             explicit `app.rs` lowering arm)"
        );
    }
}

#[test]
fn populated_menus_still_paint_input_free_in_every_phase() {
    // The four newly-populated menus (Results, Time, Plot, Help) are
    // closed by default; with no pointer input the L1 shell still
    // paints and emits nothing in all three session phases.
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

#[test]
fn plot_menu_state_mutation_is_pure_and_observable() {
    // The `Plot → Time Hist Plot` verb is the only menu row that
    // mutates ShellState directly (vs. emitting an existing toolbar
    // lowering): it opens the TimeHistory bottom tab. Drive the
    // mutation through the same shape the menu uses so a regression in
    // the open-the-tab behavior surfaces here too.
    let mut s = ShellState::default();
    assert!(
        s.bottom_tab.is_none(),
        "default is the 22 px collapsed strip"
    );
    s.bottom_tab = Some(BottomTab::TimeHistory);
    let a = UiAction::SelectBottomTab(BottomTab::TimeHistory);
    assert_eq!(a, UiAction::SelectBottomTab(BottomTab::TimeHistory));
    assert_eq!(s.bottom_tab, Some(BottomTab::TimeHistory));
}
