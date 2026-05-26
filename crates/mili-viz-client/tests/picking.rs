//! Client-side picking gating test (wireframe-parity "Picking" /
//! MVP-cut 4). All **always-on** pure logic — picking is GPU-free
//! ray-cast against the cached hull; the windowed click path is not
//! headlessly verifiable in CI (no display), so the ray math
//! ([`Camera::ray_from_screen`]), the hull intersection
//! ([`Mesh::pick`]) and the [`ShellState`] readout are tested
//! directly, mirroring the m4 pattern.

use mili_viz_client::{
    build_shell_ui, decode_catalog, decode_mvg, Camera, ClassMembership, Mesh, ResultCatalog,
    ShellState, UiAction,
};

/// A 2×2 quad in the z=0 plane, facing -Z (so a camera on +Z sees it).
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
        element_edges: None,
        tri_flags: None,
        tri_member_id: None,
    }
}

#[test]
fn ray_through_viewport_centre_hits_the_framed_hull() {
    let mesh = quad();
    let (center, radius) = mesh.bounds();
    let camera = Camera::looking_at(center, radius);
    let (w, h) = (200u32, 200u32);

    // The centre pixel of a framed model must hit it.
    let (o, d) = camera.ray_from_screen(100.0, 100.0, w, h);
    let hit = mesh.pick(o, d).expect("centre ray hits the framed quad");
    assert!(hit.distance > 0.0, "a forward hit");
    // The hit point is on the z=0 plane the quad lives in.
    assert!(hit.point[2].abs() < 1e-3, "hit on the quad plane: {hit:?}");
    assert!(hit.scalar.is_some(), "MVG2 scalar carried through");

    // A ray out at a far corner misses the hull entirely.
    let (o2, d2) = camera.ray_from_screen(1.0, 1.0, w, h);
    assert!(
        mesh.pick(o2, d2).is_none(),
        "a corner ray misses the centred quad"
    );
}

#[test]
fn pick_reports_nearest_node_and_is_two_sided() {
    let mesh = quad();
    // Straight down -Z onto the +X/+Y corner: nearest node is index 2
    // (1,1,0); the ray hits regardless of triangle winding.
    let hit = mesh
        .pick(glam::vec3(0.9, 0.9, 5.0), glam::vec3(0.0, 0.0, -1.0))
        .expect("hits near the (1,1) corner");
    assert_eq!(hit.node, 2, "nearest node is the (1,1,0) corner");
    assert_eq!(hit.scalar, Some(30.0), "scalar at node 2");

    // From behind (-Z side) the same hull still picks (two-sided, like
    // the no-cull renderer).
    let back = mesh.pick(glam::vec3(0.0, 0.0, -5.0), glam::vec3(0.0, 0.0, 1.0));
    assert!(back.is_some(), "two-sided pick from behind");
}

#[test]
fn shell_picking_toggle_and_readout_are_pure() {
    let mut s = ShellState::default();
    assert!(!s.picking, "off by default — status bar stays —");
    assert_eq!(s.pick, "—");

    let a = s.toggle_picking();
    assert!(s.picking);
    assert_eq!(a, UiAction::TogglePicking, "observability-only");

    // (0.6,0.9): y>x ⇒ the upper-left triangle (tri 1, verts 0,2,3);
    // nearest corner is node 2 (1,1,0) → scalar 30.
    let hit = quad()
        .pick(glam::vec3(0.6, 0.9, 5.0), glam::vec3(0.0, 0.0, -1.0))
        .unwrap();
    s.apply_pick(Some(&hit));
    assert_eq!(s.pick, "node 2 · tri 1 · v=3.000e1");
    s.apply_pick(None);
    assert_eq!(s.pick, "(no hit)");

    // Turning picking off resets the readout.
    s.toggle_picking();
    assert!(!s.picking);
    assert_eq!(s.pick, "—");

    // The pure layout still runs head­lessly with picking on and emits
    // no actions without pointer input (the m3/m4 pattern).
    s.toggle_picking();
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
    assert!(!out.shapes.is_empty(), "the L1 shell must still paint");
}

