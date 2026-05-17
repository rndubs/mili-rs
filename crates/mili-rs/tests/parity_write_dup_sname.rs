//! Phase 3 closeout (decision 26, `planning/mili-py/m4.md` § "Phase 3")
//! — cross-impl parity for the **duplicate-sname-within-a-directory-
//! type** write path vs. the upstream `mili.afileIO.AFileWriter`
//! oracle (driven via `_MiliInternal.copy_non_state_data`, which
//! re-serialises the `.A` through `AFileWriter.write`).
//!
//! No d3samp6 fragment has duplicate snames within a directory type
//! (the bound that lets `parity_write_append.rs` raw-copy payload
//! byte-ranges), but the wider corpus does: 57 fixtures
//! (`reference/mili-python/tests/data/**` + `reference/mili/test/
//! xmilics/**`) carry duplicate ELEM_CONNS / CLASS_DEF snames. Upstream
//! merges them at parse (`np.concatenate`) and writes a single payload
//! per sname, updating only the **first** matching decl's
//! offset/length while every later duplicate keeps a stale original
//! offset/length yet is still emitted. This gate round-trips
//! representative duplicate-sname fixtures and byte-diffs the
//! Rust-written `.A` against the upstream golden.
//!
//! Gated on the `parity` feature; skip-not-fail when the corpus or the
//! `mili` package is absent (matches the other `parity_*.rs` /
//! CLAUDE.md). All scratch files live under a unique temp dir removed
//! at the end — nothing is written into the corpus.

#![cfg(feature = "parity")]

mod parity_support;

use std::fs;
use std::path::{Path, PathBuf};

use parity_support::skip_if_no_mili_python;
use pyo3::prelude::*;

use mili_rs::Database;

/// Repo-root-relative corpus root (`reference/...`).
fn reference(rel: &[&str]) -> PathBuf {
    let mut p = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("reference");
    for c in rel {
        p = p.join(c);
    }
    p
}

fn scratch_dir() -> PathBuf {
    let mut d = std::env::temp_dir();
    d.push(format!("mili_rs_parity_dupsname_{}", std::process::id()));
    let _ = fs::remove_dir_all(&d);
    fs::create_dir_all(&d).expect("create scratch dir");
    d
}

/// Upstream `_MiliInternal(dir, base).copy_non_state_data(new_base)` —
/// re-serialises the `.A` via `AFileWriter.write`.
fn oracle_copy(py: Python<'_>, dir: &Path, base: &str, new_base: &Path) -> PyResult<()> {
    let m = py.import_bound("mili.miliinternal")?;
    let db = m
        .getattr("_MiliInternal")?
        .call1((dir.to_str().unwrap(), base))?;
    db.call_method1("copy_non_state_data", (new_base.to_str().unwrap(),))?;
    Ok(())
}

fn assert_bytes_eq(a: &Path, b: &Path, tag: &str) {
    let av = fs::read(a).unwrap_or_else(|e| panic!("{tag}: read {a:?}: {e}"));
    let bv = fs::read(b).unwrap_or_else(|e| panic!("{tag}: read {b:?}: {e}"));
    assert_eq!(
        av.len(),
        bv.len(),
        "{tag}: length mismatch (rust={}, oracle={})",
        av.len(),
        bv.len()
    );
    if let Some(i) = av.iter().zip(bv.iter()).position(|(x, y)| x != y) {
        let lo = i.saturating_sub(8);
        let hi = (i + 16).min(av.len());
        panic!(
            "{tag}: byte divergence at offset {i}\n  rust  ={:?}\n  oracle={:?}",
            &av[lo..hi],
            &bv[lo..hi]
        );
    }
}

