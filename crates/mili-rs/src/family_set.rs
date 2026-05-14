//! Multi-A-file orchestration for MPI-segmented mili families.
//!
//! An MPI-parallel run writes one independent mili family per rank
//! (`run.plt000A`, `run.plt001A`, …). The C library opens each as a
//! separate family (`reference/mili/src/mili.c:445`); mili-python's
//! `LoopWrapper` / `ServerWrapper` fan calls out across fragments and
//! merge the results
//! (`reference/mili-python/src/mili/parallel.py:19-356`,
//! `reductions.py`). [`DatabaseSet`] moves that orchestration into Rust
//! so `mili-py` can bind a single object instead of layering a Python
//! wrapper on top of per-rank [`Database`] instances.
//!
//! Fragment discovery mirrors mili-python (`afileIO.py:34-57`) exactly:
//! match every entry in the resolved directory against
//! `<base>(\d*)A$` and sort by the numeric suffix. The user supplies
//! the literal base — e.g. `basic1.plt` for fragments
//! `basic1.plt000A`, `basic1.plt001A`, … or `d3samp6.th` for the
//! single-fragment `d3samp6.thA`. The path is split into
//! `(parent_dir, file_name)` and the file-name component is used as
//! the base; no `.plt` / digit / `A` stripping is applied, matching
//! `reader.open_database`'s `os.path.basename(base)` step
//! (`reference/mili-python/src/mili/reader.py:45-53`).
//!
//! Merge semantics follow `reductions.py`:
//!
//! - [`DatabaseSet::labels`]: `list_concatenate_unique` — concat across
//!   fragments, dedupe preserving first-occurrence order.
//! - [`DatabaseSet::connectivity`] / [`DatabaseSet::nodes`]: plain
//!   concatenation per `reduce_connectivity` / `list_concatenate`. No
//!   remap of node-id columns: each fragment's connectivity references
//!   its own local node space, and a remap pass would change physical
//!   meaning for ghost-layer nodes.
//! - [`DatabaseSet::query`]: concat per-fragment results along the
//!   entity axis (rank-0 entities, rank-1 entities, …), then dedupe by
//!   label keeping the first occurrence per `merge_result_dictionaries`.
//! - [`DatabaseSet::times`] / [`DatabaseSet::state_count`]: take
//!   rank-0's value after checking every other fragment agrees. The
//!   time axis is the one cross-fragment invariant a coherent query
//!   actually needs; everything else (svar dict, class set) is lenient
//!   and a fragment missing a class / svar just contributes zero rows.

use std::cmp::Ordering;
use std::collections::HashSet;
use std::path::{Path, PathBuf};

use rayon::prelude::*;

use crate::error::{MiliError, Result};
use crate::family::{Database, QueryArgs};
use crate::mesh::{MeshId, ObjectClass};
use crate::query::StateValues;
use crate::state::StateMeta;
use crate::svar::NumType;

/// Multi-A-file mili family. Owns one [`Database`] per fragment, in
/// ascending rank order (the numeric suffix from the `.A` filename).
pub struct DatabaseSet {
    fragments: Vec<Database>,
}

