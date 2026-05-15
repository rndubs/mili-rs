//! Corpus-wide parity sweep against mili-python.
//!
//! One representative svar per fixture from
//! `reference/mili-python/tests/data/serial/`, queried at a handful of
//! representative states and compared bit-exact against
//! `mili-python`'s `db.query(...)['data'].flatten()`.
//!
//! Together with `parity_basic1.rs` and `parity_array_subscript.rs`
//! these tests close Step 16 item 1 — covering every fixture in the
//! corpus with at least one bit-exact parity oracle, so we don't ship
//! `mili-py` on top of a reader that is only spot-checked on two
//! fixtures.
//!
//! Gated on the `parity` feature; skips when the corpus or
//! `mili-python` is absent.

#![cfg(feature = "parity")]

mod parity_support;

use parity_support::{
    assert_flat_eq_f32, assert_flat_eq_f64, corpus_path, open_database, query_f32, query_f64,
    rust_to_py_states, skip_if_no_mili_python, OracleQuery,
};
use pyo3::prelude::*;

use mili_rs::{Database, MeshId, QueryArgs, StateValues};

/// One row in the corpus matrix: pick a representative query for each
/// fixture (class + svar) and assert bit-exact equality at the chosen
/// state indices. Doubles get the f64 helper; everything else f32.
struct Row {
    fixture: &'static str,
    rel_dir: &'static str,
    a_file: &'static str,
    base: &'static str,
    svar: &'static str,
    class: &'static str,
    states: &'static [usize],
    /// `None` → f32, `Some(true)` → f64
    f64: bool,
}

const CORPUS: &[Row] = &[
    Row {
        fixture: "beam_udi",
        rel_dir: "beam_udi",
        a_file: "beam_udi.pltA",
        base: "beam_udi.plt",
        svar: "axf",
        class: "beam",
        states: &[0, 10, 20],
        f64: false,
    },
    Row {
        fixture: "d3samp4",
        rel_dir: "d3samp4",
        a_file: "d3samp4.pltA",
        base: "d3samp4.plt",
        svar: "sand",
        class: "brick",
        states: &[0, 5, 10],
        f64: false,
    },
    Row {
        fixture: "dbl_nodtang",
        rel_dir: "dbl_nodtang",
        a_file: "dblplt000A",
        base: "dblplt000",
        svar: "nodpos",
        class: "node",
        states: &[0, 60, 121],
        f64: true,
    },
    Row {
        fixture: "fdamp1",
        rel_dir: "fdamp1",
        a_file: "fdamp1.pltA",
        base: "fdamp1.plt",
        svar: "stress",
        class: "brick",
        states: &[0, 10, 20],
        f64: false,
    },
    Row {
        fixture: "labeling",
        rel_dir: "labeling",
        a_file: "dblplt003A",
        base: "dblplt003",
        svar: "nodpos",
        class: "node",
        states: &[0, 1, 2],
        f64: true,
    },
    Row {
        fixture: "mstate",
        rel_dir: "mstate",
        a_file: "d3samp6.plt_cA",
        base: "d3samp6.plt_c",
        svar: "axf",
        class: "beam",
        states: &[0, 50, 100],
        f64: false,
    },
    Row {
        fixture: "rigid_body_1",
        rel_dir: "rigid_body_1",
        a_file: "rigid_body1.pltA",
        base: "rigid_body1.plt",
        svar: "sand",
        class: "brick",
        states: &[0, 10, 20],
        f64: false,
    },
    Row {
        fixture: "sstate",
        rel_dir: "sstate",
        a_file: "d3samp6.pltA",
        base: "d3samp6.plt",
        svar: "axf",
        class: "beam",
        states: &[0, 50, 100],
        f64: false,
    },
    Row {
        fixture: "tet",
        rel_dir: "tet",
        a_file: "tet1_t4.pltA",
        base: "tet1_t4.plt",
        svar: "sand",
        class: "tet",
        states: &[0, 40, 80],
        f64: false,
    },
    Row {
        fixture: "vrt_BS",
        rel_dir: "vrt_BS",
        a_file: "vrt_BS.pltA",
        base: "vrt_BS.plt",
        svar: "axf",
        class: "beam",
        states: &[0, 5, 10],
        f64: false,
    },
];

