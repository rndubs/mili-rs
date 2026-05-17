//! `PyMiliDatabase` — a thin wrapper over `mili_rs::Database` *or*
//! `mili_rs::DatabaseSet`, chosen at open time by fragment count (the
//! pinned FFI contract: no `LoopWrapper` / `ServerWrapper` port; the
//! fan-out/merge job lives in `DatabaseSet`).
//!
//! M1 binds the read-only metadata surface only. Returns are small
//! Python lists / ints / dicts; bulk arrays (`nodes`, `connectivity`,
//! `query`) and the `IntoPyArray` zero-copy path land in M2/M3.

use std::collections::HashMap;
use std::path::PathBuf;

use numpy::ndarray::{Array2, Array3};
use numpy::{IntoPyArray, PyArray1, PyArray2, PyArrayMethods};
use pyo3::prelude::*;
use pyo3::types::{PyDict, PyList};

use mili_rs::{
    Database, DatabaseSet, MaterialArg, MeshId, MiliError, NodesOfElems, ParamPy, QueryArgs,
    QueryResult, StateValues,
};

use crate::errors::{to_pyerr, MiliPythonError};

// Boxed: `Database` (mmap + tables) and `DatabaseSet` (a `Vec` of
// them) differ enormously in size; boxing keeps the enum small.
enum Backend {
    Single(Box<Database>),
    Set(Box<DatabaseSet>),
}

#[pyclass(name = "PyMiliDatabase", module = "milox._native")]
pub struct PyMiliDatabase {
    backend: Backend,
    /// Canonical mesh id (smallest present; the corpus is single-mesh).
    /// Upstream's metadata surface is mesh-global; we resolve the one
    /// mesh once at open.
    mesh: MeshId,
}

fn canonical_mesh(db: &Database) -> MeshId {
    db.meshes()
        .meshes()
        .map(|m| m.id)
        .min()
        .unwrap_or(MeshId(0))
}

#[pymethods]
impl PyMiliDatabase {
    /// Single-fragment open. The Python shim resolves the `.A` file
    /// path (filename-root parsing stays in Python).
    #[staticmethod]
    fn open_single(py: Python<'_>, a_path: PathBuf) -> PyResult<Self> {
        let db = py
            .allow_threads(|| Database::open(&a_path))
            .map_err(|e| to_pyerr(&e))?;
        let mesh = canonical_mesh(&db);
        Ok(Self {
            backend: Backend::Single(Box::new(db)),
            mesh,
        })
    }

    /// Multi-fragment open. `base` is the family base path;
    /// `DatabaseSet::open` re-discovers fragments and fans out (rayon —
    /// hence `allow_threads`).
    #[staticmethod]
    fn open_set(py: Python<'_>, base: PathBuf) -> PyResult<Self> {
        let set = py
            .allow_threads(|| DatabaseSet::open(&base))
            .map_err(|e| to_pyerr(&e))?;
        let mesh = set.fragment(0).map_or(MeshId(0), canonical_mesh);
        Ok(Self {
            backend: Backend::Set(Box::new(set)),
            mesh,
        })
    }

    /// Simulation time per state. Upstream returns `np.float64`; we
    /// widen `f32` → `f64` here so a direct numeric compare matches.
    fn times(&self) -> Vec<f64> {
        let t = match &self.backend {
            Backend::Single(db) => db.times(),
            Backend::Set(s) => s.times(),
        };
        t.into_iter().map(f64::from).collect()
    }

    fn state_count(&self) -> usize {
        match &self.backend {
            Backend::Single(db) => db.state_count(),
            Backend::Set(s) => s.state_count(),
        }
    }

    // ---- M4-followup Phase G: primal-only `_MiliInternal` reshapes ----
    // Thin pass-throughs over the Rust core (`mili_rs::reshape`). The
    // reshapes are mesh-global metadata (svar/srec/class tables, params)
    // and identical across a family's fragments, so the set backend
    // resolves through fragment 0 — the same convention the existing
    // metadata accessors use. The redirect-harness target is the serial
    // sstate corpus; multi-fragment label/material *merge* of these is
    // Phase H. See planning/mili-py/m4.md decision 19.

    /// Labels of a single class — upstream `labels(class_name)`
    /// (`miliinternal.py:572-585`): `np.empty` when the class declares
    /// none. (The no-arg `labels()` dict accessor is separate.)
    fn labels_of_class(&self, py: Python<'_>, class_name: &str) -> PyResult<Vec<i32>> {
        let mesh = self.mesh;
        Ok(py
            .allow_threads(|| match &self.backend {
                Backend::Single(db) => db.labels(mesh, class_name),
                Backend::Set(s) => s.labels(mesh, class_name),
            })
            .map_err(|e| to_pyerr(&e))?
            .unwrap_or_default())
    }

    fn srec_fmt_qty(&self) -> i32 {
        self.db0().srec_fmt_qty()
    }

    fn superclass_from_class_name(&self, class_name: &str) -> i32 {
        // Merged-set semantics mirror upstream
        // `reductions.reduce_superclass_from_class_names`
        // (reductions.py:143-148): the per-proc `_MiliInternal` results
        // reduce to the first that is not M_INVALID_LABEL. A class can
        // be declared on a non-rank-0 fragment only (an MPI rank with no
        // elements of that class never declares it), so scanning
        // fragment 0 alone misses it. Scan fragments, first hit wins.
        self.frags()
            .iter()
            .find_map(|db| db.superclass_code(self.mesh, class_name))
            .unwrap_or(-1)
    }

    fn mesh_object_classes<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyList>> {
        let mesh = self.mesh;
        let info = py
            .allow_threads(|| self.db0().mesh_object_classes(mesh))
            .map_err(|e| to_pyerr(&e))?;
        let rows: Vec<_> = info
            .into_iter()
            .map(|c| {
                (
                    c.short_name,
                    c.mesh_id,
                    c.long_name,
                    c.sclass,
                    c.elem_qty,
                    c.idents_exist,
                )
            })
            .collect();
        Ok(PyList::new_bound(py, rows))
    }

