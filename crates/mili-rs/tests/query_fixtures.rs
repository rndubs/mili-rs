//! Single-svar single-state query parity against the reference
//! corpus. Skips silently when the submodule is absent.
//!
//! mili-python's existing golden assertions for `basic1` all live on
//! the parallel variant (`data/parallel/basic1/`) or require
//! integration-point filtering (Step 11), so the goldens-from-Python
//! pinning lands later. For Step 9 we lean on:
//!
//! 1. Shape — basic1's `nodpos` over 1400 nodes gives `1400 * 3` f32s.
//! 2. Self-consistency — the API output equals a direct decode of the
//!    bytes at the offset computed by hand from the C formula
//!    `state.offset + 8 + sum_prior_subrec_sizes + N * lump_offsets[s]`
//!    (`reference/mili/src/srec.c:2332-2333`).
//! 3. OBJECT_ORDERED rejection — basic1's brick subrecs are
//!    object-ordered, so querying a brick-only svar returns the
//!    Step-10 `Unsupported` error.
//! 4. Error semantics — unknown svar / class / state.

use std::path::{Path, PathBuf};

use mili_rs::{Database, MiliError, StateValues};

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

#[test]
fn basic1_nodpos_state_zero_has_full_shape() {
    let path = corpus_path(&["serial", "basic1", "basic1.pltA"]);
    if !path.exists() {
        eprintln!("skip: basic1 absent");
        return;
    }
    let db = Database::open(&path).expect("open basic1");

    let values = db
        .state_var_values("nodpos", "node", 0)
        .expect("read nodpos at state 0");
    match values {
        StateValues::F32(v) => assert_eq!(v.len(), 1400 * 3),
        other => panic!("expected f32 for nodpos, got {:?}", other.num_type()),
    }
}

#[test]
fn basic1_nodvel_at_state_50_self_consistent() {
    // basic1's first state-file is `basic1.plt00`. With the node subrec
    // at index 4 holding [nodpos(3), nodvel(3), nodacc(3)] in
    // RESULT_ORDERED over N=1400 nodes, nodvel's slab is at
    //   state.offset + 8                                  (per-state hdr)
    //   + sum_{i<4} N_i * bytes_per_obj_i = 0+0+504+24    (prior subrecs)
    //   + N * lump_offsets[1] = 1400 * 12 = 16800         (nodvel slab)
    // Read those bytes directly and compare against the API.
    let path = corpus_path(&["serial", "basic1", "basic1.pltA"]);
    if !path.exists() {
        return;
    }
    let db = Database::open(&path).unwrap();

    let state = db.states()[50];
    let state_file = corpus_path(&["serial", "basic1", "basic1.plt00"]);
    let raw = std::fs::read(&state_file).expect("read state file");

    let slab_start = (state.offset as usize) + 8 + 528 + 16800;
    let slab_len = 1400 * 3 * 4;
    let bytes = &raw[slab_start..slab_start + slab_len];
    let mut direct: Vec<f32> = Vec::with_capacity(1400 * 3);
    for chunk in bytes.chunks_exact(4) {
        direct.push(f32::from_le_bytes(chunk.try_into().unwrap()));
    }

    let api = db.state_var_values("nodvel", "node", 50).unwrap();
    let StateValues::F32(v) = api else {
        panic!("nodvel should decode as f32");
    };
    assert_eq!(v.len(), direct.len());
    for (a, b) in v.iter().zip(direct.iter()) {
        assert_eq!(
            a.to_bits(),
            b.to_bits(),
            "API result diverges from direct decode"
        );
    }
}

#[test]
fn basic1_object_ordered_subrec_errors_until_step_10() {
    let path = corpus_path(&["serial", "basic1", "basic1.pltA"]);
    if !path.exists() {
        return;
    }
    let db = Database::open(&path).unwrap();
    // basic1's `sand` lives in OBJECT_ORDERED brick subrecs. Step 10
    // adds the OBJECT_ORDERED gather; Step 9 surfaces a clean error.
    let err = db.state_var_values("sand", "brick", 0).unwrap_err();
    assert!(
        matches!(err, MiliError::Unsupported(_)),
        "expected Unsupported, got {err:?}"
    );
}

#[test]
fn basic1_unknown_svar_errors() {
    let path = corpus_path(&["serial", "basic1", "basic1.pltA"]);
    if !path.exists() {
        return;
    }
    let db = Database::open(&path).unwrap();
    assert!(matches!(
        db.state_var_values("not_a_real_svar", "node", 0)
            .unwrap_err(),
        MiliError::UnknownSvar(_)
    ));
}

#[test]
fn basic1_unknown_class_errors() {
    let path = corpus_path(&["serial", "basic1", "basic1.pltA"]);
    if !path.exists() {
        return;
    }
    let db = Database::open(&path).unwrap();
    // 'nodpos' exists but only on the 'node' class — querying it on
    // 'brick' must surface NoMatchingSubrec.
    let err = db.state_var_values("nodpos", "brick", 0).unwrap_err();
    assert!(matches!(err, MiliError::NoMatchingSubrec { .. }));
}

#[test]
fn basic1_state_out_of_range_errors() {
    let path = corpus_path(&["serial", "basic1", "basic1.pltA"]);
    if !path.exists() {
        return;
    }
    let db = Database::open(&path).unwrap();
    let n = db.state_count();
    assert!(matches!(
        db.state_var_values("nodpos", "node", n).unwrap_err(),
        MiliError::StateOutOfRange(_, _)
    ));
}