fn run_row(row: &Row) {
    let a = corpus_path(&["serial", row.rel_dir, row.a_file]);
    let base = corpus_path(&["serial", row.rel_dir, row.base]);
    if !a.exists() {
        eprintln!("skip: {} absent", row.fixture);
        return;
    }
    if skip_if_no_mili_python() {
        eprintln!("skip: mili-python not importable");
        return;
    }

    let db = Database::open(&a).unwrap_or_else(|e| panic!("{}: open: {e}", row.fixture));

    let py_states = rust_to_py_states(row.states);

    if row.f64 {
        let rust = db
            .query(&QueryArgs {
                svar: row.svar,
                class: row.class,
                labels: None,
                states: row.states,
                materials: None,
                ips: None,
                subrec: None,
            })
            .unwrap_or_else(|e| panic!("{}: rust query: {e}", row.fixture));
        let StateValues::F64(rust) = rust else {
            panic!("{}: expected f64, got something else", row.fixture);
        };

        Python::with_gil(|py| {
            let pdb = open_database(py, &base).unwrap();
            let oracle = query_f64(
                py,
                &pdb,
                row.svar,
                row.class,
                &OracleQuery {
                    states: Some(&py_states),
                    ..Default::default()
                },
            )
            .unwrap_or_else(|e| panic!("{}: py query: {e}", row.fixture));
            assert_flat_eq_f64(&rust, &oracle.flat, row.fixture);
        });
    } else {
        let rust = db
            .query(&QueryArgs {
                svar: row.svar,
                class: row.class,
                labels: None,
                states: row.states,
                materials: None,
                ips: None,
                subrec: None,
            })
            .unwrap_or_else(|e| panic!("{}: rust query: {e}", row.fixture));
        let StateValues::F32(rust) = rust else {
            panic!("{}: expected f32, got something else", row.fixture);
        };

        Python::with_gil(|py| {
            let pdb = open_database(py, &base).unwrap();
            let oracle = query_f32(
                py,
                &pdb,
                row.svar,
                row.class,
                &OracleQuery {
                    states: Some(&py_states),
                    ..Default::default()
                },
            )
            .unwrap_or_else(|e| panic!("{}: py query: {e}", row.fixture));
            assert_flat_eq_f32(&rust, &oracle.flat, row.fixture);
        });
    }
}

#[test]
fn parity_corpus_beam_udi() {
    run_row(&CORPUS[0]);
}
#[test]
fn parity_corpus_d3samp4() {
    run_row(&CORPUS[1]);
}
#[test]
fn parity_corpus_dbl_nodtang() {
    run_row(&CORPUS[2]);
}
#[test]
fn parity_corpus_fdamp1() {
    run_row(&CORPUS[3]);
}
#[test]
fn parity_corpus_labeling() {
    run_row(&CORPUS[4]);
}
#[test]
fn parity_corpus_mstate() {
    run_row(&CORPUS[5]);
}
#[test]
fn parity_corpus_rigid_body_1() {
    run_row(&CORPUS[6]);
}
#[test]
fn parity_corpus_sstate() {
    run_row(&CORPUS[7]);
}
#[test]
fn parity_corpus_tet() {
    run_row(&CORPUS[8]);
}
#[test]
fn parity_corpus_vrt_bs() {
    run_row(&CORPUS[9]);
}

/// `dir_version_2` is a directory-v2 fixture. The corpus ships only
/// the `.A` (directory + metadata) file — no state files exist on
/// disk, so a state-data query can't be checked end-to-end. mili-
/// python's `db.query(...)` on this fixture hits an `IndexError` for
/// the same reason. What we _can_ assert: `Database::open` succeeds,
/// directory walk surfaces the right shape, and the metadata-level
/// accessors return plausible values. The v2 → v3 entry widening
/// itself is already covered by `directory_fixtures.rs`; this test
/// pins that the public `Database::open` path stays compatible.
#[test]
fn parity_corpus_dir_version_2_open_only() {
    let a = corpus_path(&["serial", "dir_version_2", "dblplt2009A"]);
    if !a.exists() {
        eprintln!("skip: dir_version_2 absent");
        return;
    }
    let db = Database::open(&a).expect("dir_version_2 open");
    assert!(db.state_count() > 0, "dir_version_2 has states");
    let nodes = db
        .nodes(MeshId(0), "node")
        .expect("nodes(0) lookup")
        .expect("dir_version_2 has nodal class");
    assert!(
        nodes.node_count() > 0,
        "dir_version_2 must have at least one node"
    );
}