    fn subrecords<'py>(&self, py: Python<'py>) -> Bound<'py, PyList> {
        let mesh = self.mesh;
        let info = py.allow_threads(|| self.db0().subrecords(mesh));
        let rows: Vec<_> = info
            .into_iter()
            .map(|s| {
                (
                    s.name,
                    s.class_name,
                    s.superclass,
                    s.organization,
                    s.qty_svars,
                    s.svar_names,
                    s.ordinal_blocks,
                )
            })
            .collect();
        PyList::new_bound(py, rows)
    }

    fn state_variables_info<'py>(&self, py: Python<'py>) -> Bound<'py, PyList> {
        let info = py.allow_threads(|| self.db0().state_variables());
        let rows: Vec<_> = info
            .into_iter()
            .map(|v| {
                (
                    v.name,
                    v.title,
                    v.data_type,
                    v.agg_type,
                    v.list_size,
                    v.order,
                    v.dims,
                    v.comp_names,
                    v.containing_svar_names,
                )
            })
            .collect();
        PyList::new_bound(py, rows)
    }

    fn queriable_svars(&self, vector_only: bool, show_ips: bool) -> Vec<String> {
        self.db0().queriable_svars(vector_only, show_ips)
    }

    /// `(classes, found)` — `found=False` means the svar is unknown
    /// (upstream sets the error code and returns `[]`).
    fn classes_of_state_variable(&self, svar: &str) -> (Vec<String>, bool) {
        match self.db0().classes_of_state_variable(svar) {
            Some(c) => (c, true),
            None => (vec![], false),
        }
    }

    /// `(svars, found)` — `found=False` means the class is unknown.
    fn state_variables_of_class(&self, class_name: &str) -> (Vec<String>, bool) {
        match self.db0().state_variables_of_class(self.mesh, class_name) {
            Some(c) => (c, true),
            None => (vec![], false),
        }
    }

    /// `(svars, svar_ok, class_ok)`.
    fn containing_state_variables_of_class(
        &self,
        svar: &str,
        class_name: &str,
    ) -> (Vec<String>, bool, bool) {
        let svar_ok = self.db0_has_svar(svar);
        let class_ok = self.db0_has_class(class_name);
        if !svar_ok || !class_ok {
            return (vec![], svar_ok, class_ok);
        }
        let v = self
            .db0()
            .containing_state_variables_of_class(self.mesh, svar, class_name)
            .unwrap_or_default();
        (v, true, true)
    }

    /// `(comps, code)` — `code` 0=ok, 1=unknown svar, 2=not a vector.
    fn components_of_vector_svar(&self, svar: &str) -> (Vec<String>, i32) {
        match self.db0().components_of_vector_svar(svar) {
            Ok(c) => (c, 0),
            Err(false) => (vec![], 1),
            Err(true) => (vec![], 2),
        }
    }

    fn state_variable_titles<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyDict>> {
        let d = PyDict::new_bound(py);
        for (k, v) in self.db0().state_variable_titles() {
            d.set_item(k, v)?;
        }
        Ok(d)
    }

    /// `(ips, svar_ok, class_ok)`.
    fn int_points_of_state_variable(
        &self,
        py: Python<'_>,
        svar_name: &str,
        class_name: &str,
    ) -> (Vec<i32>, bool, bool) {
        let svar_ok = self.db0_has_svar(svar_name);
        let class_ok = self.db0_has_class(class_name);
        if !svar_ok || !class_ok {
            return (vec![], svar_ok, class_ok);
        }
        let mesh = self.mesh;
        let v = py
            .allow_threads(|| {
                self.db0()
                    .int_points_of_state_variable(mesh, svar_name, class_name)
            })
            .unwrap_or_default();
        (v, true, true)
    }

    /// `(materials, class_ok)`.
    fn materials_of_class_name(
        &self,
        py: Python<'_>,
        class_name: &str,
    ) -> PyResult<(Vec<i32>, bool)> {
        let mesh = self.mesh;
        let r = py
            .allow_threads(|| self.db0().materials_of_class_name(mesh, class_name))
            .map_err(|e| to_pyerr(&e))?;
        Ok(match r {
            Some(v) => (v, true),
            None => (vec![], false),
        })
    }

    /// `(parts, class_ok)`.
    fn parts_of_class_name(&self, py: Python<'_>, class_name: &str) -> PyResult<(Vec<i32>, bool)> {
        let mesh = self.mesh;
        let r = py
            .allow_threads(|| self.db0().parts_of_class_name(mesh, class_name))
            .map_err(|e| to_pyerr(&e))?;
        Ok(match r {
            Some(v) => (v, true),
            None => (vec![], false),
        })
    }

    fn material_classes(
        &self,
        py: Python<'_>,
        material: &Bound<'_, PyAny>,
    ) -> PyResult<Vec<String>> {
        let arg = material_arg(material)?;
        let mesh = self.mesh;
        py.allow_threads(|| self.db0().material_classes(mesh, &arg))
            .map_err(|e| to_pyerr(&e))
    }

    /// `(labels, class_ok)`.
    fn class_labels_of_material(
        &self,
        py: Python<'_>,
        material: &Bound<'_, PyAny>,
        class_name: &str,
    ) -> PyResult<(Vec<i32>, bool)> {
        let arg = material_arg(material)?;
        let mesh = self.mesh;
        let r = py
            .allow_threads(|| self.db0().class_labels_of_material(mesh, &arg, class_name))
            .map_err(|e| to_pyerr(&e))?;
        Ok(match r {
            Some(v) => (v, true),
            None => (vec![], false),
        })
    }

    fn all_labels_of_material<'py>(
        &self,
        py: Python<'py>,
        material: &Bound<'py, PyAny>,
    ) -> PyResult<Bound<'py, PyDict>> {
        let arg = material_arg(material)?;
        let mesh = self.mesh;
        let pairs = py
            .allow_threads(|| self.db0().all_labels_of_material(mesh, &arg))
            .map_err(|e| to_pyerr(&e))?;
        let d = PyDict::new_bound(py);
        for (cls, lbls) in pairs {
            d.set_item(cls, lbls)?;
        }
        Ok(d)
    }

    fn parameters<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyDict>> {
        let pd = py
            .allow_threads(|| self.db0().parameters())
            .map_err(|e| to_pyerr(&e))?;
        let d = PyDict::new_bound(py);
        for (k, v) in pd {
            d.set_item(k, param_to_py(py, v))?;
        }
        Ok(d)
    }

    /// `Some(value)` or `None` (the caller applies its default).
    fn parameter<'py>(&self, py: Python<'py>, name: &str) -> PyResult<Option<Bound<'py, PyAny>>> {
        let v = py
            .allow_threads(|| self.db0().parameter(name))
            .map_err(|e| to_pyerr(&e))?;
        Ok(v.map(|p| param_to_py(py, p)))
    }

    fn metadata<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyDict>> {
        let m = py
            .allow_threads(|| self.db0().metadata())
            .map_err(|e| to_pyerr(&e))?;
        let d = PyDict::new_bound(py);
        d.set_item("code_name", m.code_name)?;
        d.set_item("username", m.username)?;
        d.set_item("job_id", m.job_id)?;
        d.set_item("nprocs", m.nprocs)?;
        d.set_item("date", m.date)?;
        d.set_item("host_name", m.host_name)?;
        d.set_item("library_version", m.library_version)?;
        Ok(d)
    }

    /// No-op success: the Rust core holds an immutable mmap of the
    /// already-parsed A-file, so there is no state-map cache to refresh
    /// (upstream `reload_state_maps`, `miliinternal.py:306-316`,
    /// re-parses; here it is current by construction).
    fn reload_state_maps(&self) -> bool {
        true
    }

    fn mesh_dimensions(&self) -> PyResult<i32> {
        match &self.backend {
            Backend::Single(db) => db.mesh_dimensions(),
            Backend::Set(s) => s.mesh_dimensions(),
        }
        .map_err(|e| to_pyerr(&e))
    }

    fn class_names(&self) -> Vec<String> {
        match &self.backend {
            Backend::Single(db) => db.class_names(self.mesh),
            Backend::Set(s) => s.class_names(self.mesh),
        }
    }

    /// `{class_name: [labels]}` over every class that declares labels
    /// (upstream's no-arg `labels()` returns the full dict).
    fn labels<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyDict>> {
        let mesh = self.mesh;
        let collected: Vec<(String, Vec<i32>)> = py.allow_threads(|| {
            let names = match &self.backend {
                Backend::Single(db) => db.class_names(mesh),
                Backend::Set(s) => s.class_names(mesh),
            };
            let mut out = Vec::new();
            for name in names {
                let labels = match &self.backend {
                    Backend::Single(db) => db.labels(mesh, &name),
                    Backend::Set(s) => s.labels(mesh, &name),
                };
                if let Ok(Some(v)) = labels {
                    out.push((name, v));
                }
            }
            out
        });
        let d = PyDict::new_bound(py);
        for (k, v) in collected {
            d.set_item(k, v)?;
        }
        Ok(d)
    }

    /// Per-state map. Upstream `StateMap` carries `file_number`,
    /// `file_offset`, `time`; we expose the comparable fields as a list
    /// of dicts. (`state_map_id` has no direct `mili-rs` analogue and
    /// is not part of the decision-4 rank-0 parity contract.)
    fn state_maps<'py>(&self, py: Python<'py>) -> PyResult<Vec<Bound<'py, PyDict>>> {
        let smaps = match &self.backend {
            Backend::Single(db) => db.states(),
            Backend::Set(s) => s.state_maps(),
        };
        let mut out = Vec::with_capacity(smaps.len());
        for sm in smaps {
            let d = PyDict::new_bound(py);
            d.set_item("file_number", sm.file)?;
            d.set_item("file_offset", sm.offset)?;
            d.set_item("time", f64::from(sm.time))?;
            out.push(d);
        }
        Ok(out)
    }

    fn materials<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyDict>> {
        let m = py
            .allow_threads(|| match &self.backend {
                Backend::Single(db) => db.materials(),
                Backend::Set(s) => s.materials(),
            })
            .map_err(|e| to_pyerr(&e))?;
        map_to_pydict(py, m)
    }

    fn material_numbers(&self, py: Python<'_>) -> PyResult<Vec<i32>> {
        py.allow_threads(|| match &self.backend {
            Backend::Single(db) => db.material_numbers(),
            Backend::Set(s) => s.material_numbers(),
        })
        .map_err(|e| to_pyerr(&e))
    }

    fn element_sets<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyDict>> {
        let m = py
            .allow_threads(|| match &self.backend {
                Backend::Single(db) => db.element_sets(),
                Backend::Set(s) => s.element_sets(),
            })
            .map_err(|e| to_pyerr(&e))?;
        map_to_pydict(py, m)
    }

    /// Per-material integration points. Upstream keys are strings
    /// (`element-set name[-1:]`); `mili-rs` keys by parsed material id,
    /// so we stringify the id. Matches upstream for single-digit set
    /// names (the corpus); see status.md "Element-set name → material
    /// id parse rule".
    fn integration_points<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyDict>> {
        let m = py
            .allow_threads(|| match &self.backend {
                Backend::Single(db) => db.integration_points(),
                Backend::Set(s) => s.integration_points(),
            })
            .map_err(|e| to_pyerr(&e))?;
        let d = PyDict::new_bound(py);
        for (mat, ips) in m {
            d.set_item(mat.0.to_string(), ips)?;
        }
        Ok(d)
    }

    /// Initial nodal coordinates — `np.float32`, shape
    /// `(n_nodes, mesh_dim)`. Matches upstream `nodes()`
    /// (`miliinternal.py:341` / merged `milidatabase.py:167`). The
    /// owned `Vec<f32>` is moved into numpy via `into_pyarray_bound`
    /// (no FFI byte copy); the core decode/merge runs GIL-free.
    fn nodes<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyArray2<f32>>> {
        let mesh = self.mesh;
        let (data, dims) = py
            .allow_threads(|| match &self.backend {
                Backend::Single(db) => db.node_coords(mesh),
                Backend::Set(s) => s.node_coords(mesh),
            })
            .map_err(|e| to_pyerr(&e))?
            .unwrap_or_default();
        array2(py, data, dims.max(1))
    }

    /// Element connectivity as element **labels** — `np.int32`.
    ///
    /// With `class_name`: a `(n_elem, nodes_per_elem + 1)` array (node
    /// columns label-substituted, trailing column = raw material
    /// number; the part column is dropped). A class with no
    /// connectivity yields an empty 1-D array, matching upstream
    /// `np.empty([0], np.int32)`.
    ///
    /// Without `class_name`: a `{class_name: array}` dict over element
    /// classes only (upstream `__conns_labels` keys). Matches upstream
    /// `connectivity()` (`miliinternal.py:608`, merged
    /// `milidatabase.py:reduce_connectivity`).
    #[pyo3(signature = (class_name=None))]
    fn connectivity<'py>(
        &self,
        py: Python<'py>,
        class_name: Option<String>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let mesh = self.mesh;
        if let Some(name) = class_name {
            let got = py
                .allow_threads(|| match &self.backend {
                    Backend::Single(db) => db.connectivity_labels(mesh, &name),
                    Backend::Set(s) => s.connectivity_labels(mesh, &name),
                })
                .map_err(|e| to_pyerr(&e))?;
            return match got {
                Some((data, ncols)) => Ok(array2(py, data, ncols)?.into_any()),
                None => Ok(PyArray1::<i32>::zeros_bound(py, 0, false).into_any()),
            };
        }
        let names = py.allow_threads(|| match &self.backend {
            Backend::Single(db) => db.class_names(mesh),
            Backend::Set(s) => s.class_names(mesh),
        });
        let d = PyDict::new_bound(py);
        for name in names {
            let got = py
                .allow_threads(|| match &self.backend {
                    Backend::Single(db) => db.connectivity_labels(mesh, &name),
                    Backend::Set(s) => s.connectivity_labels(mesh, &name),
                })
                .map_err(|e| to_pyerr(&e))?;
            if let Some((data, ncols)) = got {
                d.set_item(name, array2(py, data, ncols)?)?;
            }
        }
        Ok(d.into_any())
    }

    /// Element connectivity as zero-based node **ids** — `np.int32`.
    /// Same shape contract as [`Self::connectivity`]; node columns are
    /// the fortran node id minus 1, last column the raw material
    /// number. Matches upstream `connectivity_ids()`
    /// (`miliinternal.py:631`).
    #[pyo3(signature = (class_name=None))]
    fn connectivity_ids<'py>(
        &self,
        py: Python<'py>,
        class_name: Option<String>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let mesh = self.mesh;
        if let Some(name) = class_name {
            let got = py
                .allow_threads(|| self.db0().connectivity_ids(mesh, &name))
                .map_err(|e| to_pyerr(&e))?;
            return match got {
                Some((data, ncols)) => Ok(array2(py, data, ncols)?.into_any()),
                None => Ok(PyArray1::<i32>::zeros_bound(py, 0, false).into_any()),
            };
        }
        let names = self.db0().class_names(mesh);
        let d = PyDict::new_bound(py);
        for name in names {
            let got = py
                .allow_threads(|| self.db0().connectivity_ids(mesh, &name))
                .map_err(|e| to_pyerr(&e))?;
            if let Some((data, ncols)) = got {
                d.set_item(name, array2(py, data, ncols)?)?;
            }
        }
        Ok(d.into_any())
    }

    /// `nodes_of_elems` (`miliinternal.py:920-953`). Returns
    /// `(nodes_2d, elem_labels_2d, code)`; `code` 0 = ok, 1 = unknown
    /// class, 2 = none of the labels exist, 3 = class has no element
    /// connectivity. On any error the arrays are empty `(0,0)` (the
    /// adapter sets the matching ERROR return code so the
    /// `MiliDatabase` wrapper raises before the value is used).
    fn nodes_of_elems<'py>(
        &self,
        py: Python<'py>,
        class_name: &str,
        elem_labels: &Bound<'py, PyAny>,
    ) -> PyResult<(Bound<'py, PyAny>, Bound<'py, PyAny>, i32)> {
        let labels = extract_int_list::<i32>(elem_labels)?;
        let mesh = self.mesh;
        let res = py
            .allow_threads(|| self.db0().nodes_of_elems(mesh, class_name, &labels))
            .map_err(|e| to_pyerr(&e))?;
        let empty =
            || -> PyResult<Bound<'py, PyAny>> { Ok(array2::<i32>(py, vec![], 0)?.into_any()) };
        match res {
            NodesOfElems::UnknownClass => Ok((empty()?, empty()?, 1)),
            NodesOfElems::NoneExist => Ok((empty()?, empty()?, 2)),
            NodesOfElems::NoConnectivity => Ok((empty()?, empty()?, 3)),
            NodesOfElems::Ok {
                nodes,
                ncols,
                elems,
            } => Ok((
                array2(py, nodes, ncols)?.into_any(),
                array2(py, elems, 1)?.into_any(),
                0,
            )),
        }
    }

    /// `faces` (`miliinternal.py:649-685`). Returns `(code, faces)`;
    /// `code` 0 = ok, 1 = unknown class, 2 = non-HEX class, 3 = label
    /// missing. `faces` is `Some([24])` (6 faces × 4 node labels,
    /// row-major) only when `code == 0`.
    fn faces(
        &self,
        py: Python<'_>,
        class_name: &str,
        label: i32,
    ) -> PyResult<(i32, Option<Vec<i32>>)> {
        let mesh = self.mesh;
        let res = py
            .allow_threads(|| self.db0().faces(mesh, class_name, label))
            .map_err(|e| to_pyerr(&e))?;
        Ok(match res {
            mili_rs::Faces::UnknownClass => (1, None),
            mili_rs::Faces::NotHex => (2, None),
            mili_rs::Faces::LabelMissing => (3, None),
            mili_rs::Faces::Ok(f) => (0, Some(f.concat())),
        })
    }

    /// `nodes_of_material` (`miliinternal.py:955-971`) — sorted unique
    /// node labels. The adapter pre-validates the material *type*.
    fn nodes_of_material(&self, py: Python<'_>, material: &Bound<'_, PyAny>) -> PyResult<Vec<i32>> {
        let arg = material_arg(material)?;
        let mesh = self.mesh;
        py.allow_threads(|| self.db0().nodes_of_material(mesh, &arg))
            .map_err(|e| to_pyerr(&e))
    }

    /// `MiliDatabase.measure` (`milidatabase.py:882-923`): the
    /// centroid-to-centroid distance per state. Returns
    /// `(distances_f32, state_numbers_i32)` — the upstream
    /// `(distance, A_states)` tuple.
    #[pyo3(signature = (a_class, a_label, b_class, b_label, states=None))]
    #[allow(clippy::type_complexity)]
    fn measure<'py>(
        &self,
        py: Python<'py>,
        a_class: &str,
        a_label: i32,
        b_class: &str,
        b_label: i32,
        states: Option<&Bound<'py, PyAny>>,
    ) -> PyResult<(Bound<'py, PyArray1<f32>>, Bound<'py, PyArray1<i32>>)> {
        let n_states = self.state_count();
        let state_nums: Vec<i64> = match states {
            None => (1..=n_states as i64).collect(),
            Some(s) if s.is_none() => (1..=n_states as i64).collect(),
            Some(s) => {
                let mut v: Vec<i64> = extract_int_list::<i64>(s)?
                    .into_iter()
                    .map(|x| if x < 0 { n_states as i64 + x + 1 } else { x })
                    .collect();
                v.sort_unstable();
                v.dedup();
                v
            }
        };
        let state_idx: Vec<usize> = state_nums
            .iter()
            .map(|&s| {
                usize::try_from(s - 1).map_err(|_| {
                    MiliPythonError::new_err(format!(
                        "Attempting to query states that do not exist. \
                         Minimum state = 1, Maximum state = {n_states}"
                    ))
                })
            })
            .collect::<PyResult<_>>()?;
        let mesh = self.mesh;
        let dist = py
            .allow_threads(|| {
                self.db0()
                    .measure(mesh, a_class, a_label, b_class, b_label, &state_idx)
            })
            .map_err(|e| to_pyerr(&e))?;
        let st: Vec<i32> = state_nums.iter().map(|&s| s as i32).collect();
        Ok((dist.into_pyarray_bound(py), st.into_pyarray_bound(py)))
    }

    // ---- Phase H: geometric-mesh-info + adjacency (serial) ----
    //
    // The GeometricMeshInfo / serial AdjacencyMapping surface; each is
    // a bit-exact Rust-core port (`mili_rs::adjacency`) — the milox
    // `geometric_mesh_info` / `adjacency` modules are thin adapters.

    /// `GeometricMeshInfo.compute_centroid` — `None` mirrors upstream's
    /// `return None` (unknown class / missing label / no connectivity).
    #[pyo3(signature = (class_name, label, state))]
    fn gmi_compute_centroid(
        &self,
        py: Python<'_>,
        class_name: &str,
        label: i32,
        state: i64,
    ) -> PyResult<Option<Vec<f64>>> {
        let mesh = self.mesh;
        py.allow_threads(|| {
            self.db0()
                .gmi_compute_centroid(mesh, class_name, label, state)
        })
        .map_err(|e| to_pyerr(&e))
    }

    /// `GeometricMeshInfo.nearest_node` → `(label, distance)`.
    #[pyo3(signature = (point, state, material=None))]
    fn gmi_nearest_node(
        &self,
        py: Python<'_>,
        point: &Bound<'_, PyAny>,
        state: i64,
        material: Option<&Bound<'_, PyAny>>,
    ) -> PyResult<(i32, f64)> {
        let pt = float_vec(point)?;
        let mats = material_args(material)?;
        let mesh = self.mesh;
        py.allow_threads(|| {
            self.db0()
                .gmi_nearest_node(mesh, &pt, state, mats.as_deref())
        })
        .map_err(|e| to_pyerr(&e))
    }

    /// `GeometricMeshInfo.nearest_element` → `(class, label, distance)`.
    #[pyo3(signature = (point, state, material=None, entity_type=None, superclass=None))]
    fn gmi_nearest_element(
        &self,
        py: Python<'_>,
        point: &Bound<'_, PyAny>,
        state: i64,
        material: Option<&Bound<'_, PyAny>>,
        entity_type: Option<String>,
        superclass: Option<i32>,
    ) -> PyResult<(String, i32, f64)> {
        let pt = float_vec(point)?;
        let mats = material_args(material)?;
        let mesh = self.mesh;
        py.allow_threads(|| {
            self.db0().gmi_nearest_element(
                mesh,
                &pt,
                state,
                mats.as_deref(),
                entity_type.as_deref(),
                superclass,
            )
        })
        .map_err(|e| to_pyerr(&e))
    }

    /// `GeometricMeshInfo.nodes_within_radius` → node labels.
    #[pyo3(signature = (center, radius, state, material=None))]
    fn gmi_nodes_within_radius(
        &self,
        py: Python<'_>,
        center: &Bound<'_, PyAny>,
        radius: f64,
        state: i64,
        material: Option<&Bound<'_, PyAny>>,
    ) -> PyResult<Vec<i32>> {
        let c = float_vec(center)?;
        let mats = material_args(material)?;
        let mesh = self.mesh;
        py.allow_threads(|| {
            self.db0()
                .gmi_nodes_within_radius(mesh, &c, radius, state, mats.as_deref())
        })
        .map_err(|e| to_pyerr(&e))
    }

    /// `GeometricMeshInfo.elems_of_nodes` → ordered `{class: labels}`.
    #[pyo3(signature = (node_labels, material=None))]
    fn gmi_elems_of_nodes<'py>(
        &self,
        py: Python<'py>,
        node_labels: &Bound<'py, PyAny>,
        material: Option<&Bound<'py, PyAny>>,
    ) -> PyResult<Bound<'py, PyDict>> {
        let labels = extract_int_list::<i32>(node_labels)?;
        let mats = material_args(material)?;
        let mesh = self.mesh;
        let out = py
            .allow_threads(|| {
                self.db0()
                    .gmi_elems_of_nodes(mesh, &labels, mats.as_deref())
            })
            .map_err(|e| to_pyerr(&e))?;
        ordered_class_dict(py, out)
    }

    /// `AdjacencyMapping.mesh_entities_near_coordinate` (serial).
    #[pyo3(signature = (coordinate, state, radius, material=None))]
    fn adj_mesh_entities_near_coordinate<'py>(
        &self,
        py: Python<'py>,
        coordinate: &Bound<'py, PyAny>,
        state: i64,
        radius: f64,
        material: Option<&Bound<'py, PyAny>>,
    ) -> PyResult<Bound<'py, PyDict>> {
        let c = float_vec(coordinate)?;
        let mats = material_args(material)?;
        let mesh = self.mesh;
        let out = py
            .allow_threads(|| {
                self.db0().adj_mesh_entities_near_coordinate(
                    mesh,
                    &c,
                    state,
                    radius,
                    mats.as_deref(),
                )
            })
            .map_err(|e| to_pyerr(&e))?;
        ordered_class_dict(py, out)
    }

    /// `AdjacencyMapping.mesh_entities_within_radius` (serial). `None`
    /// → upstream raises `ValueError` (centroid not computable).
    #[pyo3(signature = (class_name, label, state, radius, material=None))]
    fn adj_mesh_entities_within_radius<'py>(
        &self,
        py: Python<'py>,
        class_name: &str,
        label: i32,
        state: i64,
        radius: f64,
        material: Option<&Bound<'py, PyAny>>,
    ) -> PyResult<Option<Bound<'py, PyDict>>> {
        let mats = material_args(material)?;
        let mesh = self.mesh;
        let out = py
            .allow_threads(|| {
                self.db0().adj_mesh_entities_within_radius(
                    mesh,
                    class_name,
                    label,
                    state,
                    radius,
                    mats.as_deref(),
                )
            })
            .map_err(|e| to_pyerr(&e))?;
        match out {
            Some(v) => Ok(Some(ordered_class_dict(py, v)?)),
            None => Ok(None),
        }
    }

    /// `AdjacencyMapping.neighbor_elements` (serial). Returns
    /// `(code, dict)`; code 0 = ok, 1 = no labels for the class,
    /// 2 = label missing (upstream raises `ValueError` for 1/2).
    #[pyo3(signature = (class_name, label, material=None, neighbor_radius=1))]
    fn adj_neighbor_elements<'py>(
        &self,
        py: Python<'py>,
        class_name: &str,
        label: i32,
        material: Option<&Bound<'py, PyAny>>,
        neighbor_radius: i64,
    ) -> PyResult<(i32, Bound<'py, PyDict>)> {
        let mats = material_args(material)?;
        let mesh = self.mesh;
        let res = py
            .allow_threads(|| {
                self.db0().adj_neighbor_elements(
                    mesh,
                    class_name,
                    label,
                    mats.as_deref(),
                    neighbor_radius,
                )
            })
            .map_err(|e| to_pyerr(&e))?;
        match res {
            mili_rs::NeighborElems::NoLabels => Ok((1, PyDict::new_bound(py))),
            mili_rs::NeighborElems::LabelMissing => Ok((2, PyDict::new_bound(py))),
            mili_rs::NeighborElems::Ok(v) => Ok((0, ordered_class_dict(py, v)?)),
        }
    }

    /// `AdjacencyMapping.neighbor_nodes` (serial) → neighbour node
    /// labels (sorted unique, minus the searched nodes).
    fn adj_neighbor_nodes(
        &self,
        py: Python<'_>,
        class_name: &str,
        label: i32,
    ) -> PyResult<Vec<i32>> {
        let mesh = self.mesh;
        py.allow_threads(|| self.db0().adj_neighbor_nodes(mesh, class_name, label))
            .map_err(|e| to_pyerr(&e))
    }

    /// Primal `query()` — the upstream `QueryDict` shape (M3,
    /// `planning/mili-py/m3.md`): `{svar: {class_name, source, title,
    /// data, layout:{states,labels,components,times}, modifier}}`.
    ///
    /// `svar_names` is a single name or a list; `entity_type` the
    /// class short name. `states` are 1-based state numbers (negative
    /// = from the end, `-1` is the last); `None` means all states.
    /// The core gather runs GIL-free; `data` is the owned `Vec` moved
    /// into a 3-D numpy array via `into_pyarray_bound` (no FFI copy).
    #[pyo3(signature = (svar_names, entity_type, material=None, labels=None, states=None, ips=None, write_data=None, **kwargs))]
    #[allow(clippy::too_many_arguments)]
    fn query<'py>(
        &self,
        py: Python<'py>,
        svar_names: &Bound<'py, PyAny>,
        entity_type: String,
        material: Option<&Bound<'py, PyAny>>,
        labels: Option<&Bound<'py, PyAny>>,
        states: Option<&Bound<'py, PyAny>>,
        ips: Option<&Bound<'py, PyAny>>,
        write_data: Option<&Bound<'py, PyAny>>,
        kwargs: Option<&Bound<'py, PyDict>>,
    ) -> PyResult<Bound<'py, PyDict>> {
        // svar_names: str -> [str]; list -> deduped, input order kept
        // (upstream `list(set(...))` only affects dict-key order, which
        // is parity-irrelevant for dict equality;
        // `miliinternal.py:1072-1075`).
        let raw_svars: Vec<String> = if let Ok(s) = svar_names.extract::<String>() {
            vec![s]
        } else {
            svar_names.extract::<Vec<String>>()?
        };
        let mut svars: Vec<String> = Vec::with_capacity(raw_svars.len());
        for s in raw_svars {
            if !svars.contains(&s) {
                svars.push(s);
            }
        }

        if write_data.is_some_and(|w| !w.is_none()) {
            return Err(MiliPythonError::new_err(
                "write_data (the database write path) is deferred to Phase 3",
            ));
        }

        // Hidden **kwargs: validate exactly per
        // `miliinternal.py:1159` and surface the typed hierarchy.
        let mut subrec_name: Option<String> = None;
        // `reference_state` for the nodal-displacement derived family
        // (`derived.py:982/948`); 0 = initial nodal coordinates.
        let mut reference_state: i64 = 0;
        // Whether the caller explicitly passed `reference_state`
        // (mat_cog_disp's default is state 1, not 0).
        let mut reference_state_set = false;
        // `face` (1-6) for the surfstrain derived family
        // (`derived.py.__compute_surface_strain`); validated per-svar.
        let mut face: Option<i64> = None;
        if let Some(kw) = kwargs {
            const ALLOWED: [&str; 5] = [
                "output_object_labels",
                "subrec",
                "source",
                "reference_state",
                "face",
            ];
            let mut unexpected: Vec<String> = Vec::new();
            for (k, _) in kw.iter() {
                let ks: String = k.extract()?;
                if !ALLOWED.contains(&ks.as_str()) {
                    unexpected.push(ks);
                }
            }
            if !unexpected.is_empty() {
                return Err(MiliPythonError::new_err(format!(
                    "The following unexpected keywords were provided to the query method: {{{}}}",
                    unexpected.join(", ")
                )));
            }
            if let Some(v) = kw.get_item("source")? {
                let src: String = v.extract()?;
                if src != "primal" {
                    return Err(MiliPythonError::new_err(
                        "source='derived' requires the derived-results layer \
                         (Python-over-primal; M4-followup, see planning/mili-py/m4.md)",
                    ));
                }
            }
            if let Some(v) = kw.get_item("face")? {
                if !v.is_none() {
                    face = Some(v.extract()?);
                }
            }
            if let Some(v) = kw.get_item("reference_state")? {
                if !v.is_none() {
                    reference_state = v.extract()?;
                    reference_state_set = true;
                }
            }
            if let Some(v) = kw.get_item("output_object_labels")? {
                if !v.extract::<bool>().unwrap_or(true) {
                    return Err(MiliPythonError::new_err(
                        "output_object_labels=False is not yet supported \
                         (M4-followup; the core emits real entity labels)",
                    ));
                }
            }
            if let Some(v) = kw.get_item("subrec")? {
                if !v.is_none() {
                    subrec_name = Some(v.extract()?);
                }
            }
        }

        // material: name or number -> material number list. Mirrors
        // `miliinternal.py:875-891` (digit-string promoted to int;
        // name resolved through the materials() map).
        let material_nums: Option<Vec<i32>> = match material {
            None => None,
            Some(m) if m.is_none() => None,
            Some(m) => Some(self.resolve_material(py, m)?),
        };

        // labels / ips accept a scalar int or a list (upstream
        // `argument_to_ndarray`); ips is uniqued+sorted
        // (`miliinternal.py:1090`).
        let labels_vec: Option<Vec<i32>> = match labels {
            None => None,
            Some(l) if l.is_none() => None,
            Some(l) => Some(extract_int_list::<i32>(l)?),
        };
        let mut ips_vec: Option<Vec<usize>> = match ips {
            None => None,
            Some(i) if i.is_none() => None,
            Some(i) => {
                let mut v = extract_int_list::<i64>(i)?;
                v.sort_unstable();
                v.dedup();
                Some(v.into_iter().map(|x| x.max(0) as usize).collect())
            }
        };
        if ips_vec.as_ref().is_some_and(Vec::is_empty) {
            ips_vec = None;
        }

        let n_states = self.state_count();
        // states: scalar/list -> 1-based numbers, negatives resolved
        // (`-1` = last), then uniqued + sorted ascending
        // (`miliinternal.py:1040-1045`).
        let state_nums: Vec<i64> = match states {
            None => (1..=n_states as i64).collect(),
            Some(s) if s.is_none() => (1..=n_states as i64).collect(),
            Some(s) => {
                let mut v: Vec<i64> = extract_int_list::<i64>(s)?
                    .into_iter()
                    .map(|x| if x < 0 { n_states as i64 + x + 1 } else { x })
                    .collect();
                v.sort_unstable();
                v.dedup();
                v
            }
        };
        let state_idx: Vec<usize> = state_nums
            .iter()
            .map(|&s| {
                usize::try_from(s - 1).map_err(|_| {
                    MiliPythonError::new_err(format!(
                        "Attempting to query states that do not exist. \
                         Minimum state = 1, Maximum state = {n_states}"
                    ))
                })
            })
            .collect::<PyResult<_>>()?;
        let ips_ref = ips_vec.as_deref();
        // labels ∩ material when both given; material-only selects all
        // of that material's labels (`miliinternal.py:1060-1066`).
        let resolved_labels: Option<Vec<i32>> = labels_vec;
        let labels_ref = resolved_labels.as_deref();
        let materials_ref = material_nums.as_deref();
        let subrec_ref = subrec_name.as_deref();

        let times = self.times(); // f64, one per state
        let mesh = self.mesh;
        let out = PyDict::new_bound(py);
        for svar in &svars {
            // Nodal-displacement derived family. Not primal svars, so
            // upstream resolves the source to 'derived' and computes
            // them from the primal `ux`/`uy`/`uz` query minus a
            // per-node reference: at `reference_state == 0` the initial
            // nodal coordinate (`db.nodes()`), otherwise the primal
            // component queried at that state
            // (`derived.py.__get_nodal_reference_positions`). The
            // reduction (`ResultModifier`) post-process stays
            // Python-over-primal in `milox`.
            let disp_mag = mili_rs::node_disp_mag_spec(svar);
            let disp_comp = mili_rs::node_disp_spec(svar);
            let (res, source): (QueryResult, &str) = if disp_comp.is_some() || disp_mag.is_some() {
                // Directions this derived needs: one for a
                // component (`disp_x`), the magnitude set otherwise.
                let dirs: Vec<usize> = match (disp_comp, disp_mag) {
                    (Some((d, _)), _) => vec![d],
                    (_, Some((ds, _))) => ds.to_vec(),
                    _ => unreachable!(),
                };
                let title = disp_comp
                    .map(|(_, t)| t)
                    .or(disp_mag.map(|(_, t)| t))
                    .unwrap();
                let ref_state = reference_state;
                let computed = py
                    .allow_threads(|| -> mili_rs::Result<QueryResult> {
                        let q = |sv: &str, st: &[usize]| -> mili_rs::Result<QueryResult> {
                            let a = QueryArgs {
                                svar: sv,
                                class: &entity_type,
                                labels: labels_ref,
                                states: st,
                                materials: materials_ref,
                                ips: ips_ref,
                                subrec: subrec_ref,
                            };
                            match &self.backend {
                                Backend::Single(db) => db.query_full(&a),
                                Backend::Set(s) => s.query_full(&a),
                            }
                        };
                        // Initial nodal coords (only needed for the
                        // reference_state == 0 path).
                        let (coords, dims) = if ref_state == 0 {
                            match &self.backend {
                                Backend::Single(db) => db.node_coords(mesh)?,
                                Backend::Set(s) => s.node_coords(mesh)?,
                            }
                            .unwrap_or_default()
                        } else {
                            (Vec::new(), 0)
                        };
                        let node_labels = if ref_state == 0 {
                            match &self.backend {
                                Backend::Single(db) => db.labels(mesh, "node")?,
                                Backend::Set(s) => s.labels(mesh, "node")?,
                            }
                            .unwrap_or_default()
                        } else {
                            Vec::new()
                        };
                        let ref_idx: Vec<usize> = if ref_state == 0 {
                            Vec::new()
                        } else {
                            vec![usize::try_from(ref_state - 1).map_err(|_| {
                                MiliError::Unsupported("reference_state out of range")
                            })?]
                        };

                        let mut primals: Vec<QueryResult> = Vec::with_capacity(dirs.len());
                        let mut refs: Vec<Vec<f32>> = Vec::with_capacity(dirs.len());
                        for &d in &dirs {
                            let pname = mili_rs::node_disp_primal(d);
                            let primal = q(pname, &state_idx)?;
                            let reference = if ref_state == 0 {
                                mili_rs::nodal_reference_from_coords(
                                    &primal.labels,
                                    &node_labels,
                                    &coords,
                                    dims.max(1),
                                    d,
                                )?
                            } else {
                                let rq = q(pname, &ref_idx)?;
                                mili_rs::nodal_reference_from_query(&primal.labels, &rq)?
                            };
                            primals.push(primal);
                            refs.push(reference);
                        }

                        if disp_comp.is_some() {
                            mili_rs::compute_node_displacement(
                                primals.pop().unwrap(),
                                &refs.pop().unwrap(),
                                svar,
                                title,
                            )
                        } else {
                            mili_rs::compute_node_displacement_magnitude(
                                &primals, &refs, svar, title,
                            )
                        }
                    })
                    .map_err(|e| to_pyerr(&e))?;
                (computed, "derived")
            } else if let Some((dir, title)) = mili_rs::node_vel_spec(svar)
                .map(|s| (s, false))
                .or(mili_rs::node_acc_spec(svar).map(|s| (s, true)))
                .map(|((d, t), is_acc)| (d, (t, is_acc)))
            {
                // Nodal velocity / acceleration: finite difference of
                // the primal `u<dir>` over states + the f32 per-state
                // times (`derived.py.__compute_node_{velocity,
                // acceleration}`). Gather the primal at every state the
                // stencil touches in one query, then the core does the
                // parity-sensitive difference math.
                let (title, is_acc) = title;
                let primal_name = mili_rs::node_disp_primal(dir);
                let max_state = n_states as i64;
                // 1-based states the stencil needs across all requested.
                let mut needed: Vec<i64> = Vec::new();
                for &s in &state_nums {
                    if is_acc {
                        if s == 1 {
                            needed.extend([1, 2, 3]);
                        } else if s == max_state {
                            needed.extend([max_state, max_state - 1, max_state - 2]);
                        } else {
                            needed.extend([s - 1, s, s + 1]);
                        }
                    } else {
                        needed.push(s);
                        if s != 1 {
                            needed.push(s - 1);
                        }
                    }
                }
                needed.retain(|&n| n >= 1 && n <= max_state);
                needed.sort_unstable();
                needed.dedup();
                let needed_idx: Vec<usize> = needed.iter().map(|&n| (n - 1) as usize).collect();
                let req_states = state_nums.clone();
                let computed = py
                    .allow_threads(|| -> mili_rs::Result<QueryResult> {
                        let gargs = QueryArgs {
                            svar: primal_name,
                            class: &entity_type,
                            labels: labels_ref,
                            states: &needed_idx,
                            materials: materials_ref,
                            ips: ips_ref,
                            subrec: subrec_ref,
                        };
                        let gathered = match &self.backend {
                            Backend::Single(db) => db.query_full(&gargs)?,
                            Backend::Set(s) => s.query_full(&gargs)?,
                        };
                        let raw_times: Vec<f32> = match &self.backend {
                            Backend::Single(db) => db.times(),
                            Backend::Set(s) => s.times(),
                        };
                        if is_acc {
                            mili_rs::compute_node_acceleration(
                                gathered,
                                &needed,
                                &req_states,
                                &raw_times,
                                max_state,
                                svar,
                                title,
                            )
                        } else {
                            mili_rs::compute_node_velocity(
                                gathered,
                                &needed,
                                &req_states,
                                &raw_times,
                                svar,
                                title,
                            )
                        }
                    })
                    .map_err(|e| to_pyerr(&e))?;
                (computed, "derived")
            } else if svar == "centroid" {
                // centroid: element/node centroid geometry over the
                // internal `nodpos` gather (`derived.py.
                // __compute_centroid`). Self-contained in the core
                // (reuses the bit-exact `measure` centroid); the result
                // class is the requested element class.
                let computed = py
                    .allow_threads(|| -> mili_rs::Result<QueryResult> {
                        self.db0()
                            .centroid_query(mesh, &entity_type, labels_ref, &state_idx)
                    })
                    .map_err(|e| to_pyerr(&e))?;
                (computed, "derived")
            } else if svar == "element_volume" {
                // element_volume: M_HEX/M_TET volume over the internal
                // elem→node→nodpos gather (`derived.py.
                // __compute_element_volume`). Self-contained in core.
                let computed = py
                    .allow_threads(|| -> mili_rs::Result<QueryResult> {
                        self.db0()
                            .element_volume_query(mesh, &entity_type, labels_ref, &state_idx)
                    })
                    .map_err(|e| to_pyerr(&e))?;
                (computed, "derived")
            } else if svar == "relative_volume" {
                // relative_volume: det F (deformation-gradient
                // determinant) over the internal elem→node→nodpos
                // gather + the state-0 reference node coordinates
                // (`derived.py.__compute_relative_volume`). Only
                // reference_state == 0 is exercised by the corpus; a
                // non-zero reference_state is a typed error (extension
                // scope), never a silent wrong answer.
                if reference_state != 0 {
                    return Err(MiliPythonError::new_err(
                        "relative_volume with a non-zero reference_state \
                         is not yet supported (M4-followup extension)",
                    ));
                }
                let computed = py
                    .allow_threads(|| -> mili_rs::Result<QueryResult> {
                        self.db0()
                            .relative_volume_query(mesh, &entity_type, labels_ref, &state_idx)
                    })
                    .map_err(|e| to_pyerr(&e))?;
                (computed, "derived")
            } else if svar == "area" {
                // area: M_QUAD surface area over the internal
                // elem→node→nodpos gather (`derived.py.
                // __compute_quad_area`). Self-contained in core.
                let computed = py
                    .allow_threads(|| -> mili_rs::Result<QueryResult> {
                        self.db0()
                            .quad_area_query(mesh, &entity_type, labels_ref, &state_idx)
                    })
                    .map_err(|e| to_pyerr(&e))?;
                (computed, "derived")
            } else if let Some((pname, title)) = mili_rs::mat_cog_disp_spec(svar) {
                // mat_cog_disp_<d> = matcg<d>(s) - matcg<d>(ref);
                // upstream's reference_state default is **state 1**
                // (not 0) unless the caller overrides it
                // (`derived.py.__compute_material_cog_displacement`).
                let ref_state = if reference_state_set {
                    reference_state
                } else {
                    1
                };
                let ref_idx = usize::try_from(ref_state - 1).map_err(|_| {
                    MiliPythonError::new_err("mat_cog_disp: reference_state out of range")
                })?;
                let computed = py
                    .allow_threads(|| -> mili_rs::Result<QueryResult> {
                        let mk = |st: &[usize]| -> mili_rs::Result<QueryResult> {
                            let a = QueryArgs {
                                svar: pname,
                                class: &entity_type,
                                labels: labels_ref,
                                states: st,
                                materials: materials_ref,
                                ips: ips_ref,
                                subrec: subrec_ref,
                            };
                            match &self.backend {
                                Backend::Single(db) => db.query_full(&a),
                                Backend::Set(s) => s.query_full(&a),
                            }
                        };
                        let primal = mk(&state_idx)?;
                        let reference = mk(&[ref_idx])?;
                        mili_rs::compute_mat_cog_disp(primal, &reference, svar, title)
                    })
                    .map_err(|e| to_pyerr(&e))?;
                (computed, "derived")
            } else if let Some((title, primal_candidates)) = mili_rs::contact_force_spec(svar) {
                // normal_force / force_<d> = contact primal * the
                // M_QUAD `area` derived (`derived.py.
                // __compute_{normal_force,force}`). Cross-derived:
                // gather the primal, compute `area` over the same
                // labels/states in core, multiply (label-aligned).
                let pname = primal_candidates
                    .iter()
                    .copied()
                    .find(|p| self.db0_has_svar(p))
                    .unwrap_or(primal_candidates[0]);
                let computed = py
                    .allow_threads(|| -> mili_rs::Result<QueryResult> {
                        let pa = QueryArgs {
                            svar: pname,
                            class: &entity_type,
                            labels: labels_ref,
                            states: &state_idx,
                            materials: materials_ref,
                            ips: ips_ref,
                            subrec: subrec_ref,
                        };
                        let primal = match &self.backend {
                            Backend::Single(db) => db.query_full(&pa)?,
                            Backend::Set(s) => s.query_full(&pa)?,
                        };
                        let area = self.db0().quad_area_query(
                            mesh,
                            &entity_type,
                            labels_ref,
                            &state_idx,
                        )?;
                        mili_rs::compute_contact_force(primal, &area, svar, title)
                    })
                    .map_err(|e| to_pyerr(&e))?;
                (computed, "derived")
            } else if let Some(title) = mili_rs::eps_rate_spec(svar) {
                // eps_rate: finite difference of the `eps` primal over
                // states (`derived.py.__compute_plastic_strain_rate`).
                // Gather `eps` at every stencil-touched state in one
                // query; the core does the difference math.
                let max_state = n_states as i64;
                let mut needed: Vec<i64> = Vec::new();
                for &s in &state_nums {
                    needed.push(s);
                    if s != 1 {
                        needed.push(s - 1);
                    }
                    if s != 1 && s != max_state {
                        needed.push(s + 1);
                    }
                }
                needed.retain(|&n| n >= 1 && n <= max_state);
                needed.sort_unstable();
                needed.dedup();
                let needed_idx: Vec<usize> = needed.iter().map(|&n| (n - 1) as usize).collect();
                let req_states = state_nums.clone();
                let computed = py
                    .allow_threads(|| -> mili_rs::Result<QueryResult> {
                        let gargs = QueryArgs {
                            svar: "eps",
                            class: &entity_type,
                            labels: labels_ref,
                            states: &needed_idx,
                            materials: materials_ref,
                            ips: ips_ref,
                            subrec: subrec_ref,
                        };
                        let gathered = match &self.backend {
                            Backend::Single(db) => db.query_full(&gargs)?,
                            Backend::Set(s) => s.query_full(&gargs)?,
                        };
                        let raw_times: Vec<f32> = match &self.backend {
                            Backend::Single(db) => db.times(),
                            Backend::Set(s) => s.times(),
                        };
                        mili_rs::compute_eps_rate(
                            gathered,
                            &needed,
                            &req_states,
                            &raw_times,
                            max_state,
                            svar,
                            title,
                        )
                    })
                    .map_err(|e| to_pyerr(&e))?;
                (computed, "derived")
            } else if let Some((inv, title)) = mili_rs::stress_invariant_spec(svar) {
                // Scalar stress invariants (pressure / eff_stress /
                // triaxiality / norm_press): pure element-wise math
                // over the 6 stress component primals on the requested
                // element class (`derived.py.__compute_{pressure,
                // effective_stress,triaxiality,normalized_pressure}`).
                // Gather each primal with the request's own
                // labels/states/ips, then the core does the
                // parity-sensitive arithmetic. No eigvalsh — the
                // principal-stress family is a later sub-slice.
                let primal_names = mili_rs::stress_invariant_primals(inv);
                let computed = py
                    .allow_threads(|| -> mili_rs::Result<QueryResult> {
                        let mut primals: Vec<QueryResult> = Vec::with_capacity(primal_names.len());
                        for pn in primal_names {
                            let a = QueryArgs {
                                svar: pn,
                                class: &entity_type,
                                labels: labels_ref,
                                states: &state_idx,
                                materials: materials_ref,
                                ips: ips_ref,
                                subrec: subrec_ref,
                            };
                            let p = match &self.backend {
                                Backend::Single(db) => db.query_full(&a)?,
                                Backend::Set(s) => s.query_full(&a)?,
                            };
                            primals.push(p);
                        }
                        mili_rs::compute_stress_invariant(inv, &primals, svar, title)
                    })
                    .map_err(|e| to_pyerr(&e))?;
                (computed, "derived")
            } else if let Some((kind, title)) = mili_rs::principal_stress_spec(svar) {
                // Eigenvalue-based stress invariants (prin_stress* /
                // prin_dev_stress* / max_shear_stress): build the
                // symmetric stress (or deviatoric) 3x3 from the 6
                // component primals on the requested element class and
                // read `eigvalsh` (a symmetric-3x3 Jacobi eigensolver
                // in the core — bit-identical to numpy's f32 eigvalsh
                // at every literal-checked point). Same gather as the
                // scalar invariants.
                let primal_names = mili_rs::principal_stress_primals();
                let computed = py
                    .allow_threads(|| -> mili_rs::Result<QueryResult> {
                        let mut primals: Vec<QueryResult> = Vec::with_capacity(primal_names.len());
                        for pn in primal_names {
                            let a = QueryArgs {
                                svar: pn,
                                class: &entity_type,
                                labels: labels_ref,
                                states: &state_idx,
                                materials: materials_ref,
                                ips: ips_ref,
                                subrec: subrec_ref,
                            };
                            let p = match &self.backend {
                                Backend::Single(db) => db.query_full(&a)?,
                                Backend::Set(s) => s.query_full(&a)?,
                            };
                            primals.push(p);
                        }
                        mili_rs::compute_principal_stress(kind, &primals, svar, title)
                    })
                    .map_err(|e| to_pyerr(&e))?;
                (computed, "derived")
            } else if let Some((kind, title)) = mili_rs::principal_strain_spec(svar) {
                // Strain invariants (vol_strain / prin_strain* /
                // prin_dev_strain*): vol_strain is the trivial strain
                // trace; the principal strains reuse the same
                // symmetric-3x3 Jacobi eigensolver as the stress
                // family on the 6 strain components (vol_strain reads
                // only the 3 normals). Same gather pattern.
                let primal_names = mili_rs::principal_strain_primals(kind);
                let computed = py
                    .allow_threads(|| -> mili_rs::Result<QueryResult> {
                        let mut primals: Vec<QueryResult> = Vec::with_capacity(primal_names.len());
                        for pn in primal_names {
                            let a = QueryArgs {
                                svar: pn,
                                class: &entity_type,
                                labels: labels_ref,
                                states: &state_idx,
                                materials: materials_ref,
                                ips: ips_ref,
                                subrec: subrec_ref,
                            };
                            let p = match &self.backend {
                                Backend::Single(db) => db.query_full(&a)?,
                                Backend::Set(s) => s.query_full(&a)?,
                            };
                            primals.push(p);
                        }
                        mili_rs::compute_principal_strain(kind, &primals, svar, title)
                    })
                    .map_err(|e| to_pyerr(&e))?;
                (computed, "derived")
            } else if let Some((kind, title)) = mili_rs::magnitude_spec(svar) {
                // sqrt-of-sum-of-component-squares magnitudes
                // (nodtangmag / shear_magnitude): same element-wise
                // pattern as the scalar invariants, no connectivity.
                let primal_names = mili_rs::magnitude_primals(kind);
                let computed = py
                    .allow_threads(|| -> mili_rs::Result<QueryResult> {
                        let mut primals: Vec<QueryResult> = Vec::with_capacity(primal_names.len());
                        for pn in primal_names {
                            let a = QueryArgs {
                                svar: pn,
                                class: &entity_type,
                                labels: labels_ref,
                                states: &state_idx,
                                materials: materials_ref,
                                ips: ips_ref,
                                subrec: subrec_ref,
                            };
                            let p = match &self.backend {
                                Backend::Single(db) => db.query_full(&a)?,
                                Backend::Set(s) => s.query_full(&a)?,
                            };
                            primals.push(p);
                        }
                        mili_rs::compute_magnitude(kind, &primals, svar, title)
                    })
                    .map_err(|e| to_pyerr(&e))?;
                (computed, "derived")
            } else if let Some((title, jr, ic)) = mili_rs::surfstrain_spec(svar) {
                // surfstrain{x,y,z,xy,yz,zx}: per-face Hex surface
                // strain over the elem→node `nodpos` gather + the
                // state-0 reference node coords
                // (`derived.py.__compute_surface_strain`). The `face`
                // kwarg (1-6) is mandatory; the error messages mirror
                // upstream's `ValueError`s (surfaced as
                // `MiliPythonError`). Only `reference_state == 0` is
                // exercised by the corpus; a non-zero value is a typed
                // error (extension scope), never a silent wrong answer.
                let f = face.ok_or_else(|| {
                    MiliPythonError::new_err(
                        "A valid face number (1-6) must be specified. \
                         Use the keyword argument 'face'.",
                    )
                })?;
                if !(1..=6).contains(&f) {
                    return Err(MiliPythonError::new_err(format!(
                        "The provided face ({f}) is invalid. \
                         Valid face numbers include 1-6"
                    )));
                }
                if reference_state != 0 {
                    return Err(MiliPythonError::new_err(
                        "surfstrain with a non-zero reference_state \
                         is not yet supported (M4-followup extension)",
                    ));
                }
                let computed = py
                    .allow_threads(|| -> mili_rs::Result<QueryResult> {
                        self.db0().surface_strain_query(
                            mesh,
                            &entity_type,
                            labels_ref,
                            &state_idx,
                            f,
                            jr,
                            ic,
                            svar,
                            title,
                        )
                    })
                    .map_err(|e| to_pyerr(&e))?;
                (computed, "derived")
            } else {
                let args = QueryArgs {
                    svar,
                    class: &entity_type,
                    labels: labels_ref,
                    states: &state_idx,
                    materials: materials_ref,
                    ips: ips_ref,
                    subrec: subrec_ref,
                };
                let primal = py
                    .allow_threads(|| match &self.backend {
                        Backend::Single(db) => db.query_full(&args),
                        Backend::Set(s) => s.query_full(&args),
                    })
                    .map_err(|e| to_pyerr(&e))?;
                (primal, "primal")
            };

            let n_st = state_idx.len();
            let n_lab = res.labels.len();
            // Atom count comes from the flat `[state][label][atom]`
            // length, not `components`. For primal / nodal-derived
            // results the two agree; for the stress invariants the
            // result keeps the primal's per-IP atom axis while
            // `components` is the single derived name (upstream
            // `__initialize_result_dictionary` sets
            // `components = [result_name]` over `np.empty_like(primal)`
            // — m4 derived layout). Falls back to `components` only
            // when the entity/state axes are empty (no atoms to infer).
            let denom = n_st * n_lab;
            let n_comp = if denom == 0 {
                res.components.len()
            } else {
                res.values.len() / denom
            };

            let layout = PyDict::new_bound(py);
            layout.set_item(
                "states",
                state_nums
                    .iter()
                    .map(|&s| s as i32)
                    .collect::<Vec<_>>()
                    .into_pyarray_bound(py),
            )?;
            layout.set_item("labels", res.labels.clone().into_pyarray_bound(py))?;
            layout.set_item("components", PyList::new_bound(py, &res.components))?;
            layout.set_item(
                "times",
                state_idx
                    .iter()
                    .map(|&i| times[i])
                    .collect::<Vec<f64>>()
                    .into_pyarray_bound(py),
            )?;

            let entry = PyDict::new_bound(py);
            entry.set_item("class_name", &res.class_name)?;
            entry.set_item("source", source)?;
            entry.set_item("title", &res.title)?;
            entry.set_item(
                "data",
                state_values_3d(py, res.values, n_st, n_lab, n_comp)?,
            )?;
            entry.set_item("layout", layout)?;
            entry.set_item("modifier", "")?;
            out.set_item(svar, entry)?;
        }
        Ok(out)
    }

    // ---- Phase I.1: per-fragment (merge_results=False) read surface ----
    //
    // Decision (planning/mili-py/phase-i.md, m4.md decision 20):
    // option (a) — `*_per_fragment()` siblings returning a per-fragment
    // list (a 1-element list for the `Single` backend, matching
    // upstream's 1-proc `_MiliInternal` selection). This is exactly the
    // `[proc.method(...) for proc in procs]` shape upstream's
    // LoopWrapper/ServerWrapper forwarding (Phase I.2) consumes. No
    // merge logic is touched — the `DatabaseSet` merge stays the
    // `merge_results=True` path (decision 19 invariant intact); these
    // accessors expose the *already-parity-correct* per-fragment
    // `Database` outputs verbatim (decision 20).

    /// Fragment (== MPI rank) count. `Single` → 1.
    fn fragment_count(&self) -> usize {
        self.frags().len()
    }

    fn times_per_fragment(&self) -> Vec<Vec<f64>> {
        self.frags()
            .iter()
            .map(|f| f.times().into_iter().map(f64::from).collect())
            .collect()
    }

    fn state_count_per_fragment(&self) -> Vec<usize> {
        self.frags().iter().map(|f| f.state_count()).collect()
    }

    fn mesh_dimensions_per_fragment(&self) -> PyResult<Vec<i32>> {
        self.frags()
            .iter()
            .map(|f| f.mesh_dimensions().map_err(|e| to_pyerr(&e)))
            .collect()
    }

    fn srec_fmt_qty_per_fragment(&self) -> Vec<i32> {
        self.frags().iter().map(|f| f.srec_fmt_qty()).collect()
    }

    fn class_names_per_fragment(&self) -> Vec<Vec<String>> {
        let mesh = self.mesh;
        self.frags().iter().map(|f| f.class_names(mesh)).collect()
    }

    fn material_numbers_per_fragment(&self, py: Python<'_>) -> PyResult<Vec<Vec<i32>>> {
        py.allow_threads(|| {
            self.frags()
                .iter()
                .map(|f| f.material_numbers())
                .collect::<mili_rs::Result<Vec<_>>>()
        })
        .map_err(|e| to_pyerr(&e))
    }

    /// Per-fragment labels of a single class (upstream
    /// `labels(class_name)`; `[]` when the fragment declares none).
    fn labels_of_class_per_fragment(
        &self,
        py: Python<'_>,
        class_name: &str,
    ) -> PyResult<Vec<Vec<i32>>> {
        let mesh = self.mesh;
        py.allow_threads(|| {
            self.frags()
                .iter()
                .map(|f| f.labels(mesh, class_name).map(Option::unwrap_or_default))
                .collect::<mili_rs::Result<Vec<_>>>()
        })
        .map_err(|e| to_pyerr(&e))
    }

    /// Per-fragment `{class_name: [labels]}` (the no-arg `labels()`
    /// dict, per fragment).
    fn labels_per_fragment<'py>(&self, py: Python<'py>) -> PyResult<Vec<Bound<'py, PyDict>>> {
        let mesh = self.mesh;
        let collected: Vec<Vec<(String, Vec<i32>)>> = py.allow_threads(|| {
            self.frags()
                .iter()
                .map(|f| {
                    let mut out = Vec::new();
                    for name in f.class_names(mesh) {
                        if let Ok(Some(v)) = f.labels(mesh, &name) {
                            out.push((name, v));
                        }
                    }
                    out
                })
                .collect()
        });
        collected
            .into_iter()
            .map(|frag| {
                let d = PyDict::new_bound(py);
                for (k, v) in frag {
                    d.set_item(k, v)?;
                }
                Ok(d)
            })
            .collect()
    }

    fn materials_per_fragment<'py>(&self, py: Python<'py>) -> PyResult<Vec<Bound<'py, PyDict>>> {
        let maps = py
            .allow_threads(|| {
                self.frags()
                    .iter()
                    .map(|f| f.materials())
                    .collect::<mili_rs::Result<Vec<_>>>()
            })
            .map_err(|e| to_pyerr(&e))?;
        maps.into_iter().map(|m| map_to_pydict(py, m)).collect()
    }

    fn parameters_per_fragment<'py>(&self, py: Python<'py>) -> PyResult<Vec<Bound<'py, PyDict>>> {
        let frags = self.frags();
        let mut out = Vec::with_capacity(frags.len());
        for f in frags {
            let pd = py
                .allow_threads(|| f.parameters())
                .map_err(|e| to_pyerr(&e))?;
            let d = PyDict::new_bound(py);
            for (k, v) in pd {
                d.set_item(k, param_to_py(py, v))?;
            }
            out.push(d);
        }
        Ok(out)
    }

    /// Per-fragment state-map list (the fragment-local `file_number` /
    /// `file_offset` / `time`; not the rank-0 reduction).
    fn state_maps_per_fragment<'py>(
        &self,
        py: Python<'py>,
    ) -> PyResult<Vec<Vec<Bound<'py, PyDict>>>> {
        let mut out = Vec::new();
        for f in self.frags() {
            let mut frag = Vec::new();
            for sm in f.states() {
                let d = PyDict::new_bound(py);
                d.set_item("file_number", sm.file)?;
                d.set_item("file_offset", sm.offset)?;
                d.set_item("time", f64::from(sm.time))?;
                frag.push(d);
            }
            out.push(frag);
        }
        Ok(out)
    }

    fn mesh_object_classes_per_fragment<'py>(
        &self,
        py: Python<'py>,
    ) -> PyResult<Vec<Bound<'py, PyList>>> {
        let mesh = self.mesh;
        let per = py
            .allow_threads(|| {
                self.frags()
                    .iter()
                    .map(|f| f.mesh_object_classes(mesh))
                    .collect::<mili_rs::Result<Vec<_>>>()
            })
            .map_err(|e| to_pyerr(&e))?;
        Ok(per
            .into_iter()
            .map(|info| {
                let rows: Vec<_> = info
                    .into_iter()
                    .map(|c| {
                        (
                            c.short_name,
                            c.mesh_id,
                            c.long_name,
                            c.sclass,
                            c.elem_qty,
                            c.idents_exist,
                        )
                    })
                    .collect();
                PyList::new_bound(py, rows)
            })
            .collect())
    }

    fn subrecords_per_fragment<'py>(&self, py: Python<'py>) -> Vec<Bound<'py, PyList>> {
        let mesh = self.mesh;
        let per: Vec<_> = py.allow_threads(|| {
            self.frags()
                .iter()
                .map(|f| f.subrecords(mesh))
                .collect::<Vec<_>>()
        });
        per.into_iter()
            .map(|info| {
                let rows: Vec<_> = info
                    .into_iter()
                    .map(|s| {
                        (
                            s.name,
                            s.class_name,
                            s.superclass,
                            s.organization,
                            s.qty_svars,
                            s.svar_names,
                            s.ordinal_blocks,
                        )
                    })
                    .collect();
                PyList::new_bound(py, rows)
            })
            .collect()
    }

    /// Per-fragment initial nodal coordinates — one `np.float32`
    /// `(n_nodes, mesh_dim)` array per fragment.
    fn nodes_per_fragment<'py>(&self, py: Python<'py>) -> PyResult<Vec<Bound<'py, PyArray2<f32>>>> {
        let mesh = self.mesh;
        let per = py
            .allow_threads(|| {
                self.frags()
                    .iter()
                    .map(|f| f.node_coords(mesh))
                    .collect::<mili_rs::Result<Vec<_>>>()
            })
            .map_err(|e| to_pyerr(&e))?;
        per.into_iter()
            .map(|got| {
                let (data, dims) = got.unwrap_or_default();
                array2(py, data, dims.max(1))
            })
            .collect()
    }

    /// Per-fragment element connectivity as zero-based node **ids**.
    /// Same per-fragment-list contract as [`Self::connectivity_ids`]:
    /// with `class_name` → one array per fragment; without → one
    /// `{class_name: array}` dict per fragment.
    #[pyo3(signature = (class_name=None))]
    fn connectivity_ids_per_fragment<'py>(
        &self,
        py: Python<'py>,
        class_name: Option<String>,
    ) -> PyResult<Vec<Bound<'py, PyAny>>> {
        let mesh = self.mesh;
        let frags = self.frags();
        let mut out = Vec::with_capacity(frags.len());
        if let Some(name) = class_name {
            for f in frags {
                let got = py
                    .allow_threads(|| f.connectivity_ids(mesh, &name))
                    .map_err(|e| to_pyerr(&e))?;
                out.push(match got {
                    Some((data, ncols)) => array2(py, data, ncols)?.into_any(),
                    None => PyArray1::<i32>::zeros_bound(py, 0, false).into_any(),
                });
            }
            return Ok(out);
        }
        for f in frags {
            let names = f.class_names(mesh);
            let d = PyDict::new_bound(py);
            for name in names {
                let got = py
                    .allow_threads(|| f.connectivity_ids(mesh, &name))
                    .map_err(|e| to_pyerr(&e))?;
                if let Some((data, ncols)) = got {
                    d.set_item(name, array2(py, data, ncols)?)?;
                }
            }
            out.push(d.into_any());
        }
        Ok(out)
    }

    /// Per-fragment `(materials, class_ok)` for a class name.
    fn materials_of_class_name_per_fragment(
        &self,
        py: Python<'_>,
        class_name: &str,
    ) -> PyResult<Vec<(Vec<i32>, bool)>> {
        let mesh = self.mesh;
        py.allow_threads(|| {
            self.frags()
                .iter()
                .map(|f| {
                    f.materials_of_class_name(mesh, class_name)
                        .map(|r| r.map_or((vec![], false), |v| (v, true)))
                })
                .collect::<mili_rs::Result<Vec<_>>>()
        })
        .map_err(|e| to_pyerr(&e))
    }

    /// Per-fragment `(parts, class_ok)` for a class name.
    fn parts_of_class_name_per_fragment(
        &self,
        py: Python<'_>,
        class_name: &str,
    ) -> PyResult<Vec<(Vec<i32>, bool)>> {
        let mesh = self.mesh;
        py.allow_threads(|| {
            self.frags()
                .iter()
                .map(|f| {
                    f.parts_of_class_name(mesh, class_name)
                        .map(|r| r.map_or((vec![], false), |v| (v, true)))
                })
                .collect::<mili_rs::Result<Vec<_>>>()
        })
        .map_err(|e| to_pyerr(&e))
    }

    /// Per-fragment **primal** `query()` — one upstream `QueryDict` per
    /// fragment (the `merge_results=False` per-proc list). Mirrors
    /// upstream's per-proc `_MiliInternal.query`, which is primal-only
    /// (the derived layer lives in the `MiliDatabase` wrapper, not
    /// `_MiliInternal`); a fragment that does not carry the class /
    /// svar contributes an empty entry (the `LoopWrapper` leniency,
    /// matching [`DatabaseSet::query`]). No entity-axis merge.
    #[pyo3(signature = (svar_names, entity_type, material=None, labels=None, states=None, ips=None))]
    #[allow(clippy::too_many_arguments)]
    fn query_per_fragment<'py>(
        &self,
        py: Python<'py>,
        svar_names: &Bound<'py, PyAny>,
        entity_type: String,
        material: Option<&Bound<'py, PyAny>>,
        labels: Option<&Bound<'py, PyAny>>,
        states: Option<&Bound<'py, PyAny>>,
        ips: Option<&Bound<'py, PyAny>>,
    ) -> PyResult<Vec<Bound<'py, PyDict>>> {
        let raw_svars: Vec<String> = if let Ok(s) = svar_names.extract::<String>() {
            vec![s]
        } else {
            svar_names.extract::<Vec<String>>()?
        };
        let mut svars: Vec<String> = Vec::with_capacity(raw_svars.len());
        for s in raw_svars {
            if !svars.contains(&s) {
                svars.push(s);
            }
        }
        for s in &svars {
            if mili_rs::node_disp_spec(s).is_some()
                || mili_rs::node_disp_mag_spec(s).is_some()
                || mili_rs::node_vel_spec(s).is_some()
                || mili_rs::node_acc_spec(s).is_some()
                || mili_rs::stress_invariant_spec(s).is_some()
                || mili_rs::principal_stress_spec(s).is_some()
                || mili_rs::principal_strain_spec(s).is_some()
                || mili_rs::magnitude_spec(s).is_some()
                || mili_rs::surfstrain_spec(s).is_some()
            {
                return Err(MiliPythonError::new_err(
                    "query_per_fragment is the primal per-proc \
                     `_MiliInternal` surface; derived families are the \
                     `MiliDatabase`-wrapper layer (Phase I.2/I.3)",
                ));
            }
        }

        let material_nums: Option<Vec<i32>> = match material {
            None => None,
            Some(m) if m.is_none() => None,
            Some(m) => Some(self.resolve_material(py, m)?),
        };
        let labels_vec: Option<Vec<i32>> = match labels {
            None => None,
            Some(l) if l.is_none() => None,
            Some(l) => Some(extract_int_list::<i32>(l)?),
        };
        let mut ips_vec: Option<Vec<usize>> = match ips {
            None => None,
            Some(i) if i.is_none() => None,
            Some(i) => {
                let mut v = extract_int_list::<i64>(i)?;
                v.sort_unstable();
                v.dedup();
                Some(v.into_iter().map(|x| x.max(0) as usize).collect())
            }
        };
        if ips_vec.as_ref().is_some_and(Vec::is_empty) {
            ips_vec = None;
        }

        let frags = self.frags();
        let mut out = Vec::with_capacity(frags.len());
        for f in frags {
            let n_states = f.state_count();
            let state_nums: Vec<i64> = match states {
                None => (1..=n_states as i64).collect(),
                Some(s) if s.is_none() => (1..=n_states as i64).collect(),
                Some(s) => {
                    let mut v: Vec<i64> = extract_int_list::<i64>(s)?
                        .into_iter()
                        .map(|x| if x < 0 { n_states as i64 + x + 1 } else { x })
                        .collect();
                    v.sort_unstable();
                    v.dedup();
                    v
                }
            };
            let state_idx: Vec<usize> = state_nums
                .iter()
                .map(|&s| {
                    usize::try_from(s - 1).map_err(|_| {
                        MiliPythonError::new_err(format!(
                            "Attempting to query states that do not exist. \
                             Minimum state = 1, Maximum state = {n_states}"
                        ))
                    })
                })
                .collect::<PyResult<_>>()?;
            let times: Vec<f64> = f.times().into_iter().map(f64::from).collect();

            let entry_dict = PyDict::new_bound(py);
            for svar in &svars {
                let args = QueryArgs {
                    svar,
                    class: &entity_type,
                    labels: labels_vec.as_deref(),
                    states: &state_idx,
                    materials: material_nums.as_deref(),
                    ips: ips_vec.as_deref(),
                    subrec: None,
                };
                let res = py.allow_threads(|| f.query_full(&args));
                match res {
                    Ok(r) => {
                        let entry = build_query_entry(py, r, &state_nums, &state_idx, &times)?;
                        entry_dict.set_item(svar, entry)?;
                    }
                    Err(MiliError::UnknownClass(_) | MiliError::NoMatchingSubrec { .. }) => {
                        // `LoopWrapper` leniency: a fragment lacking the
                        // class / svar contributes an empty entry.
                        let entry = empty_query_entry(py, &entity_type, svar, &state_nums, &times)?;
                        entry_dict.set_item(svar, entry)?;
                    }
                    Err(e) => return Err(to_pyerr(&e)),
                }
            }
            out.push(entry_dict);
        }
        Ok(out)
    }

    /// Phase 3.1 write path. Upstream `_MiliInternal.copy_non_state_data`
    /// (`miliinternal.py:1542`): write a new `.A` with no states. The
    /// parallel wrappers fan out per-fragment over `open_single`
    /// engines, so the write path is always the `Single` backend.
    fn copy_non_state_data(&self, py: Python<'_>, new_base_name: &str) -> PyResult<()> {
        match &self.backend {
            Backend::Single(db) => py
                .allow_threads(|| db.copy_non_state_data(new_base_name))
                .map_err(|e| to_pyerr(&e)),
            Backend::Set(_) => Err(MiliPythonError::new_err(
                "copy_non_state_data is a per-fragment operation; open the \
                 fragment via open_single (the parallel wrappers do this)",
            )),
        }
    }

    /// Phase 3.1 write path. Upstream `_MiliInternal.append_state`
    /// (`miliinternal.py:1433`): append one new state; returns the new
    /// state count.
    #[pyo3(signature = (new_state_time, zero_out=true, limit_states_per_file=None, limit_bytes_per_file=None))]
    fn append_state(
        &self,
        py: Python<'_>,
        new_state_time: f64,
        zero_out: bool,
        limit_states_per_file: Option<i64>,
        limit_bytes_per_file: Option<i64>,
    ) -> PyResult<usize> {
        match &self.backend {
            Backend::Single(db) => py
                .allow_threads(|| {
                    db.append_state(
                        new_state_time,
                        zero_out,
                        limit_states_per_file,
                        limit_bytes_per_file,
                    )
                })
                .map_err(|e| to_pyerr(&e)),
            Backend::Set(_) => Err(MiliPythonError::new_err(
                "append_state is a per-fragment operation; open the \
                 fragment via open_single (the parallel wrappers do this)",
            )),
        }
    }
}

