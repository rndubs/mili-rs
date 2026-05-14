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
//! is checked by a separate pyo3 harness that lands with the Phase-2
//! cross-impl parity tests; this file is the Rust-only baseline.

use std::path::{Path, PathBuf};

use criterion::{black_box, criterion_group, criterion_main, Criterion, Throughput};

use mili_rs::{Database, MeshId, QueryArgs, StateValues};

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
                    }))
                    .expect("query");
                black_box(r);
            }
        });
    });
    group.finish();
}

criterion_group!(
    benches,
    bench_open,
    bench_nodes,
    bench_query_single,
    bench_query_many
);
criterion_main!(benches);
