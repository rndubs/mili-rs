//! Cross-impl parity against mili-python on the C-library `xmilics`
//! corpus (`reference/mili/test/xmilics/`).
//!
//! Each xmilics fixture is an MPI-segmented family: N independent mili
//! databases (`<base>.plt00<r>A` per rank). The C library / xmilics
//! tooling treats each fragment as its own family; we cross-validate
//! both sides:
//!
//! 1. **Per-fragment parity** — `Database::open(<frag>.plt00rA)` vs.
//!    `mili.reader.open_database(<frag>.plt00r, suppress_parallel=True)`
//!    for a representative query on each fragment.
//! 2. **Set-level parity** — `DatabaseSet::open(<base>.plt)` vs.
//!    `mili.reader.open_database(<base>.plt)` (LoopWrapper /
//!    ServerWrapper merge) on the full multi-fragment family. Times +
//!    state count parity for now; query-merge parity is left as a
//!    follow-up because mili-python's merge has its own subtleties
//!    (e.g. column dedup heuristics) that need fixture-by-fixture
//!    triage before pinning as oracle.
//!
//! Skip-on-absent: requires both the `reference/mili` submodule and
//! the `mili` Python package importable. `parity` feature gates
//! pyo3.

#![cfg(feature = "parity")]

mod parity_support;

use std::path::{Path, PathBuf};

use parity_support::{
    assert_flat_eq_f32, open_database, query_f32, rust_to_py_states, skip_if_no_mili_python,
    OracleQuery,
};
use pyo3::prelude::*;

use mili_rs::{Database, DatabaseSet, MiliError, QueryArgs, StateValues};

fn xmilics_path(fixture: &str, rel: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("reference")
        .join("mili")
        .join("test")
        .join("xmilics")
        .join(fixture)
        .join(rel)
}

fn xmilics_dir(fixture: &str) -> PathBuf {
    xmilics_path(fixture, "")
}

fn parity_one_fragment(fixture: &str, frag: u32, fragment_count: u32) {
    // Per fixture-author convention the fragment digit width matches
    // the C library's writer: 3 digits for everything under
    // `reference/mili/test/xmilics`. Keep that explicit so the test
    // surface is obvious from the path.
    let frag_a = format!("{fixture}.plt{frag:03}A");
    let frag_base = format!("{fixture}.plt{frag:03}");
    let frag_a_path = xmilics_path(fixture, &frag_a);
    let frag_base_path = xmilics_path(fixture, &frag_base);
    if !frag_a_path.exists() {
        eprintln!(
            "skip: {fixture} fragment {frag} absent ({})",
            frag_a_path.display()
        );
        return;
    }
    // Sanity: fixture has the advertised fragment count.
    for r in 0..fragment_count {
        let p = xmilics_path(fixture, &format!("{fixture}.plt{r:03}A"));
        assert!(
            p.exists(),
            "{fixture}: expected fragment {r} to exist at {}",
            p.display()
        );
    }

    let db = Database::open(&frag_a_path).unwrap_or_else(|e| {
        panic!("rust open {fixture} fragment {frag}: {e}");
    });
    let states_total = db.state_count();
    assert!(states_total > 0, "{fixture}/{frag}: zero states");

    // Sample a handful of states across the timeline rather than every
    // state — keeps the test cheap. The mid-state is the most
    // diagnostic if a per-state offset is off by one.
    let mut rust_states: Vec<usize> = vec![0];
    if states_total >= 2 {
        rust_states.push(states_total / 2);
    }
    if states_total > 2 {
        rust_states.push(states_total - 1);
    }

    // nodpos on node is universal across the xmilics fixtures. If a
    // fixture changes that, the per-fragment open will surface it as a
    // NoMatchingSubrec which we treat as test-data-drift.
    let args = QueryArgs {
        svar: "nodpos",
        class: "node",
        labels: None,
        states: &rust_states,
        materials: None,
        ips: None,
    };
    let rust_result = match db.query(&args) {
        Ok(r) => r,
        Err(MiliError::NoMatchingSubrec { .. } | MiliError::UnknownClass(_)) => {
            eprintln!("skip: {fixture}/{frag} has no nodpos/node");
            return;
        }
        Err(e) => panic!("rust query {fixture}/{frag}: {e}"),
    };
    let StateValues::F32(rust_vals) = rust_result else {
        panic!("{fixture}/{frag}: nodpos expected f32");
    };

    Python::with_gil(|py| {
        let pdb = open_database(py, &frag_base_path).expect("py open per-fragment");
        let py_states = rust_to_py_states(&rust_states);
        let oracle = query_f32(
            py,
            &pdb,
            "nodpos",
            "node",
            &OracleQuery {
                states: Some(&py_states),
                ..Default::default()
            },
        )
        .expect("py query per-fragment");
        assert_flat_eq_f32(
            &rust_vals,
            &oracle.flat,
            &format!("{fixture}/frag{frag}: nodpos"),
        );
    });
}

