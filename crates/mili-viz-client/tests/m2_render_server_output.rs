//! Phase 5 M2 gating test (`phase-5-m2.md` § "Acceptance gate").
//!
//! Two halves, per Decision 43 (the M1 `m1_renderer.rs` shape):
//!  * `decode_mvg_*` — pure blob decode, **always runs** (no GPU, no
//!    corpus; a no-GPU CI box hard-gates this).
//!  * `render_server_output` — the end-to-end path: spawn the
//!    in-process server, `load`/`show` the `serial/basic1` corpus,
//!    resolve the `GeometryRef`, decode, render headless.
//!    **Skip-on-absent** when the corpus fixture **or** a `wgpu`
//!    adapter is missing (CLAUDE.md skip-on-absent convention; not a
//!    failure).

use std::path::{Path, PathBuf};

use mili_viz_client::{decode_mvg, fetch_server_mesh, render_mesh_to_image, Camera};

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

/// Build a synthetic `MVG1` blob (`phase-4-m2.md` Decision 11).
fn mvg1(positions: &[[f32; 3]], indices: &[u32], trimat: &[u32]) -> Vec<u8> {
    let mut b = Vec::new();
    b.extend_from_slice(b"MVG1");
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
    b
}

#[test]
fn decode_mvg_roundtrips_positions_indices_and_yields_unit_normals() {
    // A unit quad in the z=0 plane, two triangles.
    let positions = [
        [0.0, 0.0, 0.0],
        [1.0, 0.0, 0.0],
        [1.0, 1.0, 0.0],
        [0.0, 1.0, 0.0],
    ];
    let indices = [0u32, 1, 2, 0, 2, 3];
    let blob = mvg1(&positions, &indices, &[7, 7]);

    let mesh = decode_mvg(&blob).expect("synthetic MVG1 decodes");
    assert_eq!(mesh.positions, positions);
    assert_eq!(mesh.indices, indices);
    assert_eq!(mesh.normals.len(), positions.len(), "one normal per vertex");
    // The quad is planar +Z; every accumulated normal is unit length.
    for n in &mesh.normals {
        let len = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
        assert!((len - 1.0).abs() < 1e-4, "normal not unit: {n:?}");
        assert!(n[2].abs() > 0.99, "z-plane normal should be ±Z: {n:?}");
    }
}

#[test]
fn decode_mvg_rejects_bad_magic_and_truncation() {
    assert!(decode_mvg(b"NOPE........................").is_err());
    let mut blob = mvg1(&[[0.0, 0.0, 0.0]], &[], &[]);
    blob.truncate(20);
    assert!(decode_mvg(&blob).is_err(), "truncated header is rejected");
}

#[tokio::test]
async fn render_server_output() {
    let path = corpus_path(&["serial", "basic1", "basic1.pltA"]);
    if !path.exists() {
        eprintln!("skip: serial/basic1 absent (run scripts/setup-parity.sh)");
        return;
    }

    // In-process server: load + show + resolve the GeometryRef +
    // decode (phase-5-m2.md Decision 41). An empty result name is the
    // M2 no-scalar hull view (phase-4-m2.md Decision 12).
    let mesh = fetch_server_mesh(&path.to_string_lossy(), "")
        .await
        .expect("in-process load/show yields a decoded hull");
    assert!(!mesh.positions.is_empty(), "basic1 hull has vertices");
    assert!(
        !mesh.indices.is_empty() && mesh.indices.len().is_multiple_of(3),
        "hull is a triangle list"
    );
    assert!(
        mesh.indices
            .iter()
            .all(|&i| (i as usize) < mesh.positions.len()),
        "every triangle index is in range"
    );
    assert_eq!(mesh.normals.len(), mesh.positions.len());

    let (w, h) = (96u32, 96u32);
    let (center, radius) = mesh.bounds();
    let camera = Camera::looking_at(center, radius);
    let Some(px) = render_mesh_to_image(w, h, &camera, &mesh) else {
        eprintln!(
            "skip: no wgpu adapter (no GPU / software rasterizer) — \
             skip-on-absent per phase-5-m2.md Decision 43"
        );
        return;
    };
    assert_eq!(px.len() as u32, w * h * 4);

    let at = |x: u32, y: u32| {
        let i = ((y * w + x) * 4) as usize;
        [px[i], px[i + 1], px[i + 2]]
    };

    // A corner is the background clear color (dark: ~5,5,20 in u8).
    let corner = at(1, 1);
    assert!(
        corner.iter().all(|&c| c < 40),
        "corner should be the clear color, got {corner:?}"
    );

    // The auto-framed bounding sphere fills the viewport, so the
    // center ray hits the hull — it is the lit mesh, not the clear
    // color (shaded base * ambient is ≳80 on the brightest channel).
    let center_px = at(w / 2, h / 2);
    assert!(
        center_px.iter().copied().max().unwrap() > 60,
        "center should be the rendered mesh, got {center_px:?}"
    );
}
