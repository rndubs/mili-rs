//! State-map parser parity against the reference corpus.
//!
//! Covers both layouts in the wild:
//!   - inline state map in the main `.A` file (basic1)
//!   - external `<root>T` tfile with `~` end-marker (v3/serial_t)

use std::fs;
use std::path::{Path, PathBuf};

use mili_rs::{
    state::{parse_inline, parse_tfile, tfile_path},
    Directory, Header, StateMapSource,
};

fn corpus_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("reference")
        .join("mili-python")
        .join("tests")
        .join("data")
}

fn read_file(rel: &[&str]) -> Option<Vec<u8>> {
    let mut p = corpus_root();
    for c in rel {
        p = p.join(c);
    }
    fs::read(&p).ok()
}

fn corpus_path(rel: &[&str]) -> PathBuf {
    let mut p = corpus_root();
    for c in rel {
        p = p.join(c);
    }
    p
}

#[test]
fn basic1_inline_state_map_parses_to_101_states() {
    let Some(bytes) = read_file(&["serial", "basic1", "basic1.pltA"]) else {
        eprintln!("skip: basic1 absent");
        return;
    };
    let header = Header::parse(&bytes).unwrap();
    let dir = Directory::parse(&bytes, &header).unwrap();
    assert_eq!(dir.qty_states, 101, "basic1 has 101 states inline");

    let source = StateMapSource::pick(&header, &dir);
    let range = match source {
        StateMapSource::InlineA(r) => r,
        StateMapSource::ExternalTfile => panic!("basic1 should not use a tfile"),
    };
    let metas = parse_inline(&bytes, range, &header).expect("parse inline state map");
    assert_eq!(metas.len(), 101);

    // First state must reside in state file 0 at a non-negative offset.
    assert_eq!(metas[0].file, 0, "first state file index");
    assert!(metas[0].offset >= 0);
    // All states sort by time non-strictly.
    for w in metas.windows(2) {
        assert!(
            w[0].time <= w[1].time,
            "times not monotonic: {} > {}",
            w[0].time,
            w[1].time
        );
    }
    // srec format ids are non-negative and bounded.
    for m in &metas {
        assert!(m.srec_format >= 0);
    }
}

#[test]
fn serial_t_tfile_state_map_parses_with_marker() {
    let Some(a_bytes) = read_file(&["v3", "serial_t", "d3samp6.pltA"]) else {
        eprintln!("skip: v3/serial_t absent");
        return;
    };
    let header = Header::parse(&a_bytes).unwrap();
    let dir = Directory::parse(&a_bytes, &header).unwrap();
    // The whole point of this fixture: directory carries qty_states = 0;
    // state count moves into the tfile.
    assert_eq!(dir.qty_states, 0);
    assert!(matches!(
        StateMapSource::pick(&header, &dir),
        StateMapSource::ExternalTfile
    ));

    let a_path = corpus_path(&["v3", "serial_t", "d3samp6.pltA"]);
    let t_path = tfile_path(&a_path).expect("tfile path builds");
    let t_bytes = fs::read(&t_path).expect("read tfile");
    let metas = parse_tfile(&t_bytes, &header).expect("parse tfile state map");

    assert!(!metas.is_empty(), "tfile should have states");
    // d3samp6 ships 101 states (file size 2021 = 101*20 + 1).
    assert_eq!(metas.len(), (t_bytes.len() - 1) / 20);
    // Times monotonic non-strict.
    for w in metas.windows(2) {
        assert!(w[0].time <= w[1].time);
    }
}

#[test]
fn dir_version_2_inline_state_map_parses() {
    let Some(bytes) = read_file(&["serial", "dir_version_2", "dblplt2009A"]) else {
        eprintln!("skip: dir_version_2 absent");
        return;
    };
    let header = Header::parse(&bytes).unwrap();
    let dir = Directory::parse(&bytes, &header).unwrap();
    if dir.qty_states == 0 {
        // v2 with no states: still parses cleanly.
        let range = match StateMapSource::pick(&header, &dir) {
            StateMapSource::InlineA(r) => r,
            StateMapSource::ExternalTfile => panic!("expected inline for v2"),
        };
        let metas = parse_inline(&bytes, range, &header).unwrap();
        assert!(metas.is_empty());
        return;
    }
    let StateMapSource::InlineA(range) = StateMapSource::pick(&header, &dir) else {
        panic!("v2 must use inline");
    };
    let metas = parse_inline(&bytes, range, &header).expect("parse inline");
    assert_eq!(
        metas.len(),
        dir.qty_states as usize,
        "state-map count matches trailer"
    );
}
