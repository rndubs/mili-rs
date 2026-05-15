//! Cross-impl parity vs. mili-python for the named-component query
//! surface that `test_milidatabase.py`'s read half exercises:
//!
//! - `parent[comp]` named-component subscript on a plain VECTOR
//!   (`nodpos[ux]` on `node`) and on a VEC_ARRAY with an IP filter
//!   (`stress[sy]` on `beam`, `ips=2`).
//! - bare-component lookup where the component belongs to several
//!   VECTOR parents (`sx` ∈ `stress`/`stress_mid`/`stress_in`/
//!   `stress_out`) and must be disambiguated by subrecord membership
//!   for the queried class (`sx` on `brick`, carried via `stress`).
//!
//! Each is asserted bit-exact against the upstream `_MiliInternal`
//! oracle over `data/serial/sstate/d3samp6.plt`. Skip-not-fail when the
//! corpus / `mili` package is absent (CLAUDE.md convention).

#![cfg(feature = "parity")]

mod parity_support;

use parity_support::{corpus_path, open_database, query_f32, skip_if_no_mili_python, OracleQuery};
use pyo3::prelude::*;

use mili_rs::{Database, QueryArgs, StateValues};

/// `(svar, class, labels, rust_states_0based, oracle_states_1based, ips)`.
type Case<'a> = (
    &'a str,
    &'a str,
    &'a [i32],
    &'a [usize],
    &'a [i32],
    Option<&'a [i32]>,
);

#[test]
fn parity_d3samp6_component_subscript_and_multiparent_bare() {
    let plt_a = corpus_path(&["serial", "sstate", "d3samp6.pltA"]);
    let base = corpus_path(&["serial", "sstate", "d3samp6.plt"]);
    if !plt_a.exists() {
        eprintln!("skip: serial/sstate/d3samp6.plt absent");
        return;
    }
    if skip_if_no_mili_python() {
        eprintln!("skip: mili-python not importable");
        return;
    }
    let db = Database::open(&plt_a).unwrap();

    let cases: &[Case<'_>] = &[
        // named component of a plain VECTOR parent
        ("nodpos[ux]", "node", &[1, 70, 144], &[2], &[3], None),
        // bare component of a multi-parent VECTOR, disambiguated by the
        // brick subrecord that carries `stress`
        ("sx", "brick", &[1, 18, 36], &[0, 36], &[1, 37], None),
        // named component of a VEC_ARRAY parent with an IP filter
        ("stress[sy]", "beam", &[5, 20], &[70], &[71], Some(&[2])),
    ];

    for &(svar, class, labels, rstates, ostates, ips) in cases {
        let ips_us: Option<Vec<usize>> = ips.map(|v| v.iter().map(|&x| x as usize).collect());
        let rust = db
            .query(&QueryArgs {
                svar,
                class,
                labels: Some(labels),
                states: rstates,
                materials: None,
                ips: ips_us.as_deref(),
                subrec: None,
            })
            .unwrap();
        let StateValues::F32(rust) = rust else {
            panic!("{svar} is f32");
        };

        Python::with_gil(|py| {
            let pdb = open_database(py, &base).unwrap();
            let res = query_f32(
                py,
                &pdb,
                svar,
                class,
                &OracleQuery {
                    states: Some(ostates),
                    labels: Some(labels),
                    ips,
                    ..Default::default()
                },
            )
            .unwrap();
            assert_eq!(
                rust.len(),
                res.flat.len(),
                "{svar}/{class}: element count mismatch \
                 (rust={}, py={})",
                rust.len(),
                res.flat.len(),
            );
            for (i, (r, p)) in rust.iter().zip(res.flat.iter()).enumerate() {
                assert_eq!(
                    r.to_bits(),
                    p.to_bits(),
                    "{svar}/{class} divergence at atom {i}: \
                     rust={r} py={p}"
                );
            }
        });
    }
}
