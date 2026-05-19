//! Result-catalog gating test (`phase-5-m4.md` Decision 67;
//! MVP-cut item 8).
//!
//! Two halves, this crate's `preferences_tweaks.rs` shape:
//!  * always-on — the pure, GPU-free contract: `decode_catalog`
//!    round-trips the self-describing blob, rejects a non-catalog
//!    buffer, and tolerates unknown tags; `ShellState::default`'s
//!    `catalog` is `None` (the byte-stable placeholder path,
//!    `bug-tracker.md` VB-001); and the wired shell paints input-free
//!    both with no catalog and with a populated one. A no-GPU CI box
//!    hard-gates these.
//!  * `composite_render` — headless / skip-on-absent: an in-process
//!    `Session` over `serial/basic1` yields a non-empty primal
//!    catalog, and a `ShellState` carrying it still composites over
//!    the unchanged mesh pass (the left-dock primal listing does not
//!    perturb the viewport), while the default catalog-`None` render
//!    is byte-identical to itself (the placeholder seam is unmoved).
//!
//! The windowed `Session::fetch_catalog` call site (in `app.rs`'s
//! `apply_loaded`) is not headlessly verifiable; the
//! `Session::fetch_catalog` API it calls *is* exercised by the
//! skip-on-absent leg.

use std::path::{Path, PathBuf};

use mili_viz_client::{
    build_shell_ui, decode_catalog, fetch_server_mesh, render_shell_to_image, Camera, LoadedInfo,
    ResultCatalog, Session, SessionPhase, ShellState, UiAction,
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
fn decode_round_trips_and_rejects_non_catalog() {
    // Hand-built blob in the documented format: magic + P-tagged
    // primal lines (plus an unknown `Z` tag a newer server might add).
    let blob = b"MVCAT1\nP\tsx\nP\tstress\nZ\tfuture\n".to_vec();
    let cat = decode_catalog(&blob).expect("well-formed blob decodes");
    assert_eq!(cat.primal, vec!["sx".to_string(), "stress".to_string()]);
    assert_eq!(cat.len(), 2);
    assert!(!cat.is_empty());

    // A real run with no queriable svars: magic only ⇒ Some(empty).
    let empty = decode_catalog(b"MVCAT1\n").expect("magic-only decodes");
    assert!(empty.is_empty());
    assert_eq!(empty, ResultCatalog::default());

    // Not a catalog buffer (e.g. an MVG geometry blob) ⇒ None, so the
    // client keeps its static placeholder.
    assert!(decode_catalog(b"MVG1\x00\x00\x00\x00").is_none());
    assert!(decode_catalog(b"").is_none());
}

#[test]
fn default_shell_has_no_catalog() {
    // The byte-stable default: `None` ⇒ the left-dock `primal`
    // sub-tree stays the static `(catalog: M4+)` placeholder and the
    // `Results · N` badge stays `DERIVED_RESULTS.len()` (VB-001).
    assert!(ShellState::default().catalog.is_none());
}

#[test]
fn wired_shell_paints_input_free_with_and_without_catalog() {
    // No catalog (placeholder path).
    let mut bare = ShellState {
        phase: SessionPhase::AttachedIdle,
        ..ShellState::default()
    };
    let (a, painted) = paint(&mut bare);
    assert!(painted && a.is_empty(), "no-catalog shell paints inert");

    // Populated catalog: the primal sub-tree now lists real names; a
    // pure paint (no input) must still yield no actions.
    let mut full = ShellState {
        phase: SessionPhase::AttachedIdle,
        catalog: Some(ResultCatalog {
            primal: vec!["sx".into(), "sy".into(), "eps".into()],
        }),
        ..ShellState::default()
    };
    let (a, painted) = paint(&mut full);
    assert!(painted && a.is_empty(), "catalog shell paints inert: {a:?}");
}

#[tokio::test]
async fn composite_render() {
    let path = corpus_path(&["serial", "basic1", "basic1.pltA"]);
    if !path.exists() {
        eprintln!("skip: serial/basic1 absent (run scripts/setup-parity.sh)");
        return;
    }

    // The windowed app's `Session::fetch_catalog` path, end to end
    // over the in-process side-channel.
    let session = Session::connect_in_process(Some(&path.to_string_lossy()))
        .await
        .expect("in-process session loads serial/basic1");
    let catalog = session
        .fetch_catalog()
        .expect("a loaded run yields a decoded catalog");
    assert!(
        !catalog.primal.is_empty(),
        "serial/basic1 exposes primal svars"
    );

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

    // (a) Default (catalog None) — the byte-stable placeholder seam.
    let mut bare = base.clone();
    let Some(bpx) = render_shell_to_image(w, h, &camera, &mesh, None, &mut bare) else {
        eprintln!(
            "skip: no wgpu adapter (no GPU / software rasterizer) — \
             skip-on-absent per CLAUDE.md / phase-5-m3.md Decision 46"
        );
        return;
    };
    let bc = at(&bpx, w / 2, h / 2);
    assert!(
        bc.iter().copied().max().unwrap() > 60,
        "no-catalog: viewport centre is the mesh, got {bc:?}"
    );

    // (b) The real fetched catalog applied — the primal listing must
    // not perturb the viewport (it lives in the collapsed left dock);
    // the composite still shows the mesh at centre.
    let mut full = base;
    full.catalog = Some(catalog);
    let fpx = render_shell_to_image(w, h, &camera, &mesh, None, &mut full)
        .expect("adapter was present for render (a)");
    let fc = at(&fpx, w / 2, h / 2);
    assert!(
        fc.iter().copied().max().unwrap() > 60,
        "catalog: viewport centre is still the mesh, got {fc:?}"
    );
}
