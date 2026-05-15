//! Criterion read-path benches.
//!
//! Four groups, each gated on the `reference/mili-python` submodule
//! being populated. If the corpus is absent the bench is skipped so a
//! local `cargo bench` against a clean checkout doesn't hard-fail.
//!
//! Targets per `planning/mili-rs/plan.md` § "Benchmarks":
//! - `open`         — `Database::open(path)` wall time.
//! - `nodes`        — `db.nodes(MeshId(0), "node")` materialization.
//! - `query_single` — single-svar, all-states, all-objects.
//! - `query_many`   — same query repeated over a handful of svars.
//!
//! Phase-1 pass criterion (≥ 2× mili-python throughput on `query_*`)
//! is checked under the `parity` cargo feature: the
//! `mili_python_baseline` bench group runs the same workload through
//! `mili.reader.open_database` via pyo3, so a single `cargo bench
//! --features parity` produces side-by-side numbers. Without the
//! feature this file is the Rust-only baseline.

use std::path::{Path, PathBuf};

use criterion::{black_box, criterion_group, criterion_main, Criterion, Throughput};

use mili_rs::{Database, MeshId, QueryArgs, StateValues};

#[cfg(feature = "parity")]
use pyo3::prelude::*;
#[cfg(feature = "parity")]
use pyo3::types::{PyDict, PyList};

fn corpus_path(rel: &[&str]) -> PathBuf {
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

fn bench_open(c: &mut Criterion) {
    let path = corpus_path(&["serial", "basic1", "basic1.pltA"]);
    if !path.exists() {
        eprintln!("skip bench_open: basic1 absent");
        return;
    }
    let mut group = c.benchmark_group("open");
    group.bench_function("basic1", |b| {
        b.iter(|| {
            let db = Database::open(black_box(&path)).expect("open");
            black_box(db);
        });
    });
    group.finish();
}

fn bench_nodes(c: &mut Criterion) {
    let path = corpus_path(&["serial", "basic1", "basic1.pltA"]);
    if !path.exists() {
        eprintln!("skip bench_nodes: basic1 absent");
        return;
    }
    let db = Database::open(&path).expect("open");
    let mut group = c.benchmark_group("nodes");
    group.bench_function("basic1", |b| {
        b.iter(|| {
            let nodes = db
                .nodes(MeshId(0), "node")
                .expect("nodes")
                .expect("present");
            black_box(nodes);
        });
    });
    group.finish();
}

fn bench_query_single(c: &mut Criterion) {
    let path = corpus_path(&["serial", "basic1", "basic1.pltA"]);
    if !path.exists() {
        eprintln!("skip bench_query_single: basic1 absent");
        return;
    }
    let db = Database::open(&path).expect("open");
    let states: Vec<usize> = (0..db.state_count()).collect();

    let mut group = c.benchmark_group("query_single");
    // Throughput is the number of f32 atoms produced per call:
    // 1400 nodes × 3 components × N states.
    if let StateValues::F32(v) = db
        .query(&QueryArgs {
            svar: "nodpos",
            class: "node",
            labels: None,
            states: &states,
            materials: None,
            ips: None,
            subrec: None,
        })
        .expect("query")
    {
        group.throughput(Throughput::Bytes((v.len() * 4) as u64));
    }
    group.bench_function("basic1_nodpos_all_states", |b| {
        b.iter(|| {
            let r = db
                .query(black_box(&QueryArgs {
                    svar: "nodpos",
                    class: "node",
                    labels: None,
                    states: &states,
                    materials: None,
                    ips: None,
                    subrec: None,
                }))
                .expect("query");
            black_box(r);
        });
    });
    group.finish();
}

fn bench_query_many(c: &mut Criterion) {
    let path = corpus_path(&["serial", "basic1", "basic1.pltA"]);
    if !path.exists() {
        eprintln!("skip bench_query_many: basic1 absent");
        return;
    }
    let db = Database::open(&path).expect("open");
    let states: Vec<usize> = (0..db.state_count()).collect();
    // Pick a handful of distinct svars that share the `node` and
    // `brick` classes in basic1 — the bench loops over them serially
    // so the assembly cost (plan + dispatch) is measurable per call.
    let svars: &[(&str, &str)] = &[
        ("nodpos", "node"),
        ("nodvel", "node"),
        ("nodacc", "node"),
        ("sand", "brick"),
    ];

    let mut group = c.benchmark_group("query_many");
    group.bench_function("basic1_four_svars_all_states", |b| {
        b.iter(|| {
            for (svar, class) in svars {
                let r = db
                    .query(black_box(&QueryArgs {
                        svar,
                        class,
                        labels: None,
                        states: &states,
                        materials: None,
                        ips: None,
                        subrec: None,
                    }))
                    .expect("query");
                black_box(r);
            }
        });
    });
    group.finish();
}

#[cfg(feature = "parity")]
fn py_states_one_based(rust_states: &[usize]) -> Vec<i32> {
    rust_states.iter().map(|&s| (s as i32) + 1).collect()
}

#[cfg(feature = "parity")]
fn py_open<'py>(py: Python<'py>, base: &Path) -> PyResult<Bound<'py, PyAny>> {
    let reader = py.import_bound("mili.reader")?;
    let kwargs = PyDict::new_bound(py);
    kwargs.set_item("suppress_parallel", true)?;
    reader.call_method(
        "open_database",
        (base.to_str().expect("utf-8 path"),),
        Some(&kwargs),
    )
}

