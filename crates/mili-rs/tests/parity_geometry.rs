//! M4-followup Phase H (geometry sub-slice) — cross-impl parity vs.
//! the upstream `mili.miliinternal._MiliInternal` oracle for
//! `connectivity_ids`, `nodes_of_elems`, `faces`, and
//! `nodes_of_material`, swept across the full serial corpus.
//!
//! Bit-exact: same flat values and shapes upstream produces. Gated on
//! the `parity` feature; skip-not-fail when the corpus or `mili`
//! package is absent (mirrors `parity_reshape.rs` / CLAUDE.md).

#![cfg(feature = "parity")]

mod parity_support;

use parity_support::{corpus_path, skip_if_no_mili_python};
use pyo3::prelude::*;

use mili_rs::{Database, Faces, MeshId, NodesOfElems, Superclass};

struct Fx {
    rel_dir: &'static str,
    a_file: &'static str,
    base: &'static str,
}

const CORPUS: &[Fx] = &[
    Fx {
        rel_dir: "beam_udi",
        a_file: "beam_udi.pltA",
        base: "beam_udi.plt",
    },
    Fx {
        rel_dir: "d3samp4",
        a_file: "d3samp4.pltA",
        base: "d3samp4.plt",
    },
    Fx {
        rel_dir: "dbl_nodtang",
        a_file: "dblplt000A",
        base: "dblplt000",
    },
    Fx {
        rel_dir: "fdamp1",
        a_file: "fdamp1.pltA",
        base: "fdamp1.plt",
    },
    Fx {
        rel_dir: "labeling",
        a_file: "dblplt003A",
        base: "dblplt003",
    },
    Fx {
        rel_dir: "mstate",
        a_file: "d3samp6.plt_cA",
        base: "d3samp6.plt_c",
    },
    Fx {
        rel_dir: "rigid_body_1",
        a_file: "rigid_body1.pltA",
        base: "rigid_body1.plt",
    },
    Fx {
        rel_dir: "sstate",
        a_file: "d3samp6.pltA",
        base: "d3samp6.plt",
    },
    Fx {
        rel_dir: "tet",
        a_file: "tet1_t4.pltA",
        base: "tet1_t4.plt",
    },
    Fx {
        rel_dir: "vrt_BS",
        a_file: "vrt_BS.pltA",
        base: "vrt_BS.plt",
    },
];

fn oracle<'py>(py: Python<'py>, fx: &Fx) -> PyResult<Bound<'py, PyAny>> {
    let dir = corpus_path(&["serial", fx.rel_dir]);
    let m = py.import_bound("mili.miliinternal")?;
    m.getattr("_MiliInternal")?
        .call1((dir.to_str().expect("utf8"), fx.base))
}

/// numpy 2-D / 1-D int array -> (flat values, rows, cols).
fn arr_shape(o: &Bound<'_, PyAny>) -> (Vec<i32>, usize, usize) {
    let shape: Vec<usize> = o.getattr("shape").unwrap().extract().unwrap_or_default();
    let flat: Vec<i32> = o
        .call_method0("flatten")
        .and_then(|f| f.call_method0("tolist"))
        .and_then(|t| t.extract())
        .unwrap_or_default();
    let (r, c) = match shape.as_slice() {
        [r, c] => (*r, *c),
        [n] => (*n, 1),
        _ => (flat.len(), 1),
    };
    (flat, r, c)
}

