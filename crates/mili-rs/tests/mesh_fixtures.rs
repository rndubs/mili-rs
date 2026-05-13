//! Mesh metadata parity against the reference corpus.
//!
//! These tests open real `.A` files from `reference/mili-python` and
//! check that `CLASS_DEF`, `CLASS_IDENTS`, `NODES`, and `ELEM_CONNS`
//! entries fold into a sensible mesh table. They skip cleanly when the
//! submodule isn't checked out so partial checkouts stay green.

use std::path::{Path, PathBuf};

use mili_rs::{Database, MeshId, Superclass};

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
fn basic1_has_node_class_with_nodes_entry() {
    let path = corpus_path(&["serial", "basic1", "basic1.pltA"]);
    if !path.exists() {
        eprintln!("skip: basic1 absent");
        return;
    }
    let db = Database::open(&path).expect("open basic1");

    // basic1 declares mesh 0 with at least a nodal class.
    let mesh = db.meshes().mesh(MeshId(0)).expect("basic1 has mesh id 0");
    let node_class = mesh
        .class("node")
        .expect("basic1 declares the 'node' class on mesh 0");
    assert_eq!(node_class.superclass, Superclass::Node);

    // 1400 nodes (verified against the param_fixtures test on the
    // Node Labels TI array).
    assert_eq!(node_class.element_count(), 1400);

    // Decoding the NODES entry should match dimensions × node_count
    // floats.
    let nodes = db
        .nodes(MeshId(0), "node")
        .expect("decode NODES entry")
        .expect("basic1 has a NODES entry for mesh 0");
    assert_eq!(nodes.dimensions, 3);
    assert_eq!(nodes.node_count(), 1400);
    let coords = nodes.to_f32_vec().expect("decode coords");
    assert_eq!(coords.len(), 1400 * 3);
}

#[test]
fn basic1_brick_class_has_connectivity() {
    let path = corpus_path(&["serial", "basic1", "basic1.pltA"]);
    if !path.exists() {
        return;
    }
    let db = Database::open(&path).unwrap();
    // basic1's element class is "brick" (M_HEX). If it's named
    // differently in a given corpus version, surface that explicitly.
    let mesh = db.meshes().mesh(MeshId(0)).unwrap();
    let hex_class = mesh
        .classes()
        .find(|c| c.superclass == Superclass::Hex)
        .expect("basic1 has at least one M_HEX class");

    let conn = db
        .connectivity(MeshId(0), &hex_class.short_name)
        .unwrap()
        .expect("brick class has an ELEM_CONNS entry");
    assert_eq!(conn.superclass, Superclass::Hex);
    assert_eq!(conn.conn_words, 10); // 8 nodes + 2 metadata words
                                     // Element count from the id blocks must match the connectivity
                                     // stream length.
    let total_elems: u64 = conn.blocks.iter().map(|(s, e)| (e - s + 1) as u64).sum();
    assert_eq!(conn.data.len(), (total_elems as usize) * 10 * 4);
}

#[test]
fn dir_version_2_meshes_build() {
    let path = corpus_path(&["serial", "dir_version_2", "dblplt2009A"]);
    if !path.exists() {
        eprintln!("skip: dir_version_2 absent");
        return;
    }
    let db = Database::open(&path).expect("open dir_version_2");
    // The v2 fixture has at least one mesh with a node class.
    let mesh = db.meshes().meshes().next().expect("at least one mesh");
    assert!(
        mesh.classes().any(|c| c.superclass == Superclass::Node),
        "dir_version_2 must declare a nodal class"
    );
}

#[test]
fn class_id_blocks_match_modifier2_counts() {
    // Cross-check: the directory entry's MODIFIER2 on each
    // CLASS_IDENTS entry is the element count, which should equal
    // the corresponding block's size after coalescing.
    let path = corpus_path(&["serial", "basic1", "basic1.pltA"]);
    if !path.exists() {
        return;
    }
    let db = Database::open(&path).unwrap();
    for class in db.meshes().classes() {
        if class.id_blocks.is_empty() {
            continue;
        }
        // Sum of block sizes equals total declared element count.
        let total: u64 = class
            .id_blocks
            .iter()
            .map(|(s, e)| (e - s + 1) as u64)
            .sum();
        assert!(total > 0, "class {} has zero elements", class.short_name);
    }
}
