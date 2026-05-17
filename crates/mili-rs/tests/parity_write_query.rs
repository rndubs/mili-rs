//! Phase 3.2 — cross-impl parity for the `query(write_data=)`
//! write-half (decision 23, `planning/mili-py/phase-3.md` § Phase 3.2).
//!
//! The Rust-core scatter ([`mili_rs::Database::scatter_query`]) is the
//! byte-exact inverse of the read gather: given the same
//! `(svar, class, labels, states, ips)` a read would use, it must
//! write **exactly** the bytes upstream `_MiliInternal.__query`'s
//! `srec.extract_ordinals(write_data=)` path writes. This gate stages
//! a corpus, runs upstream `mili`'s `query(write_data=)` on one copy
//! and the Rust core scatter on another with the *same* write payload,
//! and diffs every state file byte-for-byte. Scenarios mirror the
//! redirected `test_modify_database` surface: scalar / vector /
//! vec-array, component subscripts, alt label order, multi-state,
//! ips, and a per-fragment (uncombined) case.
//!
//! Gated on the `parity` feature; skip-not-fail when the corpus or the
//! `mili` package is absent. All scratch lives under a unique temp dir
//! removed at the end — the corpus is never touched.

#![cfg(feature = "parity")]

mod parity_support;

use std::fs;
use std::path::{Path, PathBuf};

use parity_support::{corpus_path, skip_if_no_mili_python};
use pyo3::prelude::*;
use pyo3::types::PyList;

use mili_rs::{Database, QueryArgs};

fn scratch_dir() -> PathBuf {
    let mut d = std::env::temp_dir();
    d.push(format!("mili_rs_parity_write_query_{}", std::process::id()));
    let _ = fs::remove_dir_all(&d);
    fs::create_dir_all(&d).expect("create scratch dir");
    d
}

fn stage(src_dir: &Path, base: &str, dst: &Path) {
    fs::create_dir_all(dst).unwrap();
    for entry in fs::read_dir(src_dir).unwrap() {
        let p = entry.unwrap().path();
        let name = p.file_name().unwrap().to_str().unwrap().to_owned();
        if name.starts_with(base) {
            fs::copy(&p, dst.join(&name)).unwrap();
        }
    }
}

fn assert_state_files_eq(a_dir: &Path, b_dir: &Path, base: &str, tag: &str) {
    let mut checked = 0;
    for entry in fs::read_dir(a_dir).unwrap() {
        let p = entry.unwrap().path();
        let name = p.file_name().unwrap().to_str().unwrap().to_owned();
        // State files are `<base><NN>` (digits suffix), not the `.A`.
        if !name.starts_with(base) || name.ends_with('A') {
            continue;
        }
        let av = fs::read(&p).unwrap();
        let bv = fs::read(b_dir.join(&name)).unwrap();
        assert_eq!(
            av.len(),
            bv.len(),
            "{tag}: {name} length mismatch (oracle={}, rust={})",
            av.len(),
            bv.len()
        );
        if let Some(i) = av.iter().zip(bv.iter()).position(|(x, y)| x != y) {
            let lo = i.saturating_sub(8);
            let hi = (i + 16).min(av.len());
            panic!(
                "{tag}: {name} byte divergence at {i}\n  oracle={:?}\n  rust  ={:?}",
                &av[lo..hi],
                &bv[lo..hi]
            );
        }
        checked += 1;
    }
    assert!(checked > 0, "{tag}: no state files compared");
}

struct Case {
    rel: &'static [&'static str],
    base: &'static str,
    svar: &'static str,
    class: &'static str,
    labels: &'static [i32],
    /// 1-based state numbers (as a user passes `states=`).
    states: &'static [i64],
    /// User `ips=` ints (the FFI passes these straight through as
    /// 0-based-positional `usize`; mirror that here). Empty = no ips.
    ips: &'static [usize],
}

const CASES: &[Case] = &[
    // serial/sstate — scalar on `mat`, multi-state.
    Case {
        rel: &["serial", "sstate"],
        base: "d3samp6.plt",
        svar: "matcgx",
        class: "mat",
        labels: &[1],
        states: &[3, 4],
        ips: &[],
    },
    // serial/sstate — vector on `node`.
    Case {
        rel: &["serial", "sstate"],
        base: "d3samp6.plt",
        svar: "nodpos",
        class: "node",
        labels: &[70, 71],
        states: &[4],
        ips: &[],
    },
    // serial/sstate — vector component subscript.
    Case {
        rel: &["serial", "sstate"],
        base: "d3samp6.plt",
        svar: "nodpos[uz]",
        class: "node",
        labels: &[70, 71],
        states: &[4],
        ips: &[],
    },
    // serial/sstate — vec-array on `beam` at one ip.
    Case {
        rel: &["serial", "sstate"],
        base: "d3samp6.plt",
        svar: "stress",
        class: "beam",
        labels: &[5],
        states: &[71],
        ips: &[2],
    },
    // serial/sstate — vec-array component subscript at one ip.
    Case {
        rel: &["serial", "sstate"],
        base: "d3samp6.plt",
        svar: "stress[sy]",
        class: "beam",
        labels: &[5],
        states: &[71],
        ips: &[2],
    },
    // parallel/d3samp6 fragment 000 — per-fragment scalar (the
    // uncombined write path each LoopWrapper proc drives).
    Case {
        rel: &["parallel", "d3samp6"],
        base: "d3samp6.plt000",
        svar: "sx",
        class: "brick",
        labels: &[],
        states: &[35],
        ips: &[],
    },
];

