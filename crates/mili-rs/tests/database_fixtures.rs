//! End-to-end open against the reference corpus.
//!
//! Verifies the open path threads header, directory, param table, and
//! state-map parsing together on real fixtures.

use std::path::{Path, PathBuf};

use mili_rs::{Database, ParamValue, ScalarValue};

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
fn open_basic1_inline_state_map() {
    let path = corpus_path(&["serial", "basic1", "basic1.pltA"]);
    if !path.exists() {
        eprintln!("skip: basic1 absent");
        return;
    }
    let db = Database::open(&path).expect("open basic1");

    assert_eq!(db.header().dir_version, 3);
    assert!(!db.directory().entries.is_empty());
    assert_eq!(db.state_count(), 101);
    assert_eq!(db.mesh_dimensions().unwrap(), 3);

    // First state lives in file 0, times sort non-decreasing.
    let states = db.states();
    assert_eq!(states[0].file, 0);
    for w in states.windows(2) {
        assert!(w[0].time <= w[1].time);
    }
}

#[test]
fn open_serial_t_uses_tfile() {
    let path = corpus_path(&["v3", "serial_t", "d3samp6.pltA"]);
    if !path.exists() {
        eprintln!("skip: v3/serial_t absent");
        return;
    }
    let db = Database::open(&path).expect("open serial_t");
    // The directory carries qty_states = 0; state map is in the tfile.
    assert_eq!(db.directory().qty_states, 0);
    assert!(db.state_count() > 0);
    // d3samp6 ships 101 states.
    assert_eq!(db.state_count(), 101);
}

#[test]
fn open_dbl_nodtang_resolves_param() {
    let path = corpus_path(&["serial", "dbl_nodtang", "dblplt000A"]);
    if !path.exists() {
        eprintln!("skip: dbl_nodtang absent");
        return;
    }
    let db = Database::open(&path).expect("open dbl_nodtang");
    // MAT_NAME_1 is a TI_PARAM string inline in the .A directory.
    match db.param("MAT_NAME_1").unwrap() {
        Some(ParamValue::String(s)) => assert!(!s.is_empty()),
        other => panic!("expected string param, got {other:?}"),
    }
    // Absent params return Ok(None), not an error.
    assert!(db.param("does-not-exist").unwrap().is_none());
}

#[test]
fn open_dir_version_2_handles_v2_path() {
    let path = corpus_path(&["serial", "dir_version_2", "dblplt2009A"]);
    if !path.exists() {
        eprintln!("skip: dir_version_2 absent");
        return;
    }
    let db = Database::open(&path).expect("open dir_version_2");
    assert_eq!(db.header().dir_version, 2);
    // State count must match the trailer's QTY_STATES.
    assert_eq!(db.state_count(), db.directory().qty_states as usize);
}

#[test]
fn states_per_file_decodes() {
    let path = corpus_path(&["serial", "basic1", "basic1.pltA"]);
    if !path.exists() {
        return;
    }
    let db = Database::open(&path).unwrap();
    let v = db.param("states per file").unwrap();
    // Just check it round-trips as a scalar — basic1 carries the
    // 0 sentinel (use writer default).
    assert!(matches!(v, Some(ParamValue::Scalar(ScalarValue::I32(_)))));
}
