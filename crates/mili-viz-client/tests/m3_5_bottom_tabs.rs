//! Phase 5 M3.5 gating test (`phase-5-m3.5.md` § "Acceptance gate").
//!
//! Two halves, per the M1/M2/M3 `*_renderer.rs`/`*_shell.rs` shape:
//!  * always-on — the pure, GPU-free bottom-tabs logic: the
//!    default-collapsed strip, tab open/collapse, the verbatim
//!    Layer-0 command echo + `RunCommand` emission, the dim/error
//!    outcome rows, and the `Subscribe`-fed time-history series. A
//!    no-GPU CI box hard-gates these.
//!  * `composite_render` — the end-to-end path: spawn the in-process
//!    server, `load`/`show` `serial/basic1`, then render with the
//!    bottom tabs (a) collapsed — proving the M3 seam is byte-stable
//!    (Decision 51) — and (b) the command-line body open — proving
//!    the real body composites over the unchanged mesh pass
//!    (Decision 45). **Skip-on-absent** when the corpus or a `wgpu`
//!    adapter is missing (CLAUDE.md convention; not a failure).

use std::path::{Path, PathBuf};

use mili_viz_client::{
    build_shell_ui, fetch_server_mesh, render_shell_to_image, BottomTab, Camera, LoadedInfo,
    ResultInfo, SessionPhase, ShellState, TranscriptKind, UiAction,
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
fn bottom_tabs_default_collapsed_and_paints() {
    // Decision 51: ShellState::default leaves the tabs collapsed so
    // the M3 render footprint (and m3_egui_shell.rs) is byte-stable.
    let mut s = ShellState {
        phase: SessionPhase::AttachedIdle,
        loaded: Some(LoadedInfo {
            db: "basic1".into(),
            num_states: 3,
            state_times: vec![0.0, 1e-3, 2e-3],
            class_names: vec!["brick".into()],
        }),
        ..ShellState::default()
    };
    assert_eq!(s.bottom_tab, None, "tabs collapsed by default");
    let (actions, painted) = paint(&mut s);
    assert!(painted, "the L1 shell + collapsed tab strip must paint");
    assert!(actions.is_empty(), "no input ⇒ no actions: {actions:?}");
}

#[test]
fn tab_toggle_is_pure_client_state() {
    let mut s = ShellState::default();
    // Open: emits SelectBottomTab and sets the open tab.
    let a = s.toggle_tab(BottomTab::CommandLine);
    assert_eq!(a, UiAction::SelectBottomTab(BottomTab::CommandLine));
    assert_eq!(s.bottom_tab, Some(BottomTab::CommandLine));
    // Switching tabs: still a select, body stays open.
    let a = s.toggle_tab(BottomTab::TimeHistory);
    assert_eq!(a, UiAction::SelectBottomTab(BottomTab::TimeHistory));
    assert_eq!(s.bottom_tab, Some(BottomTab::TimeHistory));
    // Re-clicking the active tab collapses the body.
    let a = s.toggle_tab(BottomTab::TimeHistory);
    assert_eq!(a, UiAction::CollapseBottomTabs);
    assert_eq!(s.bottom_tab, None);

    // Each opened tab body paints headlessly (no GPU, no transport).
    for tab in [
        BottomTab::CommandLine,
        BottomTab::Scripting,
        BottomTab::TimeHistory,
    ] {
        let mut st = ShellState {
            bottom_tab: Some(tab),
            ..ShellState::default()
        };
        let (actions, painted) = paint(&mut st);
        assert!(painted, "{tab:?} body must paint");
        assert!(
            actions.is_empty(),
            "{tab:?}: no pointer input ⇒ no actions: {actions:?}"
        );
    }
}

#[test]
fn command_line_is_verbatim_layer0() {
    // Decision 48: a submitted line is echoed as a `griz>` row and
    // emitted verbatim as RunCommand — no client-side re-parse.
    let mut s = ShellState {
        cmdline_input: "  state 10; show sx  ".into(),
        ..ShellState::default()
    };
    let a = s.submit_command().expect("non-blank line emits an action");
    assert_eq!(a, UiAction::RunCommand("state 10; show sx".into()));
    assert!(s.cmdline_input.is_empty(), "input cleared on submit");
    assert_eq!(s.transcript.len(), 1);
    assert_eq!(s.transcript[0].kind, TranscriptKind::Command);
    assert_eq!(s.transcript[0].text, "state 10; show sx");

    // Blank lines do nothing.
    s.cmdline_input = "   ".into();
    assert!(s.submit_command().is_none());
    assert_eq!(s.transcript.len(), 1, "blank line adds no row");

    // The app appends the dim/error outcome row after Execute.
    s.push_command_outcome(true, "");
    assert_eq!(s.transcript[1].kind, TranscriptKind::Response);
    assert_eq!(s.transcript[1].text, "ok");
    s.push_command_outcome(false, "unknown command: frobnicate");
    assert_eq!(s.transcript[2].kind, TranscriptKind::Error);
    assert_eq!(s.transcript[2].text, "unknown command: frobnicate");
}

#[test]
fn time_history_accumulates_from_result_stream() {
    // Decision 50: the series is fed by the broadcast ResultState,
    // deduped per state so scrubbing back and forth is stable.
    let mut s = ShellState {
        loaded: Some(LoadedInfo {
            db: "basic1".into(),
            num_states: 3,
            state_times: vec![0.0, 1e-3, 2e-3],
            class_names: vec!["brick".into()],
        }),
        ..ShellState::default()
    };
    // No result yet ⇒ no sample.
    s.state = 1;
    s.record_time_sample();
    assert!(s.time_history.is_empty());

    s.result = Some(ResultInfo {
        name: "eff_stress".into(),
        component: String::new(),
        min: 0.0,
        max: 5.0,
        num_vertices: 8,
        num_indices: 12,
    });
    s.state = 1;
    s.record_time_sample();
    s.state = 2;
    s.result.as_mut().unwrap().max = 7.0;
    s.record_time_sample();
    assert_eq!(s.time_history.len(), 2);
    // Re-visiting state 1 replaces, never duplicates.
    s.state = 1;
    s.result.as_mut().unwrap().max = 6.0;
    s.record_time_sample();
    assert_eq!(s.time_history.len(), 2, "state 1 sample replaced");
    assert_eq!(s.time_history[0].state, 1);
    assert!((s.time_history[0].max - 6.0).abs() < 1e-9);
    assert!(s.time_history[0].t.abs() < 1e-9);
    assert_eq!(s.time_history[1].state, 2);

    // An empty result name is not a real result ⇒ ignored.
    s.result.as_mut().unwrap().name.clear();
    s.state = 3;
    s.record_time_sample();
    assert_eq!(s.time_history.len(), 2, "bare-hull view adds no sample");
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
    let is_chrome = |p: [u8; 3]| {
        let mx = p.iter().copied().max().unwrap();
        let mn = p.iter().copied().min().unwrap();
        (20..130).contains(&mx) && (mx - mn) < 45
    };

    // (a) Tabs collapsed — the M3 layout (Decision 51): the viewport
    // centre is still the mesh and the left dock chrome composites
    // over it (the m3_egui_shell.rs invariant, new code path).
    let mut collapsed = base.clone();
    let Some(px) = render_shell_to_image(w, h, &camera, &mesh, None, &mut collapsed) else {
        eprintln!(
            "skip: no wgpu adapter (no GPU / software rasterizer) — \
             skip-on-absent per phase-5-m3.5.md / phase-5-m3.md Decision 46"
        );
        return;
    };
    assert_eq!(px.len() as u32, w * h * 4);
    common::assert_mesh_visible(&px, 20, "collapsed: viewport centre should be the mesh");
    let dock_chrome = (40..h - 30).any(|y| is_chrome(at(&px, 40, y)));
    assert!(dock_chrome, "collapsed: left dock chrome over the mesh");

    // (b) Command-line tab open — the real bottom-tabs body must be
    // opaque chrome composited over the unchanged mesh pass
    // (Decision 45 with a real body, not the M3 22 px stub).
    let mut opened = base;
    opened.bottom_tab = Some(BottomTab::CommandLine);
    opened.transcript.push(mili_viz_client::TranscriptLine {
        kind: TranscriptKind::Command,
        text: "state 1".into(),
    });
    let px = render_shell_to_image(w, h, &camera, &mesh, None, &mut opened)
        .expect("adapter was present for render (a)");
    // A horizontal strip just above the 20 px status bar is inside
    // the open command-line body — it must be opaque egui chrome.
    let body_y = h - 30;
    let body_chrome = (60..w - 60).any(|x| is_chrome(at(&px, x, body_y)));
    assert!(
        body_chrome,
        "open: the bottom-tabs body must composite over the mesh pass"
    );
}