/// Build the upstream `QueryDict` entry for one resolved
/// [`QueryResult`] (the `{class_name, source, title, data, layout,
/// modifier}` shape). Factored so the per-fragment primal path and the
/// merged `query()` share the exact layout contract.
fn build_query_entry<'py>(
    py: Python<'py>,
    res: QueryResult,
    state_nums: &[i64],
    state_idx: &[usize],
    times: &[f64],
) -> PyResult<Bound<'py, PyDict>> {
    let n_st = state_idx.len();
    let n_lab = res.labels.len();
    let denom = n_st * n_lab;
    let n_comp = if denom == 0 {
        res.components.len()
    } else {
        res.values.len() / denom
    };

    let layout = PyDict::new_bound(py);
    layout.set_item(
        "states",
        state_nums
            .iter()
            .map(|&s| s as i32)
            .collect::<Vec<_>>()
            .into_pyarray_bound(py),
    )?;
    layout.set_item("labels", res.labels.clone().into_pyarray_bound(py))?;
    layout.set_item("components", PyList::new_bound(py, &res.components))?;
    layout.set_item(
        "times",
        state_idx
            .iter()
            .map(|&i| times[i])
            .collect::<Vec<f64>>()
            .into_pyarray_bound(py),
    )?;

    let entry = PyDict::new_bound(py);
    entry.set_item("class_name", &res.class_name)?;
    entry.set_item("source", "primal")?;
    entry.set_item("title", &res.title)?;
    entry.set_item(
        "data",
        state_values_3d(py, res.values, n_st, n_lab, n_comp)?,
    )?;
    entry.set_item("layout", layout)?;
    entry.set_item("modifier", "")?;
    Ok(entry)
}

