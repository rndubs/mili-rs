//! Scripting-runner gating test (`client.md` decision 3,
//! `phase-6-m2.md`; MVP-cut item 9).
//!
//! Two halves, per this crate's `m3_5_bottom_tabs.rs`/
//! `m4_view_manipulation.rs` shape:
//!  * always-on — the pure, GPU-free runner logic on [`ShellState`]:
//!    `run_script` gates on blank/in-flight and emits the verbatim
//!    `RunScript`, `push_script_output` accumulates the stream,
//!    `finish_script` clears the running flag, and the (now enabled)
//!    scripting tab paints headlessly. A no-GPU CI box hard-gates
//!    these.
//!  * `composite_render` — the end-to-end seam: spawn the in-process
//!    server, `load`/`show` `serial/basic1`, then render with the
//!    scripting tab (a) collapsed — proving the M3 composite path is
//!    byte-stable (`bug-tracker.md` VB-001 / `phase-5-m3.5.md`
//!    Decision 51) — and (b) the scripting body open — proving the
//!    real body composites over the unchanged mesh pass. **Skip-on-
//!    absent** when the corpus or a `wgpu` adapter is missing
//!    (CLAUDE.md convention; not a failure).
//!
//! The actual `pygriz` **subprocess** path (the windowed app spawning
//! a child `python` and streaming its stdout/stderr) is windowed-only
//! and is **not headlessly verifiable in CI**: it needs a real
//! display loop + a Python interpreter. It is exercised by hand in the
//! windowed client; the `ShellState` seam it drives is what this test
//! pins.

use std::path::{Path, PathBuf};

use mili_viz_client::{
    build_shell_ui, fetch_server_mesh, render_shell_to_image, BottomTab, Camera, LoadedInfo,
    SessionPhase, ShellState, UiAction,
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
fn run_script_gates_and_emits_verbatim() {
    let mut s = ShellState::default();
    // The editor is seeded with the launch() template (attach() into
    // this in-process GUI needs Phase 5 M5).
    assert!(s.script.contains("griz.launch()"));
    assert!(!s.script_running);

    let a = s.run_script().expect("a non-blank buffer emits an action");
    assert_eq!(a, UiAction::RunScript(s.script.clone()));
    assert!(s.script_running, "the run flag latches");
    assert!(s.script_output.is_empty(), "the pane is cleared on start");

    // A second Run while one is in flight is a no-op (no double-spawn).
    assert!(s.run_script().is_none());

    // A blank buffer never spawns.
    let mut blank = ShellState {
        script: "   \n\t".into(),
        ..ShellState::default()
    };
    assert!(blank.run_script().is_none());
    assert!(!blank.script_running);
}

#[test]
fn output_stream_and_finish_round_trip() {
    let mut s = ShellState::default();
    let _ = s.run_script();

    // The app folds streamed chunks in verbatim, in order.
    s.push_script_output("griz.launch() -> tcp://127.0.0.1:54321\n");
    s.push_script_output("ok\n");
    assert_eq!(
        s.script_output,
        "griz.launch() -> tcp://127.0.0.1:54321\nok\n"
    );

    // Finish clears the run flag and posts the venv/attach status.
    s.finish_script("venv: python3 (PYTHONPATH) · attach: launch · ok");
    assert!(!s.script_running, "finish releases the Run button");
    assert!(s.script_status.contains("attach: launch"));

    // After finishing, Run is armed again (idempotent re-run).
    let a = s.run_script();
    assert!(matches!(a, Some(UiAction::RunScript(_))));
}

#[test]
fn scripting_tab_paints_and_is_input_free() {
    // The tab is no longer a disabled placeholder; with no pointer
    // input it paints and emits nothing (mirrors the bottom-tabs
    // no-input invariant).
    let mut s = ShellState {
        bottom_tab: Some(BottomTab::Scripting),
        ..ShellState::default()
    };
    let (actions, painted) = paint(&mut s);
    assert!(painted, "the scripting tab body must paint");
    assert!(
        actions.is_empty(),
        "no pointer input ⇒ no actions: {actions:?}"
    );

    // Painting a running state (spinner branch) is also clean.
    let mut running = ShellState {
        bottom_tab: Some(BottomTab::Scripting),
        script_running: true,
        script_output: "partial output…\n".into(),
        ..ShellState::default()
    };
    let (actions, painted) = paint(&mut running);
    assert!(painted);
    assert!(
        actions.is_empty(),
        "running view emits nothing: {actions:?}"
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
    let is_chrome = |p: [u8; 3]| {
        let mx = p.iter().copied().max().unwrap();
        let mn = p.iter().copied().min().unwrap();
        (20..130).contains(&mx) && (mx - mn) < 45
    };

    // (a) Tabs collapsed — the byte-stable M3 seam (Decision 51): the
    // viewport centre is still the mesh, dock chrome composites over
    // it. The enabled scripting tab must not perturb this default.
    let mut collapsed = base.clone();
    let Some(px) = render_shell_to_image(w, h, &camera, &mesh, None, &mut collapsed) else {
        eprintln!(
            "skip: no wgpu adapter (no GPU / software rasterizer) — \
             skip-on-absent per CLAUDE.md / phase-5-m3.md Decision 46"
        );
        return;
    };
    assert_eq!(px.len() as u32, w * h * 4);
    let cp = at(&px, w / 2, h / 2);
    assert!(
        cp.iter().copied().max().unwrap() > 60,
        "collapsed: viewport centre should be the mesh, got {cp:?}"
    );
    let dock_chrome = (40..h - 30).any(|y| is_chrome(at(&px, 40, y)));
    assert!(dock_chrome, "collapsed: left dock chrome over the mesh");

    // (b) Scripting tab open — the real runner body must be opaque
    // chrome composited over the unchanged mesh pass.
    let mut opened = base;
    opened.bottom_tab = Some(BottomTab::Scripting);
    opened.script_output = "griz.launch() -> tcp://127.0.0.1:0\n".into();
    let px = render_shell_to_image(w, h, &camera, &mesh, None, &mut opened)
        .expect("adapter was present for render (a)");
    let body_y = h - 30;
    let body_chrome = (60..w - 60).any(|x| is_chrome(at(&px, x, body_y)));
    assert!(
        body_chrome,
        "open: the scripting-tab body must composite over the mesh pass"
    );
}