impl DatabaseSet {
    /// Discover every fragment matching `<base>(\d*)A$` under the
    /// resolved directory and open them in parallel via rayon.
    ///
    /// `base` is split into `(parent_dir, file_name)` and the file-name
    /// component is the literal base. Examples:
    ///
    /// - `path/to/basic1.plt` matches `basic1.plt000A`, `basic1.plt001A`, …
    /// - `path/to/d3samp6.th` matches `d3samp6.thA`
    /// - `path/to/runA` matches `runA` (single-fragment, no rank digits)
    ///
    /// Mirrors `mili.reader.open_database` +
    /// `mili.afileIO.afiles_by_base`
    /// (`reference/mili-python/src/mili/reader.py:45-53`,
    /// `reference/mili-python/src/mili/afileIO.py:34-57`).
    ///
    /// Errors:
    ///
    /// - [`MiliError::NoFragments`] when zero entries match.
    /// - [`MiliError::FragmentMismatch`] when fragments disagree on the
    ///   shared time axis (state count or per-state time value).
    /// - Any per-fragment [`Database::open`] error propagates as-is.
    pub fn open(base: impl AsRef<Path>) -> Result<Self> {
        let (dir, base_name) = resolve_base(base.as_ref())?;
        let mut entries: Vec<(Option<u32>, PathBuf)> = Vec::new();
        for entry in std::fs::read_dir(&dir)? {
            let entry = entry?;
            let name = entry.file_name();
            let Some(name_str) = name.to_str() else {
                continue;
            };
            if let Some(rank) = match_fragment(name_str, &base_name) {
                entries.push((rank, entry.path()));
            }
        }
        if entries.is_empty() {
            return Err(MiliError::NoFragments {
                dir,
                base: base_name,
            });
        }
        entries.sort_by(|a, b| match (a.0, b.0) {
            (Some(x), Some(y)) => x.cmp(&y),
            (None, Some(_)) => Ordering::Less,
            (Some(_), None) => Ordering::Greater,
            (None, None) => a.1.cmp(&b.1),
        });

        let fragments: Result<Vec<Database>> = entries
            .into_par_iter()
            .map(|(_, path)| Database::open(path))
            .collect();
        let fragments = fragments?;

        let set = Self { fragments };
        set.validate_time_axis()?;
        Ok(set)
    }

    fn validate_time_axis(&self) -> Result<()> {
        if self.fragments.len() <= 1 {
            return Ok(());
        }
        let first = &self.fragments[0];
        let first_times = first.times();
        for (i, frag) in self.fragments.iter().enumerate().skip(1) {
            let t = frag.times();
            if t.len() != first_times.len() {
                return Err(MiliError::FragmentMismatch {
                    fragment: i,
                    field: "state_count",
                    detail: format!(
                        "fragment 0 has {} states, fragment {} has {}",
                        first_times.len(),
                        i,
                        t.len()
                    ),
                });
            }
            for (s, (a, b)) in first_times.iter().zip(t.iter()).enumerate() {
                if a.to_bits() != b.to_bits() {
                    return Err(MiliError::FragmentMismatch {
                        fragment: i,
                        field: "time",
                        detail: format!("state {s}: fragment 0 = {a}, fragment {i} = {b}"),
                    });
                }
            }
        }
        Ok(())
    }

    /// Number of fragments (== MPI rank count for the producing run).
    pub fn fragment_count(&self) -> usize {
        self.fragments.len()
    }

    /// Access a single fragment by rank index.
    pub fn fragment(&self, rank: usize) -> Option<&Database> {
        self.fragments.get(rank)
    }

    /// All fragments in rank order.
    pub fn fragments(&self) -> &[Database] {
        &self.fragments
    }

    /// State count. Validated equal across fragments at open time.
    pub fn state_count(&self) -> usize {
        self.fragments[0].state_count()
    }

    /// Times in directory order. Validated equal across fragments at
    /// open time; returns rank-0's vector.
    pub fn times(&self) -> Vec<f32> {
        self.fragments[0].times()
    }

    /// Per-state metadata from rank 0. State offsets / file indices
    /// are fragment-local, so callers that need state byte ranges
    /// should go through the per-fragment [`Database`].
    pub fn state_maps(&self) -> &[StateMeta] {
        self.fragments[0].states()
    }

    /// Unique class names across all fragments, in first-occurrence
    /// order over the rank-ordered fragments.
    pub fn class_names(&self, mesh_id: MeshId) -> Vec<String> {
        let mut seen: HashSet<String> = HashSet::new();
        let mut out: Vec<String> = Vec::new();
        for frag in &self.fragments {
            let Some(mesh) = frag.meshes().mesh(mesh_id) else {
                continue;
            };
            for name in mesh.class_names() {
                if seen.insert(name.to_owned()) {
                    out.push(name.to_owned());
                }
            }
        }
        out
    }

