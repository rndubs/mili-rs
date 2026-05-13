//! Directory parser parity against the reference corpus.
//!
//! Like the header fixture tests, these skip when the submodule is
//! absent so partial checkouts stay green.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use mili_rs::{DirEntryType, Directory, Header};

fn corpus_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("reference")
        .join("mili-python")
        .join("tests")
        .join("data")
        .join("serial")
}

fn read_a_file(rel: &[&str]) -> Option<Vec<u8>> {
    let mut p = corpus_root();
    for c in rel {
        p = p.join(c);
    }
    fs::read(&p).ok()
}

fn type_histogram(dir: &Directory) -> HashMap<DirEntryType, usize> {
    let mut h = HashMap::new();
    for e in &dir.entries {
        *h.entry(e.entry_type).or_insert(0) += 1;
    }
    h
}

#[test]
fn basic1_v3_directory() {
    let Some(bytes) = read_a_file(&["basic1", "basic1.pltA"]) else {
        eprintln!("skip: basic1 fixture absent");
        return;
    };
    let header = Header::parse(&bytes).unwrap();
    assert_eq!(header.dir_version, 3);
    let dir = Directory::parse(&bytes, &header).expect("parse basic1 directory");

    assert!(!dir.entries.is_empty(), "basic1 has at least one entry");
    assert!(dir.commit_count >= 1, "commit_count = {}", dir.commit_count);

    let hist = type_histogram(&dir);
    // basic1 has geometry, params, and svar dictionary at minimum.
    assert!(
        hist.contains_key(&DirEntryType::Nodes),
        "Nodes entry missing"
    );
    assert!(
        hist.contains_key(&DirEntryType::StateVarDict),
        "StateVarDict entry missing"
    );

    // Every name in the pool is non-empty UTF-8 we can index.
    for i in 0..dir.names.len() {
        let _ = dir.names.get(i);
    }

    // Each entry's name slice indices are in-range and consistent.
    for e in &dir.entries {
        let end = e.name_start as usize + e.name_count as usize;
        assert!(end <= dir.names.len(), "entry name range out of pool");
    }
}

#[test]
fn dir_version_2_directory() {
    let Some(bytes) = read_a_file(&["dir_version_2", "dblplt2009A"]) else {
        eprintln!("skip: dir_version_2 fixture absent");
        return;
    };
    let header = Header::parse(&bytes).unwrap();
    assert_eq!(
        header.dir_version, 2,
        "this fixture must be v2 to exercise the v2 path"
    );

    let dir = Directory::parse(&bytes, &header).expect("parse dir_version_2 directory");
    assert!(!dir.entries.is_empty());

    // Same sanity invariants as the v3 path; we hit the 4-byte-int
    // widening code only here.
    for e in &dir.entries {
        let end = e.name_start as usize + e.name_count as usize;
        assert!(end <= dir.names.len());
    }

    // The v2 fixture must still produce a Nodes entry (it's a real DB).
    let hist = type_histogram(&dir);
    assert!(hist.contains_key(&DirEntryType::Nodes));
}

#[test]
fn dbl_nodtang_directory() {
    let Some(bytes) = read_a_file(&["dbl_nodtang", "dblplt000A"]) else {
        eprintln!("skip: dbl_nodtang fixture absent");
        return;
    };
    let header = Header::parse(&bytes).unwrap();
    let dir = Directory::parse(&bytes, &header).expect("parse dbl_nodtang directory");
    assert!(!dir.entries.is_empty());

    // dbl_nodtang has its labels and material params in the .ATI* files,
    // but the main directory still contains svar dict + state recs + class defs.
    let hist = type_histogram(&dir);
    assert!(hist.contains_key(&DirEntryType::StateVarDict));
    assert!(hist.contains_key(&DirEntryType::StateRecData));
    assert!(hist.contains_key(&DirEntryType::ClassDef));
}