#[test]
fn parity_d3samp6_fragment_0() {
    if !xmilics_dir("d3samp6").exists() {
        eprintln!("skip: reference/mili submodule absent");
        return;
    }
    if skip_if_no_mili_python() {
        eprintln!("skip: mili-python not importable");
        return;
    }
    parity_one_fragment("d3samp6", 0, 8);
}

#[test]
fn parity_d3samp6_fragment_4() {
    if !xmilics_dir("d3samp6").exists() {
        eprintln!("skip: reference/mili submodule absent");
        return;
    }
    if skip_if_no_mili_python() {
        eprintln!("skip: mili-python not importable");
        return;
    }
    parity_one_fragment("d3samp6", 4, 8);
}

#[test]
fn parity_bar1_fragment_0() {
    if !xmilics_dir("bar1").exists() {
        eprintln!("skip: reference/mili submodule absent");
        return;
    }
    if skip_if_no_mili_python() {
        eprintln!("skip: mili-python not importable");
        return;
    }
    parity_one_fragment("bar1", 0, 8);
}

#[test]
fn parity_shell_mat2_fragment_0() {
    if !xmilics_dir("shell_mat2").exists() {
        eprintln!("skip: reference/mili submodule absent");
        return;
    }
    if skip_if_no_mili_python() {
        eprintln!("skip: mili-python not importable");
        return;
    }
    parity_one_fragment("shell_mat2", 0, 11);
}

#[test]
fn parity_d3samp6_set_state_count_and_times() {
    // DatabaseSet-level parity row in the matrix: open all fragments
    // of d3samp6 in Rust, open the same family via mili-python (which
    // wraps with LoopWrapper for multi-frag + suppress_parallel=True),
    // and compare the state axis. Query-merge parity is intentionally
    // a follow-up — see the file-header note.
    if !xmilics_dir("d3samp6").exists() {
        eprintln!("skip: reference/mili submodule absent");
        return;
    }
    if skip_if_no_mili_python() {
        eprintln!("skip: mili-python not importable");
        return;
    }
    let base = xmilics_path("d3samp6", "d3samp6.plt");
    let set = DatabaseSet::open(&base).expect("rust DatabaseSet::open d3samp6");
    assert_eq!(set.fragment_count(), 8);

    Python::with_gil(|py| {
        let pdb = open_database(py, &base).expect("py open d3samp6 multi-frag");
        // With `merge_results=True` (the parity_support default), both
        // LoopWrapper and ServerWrapper reduce `times()` across
        // fragments into a single flat ndarray (the fragments share
        // the time axis, so the reduction is `zeroth_entry`). So we
        // compare the merged axis directly — no per-fragment list.
        let py_times: Vec<f32> = pdb
            .call_method0("times")
            .expect("py times")
            .call_method0("tolist")
            .expect("tolist")
            .extract()
            .expect("extract py times");
        let rust_times = set.times();
        assert_eq!(
            rust_times.len(),
            py_times.len(),
            "state-count mismatch: rust={} py={}",
            rust_times.len(),
            py_times.len()
        );
        for (i, (r, p)) in rust_times.iter().zip(py_times.iter()).enumerate() {
            assert_eq!(
                r.to_bits(),
                p.to_bits(),
                "time mismatch at state {i}: rust={r} py={p}"
            );
        }
    });
}