    /// Look up the [`ObjectClass`] for `(mesh_id, classname)` from the
    /// first fragment that declares it. Returns `None` if no fragment
    /// has the class.
    pub fn class(&self, mesh_id: MeshId, classname: &str) -> Option<&ObjectClass> {
        for frag in &self.fragments {
            if let Some(mesh) = frag.meshes().mesh(mesh_id) {
                if let Some(c) = mesh.class(classname) {
                    return Some(c);
                }
            }
        }
        None
    }

    /// Concatenated, deduplicated label list across all fragments for
    /// `(mesh_id, classname)`. Mirrors `reductions.list_concatenate_unique`:
    /// preserves first-occurrence order. Returns `Ok(None)` only when
    /// no fragment has any labels for the class (matches the per-
    /// fragment `Database::labels` semantics).
    pub fn labels(&self, mesh_id: MeshId, classname: &str) -> Result<Option<Vec<i32>>> {
        let mut seen: HashSet<i32> = HashSet::new();
        let mut out: Vec<i32> = Vec::new();
        let mut any = false;
        for frag in &self.fragments {
            let Some(local) = frag.labels(mesh_id, classname)? else {
                continue;
            };
            any = true;
            for &label in &local {
                if seen.insert(label) {
                    out.push(label);
                }
            }
        }
        if !any {
            return Ok(None);
        }
        Ok(Some(out))
    }

    /// Concatenated nodal-coordinate buffer across fragments for
    /// `(mesh_id, "node")`. Per-fragment rows are emitted in rank
    /// order; no remap is applied. Returns `Ok(None)` only when no
    /// fragment carries a `NODES` payload for the mesh.
    pub fn nodes(&self, mesh_id: MeshId) -> Result<Option<Vec<f32>>> {
        let mut out: Vec<f32> = Vec::new();
        let mut any = false;
        for frag in &self.fragments {
            let Some(local) = frag.nodes(mesh_id, "node")? else {
                continue;
            };
            any = true;
            let words = local.to_f32_vec()?;
            out.extend_from_slice(&words);
        }
        if !any {
            return Ok(None);
        }
        Ok(Some(out))
    }

    /// Concatenated `ELEM_CONNS` rows for `(mesh_id, classname)` across
    /// fragments. Per `reductions.reduce_connectivity` this is plain
    /// row concatenation in rank order — node-id columns reference
    /// each fragment's local node space and are intentionally not
    /// remapped. Returns `Ok(None)` only when no fragment has a
    /// connectivity payload for the class.
    pub fn connectivity(&self, mesh_id: MeshId, classname: &str) -> Result<Option<Vec<i32>>> {
        let mut out: Vec<i32> = Vec::new();
        let mut any = false;
        for frag in &self.fragments {
            let Some(local) = frag.connectivity(mesh_id, classname)? else {
                continue;
            };
            any = true;
            let words = local.to_i32_vec()?;
            out.extend_from_slice(&words);
        }
        if !any {
            return Ok(None);
        }
        Ok(Some(out))
    }

