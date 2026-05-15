//! M4-followup Phase H (adjacency + geometric-mesh-info sub-slice) —
//! cross-impl parity vs. the upstream oracles over the full serial
//! corpus:
//!
//! * `mili.miliinternal._MiliInternal(...).geometry` for the
//!   `GeometricMeshInfo` surface (`compute_centroid`, `nearest_node`,
//!   `nearest_element`, `nodes_within_radius`, `elems_of_nodes`);
//! * `mili.adjacency.AdjacencyMapping(reader.open_database(...))` for
//!   the `AdjacencyMapping`-only graph methods (`neighbor_nodes`,
//!   `neighbor_elements`, `mesh_entities_near_coordinate`).
//!
//! Bit-exact: same labels, flat values and shapes upstream produces.
//! Gated on the `parity` feature; skip-not-fail when the corpus or
//! `mili` package is absent (mirrors `parity_geometry.rs` / CLAUDE.md).

#![cfg(feature = "parity")]

mod parity_support;

use parity_support::{corpus_path, skip_if_no_mili_python};
use pyo3::prelude::*;
use pyo3::types::PyList;

use mili_rs::{Database, MaterialArg, MeshId, NeighborElems};

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

fn gmi<'py>(py: Python<'py>, fx: &Fx) -> PyResult<Bound<'py, PyAny>> {
    let dir = corpus_path(&["serial", fx.rel_dir]);
    let m = py.import_bound("mili.miliinternal")?;
    m.getattr("_MiliInternal")?
        .call1((dir.to_str().expect("utf8"), fx.base))
}

fn adj<'py>(py: Python<'py>, fx: &Fx) -> PyResult<Bound<'py, PyAny>> {
    let base = corpus_path(&["serial", fx.rel_dir, fx.base]);
    let reader = py.import_bound("mili.reader")?;
    let mdb = reader
        .getattr("open_database")?
        .call1((base.to_str().expect("utf8"),))?;
    let a = py.import_bound("mili.adjacency")?;
    a.getattr("AdjacencyMapping")?.call1((mdb,))
}

fn f64_vec(o: &Bound<'_, PyAny>) -> Vec<f64> {
    o.call_method0("tolist").unwrap().extract().unwrap()
}

fn i32_vec(o: &Bound<'_, PyAny>) -> Vec<i32> {
    o.call_method0("tolist")
        .and_then(|t| t.extract())
        .unwrap_or_default()
}

/// An upstream `{class: np.int32 array}` ordered dict → ordered pairs.
fn class_dict(o: &Bound<'_, PyAny>) -> Vec<(String, Vec<i32>)> {
    let mut out = Vec::new();
    let keys: Vec<String> = o
        .call_method0("keys")
        .unwrap()
        .iter()
        .unwrap()
        .map(|k| k.unwrap().extract().unwrap())
        .collect();
    for k in keys {
        let v = i32_vec(&o.get_item(&k).unwrap());
        out.push((k, v));
    }
    out
}

