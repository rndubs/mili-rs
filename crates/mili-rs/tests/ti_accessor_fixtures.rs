//! High-level TI accessor parity against the reference corpus.
//!
//! Skips when `reference/mili-python/tests/data/...` isn't checked out
//! so partial checkouts stay green.

use std::path::{Path, PathBuf};

use mili_rs::{Database, MeshId};

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
fn basic1_node_labels_concatenate_to_node_count() {
    let path = corpus_path(&["serial", "basic1", "basic1.pltA"]);
    if !path.exists() {
        eprintln!("skip: basic1 absent");
        return;
    }
    let db = Database::open(&path).expect("open basic1");
    let labels = db
        .labels(MeshId(0), "node")
        .expect("decode labels")
        .expect("basic1 declares Node Labels for mesh 0");
    // basic1 has 1400 nodes (verified in mesh_fixtures + param_fixtures).
    assert_eq!(labels.len(), 1400);
    // Canonical 1-based id range — every node label is present and in
    // [1, 1400], with the first slot equal to 1.
    assert_eq!(labels[0], 1);
    assert!(labels.iter().all(|&l| (1..=1400).contains(&l)));
}

#[test]
fn basic1_element_labels_concatenate_for_brick_class() {
    let path = corpus_path(&["serial", "basic1", "basic1.pltA"]);
    if !path.exists() {
        return;
    }
    let db = Database::open(&path).unwrap();
    let mesh = db.meshes().mesh(MeshId(0)).unwrap();
    // Find a non-nodal class; basic1 has a "brick" M_HEX class.
    let hex_class_name = mesh
        .classes()
        .find(|c| c.superclass == mili_rs::Superclass::Hex)
        .map(|c| c.short_name.clone())
        .expect("basic1 has a hex class");

    let labels = db
        .labels(MeshId(0), &hex_class_name)
        .expect("decode element labels")
        .expect("basic1 declares Element Labels for the hex class");
    let class = mesh.class(&hex_class_name).unwrap();
    // Label count equals the class's element count across all id_blocks.
    assert_eq!(labels.len() as u64, class.element_count());
}

#[test]
fn basic1_labels_for_unknown_class_returns_none() {
    let path = corpus_path(&["serial", "basic1", "basic1.pltA"]);
    if !path.exists() {
        return;
    }
    let db = Database::open(&path).unwrap();
    let r = db
        .labels(MeshId(0), "no_such_class_xyz")
        .expect("labels lookup");
    assert!(r.is_none());
}

#[test]
fn dbl_nodtang_materials_contains_mat_name_1() {
    let path = corpus_path(&["serial", "dbl_nodtang", "dblplt000A"]);
    if !path.exists() {
        eprintln!("skip: dbl_nodtang absent");
        return;
    }
    let db = Database::open(&path).expect("open dbl_nodtang");
    let mats = db.materials().expect("materials");
    // At least one material must be present, with material number 1
    // somewhere in the value lists.
    assert!(!mats.is_empty(), "expected at least one MAT_NAME_<n>");
    assert!(
        mats.values().any(|nums| nums.contains(&1)),
        "expected material number 1 to be registered: {mats:?}"
    );
}

#[test]
fn basic1_element_sets_and_integration_points_are_consistent() {
    let path = corpus_path(&["serial", "basic1", "basic1.pltA"]);
    if !path.exists() {
        return;
    }
    let db = Database::open(&path).unwrap();
    let sets = db.element_sets().expect("element_sets");
    let ips = db.integration_points().expect("integration_points");
    // basic1 may or may not declare element sets — both empty is OK.
    // What must hold: every integer-named set corresponds to one entry
    // in `ips`, with `ips.len == set_values.len - 1`.
    for (name, values) in &sets {
        let Ok(mat) = name.parse::<i32>() else {
            continue;
        };
        let m = mili_rs::MaterialId(mat);
        let got = ips.get(&m).unwrap_or_else(|| {
            panic!("set '{name}' parsed to material {mat} but is missing from integration_points")
        });
        assert_eq!(got.len() + 1, values.len());
    }
}
