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
use numpy::{IntoPyArray, PyArray1, PyArray2};
use pyo3::prelude::*;
use pyo3::types::{PyDict, PyList};

use mili_rs::{Database, DatabaseSet, MeshId, QueryArgs, QueryResult, StateValues};

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
            if kwargs_present(kw, "reference_state")? || kwargs_present(kw, "face")? {
                return Err(MiliPythonError::new_err(
                    "the 'reference_state' / 'face' kwargs require the derived / \
                     projection layer (Python-over-primal; M4-followup)",
                ));
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
        let out = PyDict::new_bound(py);
        for svar in &svars {
            let args = QueryArgs {
                svar,
                class: &entity_type,
                labels: labels_ref,
                states: &state_idx,
                materials: materials_ref,
                ips: ips_ref,
                subrec: subrec_ref,
            };
            let res: QueryResult = py
                .allow_threads(|| match &self.backend {
                    Backend::Single(db) => db.query_full(&args),
                    Backend::Set(s) => s.query_full(&args),
                })
                .map_err(|e| to_pyerr(&e))?;

            let n_st = state_idx.len();
            let n_lab = res.labels.len();
            let n_comp = res.components.len();

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
            out.set_item(svar, entry)?;
        }
        Ok(out)
    }
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

/// True if `kw` has `key` set to a non-`None` value.
fn kwargs_present(kw: &Bound<'_, PyDict>, key: &str) -> PyResult<bool> {
    Ok(match kw.get_item(key)? {
        Some(v) => !v.is_none(),
        None => false,
    })
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

fn map_to_pydict(py: Python<'_>, m: HashMap<String, Vec<i32>>) -> PyResult<Bound<'_, PyDict>> {
    let d = PyDict::new_bound(py);
    for (k, v) in m {
        d.set_item(k, v)?;
    }
    Ok(d)
}