// ──────────────────────────────────────────────────────────────────────
// Wireframe-parity #6 path (a): per-tri `member_id` + catalog resolve.
// ──────────────────────────────────────────────────────────────────────

/// Same 2-tri quad as `quad()`, but with the catalog-provided
/// per-triangle owner ids set so a pick resolves to a known
/// (class_name, label) via `ResultCatalog::resolve_member`.
fn quad_with_members() -> Mesh {
    let mut m = quad();
    // Both tris belong to class_idx=0 (`brick`), but to different
    // element rows so the pick distinguishes them.
    // member_id encoding: `class_idx << 24 | elem_row`. Both tris are
    // class_idx=0 (`brick`); only elem_row differs.
    let id_tri0: u32 = 0; // (brick, elem_row=0)
    let id_tri1: u32 = 1; // (brick, elem_row=1)
    m.tri_member_id = Some(vec![id_tri0, id_tri1]);
    m
}

fn brick_catalog() -> ResultCatalog {
    ResultCatalog {
        primal: Vec::new(),
        derived: Vec::new(),
        classes: vec![ClassMembership {
            class_idx: 0,
            name: "brick".to_string(),
            // Element labels: tri 0 ⇒ row 0 ⇒ label 41; tri 1 ⇒ row 1
            // ⇒ label 42. Picked element identity follows the label.
            labels: vec![41, 42],
        }],
    }
}

#[test]
fn pick_carries_member_id_when_geometry_blob_does() {
    let mesh = quad_with_members();
    // Aim at the lower-right triangle (tri 0, verts 0,1,2): hit near
    // the (1,0) edge interior.
    let hit = mesh
        .pick(glam::vec3(0.6, -0.4, 5.0), glam::vec3(0.0, 0.0, -1.0))
        .expect("centre ray hits the quad");
    assert_eq!(hit.tri, 0, "lower-right tri");
    assert_eq!(hit.member_id, Some(0), "carries member id of tri 0");

    // Upper-left triangle (tri 1, verts 0,2,3) — y>x.
    let hit_b = quad_with_members()
        .pick(glam::vec3(0.0, 0.6, 5.0), glam::vec3(0.0, 0.0, -1.0))
        .expect("upper-left tri hit");
    assert_eq!(hit_b.tri, 1);
    assert_eq!(hit_b.member_id, Some(1), "tri 1's member id");
}

#[test]
fn pick_omits_member_id_when_blob_has_no_column() {
    // The legacy `quad()` helper leaves `tri_member_id: None` — a
    // pick must report no member id so the shell readout falls back
    // to the `tri T · node N` form.
    let hit = quad()
        .pick(glam::vec3(0.6, -0.4, 5.0), glam::vec3(0.0, 0.0, -1.0))
        .expect("hits");
    assert!(hit.member_id.is_none(), "no column → no member");
}

#[test]
fn pick_omits_member_id_for_cap_sentinel() {
    let mut mesh = quad_with_members();
    // Stamp the cap sentinel on tri 0 — pick must surface `None`
    // (caps have no owning element).
    if let Some(m) = mesh.tri_member_id.as_mut() {
        m[0] = u32::MAX;
    }
    let hit = mesh
        .pick(glam::vec3(0.6, -0.4, 5.0), glam::vec3(0.0, 0.0, -1.0))
        .expect("hits");
    assert_eq!(hit.tri, 0);
    assert!(hit.member_id.is_none(), "cap sentinel surfaces as None");
}

