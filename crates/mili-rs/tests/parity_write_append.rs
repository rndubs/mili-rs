//! Phase 3.1 — cross-impl parity for the on-disk write path
//! (`copy_non_state_data` + `append_state`) vs. the upstream
//! `mili.miliinternal._MiliInternal` oracle (decision 22,
//! `planning/mili-py/m4.md` § "Phase 3").
//!
//! The Rust core's renormalising A-file serializer must reproduce
//! upstream `mili.afileIO.AFileWriter`'s **output** byte-for-byte
//! (it is not a byte-identity round-trip of the original `.A`). This
//! gate diffs Rust-written bytes against upstream-`mili`-written bytes
//! on the parallel d3samp6 corpus (all 8 fragments):
//!
//! - `copy_non_state_data`: the full `.A` (no states), every fragment.
//! - `append_state(100.0)` on a freshly-copied 0-state db: the updated
//!   `.A` (smap appended + `state_count` bumped) **and** the new state
//!   file (8-byte header + zeroed body + the `nodpos` / `sand` patch).
//!
//! Gated on the `parity` feature; skip-not-fail when the corpus or the
//! `mili` package is absent (mirrors the other `parity_*.rs` /
//! CLAUDE.md). All scratch files live under a unique temp dir that is
//! removed at the end — nothing is written into the corpus.

#![cfg(feature = "parity")]

mod parity_support;

use std::fs;
use std::path::{Path, PathBuf};

use parity_support::{corpus_path, skip_if_no_mili_python};
use pyo3::prelude::*;

use mili_rs::Database;

const FRAGS: [&str; 8] = ["000", "001", "002", "003", "004", "005", "006", "007"];

fn scratch_dir() -> PathBuf {
    let mut d = std::env::temp_dir();
    d.push(format!("mili_rs_parity_write_{}", std::process::id()));
    let _ = fs::remove_dir_all(&d);
    fs::create_dir_all(&d).expect("create scratch dir");
    d
}

/// Upstream `_MiliInternal(dir, base).copy_non_state_data(new_base)`.
fn oracle_copy(py: Python<'_>, dir: &Path, base: &str, new_base: &Path) -> PyResult<()> {
    let m = py.import_bound("mili.miliinternal")?;
    let db = m
        .getattr("_MiliInternal")?
        .call1((dir.to_str().unwrap(), base))?;
    db.call_method1("copy_non_state_data", (new_base.to_str().unwrap(),))?;
    Ok(())
}

/// Upstream `_MiliInternal(dir, base).append_state(100.0)`.
fn oracle_append(py: Python<'_>, dir: &Path, base: &str) -> PyResult<i64> {
    let m = py.import_bound("mili.miliinternal")?;
    let db = m
        .getattr("_MiliInternal")?
        .call1((dir.to_str().unwrap(), base))?;
    let r = db.call_method1("append_state", (100.0_f64,))?;
    r.extract()
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

#[test]
fn parity_write_append_d3samp6() {
    if skip_if_no_mili_python() {
        eprintln!("skip: mili-python not importable");
        return;
    }
    let src_dir = corpus_path(&["parallel", "d3samp6"]);
    if !src_dir.join("d3samp6.plt000A").exists() {
        eprintln!("skip: parallel/d3samp6 corpus absent");
        return;
    }

    let scratch = scratch_dir();
    let result = std::panic::catch_unwind(|| {
        Python::with_gil(|py| {
            for frag in FRAGS {
                let base = format!("d3samp6.plt{frag}");
                // Stage the fragment's A-file (+ any state files) so the
                // oracle and the Rust core read identical inputs and
                // never touch the corpus.
                for entry in fs::read_dir(&src_dir).unwrap() {
                    let p = entry.unwrap().path();
                    let name = p.file_name().unwrap().to_str().unwrap().to_owned();
                    if name.starts_with(&base) {
                        fs::copy(&p, scratch.join(&name)).unwrap();
                    }
                }

                // ---- copy_non_state_data ----------------------------
                oracle_copy(py, &scratch, &base, &scratch.join("g_d3samp6.plt"))
                    .expect("oracle copy_non_state_data");
                let rdb = Database::open(scratch.join(format!("{base}A"))).expect("open fragment");
                rdb.copy_non_state_data(scratch.join("r_d3samp6.plt").to_str().unwrap())
                    .expect("rust copy_non_state_data");
                let g_a = scratch.join(format!("g_d3samp6.plt{frag}A"));
                let r_a = scratch.join(format!("r_d3samp6.plt{frag}A"));
                assert_bytes_eq(&r_a, &g_a, &format!("copy_non_state_data[{frag}].A"));

                // ---- append_state on the copied 0-state db ----------
                let n = oracle_append(py, &scratch, &format!("g_d3samp6.plt{frag}"))
                    .expect("oracle append_state");
                assert_eq!(n, 1, "oracle append_state state count [{frag}]");

                let rdb2 = Database::open(scratch.join(format!("r_d3samp6.plt{frag}A")))
                    .expect("open copied db");
                let rn = rdb2
                    .append_state(100.0, true, None, None)
                    .expect("rust append_state");
                assert_eq!(rn, 1, "rust append_state state count [{frag}]");

                assert_bytes_eq(
                    &scratch.join(format!("r_d3samp6.plt{frag}A")),
                    &scratch.join(format!("g_d3samp6.plt{frag}A")),
                    &format!("append_state[{frag}].A"),
                );
                assert_bytes_eq(
                    &scratch.join(format!("r_d3samp6.plt{frag}00")),
                    &scratch.join(format!("g_d3samp6.plt{frag}00")),
                    &format!("append_state[{frag}].state00"),
                );

                // Tidy this fragment's scratch before the next one so a
                // stale `g_*`/`r_*` can't mask a later failure.
                for entry in fs::read_dir(&scratch).unwrap() {
                    let p = entry.unwrap().path();
                    let nm = p.file_name().unwrap().to_str().unwrap().to_owned();
                    if nm.starts_with("g_") || nm.starts_with("r_") || nm.starts_with(&base) {
                        let _ = fs::remove_file(&p);
                    }
                }
            }
        });
    });
    let _ = fs::remove_dir_all(&scratch);
    if let Err(e) = result {
        std::panic::resume_unwind(e);
    }
}
