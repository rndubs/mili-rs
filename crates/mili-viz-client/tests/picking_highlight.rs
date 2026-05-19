//! Picking viewport highlight-glyph gating test
//! (`wireframe-parity.md` "Picking" / cross-cutting Picking row;
//! MVP-cut item 4 remainder).
//!
//! Two halves, this crate's `overlays_bbox.rs` / `picking.rs` shape:
//!  * always-on — the pure, GPU-free wiring: `apply_pick` caches the
//!    hit point and a miss / picking-off clears it (no stale marker),
//!    the default is `None` (byte-stable), and `build_shell_ui` with a
//!    live camera + a cached hit paints the extra glyph shapes
//!    input-free (a deterministic shape-count delta proves the glyph
//!    actually draws, no GPU needed). A no-GPU CI box hard-gates these.
//!  * `composite_render` — headless: a picking render with a cached
//!    hit composites the distinctive accent glyph over the unchanged
//!    mesh pass, while the default (picking off) render shows none —
//!    and the default seam stays byte-stable (`bug-tracker.md`
//!    VB-001). **Skip-on-absent** when the corpus / a `wgpu` adapter
//!    is missing (CLAUDE.md convention; not a failure).
//!
//! The windowed click→ray-cast path is not headlessly verifiable in
//! CI; `Mesh::pick` + `Camera::project` are pinned by `picking.rs` /
//! `overlays_bbox.rs`, and this pins the glyph that consumes them.

use std::path::{Path, PathBuf};

use mili_viz_client::{
    build_shell_ui, fetch_server_mesh, render_shell_to_image, Camera, Mesh, SessionPhase,
    ShellState,
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

fn quad() -> Mesh {
    Mesh {
        positions: vec![
            [-1.0, -1.0, 0.0],
            [1.0, -1.0, 0.0],
            [1.0, 1.0, 0.0],
            [-1.0, 1.0, 0.0],
        ],
        normals: vec![[0.0, 0.0, 1.0]; 4],
        indices: vec![0, 1, 2, 0, 2, 3],
        scalars: Some(vec![10.0, 20.0, 30.0, 40.0]),
    }
}

fn shape_count(state: &mut ShellState) -> usize {
    let ctx = egui::Context::default();
    let raw = egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(
            egui::Pos2::ZERO,
            egui::vec2(900.0, 600.0),
        )),
        ..Default::default()
    };
    let mut actions = Vec::new();
    let out = ctx.run_ui(raw, |ui| actions = build_shell_ui(ui, state));
    assert!(actions.is_empty(), "no input ⇒ no actions: {actions:?}");
    assert!(!out.shapes.is_empty(), "the L1 shell must still paint");
    out.shapes.len()
}

#[test]
fn apply_pick_caches_then_clears_the_hit_point() {
    let mut s = ShellState::default();
    assert_eq!(s.pick_point, None, "no marker by default (byte-stable)");

    let hit = quad()
        .pick(glam::vec3(0.0, 0.0, 5.0), glam::vec3(0.0, 0.0, -1.0))
        .expect("centre ray hits the quad");
    s.apply_pick(Some(&hit));
    assert_eq!(
        s.pick_point,
        Some(hit.point),
        "the hit point is cached for the glyph"
    );

    // A miss clears it — no stale marker lingers.
    s.apply_pick(None);
    assert_eq!(s.pick_point, None, "a miss clears the marker");

    // Turning picking off also clears any cached point.
    s.apply_pick(Some(&hit));
    s.picking = true;
    let _ = s.toggle_picking();
    assert!(!s.picking);
    assert_eq!(s.pick_point, None, "picking-off clears the marker");
}

#[test]
fn glyph_adds_shapes_only_with_picking_camera_and_a_hit() {
    let mesh = quad();
    let (c, r) = mesh.bounds();
    let cam = Camera::looking_at(c, r);

    // Baseline: a live camera (so bbox/gizmo project) but picking off
    // and no cached hit — no glyph.
    let mut base = ShellState {
        phase: SessionPhase::AttachedIdle,
        camera: Some(cam),
        model_aabb: Some(mesh.aabb()),
        ..ShellState::default()
    };
    let base_shapes = shape_count(&mut base);

    // Picking on + a cached hit at the framed centre + the same live
    // camera ⇒ the glyph (ring + 2 crosshair segments) is added.
    let mut glyph = ShellState {
        phase: SessionPhase::AttachedIdle,
        camera: Some(cam),
        model_aabb: Some(mesh.aabb()),
        picking: true,
        pick_point: Some([c.x, c.y, c.z]),
        ..ShellState::default()
    };
    let glyph_shapes = shape_count(&mut glyph);
    assert!(
        glyph_shapes > base_shapes,
        "the highlight glyph must add paint shapes: {glyph_shapes} vs {base_shapes}"
    );

    // Picking on but *no* cached hit ⇒ still no glyph (count == base).
    let mut armed = ShellState {
        phase: SessionPhase::AttachedIdle,
        camera: glyph.camera,
        model_aabb: Some(mesh.aabb()),
        picking: true,
        ..ShellState::default()
    };
    assert_eq!(
        shape_count(&mut armed),
        base_shapes,
        "picking armed but no hit ⇒ no glyph yet"
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
        ..ShellState::default()
    };

    // Count pixels close to the accent amber the glyph strokes
    // (255,190,60) — distinctive vs the cool-ramp mesh / grey chrome.
    let amber = |px: &[u8]| -> usize {
        px.chunks_exact(4)
            .filter(|p| p[0] > 215 && (150..=225).contains(&p[1]) && p[2] < 115)
            .count()
    };

    // (a) Default (picking off, no hit) — the byte-stable M3 seam: no
    // glyph at all.
    let mut off = base.clone();
    let Some(off_px) = render_shell_to_image(w, h, &camera, &mesh, None, &mut off) else {
        eprintln!(
            "skip: no wgpu adapter (no GPU / software rasterizer) — \
             skip-on-absent per CLAUDE.md / phase-5-m3.md Decision 46"
        );
        return;
    };
    assert_eq!(off_px.len() as u32, w * h * 4);

    // (b) Picking on + a cached centre hit + a live camera ⇒ the
    // accent glyph composites over the unchanged mesh pass.
    let mut on = base;
    on.camera = Some(camera);
    on.picking = true;
    on.pick_point = Some([center.x, center.y, center.z]);
    let on_px = render_shell_to_image(w, h, &camera, &mesh, None, &mut on)
        .expect("adapter was present for render (a)");

    let (off_amber, on_amber) = (amber(&off_px), amber(&on_px));
    assert!(
        on_amber > off_amber,
        "the highlight glyph must paint accent pixels over the mesh: \
         on={on_amber} off={off_amber}"
    );
    // The mesh is still there under/around the glyph (not occluded).
    let i = (((h / 2) * w + w / 2) * 4) as usize;
    let centre = [on_px[i], on_px[i + 1], on_px[i + 2]];
    assert!(
        centre.iter().copied().max().unwrap() > 60,
        "viewport centre still shows the mesh, got {centre:?}"
    );
}
