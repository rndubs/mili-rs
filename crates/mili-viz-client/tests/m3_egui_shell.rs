//! Phase 5 M3 gating test (`phase-5-m3.md` § "Acceptance gate").
//!
//! Two halves, per Decision 46 (the M1/M2 `*_renderer.rs` shape):
//!  * always-on — the colormap + `MVG2` scalar decode + the pure,
//!    GPU-free `ShellState`/`build_shell_ui` logic. A no-GPU CI box
//!    hard-gates these.
//!  * `composite_render` — the end-to-end path: spawn the in-process
//!    server, `load`/`show` `serial/basic1`, decode, then render the
//!    mesh pass **and** the additive `egui` pass into one off-screen
//!    texture. **Skip-on-absent** when the corpus or a `wgpu` adapter
//!    is missing (CLAUDE.md convention; not a failure).

use std::path::{Path, PathBuf};

use mili_viz_client::{
    build_shell_ui, colormap_normalize, colormap_sample, decode_mvg, fetch_server_mesh,
    render_shell_to_image, Camera, ResultInfo, SessionPhase, ShellState,
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

/// Synthetic `MVG2` blob: `MVG1` body + trailing per-vertex scalar
/// (`phase-4-m2.md` Decision 11 superset).
fn mvg2(positions: &[[f32; 3]], indices: &[u32], trimat: &[u32], scalar: &[f32]) -> Vec<u8> {
    let mut b = Vec::new();
    b.extend_from_slice(b"MVG2");
    b.extend_from_slice(&3u32.to_le_bytes());
    b.extend_from_slice(&(positions.len() as u64).to_le_bytes());
    b.extend_from_slice(&(indices.len() as u64).to_le_bytes());
    for p in positions {
        for c in p {
            b.extend_from_slice(&c.to_le_bytes());
        }
    }
    for i in indices {
        b.extend_from_slice(&i.to_le_bytes());
    }
    for m in trimat {
        b.extend_from_slice(&m.to_le_bytes());
    }
    for s in scalar {
        b.extend_from_slice(&s.to_le_bytes());
    }
    b
}

#[test]
fn colormap_is_monotone_and_clamped() {
    let lo = colormap_sample(0.0);
    let hi = colormap_sample(1.0);
    // Cool end is blue-dominant, warm end is red-dominant.
    assert!(lo[2] > lo[0], "cool end should be blue-leaning: {lo:?}");
    assert!(hi[0] > hi[2], "warm end should be red-leaning: {hi:?}");
    // Out-of-range is clamped, not wrapped.
    let close = |a: [f32; 3], b: [f32; 3]| a.iter().zip(b).all(|(x, y)| (x - y).abs() < 1e-6);
    assert!(
        close(colormap_sample(-1.0), lo),
        "negative t clamps to cool"
    );
    assert!(close(colormap_sample(2.0), hi), "t>1 clamps to warm");
    // Degenerate range maps to the ramp midpoint (no NaN).
    assert!((colormap_normalize(5.0, 1.0, 1.0) - 0.5).abs() < 1e-6);
    assert!((colormap_normalize(2.0, 0.0, 4.0) - 0.5).abs() < 1e-6);
}

#[test]
fn mvg2_scalar_is_now_decoded() {
    // M2 ignored the MVG2 scalar; M3 keeps it (Decision 47).
    let positions = [[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]];
    let indices = [0u32, 1, 2];
    let scalar = [10.0f32, 20.0, 30.0];
    let blob = mvg2(&positions, &indices, &[7], &scalar);

    let mesh = decode_mvg(&blob).expect("synthetic MVG2 decodes");
    assert_eq!(mesh.positions, positions);
    assert_eq!(
        mesh.scalars.as_deref(),
        Some(&scalar[..]),
        "M3 keeps the per-vertex MVG2 scalar"
    );

    // A bare MVG1 still decodes with no scalar.
    let mut bare = blob.clone();
    bare[3] = b'1';
    bare.truncate(bare.len() - scalar.len() * 4);
    let bare_mesh = decode_mvg(&bare).expect("MVG1 still decodes");
    assert!(bare_mesh.scalars.is_none(), "no scalar on a bare MVG1 hull");
}

#[test]
fn shell_state_logic_is_pure_and_gpu_free() {
    let mut s = ShellState::default();
    assert_eq!(s.phase, SessionPhase::NotAttached);
    assert_eq!(s.total_states(), 1);
    assert!(s.state_time().is_none());
    // All five overlays default on.
    assert!(
        s.overlays.title
            && s.overlays.state
            && s.overlays.legend
            && s.overlays.axes
            && s.overlays.bbox
    );

    s.loaded = Some(mili_viz_client::LoadedInfo {
        db: "basic1".into(),
        num_states: 96,
        state_times: vec![0.0, 1e-3, 2e-3],
        class_names: vec!["brick".into()],
    });
    s.phase = SessionPhase::AttachedIdle;
    s.state = 3;
    assert_eq!(s.total_states(), 96);
    assert_eq!(s.state_time(), Some(2e-3));
    s.result = Some(ResultInfo {
        name: "eff_stress".into(),
        component: String::new(),
        min: 0.0,
        max: 5.0,
        num_vertices: 12,
        num_indices: 18,
    });

    // The layout is a pure fn of state: it runs head­lessly (no GPU,
    // no transport) against synthetic RawInput and, with no pointer
    // input, emits no transport actions but produces paint shapes.
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
    assert!(
        !out.shapes.is_empty(),
        "the L1 shell + overlays must paint something"
    );

    // The not-attached card path also runs without panicking.
    let mut s2 = ShellState::default();
    let _ = ctx.run_ui(egui::RawInput::default(), |ui| {
        let _ = build_shell_ui(ui, &mut s2);
    });
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

    let (w, h) = (200u32, 160u32);
    let (center, radius) = mesh.bounds();
    let camera = Camera::looking_at(center, radius);

    let mut state = ShellState {
        phase: SessionPhase::AttachedIdle,
        loaded: Some(mili_viz_client::LoadedInfo {
            db: path.to_string_lossy().into_owned(),
            num_states: 1,
            state_times: vec![0.0],
            class_names: vec!["brick".into()],
        }),
        ..ShellState::default()
    };

    let Some(px) = render_shell_to_image(w, h, &camera, &mesh, None, &mut state) else {
        eprintln!(
            "skip: no wgpu adapter (no GPU / software rasterizer) — \
             skip-on-absent per phase-5-m3.md Decision 46"
        );
        return;
    };
    assert_eq!(px.len() as u32, w * h * 4);

    let at = |x: u32, y: u32| {
        let i = ((y * w + x) * 4) as usize;
        [px[i], px[i + 1], px[i + 2]]
    };

    // The viewport centre is still the rendered mesh (the egui pass
    // is additive and the CentralPanel is transparent — Decision 45).
    let center_px = at(w / 2, h / 2);
    assert!(
        center_px.iter().copied().max().unwrap() > 60,
        "viewport centre should be the mesh, got {center_px:?}"
    );

    // A column deep inside the left dock is opaque egui chrome: it is
    // neither the dark clear colour (a bare corner would be < 40) nor
    // a transparent passthrough — the dock panel painted over it.
    // Scan a vertical strip ~40 px in (well within the 230 px dock)
    // and require at least one clearly-chrome pixel.
    let chrome = (40..h - 30).any(|y| {
        let p = at(40, y);
        let mx = p.iter().copied().max().unwrap();
        let mn = p.iter().copied().min().unwrap();
        // egui's dark panel grey is a low-but-nonzero neutral.
        (20..120).contains(&mx) && (mx - mn) < 40
    });
    assert!(
        chrome,
        "the left dock chrome must composite over the mesh pass"
    );
}