    /// Run a multi-fragment query and merge along the entity axis.
    ///
    /// Output is flat `[state][label][atom]` row-major, matching
    /// [`Database::query`]. The returned label vector is the merged
    /// entity axis: per-fragment results are concatenated in rank
    /// order, then deduplicated by label keeping the first occurrence
    /// (matches `reductions.merge_result_dictionaries`).
    ///
    /// Per-fragment errors are tolerated for the two leniencies
    /// mili-python's `LoopWrapper` provides — a fragment that does not
    /// declare the class ([`MiliError::UnknownClass`]) or has no
    /// subrec carrying the svar on the class
    /// ([`MiliError::NoMatchingSubrec`]) silently contributes zero
    /// rows. Every other error propagates.
    pub fn query(&self, args: &QueryArgs<'_>) -> Result<SetQueryResult> {
        let results: Vec<FragmentQueryResult> = self
            .fragments
            .par_iter()
            .map(|frag| match frag.query_with_labels(args) {
                Ok((values, labels)) => Ok(FragmentQueryResult::Data { values, labels }),
                Err(MiliError::UnknownClass(_) | MiliError::NoMatchingSubrec { .. }) => {
                    Ok(FragmentQueryResult::Empty)
                }
                Err(e) => Err(e),
            })
            .collect::<Result<Vec<_>>>()?;

        let non_empty: Vec<(StateValues, Vec<i32>)> = results
            .into_iter()
            .filter_map(|r| match r {
                FragmentQueryResult::Empty => None,
                FragmentQueryResult::Data { values, labels } => {
                    if labels.is_empty() {
                        None
                    } else {
                        Some((values, labels))
                    }
                }
            })
            .collect();

        if non_empty.is_empty() {
            return Err(MiliError::NoMatchingSubrec {
                svar: args.svar.to_owned(),
                class: args.class.to_owned(),
            });
        }

        // Every contributing fragment must agree on numeric type and
        // per-label atom count. Disagreement here is a real bug, not a
        // lenient case.
        let num_type = non_empty[0].0.num_type();
        let state_count = args.states.len();
        let atoms_per_label = atoms_per_label_of(&non_empty[0].0, &non_empty[0].1, state_count)?;
        for (i, (vals, labels)) in non_empty.iter().enumerate().skip(1) {
            if vals.num_type() != num_type {
                return Err(MiliError::FragmentMismatch {
                    fragment: i,
                    field: "query_num_type",
                    detail: format!(
                        "fragment 0 returned {:?}, fragment {} returned {:?}",
                        num_type,
                        i,
                        vals.num_type()
                    ),
                });
            }
            let a = atoms_per_label_of(vals, labels, state_count)?;
            if a != atoms_per_label {
                return Err(MiliError::FragmentMismatch {
                    fragment: i,
                    field: "query_atoms_per_label",
                    detail: format!("fragment 0 = {atoms_per_label}, fragment {i} = {a}"),
                });
            }
        }

        // Concatenate along the entity axis. For each state s, output
        // is frag0[s] ++ frag1[s] ++ ... — i.e. labels are
        // [frag0.labels ++ frag1.labels ++ ...], and within each state
        // the values are the per-fragment per-state chunks
        // concatenated in the same order.
        let merged_labels: Vec<i32> = non_empty
            .iter()
            .flat_map(|(_, l)| l.iter().copied())
            .collect();
        let total_labels = merged_labels.len();
        let total_count = state_count
            .checked_mul(total_labels)
            .and_then(|n| n.checked_mul(atoms_per_label))
            .ok_or(MiliError::MalformedDirectory(
                "DatabaseSet::query: merged total count overflow",
            ))?;

        let values = concat_state_values(&non_empty, state_count, atoms_per_label, total_count)?;

        // Dedupe by label keeping first occurrence — matches mili-
        // python's `np.unique(return_index=True)` post-pass in
        // `merge_result_dictionaries`. For well-formed MPI output with
        // no ghost overlap this is a no-op; the path matters only when
        // ranks share boundary entities.
        let (final_labels, keep_idx) = dedupe_first(&merged_labels);
        let values = if keep_idx.len() == total_labels {
            values
        } else {
            select_labels(
                &values,
                state_count,
                atoms_per_label,
                total_labels,
                &keep_idx,
            )
        };

        Ok(SetQueryResult {
            values,
            labels: final_labels,
            atoms_per_label,
            state_count,
        })
    }
}

/// Result of [`DatabaseSet::query`]. Carries the merged entity axis
/// (the labels vector) alongside the flat value buffer so callers know
/// what each row of the `[state][label][atom]` layout corresponds to.
#[derive(Debug, Clone, PartialEq)]
pub struct SetQueryResult {
    pub values: StateValues,
    pub labels: Vec<i32>,
    pub atoms_per_label: usize,
    pub state_count: usize,
}