#[cfg(feature = "parity")]
fn py_query<'py>(
    py: Python<'py>,
    db: &Bound<'py, PyAny>,
    svar: &str,
    class: &str,
    states: &[i32],
) -> PyResult<Bound<'py, PyAny>> {
    let kwargs = PyDict::new_bound(py);
    kwargs.set_item("states", PyList::new_bound(py, states))?;
    db.call_method("query", (svar, class), Some(&kwargs))
}

/// mili-python baseline for the same workload as `bench_query_single`
/// / `bench_query_many`. Pass criterion for Phase 1: the Rust groups
/// land ≥ 2× the throughput of these (see `planning/mili-rs/plan.md`
/// § "Benchmarks").
#[cfg(feature = "parity")]
fn bench_python_baseline(c: &mut Criterion) {
    let plt_a = corpus_path(&["serial", "basic1", "basic1.pltA"]);
    let base = corpus_path(&["serial", "basic1", "basic1.plt"]);
    if !plt_a.exists() {
        eprintln!("skip mili_python_baseline: basic1 absent");
        return;
    }
    let mili_importable = Python::with_gil(|py| py.import_bound("mili").is_ok());
    if !mili_importable {
        eprintln!("skip mili_python_baseline: mili-python not importable");
        return;
    }
    let rust_db = Database::open(&plt_a).expect("open");
    let rust_states: Vec<usize> = (0..rust_db.state_count()).collect();
    let py_states = py_states_one_based(&rust_states);

    let nodpos_bytes = match rust_db
        .query(&QueryArgs {
            svar: "nodpos",
            class: "node",
            labels: None,
            states: &rust_states,
            materials: None,
            ips: None,
            subrec: None,
        })
        .expect("rust nodpos")
    {
        StateValues::F32(v) => (v.len() * 4) as u64,
        _ => 0,
    };

    let mut group = c.benchmark_group("mili_python_baseline");
    group.throughput(Throughput::Bytes(nodpos_bytes));
    group.bench_function("basic1_nodpos_all_states", |b| {
        Python::with_gil(|py| {
            let pdb = py_open(py, &base).expect("py open");
            b.iter(|| {
                let r = py_query(py, &pdb, "nodpos", "node", &py_states).expect("py query");
                black_box(r);
            });
        });
    });
    group.bench_function("basic1_four_svars_all_states", |b| {
        let svars: &[(&str, &str)] = &[
            ("nodpos", "node"),
            ("nodvel", "node"),
            ("nodacc", "node"),
            ("sand", "brick"),
        ];
        Python::with_gil(|py| {
            let pdb = py_open(py, &base).expect("py open");
            b.iter(|| {
                for (svar, class) in svars {
                    let r = py_query(py, &pdb, svar, class, &py_states).expect("py query");
                    black_box(r);
                }
            });
        });
    });
    group.finish();
}

#[cfg(feature = "parity")]
criterion_group!(
    benches,
    bench_open,
    bench_nodes,
    bench_query_single,
    bench_query_many,
    bench_python_baseline,
);
#[cfg(not(feature = "parity"))]
criterion_group!(
    benches,
    bench_open,
    bench_nodes,
    bench_query_single,
    bench_query_many,
);
criterion_main!(benches);
