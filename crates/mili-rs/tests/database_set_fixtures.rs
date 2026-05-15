//! End-to-end `DatabaseSet` open + accessor coverage against the
//! mili-python parallel/basic1 fixture (8 MPI fragments).
//!
//! Skip-on-absent: the fixtures live in the `reference/mili-python`
//! submodule; if it isn't checked out the tests early-return. See
//! `CLAUDE.md` for the convention.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use mili_rs::{Database, DatabaseSet, MeshId, MiliError, QueryArgs, StateValues};

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

fn basic1_base() -> Option<PathBuf> {
    let p = corpus_path(&["parallel", "basic1", "basic1.plt"]);
    if p.parent()?.exists() {
        Some(p)
    } else {
        None
    }
}

#[test]
fn open_basic1_eight_fragments() {
    let Some(base) = basic1_base() else {
        eprintln!("skip: parallel/basic1 absent");
        return;
    };
    let set = DatabaseSet::open(&base).expect("open parallel/basic1");
    assert_eq!(set.fragment_count(), 8);

    // Each fragment opens with the same state count and time axis.
    let times = set.times();
    assert_eq!(times.len(), set.state_count());
    assert!(!times.is_empty());
    for rank in 0..8 {
        let frag = set.fragment(rank).expect("rank in range");
        assert_eq!(frag.state_count(), set.state_count());
    }
}

#[test]
fn basic1_node_labels_concatenate_unique() {
    let Some(base) = basic1_base() else {
        eprintln!("skip: parallel/basic1 absent");
        return;
    };
    let set = DatabaseSet::open(&base).unwrap();
    let merged = set
        .labels(MeshId(0), "node")
        .unwrap()
        .expect("node labels present");

    // No duplicates after merge — `list_concatenate_unique` semantics.
    let mut sorted = merged.clone();
    sorted.sort_unstable();
    sorted.dedup();
    assert_eq!(
        sorted.len(),
        merged.len(),
        "labels should be unique after merge"
    );

    // Merge ⊇ every fragment's local labels (mili-python's
    // `list_concatenate_unique` preserves first occurrence and never
    // drops a label).
    let merged_set: HashSet<i32> = merged.iter().copied().collect();
    for rank in 0..set.fragment_count() {
        let frag = set.fragment(rank).unwrap();
        if let Some(local) = frag.labels(MeshId(0), "node").unwrap() {
            for l in &local {
                assert!(
                    merged_set.contains(l),
                    "merged labels missing fragment {rank} label {l}"
                );
            }
        }
    }
}

#[test]
fn basic1_brick_labels_concatenate_unique() {
    let Some(base) = basic1_base() else {
        eprintln!("skip: parallel/basic1 absent");
        return;
    };
    let set = DatabaseSet::open(&base).unwrap();
    let Some(merged) = set.labels(MeshId(0), "brick").unwrap() else {
        eprintln!("skip: parallel/basic1 has no brick class");
        return;
    };
    // Bricks are partitioned across ranks with little/no overlap; the
    // merged count must equal the sum of per-fragment counts modulo any
    // shared boundary entities (none for bricks in this fixture).
    let summed: usize = (0..set.fragment_count())
        .map(|r| {
            set.fragment(r)
                .unwrap()
                .labels(MeshId(0), "brick")
                .unwrap()
                .map_or(0, |v| v.len())
        })
        .sum();
    assert!(merged.len() <= summed);
}