/// The empty `QueryDict` entry a fragment that does not carry the
/// requested class / svar contributes (the `LoopWrapper` per-proc
/// leniency): zero entities, an `(n_states, 0, 0)` data array.
fn empty_query_entry<'py>(
    py: Python<'py>,
    class_name: &str,
    svar: &str,
    state_nums: &[i64],
    times: &[f64],
) -> PyResult<Bound<'py, PyDict>> {
    let n_st = state_nums.len();
    let layout = PyDict::new_bound(py);
    layout.set_item(
        "states",
        state_nums
            .iter()
            .map(|&s| s as i32)
            .collect::<Vec<_>>()
            .into_pyarray_bound(py),
    )?;
    layout.set_item("labels", Vec::<i32>::new().into_pyarray_bound(py))?;
    layout.set_item("components", PyList::new_bound(py, [svar]))?;
    layout.set_item(
        "times",
        state_nums
            .iter()
            .enumerate()
            .map(|(i, _)| times.get(i).copied().unwrap_or(0.0))
            .collect::<Vec<f64>>()
            .into_pyarray_bound(py),
    )?;
    let entry = PyDict::new_bound(py);
    entry.set_item("class_name", class_name)?;
    entry.set_item("source", "primal")?;
    entry.set_item("title", svar)?;
    entry.set_item(
        "data",
        state_values_3d(py, StateValues::F32(vec![]), n_st, 0, 0)?,
    )?;
    entry.set_item("layout", layout)?;
    entry.set_item("modifier", "")?;
    Ok(entry)
}

