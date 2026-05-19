//! Status-bar `proto` / peer-count gating test
//! (`wireframe-parity.md` "Status bar" row; `phase-5-m4.md`).
//!
//! Two halves, this crate's `result_catalog.rs` shape:
//!  * always-on — the pure, GPU-free contract: the wired shell's
//!    status bar derives its `proto` cell from
//!    `mili_viz_proto::v1::PROTOCOL_VERSION` (its **major** — the
//!    contract identity `Hello` negotiates), so it reads `proto v1`
//!    today exactly as the old hard-coded literal (the default
//!    `ShellState` composite seam is unmoved — VB-001); the not-
//!    attached default carries **no** peer cell (byte-stable), and
//!    only the attached state gains the honest local `(1 peer)`.
//!  * `composite_render` — headless / skip-on-absent: the default
//!    not-attached `ShellState` still composites over the unchanged
//!    mesh pass (the status-bar text is drawn even when collapsed, so
//!    this guards the VB-001 seam against a stray peer/proto cell).
//!
//! Compile-time, not negotiated, is a deliberate scope call: the
//! in-process `Session` is the only transport and never runs `Hello`,
//! so the constant *is* the truth with no new runtime state — see the
//! `wireframe-parity.md` "Status bar" note. The windowed surface
//! render itself is not headlessly verifiable (no display); the text
//! invariant below is the headless proxy for it.

use std::path::{Path, PathBuf};

use mili_viz_client::{
    build_shell_ui, fetch_server_mesh, render_shell_to_image, Camera, SessionPhase, ShellState,
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

/// Collect every text run egui painted, so we can assert on the
/// status-bar cells without exposing the private builder.
fn painted_text(state: &mut ShellState) -> Vec<String> {
    let ctx = egui::Context::default();
    let raw = egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(
            egui::Pos2::ZERO,
            egui::vec2(1000.0, 700.0),
        )),
        ..Default::default()
    };
    let out = ctx.run_ui(raw, |ui| {
        let _ = build_shell_ui(ui, state);
    });
    let mut texts = Vec::new();
    for cs in &out.shapes {
        if let egui::epaint::Shape::Text(t) = &cs.shape {
            texts.push(t.galley.text().to_owned());
        }
    }
    texts
}

fn has_cell(texts: &[String], needle: &str) -> bool {
    texts.iter().any(|t| t.contains(needle))
}

#[test]
fn proto_cell_tracks_constant_major_and_is_byte_stable() {
    // The contract identity is the major component (`Hello`: "major
    // must match"); the displayed cell must be `proto v<major>`.
    let major = mili_viz_proto::v1::PROTOCOL_VERSION
        .split('.')
        .next()
        .unwrap();
    let expected = format!("proto v{major}");
    // The pre-change literal was exactly `proto v1`; the constant is
    // `1.0.0`, so the de-hard-coded cell must still be byte-identical.
    assert_eq!(expected, "proto v1", "default composite seam is unmoved");

    let mut s = ShellState::default();
    let texts = painted_text(&mut s);
    assert!(
        has_cell(&texts, &expected),
        "status bar must show {expected:?}; painted: {texts:?}"
    );
}

#[test]
fn not_attached_default_has_no_peer_cell() {
    // Default `ShellState` is `NotAttached`; this is the byte-stable
    // composite path (VB-001). It must render exactly as before: the
    // `— not attached —` cell, the proto cell, no peer cell.
    let mut s = ShellState::default();
    assert_eq!(
        s.phase,
        SessionPhase::NotAttached,
        "default is not-attached"
    );
    let texts = painted_text(&mut s);
    assert!(has_cell(&texts, "— not attached —"), "got: {texts:?}");
    assert!(
        !texts.iter().any(|t| t.contains("peer")),
        "not-attached must carry no peer cell (byte-stable): {texts:?}"
    );
}

#[test]
fn attached_state_gains_honest_local_peer_count() {
    // Enriching only the attached state: the local in-process session
    // is exactly one peer, so the truthful minimal is `(1 peer)`.
    let mut s = ShellState {
        phase: SessionPhase::AttachedIdle,
        ..ShellState::default()
    };
    let texts = painted_text(&mut s);
    assert!(
        has_cell(&texts, "(1 peer)"),
        "attached peer cell: {texts:?}"
    );
    assert!(
        has_cell(&texts, "proto v1"),
        "proto cell unchanged when attached: {texts:?}"
    );
}

#[tokio::test]
async fn composite_render() {
    // The status bar is drawn even when the dock is collapsed, so the
    // default not-attached composite must stay a valid frame (the
    // VB-001 seam): no peer/proto cell may perturb the mesh pass.
    let path = corpus_path(&["serial", "basic1", "basic1.pltA"]);
    if !path.exists() {
        eprintln!("skip: serial/basic1 absent (run scripts/setup-parity.sh)");
        return;
    }
    let mesh = fetch_server_mesh(&path.to_string_lossy(), "")
        .await
        .expect("in-process load/show yields a decoded hull");
    let (w, h) = (200u32, 200u32);
    let (center, radius) = mesh.bounds();
    let camera = Camera::looking_at(center, radius);

    let mut s = ShellState::default();
    let Some(px) = render_shell_to_image(w, h, &camera, &mesh, None, &mut s) else {
        eprintln!(
            "skip: no wgpu adapter (no GPU / software rasterizer) — \
             skip-on-absent per CLAUDE.md / phase-5-m3.md Decision 46"
        );
        return;
    };
    let at = |x: u32, y: u32| {
        let i = ((y * w + x) * 4) as usize;
        [px[i], px[i + 1], px[i + 2]]
    };
    let c = at(w / 2, h / 2);
    assert!(
        c.iter().copied().max().unwrap() > 60,
        "default not-attached composite still shows the mesh: {c:?}"
    );
}