#[test]
fn parity_geometry_corpus() {
    if skip_if_no_mili_python() {
        eprintln!("skip: mili-python not importable");
        return;
    }
    Python::with_gil(|py| {
        for fx in CORPUS {
            let a = corpus_path(&["serial", fx.rel_dir, fx.a_file]);
            if !a.exists() {
                eprintln!("skip: {} absent", fx.rel_dir);
                continue;
            }
            let db = Database::open(&a).expect("open rust db");
            let mesh = db
                .meshes()
                .meshes()
                .map(|m| m.id)
                .min()
                .unwrap_or(MeshId(0));
            let ora = oracle(py, fx).expect("open oracle");
            let tag = fx.rel_dir;
            let class_names = db.class_names(mesh);

            for cn in &class_names {
                // ---- connectivity_ids(class) ----
                let oci = ora.call_method1("connectivity_ids", (cn,)).unwrap();
                let (of, orr, occ) = arr_shape(&oci);
                match db.connectivity_ids(mesh, cn).unwrap() {
                    Some((data, ncols)) => {
                        let rows = if ncols == 0 { 0 } else { data.len() / ncols };
                        assert_eq!(
                            (data.as_slice(), rows, ncols),
                            (of.as_slice(), orr, occ),
                            "{tag}: connectivity_ids({cn})"
                        );
                    }
                    None => {
                        // Upstream: classes without ELEM_CONNS are not
                        // in __conns_ids -> empty np.int32 array.
                        assert!(
                            of.is_empty(),
                            "{tag}: connectivity_ids({cn}) rust None but oracle {of:?}"
                        );
                    }
                }

                // ---- nodes_of_elems(class, all class labels) ----
                let class_labels = db.labels(mesh, cn).unwrap().unwrap_or_default();
                if !class_labels.is_empty() {
                    let one = ora
                        .call_method1("nodes_of_elems", (cn, class_labels.clone()))
                        .unwrap();
                    let onodes = one.get_item(0).unwrap();
                    let oelems = one.get_item(1).unwrap();
                    let (onf, onr, onc) = arr_shape(&onodes);
                    let (oef, oer, oec) = arr_shape(&oelems);
                    match db.nodes_of_elems(mesh, cn, &class_labels).unwrap() {
                        NodesOfElems::Ok {
                            nodes,
                            ncols,
                            elems,
                        } => {
                            let nrows = if ncols == 0 { 0 } else { nodes.len() / ncols };
                            assert_eq!(
                                (nodes.as_slice(), nrows, ncols),
                                (onf.as_slice(), onr, onc),
                                "{tag}: nodes_of_elems({cn}) nodes"
                            );
                            // upstream elem labels are (n,1)
                            assert_eq!(
                                (elems.as_slice(), elems.len(), 1),
                                (oef.as_slice(), oer, oec),
                                "{tag}: nodes_of_elems({cn}) elems"
                            );
                        }
                        other => {
                            // The only legitimate non-Ok over the full
                            // class label list is a class with no
                            // ELEM_CONNS (upstream returns the empty
                            // (1,0) sentinel + ERROR).
                            assert!(
                                onf.is_empty(),
                                "{tag}: nodes_of_elems({cn}) rust {other:?} \
                                 but oracle nodes {onf:?}"
                            );
                        }
                    }
                }

                // ---- faces(class, label) for HEX classes ----
                if db.superclass_code(mesh, cn) == Some(Superclass::Hex as i32) {
                    for &lbl in &class_labels {
                        let od = ora.call_method1("faces", (cn, lbl)).unwrap();
                        let Faces::Ok(rf) = db.faces(mesh, cn, lbl).unwrap() else {
                            panic!("{tag}: faces({cn},{lbl}) rust non-Ok");
                        };
                        for f in 1..=6usize {
                            let ov: Vec<i32> = od
                                .get_item(f)
                                .unwrap()
                                .call_method0("tolist")
                                .unwrap()
                                .extract()
                                .unwrap();
                            assert_eq!(rf[f - 1].to_vec(), ov, "{tag}: faces({cn},{lbl})[{f}]");
                        }
                    }
                }
            }

            // ---- nodes_of_material(num) over every material ----
            for num in db.material_numbers().unwrap() {
                let om: Vec<i32> = ora
                    .call_method1("nodes_of_material", (num,))
                    .unwrap()
                    .call_method0("tolist")
                    .unwrap()
                    .extract()
                    .unwrap();
                let rm = db
                    .nodes_of_material(mesh, &mili_rs::MaterialArg::Num(num))
                    .unwrap();
                assert_eq!(rm, om, "{tag}: nodes_of_material({num})");
            }
        }
    });
}