/// Reshape a flat `[state][label][atom]` [`StateValues`] into a 3-D
/// numpy array `(n_states, n_labels, n_comps)` with the svar's native
/// dtype, via `into_pyarray_bound` (numpy adopts the heap buffer —
/// the pinned zero-FFI-copy path; upstream `data` is writable
/// `np.empty`-filled, so leaving the array writable matches —
/// m3.md decision 14).
fn state_values_3d<'py>(
    py: Python<'py>,
    values: StateValues,
    s: usize,
    l: usize,
    c: usize,
) -> PyResult<Bound<'py, PyAny>> {
    fn build<'py, T: numpy::Element>(
        py: Python<'py>,
        v: Vec<T>,
        shape: (usize, usize, usize),
    ) -> PyResult<Bound<'py, PyAny>> {
        let arr = Array3::from_shape_vec(shape, v)
            .map_err(|e| pyo3::exceptions::PyValueError::new_err(e.to_string()))?;
        Ok(arr.into_pyarray_bound(py).into_any())
    }
    match values {
        StateValues::F32(v) => build(py, v, (s, l, c)),
        StateValues::F64(v) => build(py, v, (s, l, c)),
        StateValues::I32(v) => build(py, v, (s, l, c)),
        StateValues::I64(v) => build(py, v, (s, l, c)),
    }
}