enum FragmentQueryResult {
    Empty,
    Data {
        values: StateValues,
        labels: Vec<i32>,
    },
}

fn atoms_per_label_of(values: &StateValues, labels: &[i32], state_count: usize) -> Result<usize> {
    let total = values.len();
    let divisor = state_count
        .checked_mul(labels.len())
        .ok_or(MiliError::MalformedDirectory(
            "DatabaseSet::query: state * label overflow",
        ))?;
    if divisor == 0 {
        return Ok(0);
    }
    if total % divisor != 0 {
        return Err(MiliError::MalformedDirectory(
            "DatabaseSet::query: value count not a multiple of state * label",
        ));
    }
    Ok(total / divisor)
}

fn concat_state_values(
    parts: &[(StateValues, Vec<i32>)],
    state_count: usize,
    atoms_per_label: usize,
    total_count: usize,
) -> Result<StateValues> {
    macro_rules! concat_typed {
        ($variant:ident, $ty:ty) => {{
            let mut out: Vec<$ty> = vec![<$ty>::default(); total_count];
            let row = atoms_per_label;
            let merged_labels_total: usize = parts.iter().map(|(_, l)| l.len()).sum();
            let dst_stride = merged_labels_total * row;
            let mut frag_offset_labels: usize = 0;
            for (vals, labels) in parts {
                let src = match vals {
                    StateValues::$variant(v) => v,
                    _ => {
                        return Err(MiliError::MalformedDirectory(
                            "DatabaseSet::query: mixed numeric types after type check",
                        ))
                    }
                };
                let frag_labels = labels.len();
                let src_stride = frag_labels * row;
                for s in 0..state_count {
                    let src_start = s * src_stride;
                    let dst_start = s * dst_stride + frag_offset_labels * row;
                    out[dst_start..dst_start + src_stride]
                        .copy_from_slice(&src[src_start..src_start + src_stride]);
                }
                frag_offset_labels += frag_labels;
            }
            StateValues::$variant(out)
        }};
    }
    let num_type = parts[0].0.num_type();
    Ok(match num_type {
        NumType::Float4 => concat_typed!(F32, f32),
        NumType::Float8 => concat_typed!(F64, f64),
        NumType::Int4 => concat_typed!(I32, i32),
        NumType::Int8 => concat_typed!(I64, i64),
    })
}

/// Return `(unique_labels_in_first_occurrence_order, indices_into_input)`.
fn dedupe_first(labels: &[i32]) -> (Vec<i32>, Vec<usize>) {
    let mut seen: HashSet<i32> = HashSet::with_capacity(labels.len());
    let mut unique = Vec::with_capacity(labels.len());
    let mut keep = Vec::with_capacity(labels.len());
    for (i, &l) in labels.iter().enumerate() {
        if seen.insert(l) {
            unique.push(l);
            keep.push(i);
        }
    }
    (unique, keep)
}

fn select_labels(
    values: &StateValues,
    state_count: usize,
    atoms_per_label: usize,
    in_labels: usize,
    keep_idx: &[usize],
) -> StateValues {
    macro_rules! select_typed {
        ($variant:ident, $ty:ty) => {{
            let StateValues::$variant(src) = values else {
                unreachable!("num_type mismatch in select_labels")
            };
            let out_labels = keep_idx.len();
            let mut out: Vec<$ty> =
                vec![<$ty>::default(); state_count * out_labels * atoms_per_label];
            let src_stride = in_labels * atoms_per_label;
            let dst_stride = out_labels * atoms_per_label;
            for s in 0..state_count {
                for (new_i, &old_i) in keep_idx.iter().enumerate() {
                    let src_start = s * src_stride + old_i * atoms_per_label;
                    let dst_start = s * dst_stride + new_i * atoms_per_label;
                    out[dst_start..dst_start + atoms_per_label]
                        .copy_from_slice(&src[src_start..src_start + atoms_per_label]);
                }
            }
            StateValues::$variant(out)
        }};
    }
    match values.num_type() {
        NumType::Float4 => select_typed!(F32, f32),
        NumType::Float8 => select_typed!(F64, f64),
        NumType::Int4 => select_typed!(I32, i32),
        NumType::Int8 => select_typed!(I64, i64),
    }
}

