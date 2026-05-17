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
//! 3. OBJECT_ORDERED gather — basic1's brick subrecs are
//!    object-ordered, so querying a brick-only svar must return
//!    typed values from the OO offset math.
//! 4. Error semantics — unknown svar / class / state.

use std::path::{Path, PathBuf};

use mili_rs::{Database, MiliError, QueryArgs, StateValues};

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
    //   state.offset + 8                                   (per-state hdr)
    //   + sum_{i<4} N_i * bytes_per_obj_i = 76+84+504+24   (prior subrecs)
    //   + N * lump_offsets[1] = 1400 * 12 = 16800          (nodvel slab)
    // The 76 + 84 bytes are the M_MESH-superclass `glob` and `cpu_time`
    // subrecs — basic1's writer emits `block_count = 0` for them, and
    // `SrecTable::patch_m_mesh_classes` synthesises `(1, 1)` at open
    // time so the offset math accounts for the one object's worth of
    // data each carries. (`cpu_time` is a 21-atom vector — that's the
    // 84.) See `reference/mili-python/src/mili/afileIO.py:439-441`.
    let path = corpus_path(&["serial", "basic1", "basic1.pltA"]);
    if !path.exists() {
        return;
    }
    let db = Database::open(&path).unwrap();

    let state = db.states()[50];
    let state_file = corpus_path(&["serial", "basic1", "basic1.plt00"]);
    let raw = std::fs::read(&state_file).expect("read state file");

    let slab_start = (state.offset as usize) + 8 + 688 + 16800;
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
fn basic1_object_ordered_sand_decodes_one_f32_per_brick() {
    let path = corpus_path(&["serial", "basic1", "basic1.pltA"]);
    if !path.exists() {
        return;
    }
    let db = Database::open(&path).unwrap();
    // basic1's `sand` lives in OBJECT_ORDERED brick subrecs (one f32
    // per element). The brick class spans 36 elements in basic1.
    let values = db.state_var_values("sand", "brick", 0).unwrap();
    let StateValues::F32(v) = values else {
        panic!("expected f32 for sand");
    };
    let bricks = db
        .meshes()
        .mesh(mili_rs::MeshId(0))
        .unwrap()
        .class("brick")
        .unwrap()
        .element_count() as usize;
    assert_eq!(v.len(), bricks);
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
fn basic1_nodpos_label_filter_returns_subset_in_ascending_class_order() {
    let path = corpus_path(&["serial", "basic1", "basic1.pltA"]);
    if !path.exists() {
        return;
    }
    let db = Database::open(&path).unwrap();

    // Pull all-nodes nodpos at state 0, then a 3-label subset and
    // confirm bytes match the corresponding rows of the full read.
    // The entity axis follows class-label-array (ascending) order,
    // *not* the argument order, mirroring upstream
    // `np.where(np.isin(labels_of_class, labels))[0]`
    // (`miliinternal.py:1183`) — verified bit-exact vs the `mili`
    // oracle (`basic1` `node`, labels `[5,1,1400]` → `[1,5,1400]`).
    let all = db.state_var_values("nodpos", "node", 0).unwrap();
    let StateValues::F32(all) = all else {
        panic!("nodpos is f32");
    };
    let labels = [5_i32, 1, 1400];
    let states = [0_usize];
    let subset = db
        .query(&QueryArgs {
            svar: "nodpos",
            class: "node",
            labels: Some(&labels),
            states: &states,
            materials: None,
            ips: None,
            subrec: None,
        })
        .unwrap();
    let StateValues::F32(subset) = subset else {
        panic!("nodpos is f32");
    };
    assert_eq!(subset.len(), 3 * 3);
    // basic1 `node` labels are the contiguous `1..=1400`, so ascending
    // class order is simply the sorted request.
    let expected_order = [1_i32, 5, 1400];
    for (i, &label) in expected_order.iter().enumerate() {
        let ord = (label - 1) as usize;
        for c in 0..3 {
            assert_eq!(
                subset[i * 3 + c].to_bits(),
                all[ord * 3 + c].to_bits(),
                "label {label}, comp {c}"
            );
        }
    }
}

#[test]
fn basic1_multi_state_nodpos_concatenates_in_state_order() {
    let path = corpus_path(&["serial", "basic1", "basic1.pltA"]);
    if !path.exists() {
        return;
    }
    let db = Database::open(&path).unwrap();
    let s0 = db.state_var_values("nodpos", "node", 0).unwrap();
    let s50 = db.state_var_values("nodpos", "node", 50).unwrap();
    let StateValues::F32(s0) = s0 else {
        unreachable!()
    };
    let StateValues::F32(s50) = s50 else {
        unreachable!()
    };

    let states = [0_usize, 50];
    let multi = db
        .query(&QueryArgs {
            svar: "nodpos",
            class: "node",
            labels: None,
            states: &states,
            materials: None,
            ips: None,
            subrec: None,
        })
        .unwrap();
    let StateValues::F32(multi) = multi else {
        unreachable!()
    };
    assert_eq!(multi.len(), s0.len() + s50.len());
    for (a, b) in multi.iter().zip(s0.iter().chain(s50.iter())) {
        assert_eq!(a.to_bits(), b.to_bits());
    }
}

#[test]
fn basic1_label_filter_routes_to_object_ordered_brick() {
    let path = corpus_path(&["serial", "basic1", "basic1.pltA"]);
    if !path.exists() {
        return;
    }
    let db = Database::open(&path).unwrap();
    let all = db.state_var_values("sand", "brick", 0).unwrap();
    let StateValues::F32(all) = all else {
        panic!("sand is f32");
    };
    let labels = [3_i32, 1];
    let states = [0_usize];
    let subset = db
        .query(&QueryArgs {
            svar: "sand",
            class: "brick",
            labels: Some(&labels),
            states: &states,
            materials: None,
            ips: None,
            subrec: None,
        })
        .unwrap();
    let StateValues::F32(subset) = subset else {
        panic!("sand is f32");
    };
    // Entity axis is class-label-array (ascending) order, not the
    // argument order — `[3,1]` → `[1,3]` (oracle-faithful, see
    // `basic1_nodpos_label_filter_returns_subset_in_ascending_class_order`).
    assert_eq!(subset.len(), 2);
    assert_eq!(subset[0].to_bits(), all[0].to_bits()); // label 1 -> ord 0
    assert_eq!(subset[1].to_bits(), all[2].to_bits()); // label 3 -> ord 2
}

#[test]
fn basic1_label_not_found_errors() {
    let path = corpus_path(&["serial", "basic1", "basic1.pltA"]);
    if !path.exists() {
        return;
    }
    let db = Database::open(&path).unwrap();
    let labels = [999_999_i32];
    let states = [0_usize];
    let err = db
        .query(&QueryArgs {
            svar: "nodpos",
            class: "node",
            labels: Some(&labels),
            states: &states,
            materials: None,
            ips: None,
            subrec: None,
        })
        .unwrap_err();
    assert!(matches!(err, MiliError::LabelNotFound { .. }));
}

#[test]
fn d3samp4_vec_array_ip_filter_slices_components_fastest_layout() {
    // d3samp4's `es_1a` is a vec_array svar with dims=[2] (two
    // integration points) and components ["stress" (6 atoms), "eps"
    // (1 atom)] — 14 atoms per object total. We confirm the inner
    // ordering is components-fastest, IP-slowest (per mili-python's
    // `comp_layout` in `reference/mili-python/src/mili/datatypes.py:
    // 236-247`). `ips=` for an element-set svar queried by its own
    // name is an integration-point *label*, not a 0-based position
    // (upstream `.index(ip)` against the element-set payload —
    // `miliinternal.py:191,1251-1270`); `es_1a`'s IP labels are
    // `[1, 2]`, so `ips=[1]` must return atoms [0..7] of the
    // unfiltered read, `ips=[2]` must return atoms [7..14], and
    // requesting both must concatenate them in order.
    let path = corpus_path(&["serial", "d3samp4", "d3samp4.pltA"]);
    if !path.exists() {
        return;
    }
    let db = Database::open(&path).unwrap();
    let labels = [24_i32];
    let states = [9_usize];

    let full = db
        .query(&QueryArgs {
            svar: "es_1a",
            class: "shell",
            labels: Some(&labels),
            states: &states,
            materials: None,
            ips: None,
            subrec: None,
        })
        .unwrap();
    let StateValues::F32(full) = full else {
        panic!("es_1a is f32");
    };
    assert_eq!(full.len(), 14);

    let ips_first = [1_usize];
    let only_ip0 = db
        .query(&QueryArgs {
            svar: "es_1a",
            class: "shell",
            labels: Some(&labels),
            states: &states,
            materials: None,
            ips: Some(&ips_first),
            subrec: None,
        })
        .unwrap();
    let StateValues::F32(only_ip0) = only_ip0 else {
        unreachable!()
    };
    assert_eq!(only_ip0.len(), 7);
    for i in 0..7 {
        assert_eq!(only_ip0[i].to_bits(), full[i].to_bits());
    }

    let ips_second = [2_usize];
    let only_ip1 = db
        .query(&QueryArgs {
            svar: "es_1a",
            class: "shell",
            labels: Some(&labels),
            states: &states,
            materials: None,
            ips: Some(&ips_second),
            subrec: None,
        })
        .unwrap();
    let StateValues::F32(only_ip1) = only_ip1 else {
        unreachable!()
    };
    assert_eq!(only_ip1.len(), 7);
    for i in 0..7 {
        assert_eq!(only_ip1[i].to_bits(), full[7 + i].to_bits());
    }

    // Requesting both IPs in reverse label order: label-2 block then
    // label-1 block.
    let ips_rev = [2_usize, 1];
    let rev = db
        .query(&QueryArgs {
            svar: "es_1a",
            class: "shell",
            labels: Some(&labels),
            states: &states,
            materials: None,
            ips: Some(&ips_rev),
            subrec: None,
        })
        .unwrap();
    let StateValues::F32(rev) = rev else {
        unreachable!()
    };
    assert_eq!(rev.len(), 14);
    for i in 0..7 {
        assert_eq!(rev[i].to_bits(), full[7 + i].to_bits());
        assert_eq!(rev[7 + i].to_bits(), full[i].to_bits());
    }
}

#[test]
fn d3samp4_vec_array_object_ordered_self_consistent() {
    // d3samp4's `1shell_mmsvn_rec` is OBJECT_ORDERED carrying only
    // `es_1a` over labels (1, 3844) — per-object byte size 56. Verify
    // the API output for one label matches a direct decode of bytes
    // at the computed in-state offset.
    let path = corpus_path(&["serial", "d3samp4", "d3samp4.pltA"]);
    if !path.exists() {
        return;
    }
    let db = Database::open(&path).unwrap();
    let state = db.states()[9];
    let state_file = corpus_path(&["serial", "d3samp4", "d3samp4.plt00"]);
    let raw = std::fs::read(&state_file).unwrap();

    // Compute 1shell_mmsvn_rec start by walking subrecs to it.
    let srec = db.srecs().get(state.srec_format).unwrap();
    let mut running: u64 = (state.offset as u64) + 8;
    let mut target_start: Option<u64> = None;
    for sub in &srec.subrecords {
        if sub.name == "1shell_mmsvn_rec" {
            target_start = Some(running);
            break;
        }
        let mut per_obj: u64 = 0;
        for sn in &sub.svar_names {
            let sv = db.svars().get(sn).unwrap();
            per_obj += (sv.atoms * sv.num_type.width()) as u64;
        }
        let n: u64 = sub.id_blocks.iter().map(|&(s, e)| (e - s + 1) as u64).sum();
        running += n * per_obj;
    }
    let start = target_start.unwrap() as usize;
    let label = 24usize;
    let ord = label - 1;
    let off = start + ord * 56;
    let direct: Vec<f32> = raw[off..off + 56]
        .chunks_exact(4)
        .map(|c| f32::from_le_bytes(c.try_into().unwrap()))
        .collect();

    let labels = [label as i32];
    let states = [9_usize];
    let api = db
        .query(&QueryArgs {
            svar: "es_1a",
            class: "shell",
            labels: Some(&labels),
            states: &states,
            materials: None,
            ips: None,
            subrec: None,
        })
        .unwrap();
    let StateValues::F32(api) = api else {
        unreachable!()
    };
    assert_eq!(api.len(), direct.len());
    for (a, b) in api.iter().zip(direct.iter()) {
        assert_eq!(a.to_bits(), b.to_bits(), "API differs from direct decode");
    }
}

#[test]
fn basic1_material_filter_selects_matching_brick_labels() {
    // basic1's brick class splits 238 elements across 7 materials of
    // 34 elements each (labels 1..34 are mat 1, 35..68 mat 2, etc.).
    // Material 1 must select exactly its 34 labels and an unknown
    // material id must surface a typed `UnknownMaterial` error.
    let path = corpus_path(&["serial", "basic1", "basic1.pltA"]);
    if !path.exists() {
        return;
    }
    let db = Database::open(&path).unwrap();
    let all = db.state_var_values("sand", "brick", 0).unwrap();
    let StateValues::F32(all) = all else {
        panic!("sand is f32");
    };
    assert_eq!(all.len(), 238);

    let materials = [1_i32];
    let states = [0_usize];
    let by_mat = db
        .query(&QueryArgs {
            svar: "sand",
            class: "brick",
            labels: None,
            states: &states,
            materials: Some(&materials),
            ips: None,
            subrec: None,
        })
        .unwrap();
    let StateValues::F32(by_mat) = by_mat else {
        unreachable!()
    };
    assert_eq!(by_mat.len(), 34);
    for i in 0..34 {
        assert_eq!(by_mat[i].to_bits(), all[i].to_bits());
    }

    // Material 3 → labels 69..=102 → ordinals 68..=101 in the all-read.
    let materials = [3_i32];
    let mat3 = db
        .query(&QueryArgs {
            svar: "sand",
            class: "brick",
            labels: None,
            states: &states,
            materials: Some(&materials),
            ips: None,
            subrec: None,
        })
        .unwrap();
    let StateValues::F32(mat3) = mat3 else {
        unreachable!()
    };
    assert_eq!(mat3.len(), 34);
    for i in 0..34 {
        assert_eq!(mat3[i].to_bits(), all[68 + i].to_bits());
    }

    let bogus = [9999_i32];
    let err = db
        .query(&QueryArgs {
            svar: "sand",
            class: "brick",
            labels: None,
            states: &states,
            materials: Some(&bogus),
            ips: None,
            subrec: None,
        })
        .unwrap_err();
    assert!(matches!(err, MiliError::UnknownMaterial { material: 9999 }));
}

#[test]
fn basic1_ips_filter_on_scalar_svar_is_ignored() {
    // Upstream `_MiliInternal` silently ignores `ips` for any
    // non-VEC_ARRAY svar (it only ever builds `matching_int_points` for
    // `__int_points`-linked svars). Verified vs the oracle:
    // `query("sand","brick",ips=[1])` on basic1 returns the same result
    // as without `ips`. mili-rs previously surfaced a stricter
    // `IpFilterNotApplicable` here with no oracle basis — corrected to
    // match the corpus (planning/mili-py/m4.md Phase H reductions
    // sub-slice; the strict variant diverged from upstream).
    let path = corpus_path(&["serial", "basic1", "basic1.pltA"]);
    if !path.exists() {
        return;
    }
    let db = Database::open(&path).unwrap();
    let states = [0_usize];
    let base = |ips: Option<&[usize]>| {
        db.query_full(&QueryArgs {
            svar: "sand",
            class: "brick",
            labels: None,
            states: &states,
            materials: None,
            ips,
            subrec: None,
        })
        .unwrap()
    };
    let no_ips = base(None);
    let ips = [0_usize];
    let with_ips = base(Some(&ips));
    assert_eq!(with_ips.values, no_ips.values);
    assert_eq!(with_ips.labels, no_ips.labels);
    assert_eq!(with_ips.components, no_ips.components);
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

#[test]
fn d3samp6_hx_subscript_matches_full_array_atom() {
    // Step 11 — `hx[3]` on `brick` selects the 3rd atom (1-based) of
    // every brick's 8-atom `hx` array. The parity check compares the
    // subscripted read against atom-3 of the full array read, both
    // pulled through the public API. mili-python's golden for the same
    // query is `test_bugfixes.py::SerialArrayStateVariables::
    // test_query_array_components` — labels [2,5,10], state 6.
    let path = corpus_path(&["th", "serial", "d3samp6.thA"]);
    if !path.exists() {
        return;
    }
    let db = Database::open(&path).unwrap();
    // d3samp6.th's brick `hx` is M_FLOAT4 dims=[8].
    let hx = db.svars().get("hx").expect("hx in dict");
    assert!(matches!(hx.agg, mili_rs::SvarAgg::Array { ref dims } if dims == &[8]));

    // mili-python states are 1-based; the Rust API uses 0-based indices
    // into `Database::states()`. test_query_array_components uses
    // state=6 (1-based) and labels=[2,5,10].
    let labels = [2_i32, 5, 10];
    let states = [5_usize];

    let full = db
        .query(&QueryArgs {
            svar: "hx",
            class: "brick",
            labels: Some(&labels),
            states: &states,
            materials: None,
            ips: None,
            subrec: None,
        })
        .unwrap();
    let StateValues::F32(full) = full else {
        panic!("hx is f32");
    };
    assert_eq!(full.len(), 3 * 8);

    let sub = db
        .query(&QueryArgs {
            svar: "hx[3]",
            class: "brick",
            labels: Some(&labels),
            states: &states,
            materials: None,
            ips: None,
            subrec: None,
        })
        .unwrap();
    let StateValues::F32(sub) = sub else {
        panic!("hx[3] is f32");
    };
    assert_eq!(sub.len(), 3);

    // hx[3] (1-based) → atom index 2 of each per-object 8-atom slot.
    for (i, _label) in labels.iter().enumerate() {
        assert_eq!(
            sub[i].to_bits(),
            full[i * 8 + 2].to_bits(),
            "label idx {i}, atom 2 differs between hx[3] and hx[..]"
        );
    }
}

#[test]
#[allow(clippy::excessive_precision, clippy::unreadable_literal)]
fn d3samp6_hx_subscript_matches_mili_python_golden() {
    // Direct golden parity against mili-python's
    // `test_bugfixes.py::SerialArrayStateVariables::
    // test_query_array_components` numeric fixture (lines 345-365).
    let path = corpus_path(&["th", "serial", "d3samp6.thA"]);
    if !path.exists() {
        return;
    }
    let db = Database::open(&path).unwrap();
    let labels = [2_i32, 5, 10];
    let states = [5_usize];
    let v = db
        .query(&QueryArgs {
            svar: "hx[3]",
            class: "brick",
            labels: Some(&labels),
            states: &states,
            materials: None,
            ips: None,
            subrec: None,
        })
        .unwrap();
    let StateValues::F32(v) = v else {
        panic!("hx[3] is f32");
    };
    assert_eq!(v.len(), 3);
    let expected: [f32; 3] = [8.6602551e-01, -3.2783543e-08, 4.9999997e-01];
    for (i, want) in expected.iter().enumerate() {
        assert_eq!(
            v[i].to_bits(),
            want.to_bits(),
            "atom {i}: got {} want {}",
            v[i],
            want
        );
    }
}

#[test]
fn d3samp6_hx_subscript_errors_match_mili_python() {
    // test_query_array_exceptions: hx[0], hx[9], hx[-2], hx[1,1] all
    // must surface a typed error rather than silently succeeding.
    let path = corpus_path(&["th", "serial", "d3samp6.thA"]);
    if !path.exists() {
        return;
    }
    let db = Database::open(&path).unwrap();
    let states = [0_usize];
    for bad in ["hx[0]", "hx[9]", "hx[-2]", "hx[1,1]"] {
        let err = db
            .query(&QueryArgs {
                svar: bad,
                class: "brick",
                labels: None,
                states: &states,
                materials: None,
                ips: None,
                subrec: None,
            })
            .unwrap_err();
        assert!(
            matches!(err, MiliError::InvalidSubscript { .. }),
            "expected InvalidSubscript for {bad}, got {err:?}"
        );
    }
}
