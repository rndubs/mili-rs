//! Parity check that the byteswap-aware decode path through
//! [`mili_rs::buffer::MiliBuffer`] (used by `Nodes::to_f32_vec`) still
//! returns bit-identical floats to a naive byte-by-byte reference.
//!
//! Skips when the `reference/mili-python` submodule isn't checked out,
//! same convention as `mesh_fixtures.rs`.

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
fn basic1_nodes_match_naive_decode() {
    let path = corpus_path(&["serial", "basic1", "basic1.pltA"]);
    if !path.exists() {
        eprintln!("skip: basic1 absent");
        return;
    }
    let db = Database::open(&path).expect("open basic1");
    let nodes = db
        .nodes(MeshId(0), "node")
        .expect("decode NODES")
        .expect("basic1 has node NODES entry");

    // Reference: decode the raw byte slice directly with from_le_bytes,
    // outside the crate's shared byteswap path, so a regression in
    // `endian::for_each_swap` / `MiliBuffer` surfaces here.
    let mut reference: Vec<f32> = Vec::with_capacity(nodes.data.len() / 4);
    for chunk in nodes.data.chunks_exact(4) {
        let arr: [u8; 4] = chunk.try_into().unwrap();
        reference.push(f32::from_le_bytes(arr));
    }
    let migrated = nodes.to_f32_vec().expect("to_f32_vec");
    assert_eq!(migrated.len(), reference.len());
    for (a, b) in migrated.iter().zip(reference.iter()) {
        assert_eq!(a.to_bits(), b.to_bits());
    }
}