/// Split a user-supplied base path into `(parent_dir, file_name)`. The
/// file-name component is the literal base passed to fragment
/// matching; no `A` / digit / `.plt` stripping is applied (matches
/// `mili.reader.open_database`'s `os.path.basename(base)` step).
fn resolve_base(path: &Path) -> Result<(PathBuf, String)> {
    let dir = match path.parent() {
        Some(p) if !p.as_os_str().is_empty() => p.to_path_buf(),
        _ => PathBuf::from("."),
    };
    let file_name =
        path.file_name()
            .and_then(|n| n.to_str())
            .ok_or(MiliError::MalformedDirectory(
                "DatabaseSet::open: base path has no filename component",
            ))?;
    Ok((dir, file_name.to_owned()))
}

/// Return `Some(rank)` if `name` matches `^<base>(\d*)A$`, where the
/// numeric suffix decodes to `rank` (or `None` for a bare `<base>A`
/// with no digits). Returns `None` if the name does not match.
fn match_fragment(name: &str, base: &str) -> Option<Option<u32>> {
    let after_base = name.strip_prefix(base)?;
    let digits = after_base.strip_suffix('A')?;
    if digits.is_empty() {
        return Some(None);
    }
    if !digits.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    digits.parse::<u32>().ok().map(Some)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn match_fragment_accepts_bare_a() {
        assert_eq!(match_fragment("runA", "run"), Some(None));
    }

    #[test]
    fn match_fragment_decodes_rank() {
        assert_eq!(match_fragment("run000A", "run"), Some(Some(0)));
        assert_eq!(match_fragment("run007A", "run"), Some(Some(7)));
        assert_eq!(match_fragment("run123A", "run"), Some(Some(123)));
    }

    #[test]
    fn match_fragment_rejects_non_matches() {
        assert_eq!(match_fragment("run.plt000A", "run"), None); // base doesn't match
        assert_eq!(match_fragment("run000", "run"), None); // no A
        assert_eq!(match_fragment("runXA", "run"), None); // non-digit middle
        assert_eq!(match_fragment("otherA", "run"), None); // different base
    }

    #[test]
    fn resolve_base_splits_dir_and_file() {
        let (dir, base) = resolve_base(Path::new("a/b/basic1.plt")).unwrap();
        assert_eq!(dir, Path::new("a/b"));
        assert_eq!(base, "basic1.plt");

        let (dir, base) = resolve_base(Path::new("basic1.plt")).unwrap();
        assert_eq!(dir, Path::new("."));
        assert_eq!(base, "basic1.plt");

        let (dir, base) = resolve_base(Path::new("d3samp6.th")).unwrap();
        assert_eq!(dir, Path::new("."));
        assert_eq!(base, "d3samp6.th");
    }

    #[test]
    fn match_fragment_accepts_dotted_base() {
        assert_eq!(
            match_fragment("basic1.plt000A", "basic1.plt"),
            Some(Some(0))
        );
        assert_eq!(
            match_fragment("basic1.plt007A", "basic1.plt"),
            Some(Some(7))
        );
        assert_eq!(match_fragment("d3samp6.thA", "d3samp6.th"), Some(None));
    }

    #[test]
    fn dedupe_first_keeps_first_occurrence() {
        let (u, idx) = dedupe_first(&[1, 2, 3, 2, 4, 1]);
        assert_eq!(u, vec![1, 2, 3, 4]);
        assert_eq!(idx, vec![0, 1, 2, 4]);
    }

    #[test]
    fn dedupe_first_noop_on_unique() {
        let (u, idx) = dedupe_first(&[10, 20, 30]);
        assert_eq!(u, vec![10, 20, 30]);
        assert_eq!(idx, vec![0, 1, 2]);
    }
}