/// Oracle: `_MiliInternal(dir, base)` → read, perturb deterministically,
/// `query(..., write_data=)`. Returns `(wd_labels, wd_values_f64)` so
/// the Rust side scatters the *identical* payload.
#[allow(clippy::too_many_arguments)]
fn oracle_write(py: Python<'_>, dir: &Path, c: &Case) -> PyResult<(Vec<i32>, Vec<f64>)> {
    let np = py.import_bound("numpy")?;
    let m = py.import_bound("mili.miliinternal")?;
    let db = m
        .getattr("_MiliInternal")?
        .call1((dir.to_str().unwrap(), c.base))?;

    let kw = pyo3::types::PyDict::new_bound(py);
    if !c.labels.is_empty() {
        kw.set_item("labels", PyList::new_bound(py, c.labels))?;
    }
    kw.set_item("states", PyList::new_bound(py, c.states))?;
    if !c.ips.is_empty() {
        kw.set_item(
            "ips",
            PyList::new_bound(py, c.ips.iter().map(|&x| x as i64).collect::<Vec<_>>()),
        )?;
    }
    let res = db.call_method("query", (c.svar, c.class), Some(&kw))?;
    let entry = res.get_item(c.svar)?;
    let data = entry.get_item("data")?;
    // Deterministic perturbation both impls write identically.
    let perturbed = np.call_method1("add", (&data, np.call_method1("float32", (1.5_f64,))?))?;
    entry.set_item("data", &perturbed)?;

    let wd_labels: Vec<i32> = np
        .call_method1("asarray", (entry.get_item("layout")?.get_item("labels")?,))?
        .call_method1("astype", ("int32",))?
        .call_method0("flatten")?
        .call_method0("tolist")?
        .extract()?;
    let wd_values: Vec<f64> = np
        .call_method1("asarray", (&perturbed,))?
        .call_method1("astype", ("float64",))?
        .call_method0("flatten")?
        .call_method0("tolist")?
        .extract()?;

    let wkw = pyo3::types::PyDict::new_bound(py);
    if !c.labels.is_empty() {
        wkw.set_item("labels", PyList::new_bound(py, c.labels))?;
    }
    wkw.set_item("states", PyList::new_bound(py, c.states))?;
    if !c.ips.is_empty() {
        wkw.set_item(
            "ips",
            PyList::new_bound(py, c.ips.iter().map(|&x| x as i64).collect::<Vec<_>>()),
        )?;
    }
    wkw.set_item("write_data", &res)?;
    db.call_method("query", (c.svar, c.class), Some(&wkw))?;
    Ok((wd_labels, wd_values))
}

#[test]
fn parity_write_query() {
    if skip_if_no_mili_python() {
        eprintln!("skip: mili-python not importable");
        return;
    }
    let scratch = scratch_dir();
    let result = std::panic::catch_unwind(|| {
        Python::with_gil(|py| {
            for (i, c) in CASES.iter().enumerate() {
                let src = corpus_path(c.rel);
                if !src.join(format!("{}A", c.base)).exists() {
                    eprintln!("skip: {} {} corpus absent", c.rel.join("/"), c.base);
                    continue;
                }
                let odir = scratch.join(format!("o{i}"));
                let rdir = scratch.join(format!("r{i}"));
                stage(&src, c.base, &odir);
                stage(&src, c.base, &rdir);

                let (wd_labels, wd_values) =
                    oracle_write(py, &odir, c).expect("oracle query(write_data=)");

                let rdb =
                    Database::open(rdir.join(format!("{}A", c.base))).expect("open rust copy");
                let states0: Vec<usize> = c.states.iter().map(|&s| (s - 1) as usize).collect();
                let labels_opt = if c.labels.is_empty() {
                    None
                } else {
                    Some(c.labels)
                };
                let ips_opt = if c.ips.is_empty() { None } else { Some(c.ips) };
                let args = QueryArgs {
                    svar: c.svar,
                    class: c.class,
                    labels: labels_opt,
                    states: &states0,
                    materials: None,
                    ips: ips_opt,
                    subrec: None,
                };
                rdb.scatter_query(&args, &wd_labels, &wd_values)
                    .expect("rust scatter_query");

                assert_state_files_eq(
                    &odir,
                    &rdir,
                    c.base,
                    &format!("{}::{}/{}", c.svar, c.rel.join("/"), c.base),
                );
            }
        });
    });
    let _ = fs::remove_dir_all(&scratch);
    if let Err(e) = result {
        std::panic::resume_unwind(e);
    }
}
