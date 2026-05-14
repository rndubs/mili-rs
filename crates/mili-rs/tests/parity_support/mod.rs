//! Cross-impl parity oracle: drives mili-python via pyo3 and returns
//! results in the same flat layout `Database::query` does.
//!
//! Gated behind the `parity` feature. Each parity test starts with a
//! `skip_if_no_mili_python()` guard so a missing `mili` import or an
//! absent fixture degrades to skip-not-fail, matching the existing
//! corpus-test convention in `tests/query_fixtures.rs`.

#![cfg(feature = "parity")]
#![allow(dead_code)] // Each `tests/*.rs` integration binary pulls in this
                     // module independently — not every binary uses every
                     // helper, so per-helper dead-code warnings are normal.

use std::path::{Path, PathBuf};

use pyo3::prelude::*;
use pyo3::types::{PyDict, PyList};

/// Path inside `reference/mili-python/tests/data/...`.
pub fn corpus_path(rel: &[&str]) -> PathBuf {
    let mut p = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("reference")
        .join("mili-python")
        .join("tests")
        .join("data");
    for c in rel {
        p = p.join(c);
    }
    p
}

/// True when the `mili` package isn't importable. Tests use this to
/// skip-not-fail when running on a checkout that hasn't `pip install`ed
/// the `reference/mili-python` submodule.
pub fn skip_if_no_mili_python() -> bool {
    Python::with_gil(|py| py.import_bound("mili").is_err())
}

/// `mili.reader.open_database(base, suppress_parallel=True)`.
///
/// `base` is the path without the `A` / `00` suffix — e.g.
/// `.../basic1/basic1.plt`, not `.../basic1/basic1.pltA`.
pub fn open_database<'py>(py: Python<'py>, base: &Path) -> PyResult<Bound<'py, PyAny>> {
    let reader = py.import_bound("mili.reader")?;
    let kwargs = PyDict::new_bound(py);
    kwargs.set_item("suppress_parallel", true)?;
    reader.call_method(
        "open_database",
        (base.to_str().expect("utf-8 path"),),
        Some(&kwargs),
    )
}

/// Optional filters in a `db.query(...)` call. `labels` / `states` /
/// `ips` are passed through as Python lists; `material` is forwarded as
/// an int when `Some`.
#[derive(Default, Clone)]
pub struct OracleQuery<'a> {
    pub labels: Option<&'a [i32]>,
    pub states: Option<&'a [i32]>,
    pub material: Option<i32>,
    pub ips: Option<&'a [i32]>,
}

/// Result of a mili-python query, flattened to match the Rust API's
/// state-slow / object-medium / atom-fast row-major order.
pub struct OracleResult {
    pub flat: Vec<f32>,
    pub shape: (usize, usize, usize),
    pub layout_labels: Vec<i32>,
    pub layout_states: Vec<i32>,
}

/// Run `db.query(svar, class, ...)` and flatten the f32 `data` array.
///
/// Asserts the result is f32 — caller picks the right helper for the
/// numtype it expects. (mili-python keeps doubles in `data` as f64; if
/// a parity test ever needs f64 we add a `_f64` sibling.)
pub fn query_f32(
    py: Python<'_>,
    db: &Bound<'_, PyAny>,
    svar: &str,
    class: &str,
    q: &OracleQuery<'_>,
) -> PyResult<OracleResult> {
    let kwargs = PyDict::new_bound(py);
    if let Some(s) = q.states {
        kwargs.set_item("states", PyList::new_bound(py, s))?;
    }
    if let Some(l) = q.labels {
        kwargs.set_item("labels", PyList::new_bound(py, l))?;
    }
    if let Some(m) = q.material {
        kwargs.set_item("material", m)?;
    }
    if let Some(ips) = q.ips {
        kwargs.set_item("ips", PyList::new_bound(py, ips))?;
    }
    let res = db.call_method("query", (svar, class), Some(&kwargs))?;
    let entry = res.get_item(svar)?;
    let data = entry.get_item("data")?;
    let layout = entry.get_item("layout")?;

    let shape: (usize, usize, usize) = data.getattr("shape")?.extract()?;
    let dtype_str: String = data.getattr("dtype")?.str()?.extract()?;
    assert_eq!(
        dtype_str, "float32",
        "parity helper only handles float32 results; got {dtype_str}"
    );

    let flat_obj = data.call_method0("flatten")?;
    let flat: Vec<f32> = flat_obj.extract()?;
    let layout_labels: Vec<i32> = layout
        .get_item("labels")?
        .call_method0("tolist")?
        .extract()?;
    let layout_states: Vec<i32> = layout
        .get_item("states")?
        .call_method0("tolist")?
        .extract()?;
    assert_eq!(
        flat.len(),
        shape.0 * shape.1 * shape.2,
        "mili-python flatten size mismatch with reported shape"
    );

    Ok(OracleResult {
        flat,
        shape,
        layout_labels,
        layout_states,
    })
}
