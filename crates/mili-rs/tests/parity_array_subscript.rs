//! Cross-impl parity vs. mili-python on array-svar subscripts.
//!
//! d3samp6.th's `hx` is an ARRAY-svar with dims=[8]. Step 11 wired
//! `parse_query_name` + `AtomPicker::Specific` to handle both the bare
//! query and the `hx[k]` subscript form. We already pinned three values
//! against mili-python's `test_bugfixes.py::test_query_array_components`
//! goldens by hand; this widens that to programmatic parity over the
//! full label set the fixture exposes.

#![cfg(feature = "parity")]

mod parity_support;

use parity_support::{corpus_path, open_database, query_f32, skip_if_no_mili_python, OracleQuery};
use pyo3::prelude::*;

use mili_rs::{Database, QueryArgs, StateValues};

#[test]
fn parity_d3samp6_hx_full_array() {
    let plt_a = corpus_path(&["th", "serial", "d3samp6.thA"]);
    let base = corpus_path(&["th", "serial", "d3samp6.th"]);
    if !plt_a.exists() {
        eprintln!("skip: d3samp6.th absent");
        return;
    }
    if skip_if_no_mili_python() {
        eprintln!("skip: mili-python not importable");
        return;
    }
    let db = Database::open(&plt_a).unwrap();

    let labels = [2_i32, 5, 10];
    let rust_states = [5_usize];
    let rust = db
        .query(&QueryArgs {
            svar: "hx",
            class: "brick",
            labels: Some(&labels),
            states: &rust_states,
            materials: None,
            ips: None,
        })
        .unwrap();
    let StateValues::F32(rust) = rust else {
        panic!("hx is f32");
    };
    assert_eq!(rust.len(), labels.len() * 8);

    Python::with_gil(|py| {
        let pdb = open_database(py, &base).unwrap();
        let res = query_f32(
            py,
            &pdb,
            "hx",
            "brick",
            &OracleQuery {
                states: Some(&[6]),
                labels: Some(&labels),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(res.shape, (1, labels.len(), 8));
        for (i, (r, p)) in rust.iter().zip(res.flat.iter()).enumerate() {
            assert_eq!(
                r.to_bits(),
                p.to_bits(),
                "hx divergence at atom {i}: rust={r} py={p}"
            );
        }
    });
}

#[test]
fn parity_d3samp6_hx_subscript_each_atom() {
    let plt_a = corpus_path(&["th", "serial", "d3samp6.thA"]);
    let base = corpus_path(&["th", "serial", "d3samp6.th"]);
    if !plt_a.exists() {
        eprintln!("skip: d3samp6.th absent");
        return;
    }
    if skip_if_no_mili_python() {
        eprintln!("skip: mili-python not importable");
        return;
    }
    let db = Database::open(&plt_a).unwrap();

    let labels = [2_i32, 5, 10];
    let rust_states = [5_usize];
    // hx[1] through hx[8] — exhaust every atom of the 8-wide array.
    for k in 1..=8 {
        let name = format!("hx[{k}]");
        let rust = db
            .query(&QueryArgs {
                svar: &name,
                class: "brick",
                labels: Some(&labels),
                states: &rust_states,
                materials: None,
                ips: None,
            })
            .unwrap();
        let StateValues::F32(rust) = rust else {
            panic!("{name} is f32");
        };
        assert_eq!(rust.len(), labels.len());

        Python::with_gil(|py| {
            let pdb = open_database(py, &base).unwrap();
            let res = query_f32(
                py,
                &pdb,
                &name,
                "brick",
                &OracleQuery {
                    states: Some(&[6]),
                    labels: Some(&labels),
                    ..Default::default()
                },
            )
            .unwrap();
            assert_eq!(res.shape, (1, labels.len(), 1));
            for (i, (r, p)) in rust.iter().zip(res.flat.iter()).enumerate() {
                assert_eq!(
                    r.to_bits(),
                    p.to_bits(),
                    "{name} divergence at object {i}: rust={r} py={p}"
                );
            }
        });
    }
}
