//! M4-followup Phase I.1 (parallel per-proc-unmerged surface) —
//! cross-impl parity of the per-fragment read accessors vs. the
//! upstream per-proc `_MiliInternal`.
//!
//! [`DatabaseSet::fragment(rank)`] is the `Database` the PyO3
//! `*_per_fragment()` surface (Phase I.1) exposes verbatim — no merge.
//! This gate asserts each fragment is bit-exact against
//! `mili.miliinternal._MiliInternal(dir, "<stem><rank:03>")`, the
//! per-proc reader upstream's `LoopWrapper`/`ServerWrapper` wrap with
//! `merge_results=False`, over the two parallel corpora the parallel
//! tests use (`parallel/d3samp6`, `parallel/basic1`).
//!
//! Gated on the `parity` feature; skip-not-fail when the corpus or the
//! `mili` package is absent (mirrors `parity_geometry.rs` /
//! `parity_adjacency.rs` / CLAUDE.md).

#![cfg(feature = "parity")]

mod parity_support;

use parity_support::{corpus_path, skip_if_no_mili_python};
use pyo3::prelude::*;
use pyo3::types::PyDict;

use mili_rs::{Database, DatabaseSet, MeshId, QueryArgs, StateValues};

struct Fx {
    rel_dir: &'static str,
    /// Family base (`DatabaseSet::open` re-discovers the fragments).
    base: &'static str,
    /// Per-proc base stem; the proc base is `format!("{stem}{rank:03}")`.
    proc_stem: &'static str,
    n_procs: usize,
}

const CORPUS: &[Fx] = &[
    Fx {
        rel_dir: "d3samp6",
        base: "d3samp6.plt",
        proc_stem: "d3samp6.plt",
        n_procs: 8,
    },
    Fx {
        rel_dir: "basic1",
        base: "basic1.plt",
        proc_stem: "basic1.plt",
        n_procs: 8,
    },
];

fn canonical_mesh(db: &Database) -> MeshId {
    db.meshes()
        .meshes()
        .map(|m| m.id)
        .min()
        .unwrap_or(MeshId(0))
}

/// Upstream per-proc `_MiliInternal(dir, "<stem><rank:03>")`.
fn proc_internal<'py>(
    py: Python<'py>,
    dir: &std::path::Path,
    stem: &str,
    rank: usize,
) -> PyResult<Bound<'py, PyAny>> {
    let base = format!("{stem}{rank:03}");
    py.import_bound("mili.miliinternal")?
        .getattr("_MiliInternal")?
        .call1((dir.to_str().expect("utf8"), base))
}

fn assert_i32_eq(rust: &[i32], oracle: &[i32], tag: &str) {
    assert_eq!(
        rust,
        oracle,
        "{tag}: i32 vec mismatch (rust len {}, py len {})",
        rust.len(),
        oracle.len()
    );
}

fn assert_f32_bits(rust: &[f32], oracle: &[f32], tag: &str) {
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
            "{tag}: divergence at {i}: rust={r} py={p}"
        );
    }
}