#[test]
fn parity_adjacency_corpus() {
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
            let odb = gmi(py, fx).expect("open _MiliInternal oracle");
            let og = odb.getattr("geometry").expect("geometry");
            let oa = adj(py, fx).expect("open adjacency oracle");
            let tag = fx.rel_dir;
            // Upstream `GeometricMeshInfo.compute_centroid` /
            // `AdjacencyMapping.neighbor_*` only have a defined oracle
            // for the classes in upstream's own `connectivity_ids()`
            // dict (element classes with `ELEM_CONNS`) plus the `node`
            // class. Other classes make upstream raise an uncaught
            // `IndexError` (`connectivity` is an empty array, never
            // `None`) / `MiliPythonError` (`nodes_of_elems` ERROR) —
            // outside the defined oracle surface (and never hit by the
            // milox redirect suite). Drive the sweep off the oracle's
            // own key set so the comparison is meaningful.
            let mut classes: Vec<String> = odb
                .call_method0("connectivity_ids")
                .unwrap()
                .call_method0("keys")
                .unwrap()
                .iter()
                .unwrap()
                .map(|k| k.unwrap().extract().unwrap())
                .collect();
            classes.push("node".to_owned());

            // `compute_centroid("node", …)` upstream raises an uncaught
            // `IndexError` for nodes with no `nodpos` subrec coverage
            // (e.g. the `labeling` fixture's relabeled nodes —
            // `data[0]` is empty). That region is undefined for the
            // oracle; restrict the node sweep to nodpos-covered labels
            // (an infinite-radius query == every covered node).
            let covered: std::collections::HashSet<i32> = db
                .gmi_nodes_within_radius(mesh, &[0.0, 0.0, 0.0], 1.0e300, 1, None)
                .unwrap()
                .into_iter()
                .collect();

            // ---- compute_centroid: every class, every label, state 1 ----
            for cn in &classes {
                let mut labels = db.labels(mesh, cn).unwrap().unwrap_or_default();
                if cn == "node" {
                    labels.retain(|l| covered.contains(l));
                }
                for &lbl in &labels {
                    let oc = og.call_method1("compute_centroid", (cn, lbl, 1)).unwrap();
                    let rc = db.gmi_compute_centroid(mesh, cn, lbl, 1).unwrap();
                    if oc.is_none() {
                        assert!(
                            rc.is_none(),
                            "{tag}: compute_centroid({cn},{lbl}) oracle None"
                        );
                    } else {
                        assert_eq!(
                            rc.unwrap(),
                            f64_vec(&oc),
                            "{tag}: compute_centroid({cn},{lbl})"
                        );
                    }
                }
            }

            // ---- nearest_node / nearest_element / nodes_within_radius ----
            for pt in [[0.0f64, 0.0, 0.0], [0.5, 0.5, 0.5], [1.0, 1.5, 2.8]] {
                let ptl = PyList::new_bound(py, pt);
                let on = og.call_method1("nearest_node", (ptl.clone(), 1)).unwrap();
                let (onl, ond): (i32, f64) = on.extract().unwrap();
                let (rnl, rnd) = db.gmi_nearest_node(mesh, &pt, 1, None).unwrap();
                assert_eq!((rnl, rnd), (onl, ond), "{tag}: nearest_node({pt:?})");

                let oe = og
                    .call_method1("nearest_element", (ptl.clone(), 1))
                    .unwrap();
                let (oec, oel, oed): (String, i32, f64) = oe.extract().unwrap();
                let (rec, rel, red) = db
                    .gmi_nearest_element(mesh, &pt, 1, None, None, None)
                    .unwrap();
                assert_eq!(
                    (rec, rel, red),
                    (oec, oel, oed),
                    "{tag}: nearest_element({pt:?})"
                );

                for r in [1.0f64, 0.4, 1.0e30] {
                    let ow = og
                        .call_method1("nodes_within_radius", (ptl.clone(), r, 1))
                        .unwrap();
                    let rw = db.gmi_nodes_within_radius(mesh, &pt, r, 1, None).unwrap();
                    assert_eq!(rw, i32_vec(&ow), "{tag}: nodes_within_radius({pt:?},{r})");
                }
            }

            // ---- elems_of_nodes: a sample of node labels ----
            let node_labels = db.labels(mesh, "node").unwrap().unwrap_or_default();
            let sample: Vec<i32> = node_labels.iter().step_by(7).take(8).copied().collect();
            for take in [1usize, sample.len()] {
                if take == 0 || take > sample.len() {
                    continue;
                }
                let s = &sample[..take];
                let oe = og.call_method1("elems_of_nodes", (s.to_vec(),)).unwrap();
                let re = db.gmi_elems_of_nodes(mesh, s, None).unwrap();
                assert_eq!(re, class_dict(&oe), "{tag}: elems_of_nodes({s:?})");
            }

            // ---- material-filtered geometry ----
            for num in db.material_numbers().unwrap().into_iter().take(3) {
                let ml = PyList::new_bound(py, [num]);
                let on = og
                    .call_method1(
                        "nearest_node",
                        (PyList::new_bound(py, [0.0f64, 0.0, 0.0]), 1, ml.clone()),
                    )
                    .unwrap();
                let (onl, ond): (i32, f64) = on.extract().unwrap();
                let (rnl, rnd) = db
                    .gmi_nearest_node(mesh, &[0.0, 0.0, 0.0], 1, Some(&[MaterialArg::Num(num)]))
                    .unwrap();
                assert_eq!((rnl, rnd), (onl, ond), "{tag}: nearest_node(mat={num})");
            }

            // ---- AdjacencyMapping: neighbor_nodes / neighbor_elements ----
            for cn in &classes {
                let labels = db.labels(mesh, cn).unwrap().unwrap_or_default();
                for &lbl in labels.iter().take(4) {
                    // Upstream `AdjacencyMapping.neighbor_nodes` raises
                    // an uncaught `IndexError` for a BEAM's 3rd
                    // (orientation) node (its own (2,1) `node_connections`
                    // vs 3-col beam connectivity). That region is
                    // undefined for the oracle — skip it (Rust mirrors
                    // it as a typed error, never a silent wrong answer).
                    if let Ok(onn) = oa.call_method1("neighbor_nodes", (cn, lbl)) {
                        let rnn = db.adj_neighbor_nodes(mesh, cn, lbl).unwrap();
                        assert_eq!(rnn, i32_vec(&onn), "{tag}: neighbor_nodes({cn},{lbl})");
                    }

                    if let Ok(one) = oa.call_method1("neighbor_elements", (cn, lbl)) {
                        match db.adj_neighbor_elements(mesh, cn, lbl, None, 1).unwrap() {
                            NeighborElems::Ok(v) => assert_eq!(
                                v,
                                class_dict(&one),
                                "{tag}: neighbor_elements({cn},{lbl})"
                            ),
                            other => panic!("{tag}: neighbor_elements({cn},{lbl}) rust {other:?}"),
                        }
                    }
                }
            }

            // ---- mesh_entities_near_coordinate ----
            let mnc = oa
                .call_method1(
                    "mesh_entities_near_coordinate",
                    (PyList::new_bound(py, [0.0f64, 0.0, 0.0]), 1, 1.0f64),
                )
                .unwrap();
            let rmnc = db
                .adj_mesh_entities_near_coordinate(mesh, &[0.0, 0.0, 0.0], 1, 1.0, None)
                .unwrap();
            assert_eq!(
                rmnc,
                class_dict(&mnc),
                "{tag}: mesh_entities_near_coordinate"
            );
        }
    });
}
