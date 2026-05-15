//! Cross-impl parity vs. mili-python on the `basic1` serial fixture.
//!
//! Gated on the `parity` feature; skips when `reference/mili-python` is
//! absent or the `mili` Python package can't be imported.

#![cfg(feature = "parity")]

mod parity_support;

use parity_support::{corpus_path, open_database, query_f32, skip_if_no_mili_python, OracleQuery};
use pyo3::prelude::*;

use mili_rs::{Database, QueryArgs, StateValues};

const STATES_FOR_MULTI: &[usize] = &[0_usize, 50, 100];

fn rust_to_py_states(states: &[usize]) -> Vec<i32> {
    states.iter().map(|&s| (s as i32) + 1).collect()
}

fn assert_flat_eq(rust: &[f32], oracle: &[f32], tag: &str) {
    assert_eq!(
        rust.len(),
        oracle.len(),
        "{tag}: length mismatch (rust={}, py={})",
        rust.len(),
        oracle.len()
    );
    for (i, (r, p)) in rust.iter().zip(oracle.iter()).enumerate() {
        assert_eq!(
            r.to_bits(),
            p.to_bits(),
            "{tag}: divergence at index {i}: rust={r} py={p}"
        );
    }
}

#[test]
fn parity_basic1_nodpos_all_states_all_nodes() {
    let plt_a = corpus_path(&["serial", "basic1", "basic1.pltA"]);
    let base = corpus_path(&["serial", "basic1", "basic1.plt"]);
    if !plt_a.exists() {
        eprintln!("skip: basic1 absent");
        return;
    }
    if skip_if_no_mili_python() {
        eprintln!("skip: mili-python not importable");
        return;
    }

    let db = Database::open(&plt_a).expect("open basic1");
    let rust_states: Vec<usize> = (0..db.state_count()).collect();
    let rust = db
        .query(&QueryArgs {
            svar: "nodpos",
            class: "node",
            labels: None,
            states: &rust_states,
            materials: None,
            ips: None,
            subrec: None,
        })
        .expect("rust query");
    let StateValues::F32(rust) = rust else {
        panic!("nodpos is f32");
    };

    let py_states = rust_to_py_states(&rust_states);
    Python::with_gil(|py| {
        let pdb = open_database(py, &base).expect("py open");
        let res = query_f32(
            py,
            &pdb,
            "nodpos",
            "node",
            &OracleQuery {
                states: Some(&py_states),
                ..Default::default()
            },
        )
        .expect("py query");
        assert_eq!(res.shape, (rust_states.len(), 1400, 3));
        assert_flat_eq(&rust, &res.flat, "basic1 nodpos all-states all-nodes");
    });
}

#[test]
fn parity_basic1_nodvel_label_filter_sorted() {
    // mili-python sorts the labels-filter argument before laying out
    // the result; pass them in ascending order so the Rust output's
    // argument-order layout coincides with the python sorted layout.
    let plt_a = corpus_path(&["serial", "basic1", "basic1.pltA"]);
    let base = corpus_path(&["serial", "basic1", "basic1.plt"]);
    if !plt_a.exists() {
        eprintln!("skip: basic1 absent");
        return;
    }
    if skip_if_no_mili_python() {
        eprintln!("skip: mili-python not importable");
        return;
    }
    let db = Database::open(&plt_a).unwrap();

    let labels = [1_i32, 5, 1400];
    let states = [0_usize, 50];
    let rust = db
        .query(&QueryArgs {
            svar: "nodvel",
            class: "node",
            labels: Some(&labels),
            states: &states,
            materials: None,
            ips: None,
            subrec: None,
        })
        .unwrap();
    let StateValues::F32(rust) = rust else {
        panic!("nodvel is f32");
    };

    let py_states = rust_to_py_states(&states);
    Python::with_gil(|py| {
        let pdb = open_database(py, &base).unwrap();
        let res = query_f32(
            py,
            &pdb,
            "nodvel",
            "node",
            &OracleQuery {
                states: Some(&py_states),
                labels: Some(&labels),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(res.layout_labels, labels.to_vec());
        assert_eq!(res.shape, (states.len(), labels.len(), 3));
        assert_flat_eq(&rust, &res.flat, "basic1 nodvel label-filter");
    });
}

#[test]
fn parity_basic1_sand_material_filter() {
    let plt_a = corpus_path(&["serial", "basic1", "basic1.pltA"]);
    let base = corpus_path(&["serial", "basic1", "basic1.plt"]);
    if !plt_a.exists() {
        eprintln!("skip: basic1 absent");
        return;
    }
    if skip_if_no_mili_python() {
        eprintln!("skip: mili-python not importable");
        return;
    }
    let db = Database::open(&plt_a).unwrap();

    let states = [0_usize, 50];
    for mat in [1_i32, 3, 7] {
        let materials = [mat];
        let rust = db
            .query(&QueryArgs {
                svar: "sand",
                class: "brick",
                labels: None,
                states: &states,
                materials: Some(&materials),
                ips: None,
                subrec: None,
            })
            .unwrap();
        let StateValues::F32(rust) = rust else {
            panic!("sand is f32");
        };

        let py_states = rust_to_py_states(&states);
        Python::with_gil(|py| {
            let pdb = open_database(py, &base).unwrap();
            let res = query_f32(
                py,
                &pdb,
                "sand",
                "brick",
                &OracleQuery {
                    states: Some(&py_states),
                    material: Some(mat),
                    ..Default::default()
                },
            )
            .unwrap();
            assert_flat_eq(&rust, &res.flat, &format!("basic1 sand mat={mat}"));
        });
    }
}

#[test]
fn parity_basic1_nodpos_multi_state_subset() {
    // Three non-contiguous states + label filter. Forces both readers
    // through the per-state plan dispatch + the label-filter gather.
    let plt_a = corpus_path(&["serial", "basic1", "basic1.pltA"]);
    let base = corpus_path(&["serial", "basic1", "basic1.plt"]);
    if !plt_a.exists() {
        eprintln!("skip: basic1 absent");
        return;
    }
    if skip_if_no_mili_python() {
        eprintln!("skip: mili-python not importable");
        return;
    }
    let db = Database::open(&plt_a).unwrap();
    let labels = [42_i32, 100, 700, 1234];
    let rust = db
        .query(&QueryArgs {
            svar: "nodpos",
            class: "node",
            labels: Some(&labels),
            states: STATES_FOR_MULTI,
            materials: None,
            ips: None,
            subrec: None,
        })
        .unwrap();
    let StateValues::F32(rust) = rust else {
        unreachable!()
    };

    let py_states = rust_to_py_states(STATES_FOR_MULTI);
    Python::with_gil(|py| {
        let pdb = open_database(py, &base).unwrap();
        let res = query_f32(
            py,
            &pdb,
            "nodpos",
            "node",
            &OracleQuery {
                states: Some(&py_states),
                labels: Some(&labels),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(res.shape, (STATES_FOR_MULTI.len(), labels.len(), 3));
        assert_flat_eq(&rust, &res.flat, "basic1 nodpos multi-state subset");
    });
}