/// `(src_dir, a_filename, base)` — `base` is the A-filename minus the
/// trailing `A`. Each fixture has ≥1 duplicate sname within ELEM_CONNS
/// and/or CLASS_DEF (per the decision-26 corpus audit).
fn fixtures() -> Vec<(PathBuf, &'static str, &'static str)> {
    vec![
        // ELEM_CONNS {brick×2, particle×3, quad×3} interleaved +
        // CLASS_DEF {brick×2, particle×3, quad×3}.
        (
            reference(&["mili-python", "tests", "data", "serial", "labeling"]),
            "dblplt003A",
            "dblplt003",
        ),
        // ELEM_CONNS tet×2 (no trailing proc digits in the base).
        (
            reference(&["mili-python", "tests", "data", "serial", "tet"]),
            "tet1_t4.pltA",
            "tet1_t4.plt",
        ),
        // ELEM_CONNS brick×2 + CLASS_DEF brick×2.
        (
            reference(&["mili-python", "tests", "data", "serial", "dbl_nodtang"]),
            "dblplt000A",
            "dblplt000",
        ),
        // C-library xmilics breadth: ELEM_CONNS brick×2 (parallel proc
        // fragment opened single).
        (
            reference(&["mili", "test", "xmilics", "bar5"]),
            "bar5.plt000A",
            "bar5.plt000",
        ),
        // xmilics: ELEM_CONNS brick×7.
        (
            reference(&["mili", "test", "xmilics", "basic2"]),
            "basic2.plt000A",
            "basic2.plt000",
        ),
    ]
}

#[test]
fn parity_write_dup_sname() {
    if skip_if_no_mili_python() {
        eprintln!("skip: mili-python not importable");
        return;
    }

    let scratch = scratch_dir();
    let result = std::panic::catch_unwind(|| {
        Python::with_gil(|py| {
            let mut exercised = 0usize;
            for (src_dir, a_name, base) in fixtures() {
                if !src_dir.join(a_name).exists() {
                    eprintln!("skip: fixture absent: {}", src_dir.join(a_name).display());
                    continue;
                }
                exercised += 1;

                // Stage every file for this base so the oracle and the
                // Rust core read identical inputs and never touch the
                // corpus.
                for entry in fs::read_dir(&src_dir).unwrap() {
                    let p = entry.unwrap().path();
                    let name = p.file_name().unwrap().to_str().unwrap().to_owned();
                    if name.starts_with(base) {
                        fs::copy(&p, scratch.join(&name)).unwrap();
                    }
                }

                oracle_copy(py, &scratch, base, &scratch.join("g.plt"))
                    .unwrap_or_else(|e| panic!("oracle copy_non_state_data [{base}]: {e}"));
                let rdb = Database::open(scratch.join(a_name))
                    .unwrap_or_else(|e| panic!("open fixture [{base}]: {e:?}"));
                rdb.copy_non_state_data(scratch.join("r.plt").to_str().unwrap())
                    .unwrap_or_else(|e| panic!("rust copy_non_state_data [{base}]: {e:?}"));

                // copy_non_state_data appends the base's trailing
                // `(\d+)$` digits to the new base.
                let digits: String = base
                    .chars()
                    .rev()
                    .take_while(char::is_ascii_digit)
                    .collect::<Vec<_>>()
                    .into_iter()
                    .rev()
                    .collect();
                let g_a = scratch.join(format!("g.plt{digits}A"));
                let r_a = scratch.join(format!("r.plt{digits}A"));
                assert_bytes_eq(&r_a, &g_a, &format!("copy_non_state_data[{base}].A"));

                for entry in fs::read_dir(&scratch).unwrap() {
                    let p = entry.unwrap().path();
                    let nm = p.file_name().unwrap().to_str().unwrap().to_owned();
                    if nm.starts_with("g.") || nm.starts_with("r.") || nm.starts_with(base) {
                        let _ = fs::remove_file(&p);
                    }
                }
            }
            assert!(
                exercised > 0,
                "no duplicate-sname fixture was reachable — decision-26 gate vacuous"
            );
        });
    });
    let _ = fs::remove_dir_all(&scratch);
    if let Err(e) = result {
        std::panic::resume_unwind(e);
    }
}
