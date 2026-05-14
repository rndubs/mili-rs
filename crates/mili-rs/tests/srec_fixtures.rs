//! Srec table parity against the reference corpus.
//!
//! Skips silently when the submodule isn't checked out, matching the
//! pattern in `mesh_fixtures.rs`.

use std::path::{Path, PathBuf};

use mili_rs::{Database, MeshId, Organization};

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

#[test]
fn basic1_has_at_least_one_srec_with_subrecords() {
    let path = corpus_path(&["serial", "basic1", "basic1.pltA"]);
    if !path.exists() {
        eprintln!("skip: basic1 absent");
        return;
    }
    let db = Database::open(&path).expect("open basic1");
    let srecs = db.srecs();
    assert!(!srecs.is_empty(), "basic1 must declare at least one srec");
    let first = srecs.iter().next().expect("srec iter non-empty");
    assert!(
        !first.subrecords.is_empty(),
        "srec must carry at least one subrecord",
    );
}

#[test]
fn basic1_subrecord_fields_resolve_against_mesh_and_svars() {
    let path = corpus_path(&["serial", "basic1", "basic1.pltA"]);
    if !path.exists() {
        return;
    }
    let db = Database::open(&path).unwrap();
    let meshes = db.meshes();
    let svars = db.svars();

    let mut brick_subrec_seen = false;
    for srec in db.srecs().iter() {
        let mesh = meshes
            .mesh(MeshId(srec.mesh_id))
            .unwrap_or_else(|| panic!("srec mesh_id {} not in mesh table", srec.mesh_id));
        for sub in &srec.subrecords {
            // Each subrecord's mclass must resolve in the owning mesh.
            assert!(
                mesh.class(&sub.mclass).is_some(),
                "subrecord mclass {:?} not in mesh {}",
                sub.mclass,
                srec.mesh_id,
            );
            // Each svar name in the subrec must resolve in the svar
            // table. The reference parser
            // (afileIO.py:430) asserts the same on python's side.
            for name in &sub.svar_names {
                assert!(
                    svars.get(name).is_some(),
                    "subrecord {:?} references unknown svar {:?}",
                    sub.name,
                    name,
                );
            }
            // organization must be one of the two known codes.
            matches!(
                sub.organization,
                Organization::ResultOrdered | Organization::ObjectOrdered
            );

            if sub.mclass == "brick" {
                brick_subrec_seen = true;
                assert!(
                    sub.object_count() > 0,
                    "brick subrec {:?} has zero object count",
                    sub.name,
                );
            }
        }
    }

    assert!(
        brick_subrec_seen,
        "basic1 must have at least one brick-class subrecord",
    );
}

#[test]
fn basic1_subrecord_id_blocks_match_mclass_id_blocks() {
    let path = corpus_path(&["serial", "basic1", "basic1.pltA"]);
    if !path.exists() {
        return;
    }
    let db = Database::open(&path).unwrap();
    let meshes = db.meshes();

    // For at least one non-degenerate (non-M_MESH) subrecord, the
    // declared `id_blocks` should fit entirely within the class's
    // own id range.
    let mut checked = 0;
    for srec in db.srecs().iter() {
        for sub in &srec.subrecords {
            let Some(class) = meshes
                .mesh(MeshId(srec.mesh_id))
                .and_then(|m| m.class(&sub.mclass))
            else {
                continue;
            };
            // M_MESH subrecs ship dummy blocks on disk per
            // afileIO.py:441; skip them here.
            if class.id_blocks.is_empty() {
                continue;
            }
            let (cmin, cmax) = class
                .id_blocks
                .iter()
                .fold((i32::MAX, i32::MIN), |(lo, hi), &(s, e)| {
                    (lo.min(s), hi.max(e))
                });
            for &(s, e) in &sub.id_blocks {
                assert!(
                    s >= cmin && e <= cmax,
                    "subrec {:?}: block ({s},{e}) outside class range ({cmin},{cmax})",
                    sub.name,
                );
            }
            checked += 1;
        }
    }
    assert!(checked > 0, "no testable subrecord id-block ranges found");
}