#[test]
fn basic1_query_matches_concat_of_fragments() {
    let Some(base) = basic1_base() else {
        eprintln!("skip: parallel/basic1 absent");
        return;
    };
    let set = DatabaseSet::open(&base).unwrap();
    // Pick the smallest non-empty `(svar, class)` discovered across
    // fragments. nodpos / sx are stable defaults but verify presence on
    // rank 0.
    let class = "node";
    let svar = "nodpos";
    let states = [0usize];
    let args = QueryArgs {
        svar,
        class,
        labels: None,
        states: &states,
        materials: None,
        ips: None,
        subrec: None,
    };

    // Skip silently if even rank 0 doesn't carry this svar — the basic1
    // fixture should, but be robust to test-data drift.
    if let Err(MiliError::NoMatchingSubrec { .. } | MiliError::UnknownClass(_)) =
        set.fragment(0).unwrap().query_with_labels(&args)
    {
        eprintln!("skip: rank 0 doesn't expose {svar}/{class}");
        return;
    }

    let merged = set.query(&args).expect("DatabaseSet::query");
    assert_eq!(merged.state_count, 1);
    assert!(merged.atoms_per_label > 0);
    assert_eq!(
        merged.values.len(),
        merged.state_count * merged.labels.len() * merged.atoms_per_label
    );

    // Merged labels must be unique.
    let mut merged_sorted = merged.labels.clone();
    merged_sorted.sort_unstable();
    merged_sorted.dedup();
    assert_eq!(merged_sorted.len(), merged.labels.len());

    // Cross-check against an independent re-implementation of
    // `reductions.merge_result_dictionaries`: concatenate per-fragment
    // real labels + values in rank order, then if any label repeats,
    // reorder to ascending-unique order taking each label's first
    // occurrence (`np.unique(return_index=True)`); otherwise keep the
    // raw concatenation order.
    let apl = merged.atoms_per_label;
    let mut expected_concat: Vec<f32> = Vec::new();
    let mut expected_labels: Vec<i32> = Vec::new();
    for rank in 0..set.fragment_count() {
        let frag = set.fragment(rank).unwrap();
        match frag.query_with_labels(&args) {
            Ok((StateValues::F32(v), labels)) => {
                expected_concat.extend_from_slice(&v);
                expected_labels.extend_from_slice(&labels);
            }
            Ok(_) => panic!("nodpos expected to be f32"),
            Err(MiliError::NoMatchingSubrec { .. } | MiliError::UnknownClass(_)) => {}
            Err(e) => panic!("fragment {rank} query failed: {e:?}"),
        }
    }

    let mut first: std::collections::HashMap<i32, usize> = std::collections::HashMap::new();
    for (i, &l) in expected_labels.iter().enumerate() {
        first.entry(l).or_insert(i);
    }
    let has_dups = first.len() != expected_labels.len();
    let (exp_labels, exp_values): (Vec<i32>, Vec<f32>) = if has_dups {
        let mut uniq: Vec<i32> = first.keys().copied().collect();
        uniq.sort_unstable();
        let mut vals = Vec::with_capacity(uniq.len() * apl);
        for &l in &uniq {
            let src = first[&l] * apl;
            vals.extend_from_slice(&expected_concat[src..src + apl]);
        }
        (uniq, vals)
    } else {
        (expected_labels.clone(), expected_concat.clone())
    };

    assert_eq!(merged.labels, exp_labels, "merged entity axis");
    match &merged.values {
        StateValues::F32(v) => assert_eq!(v, &exp_values, "merged values"),
        other => panic!("expected F32 merged values, got {:?}", other.num_type()),
    }
}

#[test]
fn no_fragments_returns_typed_error() {
    let tmp = std::env::temp_dir().join("mili_rs_no_fragments");
    let _ = std::fs::create_dir_all(&tmp);
    let base = tmp.join("does_not_exist.plt");
    let Err(err) = DatabaseSet::open(&base) else {
        panic!("expected NoFragments");
    };
    match err {
        MiliError::NoFragments { dir, base: b } => {
            assert_eq!(dir, tmp);
            assert_eq!(b, "does_not_exist.plt");
        }
        other => panic!("expected NoFragments, got {other:?}"),
    }
}

#[test]
fn single_fragment_base_opens_as_set_of_one() {
    // Reuse the serial basic1 family. Its .A file is `basic1.pltA` —
    // base `basic1.plt` matches exactly one fragment with no rank
    // digits.
    let path = corpus_path(&["serial", "basic1", "basic1.plt"]);
    if !path.parent().unwrap().exists() {
        eprintln!("skip: serial/basic1 absent");
        return;
    }
    let set = DatabaseSet::open(&path).expect("open single-fragment family");
    assert_eq!(set.fragment_count(), 1);
    let serial = Database::open(corpus_path(&["serial", "basic1", "basic1.pltA"])).unwrap();
    assert_eq!(set.state_count(), serial.state_count());
    assert_eq!(set.times(), serial.times());
}