#[test]
fn catalog_resolve_member_unpacks_class_and_label() {
    let cat = brick_catalog();
    assert_eq!(cat.resolve_member(0), Some(("brick", 41)));
    assert_eq!(cat.resolve_member(1), Some(("brick", 42)));
    // Out-of-range elem_row → None.
    assert!(cat.resolve_member(99).is_none());
    // Unknown class_idx → None.
    // class_idx=5 (unknown), elem_row=0.
    assert!(cat.resolve_member(5u32 << 24).is_none());
}

#[test]
fn shell_apply_pick_uses_catalog_when_member_resolves() {
    let mut s = ShellState {
        catalog: Some(brick_catalog()),
        ..ShellState::default()
    };
    s.toggle_picking();
    // Hit tri 0 → (brick, 41).
    let hit = quad_with_members()
        .pick(glam::vec3(0.6, -0.4, 5.0), glam::vec3(0.0, 0.0, -1.0))
        .unwrap();
    s.apply_pick(Some(&hit));
    assert_eq!(s.pick, "brick 41 · v=2.000e1", "resolved member + scalar");
    // The same resolve also lights the Plot tab's "+ pick" button
    // (`wireframe-parity.md` #4 picking-driven variant).
    assert_eq!(s.picked_element, Some(("brick".to_string(), 41)));

    // Hit tri 1 → (brick, 42).
    let hit_b = quad_with_members()
        .pick(glam::vec3(0.0, 0.6, 5.0), glam::vec3(0.0, 0.0, -1.0))
        .unwrap();
    s.apply_pick(Some(&hit_b));
    assert_eq!(s.pick, "brick 42 · v=3.000e1");
    assert_eq!(s.picked_element, Some(("brick".to_string(), 42)));

    // A miss clears the picked-element identity so the button greys
    // out again — no stale pick keeps it live.
    s.apply_pick(None);
    assert_eq!(s.pick, "(no hit)");
    assert!(s.picked_element.is_none());

    // Turning picking off also clears the identity (already covered
    // by the legacy test for `pick`, extended here for the new field).
    s.apply_pick(Some(&hit));
    assert_eq!(s.picked_element, Some(("brick".to_string(), 41)));
    s.toggle_picking();
    assert!(s.picked_element.is_none());
}

#[test]
fn shell_apply_pick_falls_back_when_catalog_lacks_member() {
    // Catalog present but no classes → member_id can't resolve →
    // legacy `tri T · node N` readout.
    let mut s = ShellState {
        catalog: Some(ResultCatalog::default()),
        ..ShellState::default()
    };
    s.toggle_picking();
    let hit = quad_with_members()
        .pick(glam::vec3(0.6, -0.4, 5.0), glam::vec3(0.0, 0.0, -1.0))
        .unwrap();
    s.apply_pick(Some(&hit));
    assert!(s.pick.starts_with("node "), "fallback readout: {}", s.pick);
    // The catalog couldn't resolve the picked tri's owning element,
    // so the picked-element identity is left empty — the Plot tab's
    // "+ pick" button stays greyed out (no false-positive identity).
    assert!(s.picked_element.is_none());
}

#[test]
fn decode_catalog_parses_m_tag_and_tolerates_unknown_tags() {
    // Hand-built blob: magic + a P, a D, an M row, and a Z unknown
    // tag (must drop silently). The M row carries class_idx 2, name
    // `beam`, labels 7,8,9.
    let mut blob = b"MVCAT1\n".to_vec();
    blob.extend_from_slice(b"P\tsx\n");
    blob.extend_from_slice(b"D\teff_stress\n");
    blob.extend_from_slice(b"M\t2\tbeam\t7,8,9\n");
    blob.extend_from_slice(b"Z\tfrom_the_future\n");
    blob.extend_from_slice(b"M\t0\tbrick\t1,2\n");
    let cat = decode_catalog(&blob).expect("MVCAT1 blob parses");
    assert_eq!(cat.primal, vec!["sx"]);
    assert_eq!(cat.derived, vec!["eff_stress"]);
    assert_eq!(cat.classes.len(), 2, "Z tag dropped, both M rows kept");
    assert_eq!(cat.classes[0].class_idx, 2);
    assert_eq!(cat.classes[0].name, "beam");
    assert_eq!(cat.classes[0].labels, vec![7, 8, 9]);
    assert_eq!(cat.classes[1].class_idx, 0);
    assert_eq!(cat.classes[1].name, "brick");
    assert_eq!(cat.classes[1].labels, vec![1, 2]);
}