/// Reshape an owned flat row-major `Vec<T>` into a 2-D numpy array
/// `(len / ncols, ncols)` via `into_pyarray_bound` (numpy adopts the
/// heap buffer — the pinned zero-FFI-copy return path; M3's `query()`
/// reuses this helper signature).
fn array2<T: numpy::Element>(
    py: Python<'_>,
    data: Vec<T>,
    ncols: usize,
) -> PyResult<Bound<'_, PyArray2<T>>> {
    let rows = if ncols == 0 { 0 } else { data.len() / ncols };
    let arr = Array2::from_shape_vec((rows, ncols), data)
        .map_err(|e| pyo3::exceptions::PyValueError::new_err(e.to_string()))?;
    Ok(arr.into_pyarray_bound(py))
}

impl PyMiliDatabase {
    /// Representative fragment for mesh-global metadata reshapes
    /// (Phase G). `Single` is itself; `Set` resolves through fragment
    /// 0 (fragment-invariant metadata — same convention as the other
    /// metadata accessors).
    fn db0(&self) -> &Database {
        match &self.backend {
            Backend::Single(db) => db,
            Backend::Set(s) => s.fragment(0).expect("DatabaseSet has >= 1 fragment"),
        }
    }

    /// Per-fragment `Database` list in rank order. `Single` is a
    /// 1-element list (a serial db is a 1-proc family — matches
    /// upstream's 1-proc `_MiliInternal` selection); `Set` is every
    /// fragment. Backs the Phase I.1 `*_per_fragment()` surface.
    fn frags(&self) -> Vec<&Database> {
        match &self.backend {
            Backend::Single(db) => vec![db.as_ref()],
            Backend::Set(s) => s.fragments().iter().collect(),
        }
    }

