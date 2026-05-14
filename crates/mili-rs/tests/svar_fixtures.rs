//! Svar table parity against the reference corpus.
//!
//! Skips silently when the submodule isn't checked out, matching the
//! pattern in `mesh_fixtures.rs`.

use std::path::{Path, PathBuf};

use mili_rs::{Database, NumType, SvarAgg};

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
fn basic1_has_known_scalar_svars() {
    let path = corpus_path(&["serial", "basic1", "basic1.pltA"]);
    if !path.exists() {
        eprintln!("skip: basic1 absent");
        return;
    }
    let db = Database::open(&path).expect("open basic1");
    let svars = db.svars();

    // Spot-check a handful of svars covering scalar + vector shapes.
    // `sand` is the element-kill scalar shared across element classes;
    // basic1 declares it as M_FLOAT4 (the canonical
    // `reference/mili-python/tests/test_milidatabase.py:618-639` set
    // — sourced from `data/serial/sstate/d3samp6.plt`, not basic1 —
    // lists the same name, and the on-disk encoding is consistent
    // across both fixtures).
    let sand = svars.get("sand").expect("basic1 declares 'sand'");
    assert_eq!(sand.num_type, NumType::Float4);
    assert!(matches!(sand.agg, SvarAgg::Scalar));
    assert_eq!(sand.atoms, 1);

    // `sx` is a scalar f32 component shared by stress vectors.
    let sx = svars.get("sx").expect("basic1 declares 'sx'");
    assert_eq!(sx.num_type, NumType::Float4);
    assert!(matches!(sx.agg, SvarAgg::Scalar));
}

#[test]
fn basic1_has_stress_vector_with_six_components() {
    let path = corpus_path(&["serial", "basic1", "basic1.pltA"]);
    if !path.exists() {
        return;
    }
    let db = Database::open(&path).unwrap();
    let stress = db.svars().get("stress").expect("basic1 declares 'stress'");
    match &stress.agg {
        SvarAgg::Vector { comps } => {
            assert_eq!(comps.len(), 6);
            // Components are the six unique stress-tensor entries.
            for name in ["sx", "sy", "sz", "sxy", "syz", "szx"] {
                assert!(
                    comps.iter().any(|c| c == name),
                    "stress missing component {name}"
                );
            }
        }
        other => panic!("expected Vector svar for stress, got {other:?}"),
    }
    // atoms-per-object = number of components.
    assert_eq!(stress.atoms, 6);
}

#[test]
fn basic1_svar_table_includes_components_and_parents() {
    let path = corpus_path(&["serial", "basic1", "basic1.pltA"]);
    if !path.exists() {
        return;
    }
    let db = Database::open(&path).unwrap();
    let svars = db.svars();
    // Each vector svar's components must also appear in the table —
    // mili-python's __parse_svar recurses to insert them
    // (afileIO.py:347-351). For `stress`, that means sx/sy/.../szx
    // are individually accessible scalar svars in addition to the
    // parent vector.
    for name in [
        "stress", "sx", "sy", "sz", "sxy", "syz", "szx", "nodpos", "ux", "uy", "uz",
    ] {
        assert!(svars.get(name).is_some(), "missing svar {name}");
    }
}

#[test]
fn dbl_nodtang_has_double_precision_svar() {
    // `dbl_nodtang` is the corpus fixture for double-precision svars
    // (`test_bugfixes.py:62-72`). The `nodtang` svar should resolve to
    // M_FLOAT8.
    let path = corpus_path(&["serial", "dbl_nodtang", "dblplt000A"]);
    if !path.exists() {
        eprintln!("skip: dbl_nodtang absent");
        return;
    }
    let db = Database::open(&path).expect("open dbl_nodtang");
    let nodtang = db
        .svars()
        .get("nodtang")
        .expect("dbl_nodtang declares 'nodtang' svar");
    assert_eq!(
        nodtang.num_type,
        NumType::Float8,
        "nodtang should be declared M_FLOAT8 per test_bugfixes.py:62-72",
    );
}
