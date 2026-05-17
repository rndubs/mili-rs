//! Phase 3.3 — cross-impl parity for `AppendStatesTool` (decision 24,
//! `planning/mili-py/phase-3.md` § Phase 3.3).
//!
//! `AppendStatesTool` is pure input-spec validation + orchestration
//! over the already-bit-exact `append_state` / `copy_non_state_data`
//! (Phase 3.1, `parity_write_append.rs`) and `query(write_data=)`
//! (Phase 3.2, `parity_write_query.rs`) primitives. The behavioural
//! redirect suite (`tests/test_upstream_readpath.py ::
//! test_append_states_tool`, 23 cases) already re-queries the written
//! state back; this gate adds the missing byte-level check: it drives
//! the **upstream `mili.append_states.AppendStatesTool`** on one staged
//! corpus copy and **`milox.append_states.AppendStatesTool`** on
//! another with the *identical* spec, then diffs every `.A` and state
//! file byte-for-byte.
//!
//! Gated on the `parity` feature; skip-not-fail when the corpus, the
//! `mili` package, or `milox` is absent. All scratch lives under a
//! unique temp dir removed at the end — the corpus is never touched.

#![cfg(feature = "parity")]

mod parity_support;

use std::fs;
use std::path::{Path, PathBuf};

use parity_support::skip_if_no_mili_python;
use pyo3::prelude::*;
use pyo3::types::PyDict;

fn corpus_serial_sstate() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("reference")
        .join("mili-python")
        .join("tests")
        .join("data")
        .join("serial")
        .join("sstate")
}

fn scratch_dir() -> PathBuf {
    let mut d = std::env::temp_dir();
    d.push(format!(
        "mili_rs_parity_append_states_tool_{}",
        std::process::id()
    ));
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

/// Byte-diff every file whose name starts with `base` present in both
/// dirs (the `.A`, the state files, and — for the `write` mode — the
/// freshly created output family).
fn assert_files_eq(a_dir: &Path, b_dir: &Path, prefix: &str, tag: &str) {
    let mut checked = 0;
    for entry in fs::read_dir(a_dir).unwrap() {
        let p = entry.unwrap().path();
        let name = p.file_name().unwrap().to_str().unwrap().to_owned();
        if !name.starts_with(prefix) {
            continue;
        }
        let bpath = b_dir.join(&name);
        assert!(
            bpath.exists(),
            "{tag}: {name} written by upstream but missing from milox output"
        );
        let av = fs::read(&p).unwrap();
        let bv = fs::read(&bpath).unwrap();
        assert_eq!(
            av.len(),
            bv.len(),
            "{tag}: {name} length mismatch (upstream={}, milox={})",
            av.len(),
            bv.len()
        );
        if let Some(i) = av.iter().zip(bv.iter()).position(|(x, y)| x != y) {
            let lo = i.saturating_sub(8);
            let hi = (i + 16).min(av.len());
            panic!(
                "{tag}: {name} byte divergence at {i}\n  upstream={:?}\n  milox   ={:?}",
                &av[lo..hi],
                &bv[lo..hi]
            );
        }
        checked += 1;
    }
    assert!(checked > 0, "{tag}: no files compared");
}

/// Drive `<module>.append_states.AppendStatesTool(spec).write_states()`
/// from inside `work_dir` (the tool's `output_mode="write"` path writes
/// the new family relative to cwd, mirroring the upstream test's
/// `os.chdir`-free relative-basename contract).
fn run_tool(py: Python<'_>, module: &str, work_dir: &Path, spec_py: &str) -> PyResult<()> {
    let os = py.import_bound("os")?;
    let cwd: String = os.call_method0("getcwd")?.extract()?;
    os.call_method1("chdir", (work_dir.to_str().unwrap(),))?;
    let run = || -> PyResult<()> {
        let globals = PyDict::new_bound(py);
        let modname = format!("{module}.append_states");
        let tool_mod = py.import_bound(modname.as_str())?;
        globals.set_item("AppendStatesTool", tool_mod.getattr("AppendStatesTool")?)?;
        py.run_bound(
            &format!(
                "spec = {spec_py}\n\
                 tool = AppendStatesTool(spec)\n\
                 tool.write_states()\n"
            ),
            Some(&globals),
            None,
        )
    };
    let res = run();
    os.call_method1("chdir", (cwd,))?;
    res
}

struct Case {
    tag: &'static str,
    /// Python expression for the spec dict; `{base}` is substituted
    /// with the (relative) staged basename before exec.
    spec: &'static str,
    /// Filename prefix to diff (the mutated input family for `append`,
    /// the created output family for `write`).
    diff_prefix: &'static str,
}

const BASE: &str = "v3_d3samp6.plt";

const CASES: &[Case] = &[
    Case {
        tag: "append",
        spec: r#"{
            "database_basename": "v3_d3samp6.plt",
            "output_type": ["mili"],
            "output_mode": "append",
            "states": 2,
            "time_inc": 0.001,
            "limit_states_per_file": 2,
            "state_variables": {
                "node": {"vy": {"labels": [100,101,102],
                    "data": [[100.0,100.0,100.0],[200.0,300.0,400.0]]}},
                "brick": {"sx": {"labels": [1,2,3],
                    "data": [[1.0,2.0,3.0],[4.0,5.0,6.0]]}},
            },
        }"#,
        diff_prefix: "v3_d3samp6.plt",
    },
    Case {
        tag: "write",
        spec: r#"{
            "database_basename": "v3_d3samp6.plt",
            "output_type": ["mili"],
            "output_mode": "write",
            "output_basename": "copy_d3samp6.plt",
            "states": 2,
            "time_inc": 0.001,
            "limit_bytes_per_file": 100,
            "state_variables": {
                "node": {"vy": {"labels": [100,101,102],
                    "data": [[100.0,100.0,100.0],[200.0,300.0,400.0]]}},
                "brick": {"sx": {"labels": [1,2,3],
                    "data": [[1.0,2.0,3.0],[4.0,5.0,6.0]]}},
            },
        }"#,
        diff_prefix: "copy_d3samp6.plt",
    },
];

#[test]
fn parity_write_append_states_tool() {
    if skip_if_no_mili_python() {
        eprintln!("skip: mili-python not importable");
        return;
    }
    let src = corpus_serial_sstate();
    if !src.join(format!("{BASE}A")).exists() {
        eprintln!("skip: serial/sstate corpus absent");
        return;
    }
    if Python::with_gil(|py| py.import_bound("milox").is_err()) {
        eprintln!("skip: milox not importable");
        return;
    }

    let scratch = scratch_dir();
    let result = std::panic::catch_unwind(|| {
        Python::with_gil(|py| {
            for (i, c) in CASES.iter().enumerate() {
                let odir = scratch.join(format!("o{i}"));
                let mdir = scratch.join(format!("m{i}"));
                stage(&src, BASE, &odir);
                stage(&src, BASE, &mdir);

                run_tool(py, "mili", &odir, c.spec)
                    .unwrap_or_else(|e| panic!("{}: upstream tool failed: {e}", c.tag));
                run_tool(py, "milox", &mdir, c.spec)
                    .unwrap_or_else(|e| panic!("{}: milox tool failed: {e}", c.tag));

                assert_files_eq(&odir, &mdir, c.diff_prefix, c.tag);
            }
        });
    });
    let _ = fs::remove_dir_all(&scratch);
    if let Err(e) = result {
        std::panic::resume_unwind(e);
    }
}