    fn db0_has_svar(&self, svar: &str) -> bool {
        self.db0().svars().get(svar).is_some()
    }

    fn db0_has_class(&self, class_name: &str) -> bool {
        self.db0()
            .meshes()
            .mesh(self.mesh)
            .and_then(|m| m.class(class_name))
            .is_some()
    }

    /// Resolve a `material=` argument (name or number, incl. a digit
    /// string) into the material-number list the core query path
    /// expects. Mirrors `miliinternal.py:875-891`.
    fn resolve_material(&self, py: Python<'_>, m: &Bound<'_, PyAny>) -> PyResult<Vec<i32>> {
        if let Ok(n) = m.extract::<i32>() {
            return Ok(vec![n]);
        }
        let name: String = m
            .extract()
            .map_err(|_| MiliPythonError::new_err("material must be a string or integer"))?;
        let mats = py
            .allow_threads(|| match &self.backend {
                Backend::Single(db) => db.materials(),
                Backend::Set(s) => s.materials(),
            })
            .map_err(|e| to_pyerr(&e))?;
        if let Some(nums) = mats.get(&name) {
            return Ok(nums.clone());
        }
        if let Ok(n) = name.parse::<i32>() {
            return Ok(vec![n]);
        }
        Err(MiliPythonError::new_err(format!(
            "The material '{name}' does not exist."
        )))
    }
}