#[test]
fn decode_catalog_drops_malformed_m_rows() {
    let mut blob = b"MVCAT1\n".to_vec();
    blob.extend_from_slice(b"M\tnot_a_number\tbeam\t1,2\n"); // bad class_idx
    blob.extend_from_slice(b"M\t1\t\t1,2\n"); // empty name
    blob.extend_from_slice(b"M\t1\tbeam\t\n"); // empty labels
    blob.extend_from_slice(b"M\t1\tbeam\t1,not,3\n"); // bad label
    let cat = decode_catalog(&blob).expect("magic parses");
    assert!(cat.classes.is_empty(), "every malformed M row dropped");
}

#[test]
fn mvg3_blob_round_trips_member_id_column() {
    // Hand-build a minimal MVG3 blob with bit 4 set so decode_mvg
    // populates tri_member_id. Two-tri quad in the z=0 plane; the
    // member id column carries two known packed values.
    let n_verts: u64 = 4;
    let n_idx: u64 = 6;
    let n_edges: u64 = 0;
    let flags_mask: u32 = 2 | 16; // tri_flags + tri_member_id (no scalar/edges)
    let mut blob: Vec<u8> = Vec::new();
    blob.extend_from_slice(b"MVG3");
    blob.extend_from_slice(&3u32.to_le_bytes());
    blob.extend_from_slice(&n_verts.to_le_bytes());
    blob.extend_from_slice(&n_idx.to_le_bytes());
    blob.extend_from_slice(&n_edges.to_le_bytes());
    blob.extend_from_slice(&flags_mask.to_le_bytes());
    for &p in &[
        -1.0_f32, -1.0, 0.0, 1.0, -1.0, 0.0, 1.0, 1.0, 0.0, -1.0, 1.0, 0.0,
    ] {
        blob.extend_from_slice(&p.to_le_bytes());
    }
    for &i in &[0u32, 1, 2, 0, 2, 3] {
        blob.extend_from_slice(&i.to_le_bytes());
    }
    for &m in &[7u32, 7] {
        blob.extend_from_slice(&m.to_le_bytes()); // tri_material
    }
    for &f in &[0u32, 0] {
        blob.extend_from_slice(&f.to_le_bytes()); // tri_flags
    }
    let id_tri0 = (3u32 << 24) | 0x0b; // class_idx=3, elem_row=11
    let id_tri1 = u32::MAX; // sentinel
    for &id in &[id_tri0, id_tri1] {
        blob.extend_from_slice(&id.to_le_bytes());
    }
    let mesh = decode_mvg(&blob).expect("MVG3 with member column decodes");
    assert_eq!(
        mesh.tri_member_id.as_ref().expect("column populated"),
        &vec![id_tri0, id_tri1]
    );
    // Pick on tri 1 (with the sentinel) → member_id filtered to None.
    let hit_b = mesh
        .pick(glam::vec3(0.0, 0.6, 5.0), glam::vec3(0.0, 0.0, -1.0))
        .unwrap();
    assert_eq!(hit_b.tri, 1);
    assert!(hit_b.member_id.is_none(), "sentinel pruned");
    // Pick on tri 0 → resolves the real id.
    let hit_a = mesh
        .pick(glam::vec3(0.6, -0.4, 5.0), glam::vec3(0.0, 0.0, -1.0))
        .unwrap();
    assert_eq!(hit_a.tri, 0);
    assert_eq!(hit_a.member_id, Some(id_tri0));
}