fn run_corpus(fx: &Fx) {
    let dir = corpus_path(&["parallel", fx.rel_dir]);
    let base = dir.join(fx.base);
    if !dir.exists() {
        eprintln!("skip: parallel/{} absent", fx.rel_dir);
        return;
    }
    if skip_if_no_mili_python() {
        eprintln!("skip: mili-python not importable");
        return;
    }

    let set = DatabaseSet::open(&base).expect("open DatabaseSet");
    assert_eq!(
        set.fragment_count(),
        fx.n_procs,
        "{}: fragment count",
        fx.rel_dir
    );

    Python::with_gil(|py| {
        for rank in 0..fx.n_procs {
            let frag = set.fragment(rank).expect("fragment in range");
            let mesh = canonical_mesh(frag);
            let mi = proc_internal(py, &dir, fx.proc_stem, rank)
                .unwrap_or_else(|e| panic!("{}: open proc {rank}: {e}", fx.rel_dir));
            let tag = |s: &str| format!("{}[proc {rank}] {s}", fx.rel_dir);

            // ---- scalars ----
            let py_dims: i32 = mi
                .call_method0("mesh_dimensions")
                .unwrap()
                .extract()
                .unwrap();
            assert_eq!(
                frag.mesh_dimensions().unwrap(),
                py_dims,
                "{}",
                tag("mesh_dimensions")
            );
            let py_sc: usize = mi.call_method0("state_count").unwrap().extract().unwrap();
            assert_eq!(frag.state_count(), py_sc, "{}", tag("state_count"));

            // ---- times (f64) ----
            let py_times: Vec<f64> = mi
                .call_method0("times")
                .unwrap()
                .call_method0("tolist")
                .unwrap()
                .extract()
                .unwrap();
            let rust_times: Vec<f64> = frag.times().into_iter().map(f64::from).collect();
            assert_eq!(rust_times.len(), py_times.len(), "{}", tag("times len"));
            for (i, (r, p)) in rust_times.iter().zip(py_times.iter()).enumerate() {
                assert_eq!(
                    r.to_bits(),
                    p.to_bits(),
                    "{}: state {i} rust={r} py={p}",
                    tag("times")
                );
            }

            // ---- material_numbers ----
            let py_matn: Vec<i32> = mi
                .call_method0("material_numbers")
                .unwrap()
                .call_method0("tolist")
                .unwrap()
                .extract()
                .unwrap();
            assert_i32_eq(
                &frag.material_numbers().unwrap(),
                &py_matn,
                &tag("material_numbers"),
            );

            // ---- class_names (same set; ordering is not a value
            // contract — the numeric accessors below are the bit-exact
            // gate) ----
            let py_classes: Vec<String> =
                mi.call_method0("class_names").unwrap().extract().unwrap();
            let mut rust_classes = frag.class_names(mesh);
            let mut py_sorted = py_classes.clone();
            rust_classes.sort();
            py_sorted.sort();
            assert_eq!(rust_classes, py_sorted, "{}", tag("class_names set"));

            // ---- labels per class (order IS a parity contract) ----
            let py_labels: Bound<'_, PyDict> =
                mi.call_method0("labels").unwrap().downcast_into().unwrap();
            for (k, v) in py_labels.iter() {
                let cls: String = k.extract().unwrap();
                let py_l: Vec<i32> = v.call_method0("tolist").unwrap().extract().unwrap();
                let rust_l = frag.labels(mesh, &cls).unwrap().unwrap_or_default();
                assert_i32_eq(&rust_l, &py_l, &tag(&format!("labels[{cls}]")));
            }

            // ---- nodes (f32 bits) ----
            let py_nodes_arr = mi.call_method0("nodes").unwrap();
            let py_nodes: Vec<f32> = py_nodes_arr
                .call_method0("flatten")
                .unwrap()
                .call_method0("tolist")
                .unwrap()
                .extract()
                .unwrap();
            let (rust_nodes, _dims) = frag.node_coords(mesh).unwrap().unwrap_or_default();
            assert_f32_bits(&rust_nodes, &py_nodes, &tag("nodes"));

            // ---- connectivity_ids per element class ----
            let py_conn: Bound<'_, PyDict> = mi
                .call_method0("connectivity_ids")
                .unwrap()
                .downcast_into()
                .unwrap();
            for (k, v) in py_conn.iter() {
                let cls: String = k.extract().unwrap();
                let py_c: Vec<i32> = v
                    .call_method0("flatten")
                    .unwrap()
                    .call_method0("tolist")
                    .unwrap()
                    .extract()
                    .unwrap();
                let rust_c = frag
                    .connectivity_ids(mesh, &cls)
                    .unwrap()
                    .map(|(d, _)| d)
                    .unwrap_or_default();
                assert_i32_eq(&rust_c, &py_c, &tag(&format!("connectivity_ids[{cls}]")));
            }

            // ---- primal query: nodpos / node (f32 + entity axis) ----
            let states: Vec<usize> = (0..frag.state_count()).collect();
            let (vals, rust_lbls) = frag
                .query_with_labels(&QueryArgs {
                    svar: "nodpos",
                    class: "node",
                    labels: None,
                    states: &states,
                    materials: None,
                    ips: None,
                    subrec: None,
                })
                .expect("rust nodpos/node");
            let StateValues::F32(rust_v) = vals else {
                panic!("nodpos is f32");
            };
            let entry = mi
                .call_method1("query", ("nodpos", "node"))
                .unwrap()
                .get_item("nodpos")
                .unwrap();
            let py_v: Vec<f32> = entry
                .get_item("data")
                .unwrap()
                .call_method0("flatten")
                .unwrap()
                .call_method0("tolist")
                .unwrap()
                .extract()
                .unwrap();
            let py_lbls: Vec<i32> = entry
                .get_item("layout")
                .unwrap()
                .get_item("labels")
                .unwrap()
                .call_method0("tolist")
                .unwrap()
                .extract()
                .unwrap();
            assert_i32_eq(&rust_lbls, &py_lbls, &tag("nodpos/node labels"));
            assert_f32_bits(&rust_v, &py_v, &tag("nodpos/node data"));
        }
    });
}

#[test]
fn parity_per_fragment_d3samp6() {
    run_corpus(&CORPUS[0]);
}

#[test]
fn parity_per_fragment_basic1() {
    run_corpus(&CORPUS[1]);
}