/// Extract a scalar int or an iterable of ints (upstream
/// `argument_to_ndarray`). A single Python int becomes a 1-element
/// list; a list/tuple/ndarray is taken element-wise.
fn extract_int_list<T>(obj: &Bound<'_, PyAny>) -> PyResult<Vec<T>>
where
    T: for<'a> pyo3::FromPyObject<'a>,
{
    if let Ok(v) = obj.extract::<Vec<T>>() {
        return Ok(v);
    }
    if let Ok(s) = obj.extract::<T>() {
        return Ok(vec![s]);
    }
    Err(MiliPythonError::new_err(
        "expected an integer or a list of integers",
    ))
}

/// A `material=` argument → [`MaterialArg`]. An int (or numpy
/// integer) becomes `Num`; anything else is taken as a string (the
/// core promotes digit-strings). Mirrors upstream's
/// `__valid_material_type` accepting `str | int | np.integer`.
fn material_arg(obj: &Bound<'_, PyAny>) -> PyResult<MaterialArg> {
    if let Ok(n) = obj.extract::<i32>() {
        return Ok(MaterialArg::Num(n));
    }
    let s: String = obj
        .extract()
        .map_err(|_| MiliPythonError::new_err("material must be string or int"))?;
    Ok(MaterialArg::Name(s))
}

/// A `material=` adjacency argument: `None`, a scalar (`str`/`int`),
/// or a list/tuple thereof → `Option<Vec<MaterialArg>>` (upstream's
/// `[material] if isinstance(material,(str,int)) else material`).
fn material_args(obj: Option<&Bound<'_, PyAny>>) -> PyResult<Option<Vec<MaterialArg>>> {
    let Some(o) = obj else { return Ok(None) };
    if o.is_none() {
        return Ok(None);
    }
    // A string is iterable but must be treated as one scalar material.
    if o.extract::<String>().is_err() {
        if let Ok(list) = o.downcast::<PyList>() {
            let mut v = Vec::with_capacity(list.len());
            for item in list.iter() {
                v.push(material_arg(&item)?);
            }
            return Ok(Some(v));
        }
        if let Ok(tup) = o.downcast::<pyo3::types::PyTuple>() {
            let mut v = Vec::with_capacity(tup.len());
            for item in tup.iter() {
                v.push(material_arg(&item)?);
            }
            return Ok(Some(v));
        }
    }
    Ok(Some(vec![material_arg(o)?]))
}

/// A point/coordinate argument (Python list or numpy float array) →
/// `Vec<f64>`.
fn float_vec(obj: &Bound<'_, PyAny>) -> PyResult<Vec<f64>> {
    if let Ok(v) = obj.extract::<Vec<f64>>() {
        return Ok(v);
    }
    if let Ok(a) = obj.downcast::<numpy::PyArray1<f64>>() {
        return Ok(a.to_owned_array().to_vec());
    }
    if let Ok(a) = obj.downcast::<numpy::PyArray1<f32>>() {
        return Ok(a.to_owned_array().iter().map(|&x| f64::from(x)).collect());
    }
    Err(MiliPythonError::new_err(
        "expected a list of floats or a numpy float array",
    ))
}

/// An ordered `[(class, labels)]` → an insertion-ordered Python dict
/// of `np.int32` arrays (matching upstream's ordered dict result).
fn ordered_class_dict(
    py: Python<'_>,
    items: Vec<(String, Vec<i32>)>,
) -> PyResult<Bound<'_, PyDict>> {
    let d = PyDict::new_bound(py);
    for (k, v) in items {
        d.set_item(k, v.into_pyarray_bound(py))?;
    }
    Ok(d)
}

/// [`ParamPy`] → a native Python scalar / str / list.
fn param_to_py<'py>(py: Python<'py>, v: ParamPy) -> Bound<'py, PyAny> {
    match v {
        ParamPy::Int(n) => n.into_py(py).into_bound(py),
        ParamPy::Float(n) => n.into_py(py).into_bound(py),
        ParamPy::Str(s) => s.into_py(py).into_bound(py),
        ParamPy::IntArr(a) => a.into_py(py).into_bound(py),
        ParamPy::FloatArr(a) => a.into_py(py).into_bound(py),
    }
}

fn map_to_pydict(py: Python<'_>, m: HashMap<String, Vec<i32>>) -> PyResult<Bound<'_, PyDict>> {
    let d = PyDict::new_bound(py);
    for (k, v) in m {
        d.set_item(k, v)?;
    }
    Ok(d)
}
