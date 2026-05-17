//! State-data read path: byte-offset math and gather planning for
//! single-svar queries with label, integration-point, atom-subscript,
//! and multi-state filters across both subrecord organisations.
//!
//! The byte-layout matrix in `planning/shared/format.md` § "Subrecord
//! byte-layout matrix" is the source of truth. For svar `s` at ordinal
//! `j` inside subrecord `k`, rooted at `subrec_k_start`:
//!
//! ```text
//! RESULT_ORDERED:
//!     row(j) = subrec_k_start + N_k * lump_offsets[s] + j * lump_sizes[s]
//!     row length = lump_sizes[s]
//! OBJECT_ORDERED:
//!     row(j) = subrec_k_start + j * bytes_per_object_k + lump_offsets[s]
//!     row length = lump_sizes[s]
//! ```
//!
//! The vec_array IP filter selects a subset of integration-point
//! indices within the per-object atom run. Per the `vecarray` corpus
//! and `reference/mili-python/src/mili/datatypes.py:236-247`, the inner
//! order of a `VEC_ARRAY` is components-fastest, IPs-slowest: with
//! `dims = [n_ip]` and component count `K`, atom `(ip, c)` sits at
//! `ip * K + c`. The format-doc's "array-dim-indices-fastest → IP
//! slowest → components" line in `planning/shared/format.md` § cell
//! `VEC_ARRAY` is wrong (logged under `status.md` § "Resolved questions
//! log") — the writer / reader treat components as fastest. Both
//! layouts agree for the common `prod(dims) == n_ip` case used here.
//!
//! Array-svar subscript (`"hx[3]"`, 1-based) and bare-component lookups
//! (`"sx"` resolving to the `sx` component of a `stress` VECTOR svar)
//! both go through [`parse_query_name`] + [`resolve_target`]. They
//! collapse onto the same gather primitive: an [`AtomPicker::Specific`]
//! list of 0-based atom indices into the per-object slot of the base
//! svar that actually appears in the subrecord. (`reference/mili-
//! python/src/mili/miliinternal.py:976-1016, 1272-1286`.)

use crate::error::{MiliError, Result};
use crate::srec::{derive_lumps, Lumps, Organization, Srec, Subrecord};
use crate::svar::{NumType, Svar, SvarAgg, SvarTable};

/// Typed return for a query. Variant is keyed off the svar's
/// [`NumType`]. Values are flat in `[state][label][atom]` row-major
/// order. `len == states * labels * row_atoms`, where `row_atoms` is
/// `Svar::atoms` for unfiltered reads or `comps * ips` for vec_array
/// reads with an `ips` filter.
#[derive(Debug, Clone, PartialEq)]
pub enum StateValues {
    F32(Vec<f32>),
    F64(Vec<f64>),
    I32(Vec<i32>),
    I64(Vec<i64>),
}

impl StateValues {
    pub fn len(&self) -> usize {
        match self {
            Self::F32(v) => v.len(),
            Self::F64(v) => v.len(),
            Self::I32(v) => v.len(),
            Self::I64(v) => v.len(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn num_type(&self) -> NumType {
        match self {
            Self::F32(_) => NumType::Float4,
            Self::F64(_) => NumType::Float8,
            Self::I32(_) => NumType::Int4,
            Self::I64(_) => NumType::Int8,
        }
    }
}

/// Owned, QueryDict-shaped result for one `(svar, class)` query —
/// everything upstream `mili.query()[svar]` carries except the
/// `states`/`times` layout axes (those are caller-side: the state
/// numbers passed in and `Database::times()`). The `mili-py` binding
/// attaches those and constructs the Python dict; this struct keeps
/// the parity-sensitive `components`/`title` derivation in core (M1/M2
/// precedent — the binding stays a thin pass-through).
#[derive(Debug, Clone, PartialEq)]
pub struct QueryResult {
    /// Flat `[state][label][atom]` row-major, same as [`StateValues`].
    pub values: StateValues,
    /// Entity-axis labels, one per `[label]` row.
    pub labels: Vec<i32>,
    /// Component names in `[atom]` order. Length == atoms-per-label.
    pub components: Vec<String>,
    /// Queried svar's title (`Svar::title` of the resolved base).
    pub title: String,
    /// Object-class short name (echoed from the query args).
    pub class_name: String,
}

/// One contiguous byte range to read from a single state file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ByteSlab {
    pub start: usize,
    pub len: usize,
}

/// Per-state gather plan: a flat list of byte ranges to read, in
/// `[label][atom]` output order, plus the svar's numeric type and the
/// per-row atom count after IP filtering. `state_data_start` shifts
/// the plan to a different state file via [`ReadPlan::rebased`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ReadPlan {
    pub num_type: NumType,
    pub slabs: Vec<ByteSlab>,
    /// Base byte offset (state-data-start) this plan was built against.
    pub state_data_start: u64,
    /// Entity-axis labels in `[label]` output order (one per output
    /// row, repeated for every state). Length is constant across
    /// states for a given srec format. For unfiltered queries this
    /// comes from each matching subrec's `id_blocks` in directory
    /// order; for label-filtered queries this is the input list.
    pub labels: Vec<i32>,
    /// Component-name override. `Some` only on the bare-component-of-
    /// VEC_ARRAY substitution path (Slice B), where the names carry the
    /// resolved IP labels (`f"{comp} ipt. {label}"`,
    /// `reference/mili-python/src/mili/miliinternal.py:1367`). `None`
    /// means "derive from the svar table" (`svar_query_meta`).
    pub components: Option<Vec<String>>,
}

impl ReadPlan {
    pub fn total_bytes(&self) -> usize {
        self.slabs.iter().map(|s| s.len).sum()
    }

    /// Produce a plan with every slab rebased onto a different
    /// `state_data_start`. Used to walk multiple states whose subrec
    /// layout is identical (same srec format) by computing offsets
    /// once and shifting.
    pub fn rebased(&self, new_start: u64) -> Result<Self> {
        let delta = i128::from(new_start) - i128::from(self.state_data_start);
        let mut slabs = Vec::with_capacity(self.slabs.len());
        for s in &self.slabs {
            let shifted = i128::from(s.start as u64) + delta;
            if shifted < 0 || shifted > i128::from(u64::MAX) {
                return Err(MiliError::MalformedDirectory("query: rebase overflow"));
            }
            let start = usize::try_from(shifted as u64)
                .map_err(|_| MiliError::MalformedDirectory("query: rebase exceeds usize"))?;
            slabs.push(ByteSlab { start, len: s.len });
        }
        Ok(Self {
            num_type: self.num_type,
            slabs,
            state_data_start: new_start,
            labels: self.labels.clone(),
            components: self.components.clone(),
        })
    }
}

/// Filter inputs for a single-svar query. All fields are pre-resolved
/// caller-side (label list, state list, ip list); see the top-level
/// [`crate::Database`] surface for the user-facing types.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Filter<'a> {
    /// 1-based mili object ids to select. `None` means "all objects in
    /// every matching subrec, in subrec-directory then in-subrec order".
    pub labels: Option<&'a [i32]>,
    /// Integration-point indices to keep, 0-based within the vec_array
    /// inner order (components-fastest, IP-slowest). `None` means
    /// "every IP".
    pub ips: Option<&'a [usize]>,
    /// Restrict the gather to the single subrecord with this name
    /// (the `subrec=` query kwarg, `miliinternal.py:1246-1247`).
    /// `None` means "every matching subrecord".
    pub subrec: Option<&'a str>,
}

/// svar → element-set → IP-label linkage, the core analogue of upstream
/// `_MiliInternal.__int_points` (`reference/mili-python/src/mili/
/// miliinternal.py:156-192`). Keyed by component (or vector, or
/// element-set) svar name; each entry lists the VEC_ARRAY parent svars
/// (`es_<n>a`) that carry it, with that parent's element-set payload
/// (the integration-point *labels* followed by the trailing count, as
/// written — `family.rs::element_sets`).
///
/// Built once per query by [`crate::Database::build_int_points`]; the
/// gather planner consumes it to substitute a bare component of a
/// VEC_ARRAY (Slice B) and to map user `ips=` *labels* → 0-based
/// positional indices.
#[derive(Debug, Default, Clone)]
pub(crate) struct IntPoints {
    map: std::collections::HashMap<String, Vec<IpParent>>,
}

#[derive(Debug, Clone)]
pub(crate) struct IpParent {
    /// The VEC_ARRAY svar name (e.g. `es_1a`) that carries the
    /// component, and whose subrecord we substitute onto.
    pub es_svar: String,
    /// Element-set payload exactly as written: IP labels then a single
    /// trailing count entry (`miliinternal.py:113-115` /
    /// `family.rs::element_sets`).
    pub payload: Vec<i32>,
}

impl IntPoints {
    pub(crate) fn insert(&mut self, comp: &str, es_svar: &str, payload: &[i32]) {
        let entry = self.map.entry(comp.to_owned()).or_default();
        // Mirror upstream's dict semantics: one entry per (comp, es).
        if !entry.iter().any(|p| p.es_svar == es_svar) {
            entry.push(IpParent {
                es_svar: es_svar.to_owned(),
                payload: payload.to_vec(),
            });
        }
    }

    fn parents(&self, comp: &str) -> &[IpParent] {
        self.map.get(comp).map_or(&[], Vec::as_slice)
    }

    /// Public (in-crate) view of the `(es_svar, payload)` parents of a
    /// component, used by the `int_points_of_state_variable` reshape
    /// (`reshape.rs`). Mirrors upstream `__int_points[svar]` iteration.
    pub(crate) fn parents_of(&self, comp: &str) -> &[IpParent] {
        self.parents(comp)
    }
}

/// Build the read plan for one `(svar, class, state)` tuple under the
/// given filter, rooted at the state's data block (the byte offset of
/// the first subrec inside its state file).
///
/// Errors on:
/// - svar / class not present
/// - no subrecord covers `(svar, class)`
/// - a label in `filter.labels` is not covered by any matching subrec
/// - an `ips` filter against a non-vec_array svar
/// - an `ips` value out of range
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) fn plan_state_svar(
    srec: &Srec,
    svars: &SvarTable,
    svar_input: &str,
    class_name: &str,
    state_data_start: u64,
    filter: Filter<'_>,
) -> Result<ReadPlan> {
    plan_state_svar_ip(
        srec,
        svars,
        svar_input,
        class_name,
        state_data_start,
        filter,
        &IntPoints::default(),
    )
}

/// As [`plan_state_svar`], plus the svar→element-set→IP-label linkage
/// (`int_points`) needed to substitute a bare component of a VEC_ARRAY
/// and to interpret `filter.ips` as element-set IP *labels* (Slice B,
/// `reference/mili-python/src/mili/miliinternal.py:1251-1270,1362-1378`).
/// The no-linkage path is byte-for-byte the pre-Slice-B behaviour.
// Slice B couples four resolution stages (direct → VECTOR parent →
// VEC_ARRAY substitution → consistency) in one read-plan builder; the
// branches are sequential and individually small.
#[allow(clippy::too_many_lines)]
pub(crate) fn plan_state_svar_ip(
    srec: &Srec,
    svars: &SvarTable,
    svar_input: &str,
    class_name: &str,
    state_data_start: u64,
    filter: Filter<'_>,
    int_points: &IntPoints,
) -> Result<ReadPlan> {
    let parsed = parse_query_name(svar_input)?;
    let is_subscript = matches!(parsed, QueryName::Subscript { .. });
    // A named-component subscript `parent[comp]` gathers exactly like a
    // bare component query of `comp` (`miliinternal.py:990-996,1228-1231`):
    // resolve against the component, validate it belongs to the named
    // parent, and let the bare-component / vec-array-substitution paths
    // below do the work. Only the result key (raw input) and title
    // (parent's, in `svar_query_meta`) differ.
    let parsed = match parsed {
        QueryName::CompSubscript { base, comps } => {
            let parent = svars
                .get(base)
                .ok_or_else(|| MiliError::UnknownSvar(base.to_owned()))?;
            let (SvarAgg::Vector {
                comps: parent_comps,
            }
            | SvarAgg::VecArray {
                comps: parent_comps,
                ..
            }) = &parent.agg
            else {
                return Err(MiliError::SubscriptNotApplicable {
                    svar: base.to_owned(),
                    agg: agg_label(&parent.agg),
                });
            };
            if comps.len() != 1 {
                return Err(MiliError::Unsupported(
                    "multi-component named subscript (only single-component \
                     parent[comp] supported)",
                ));
            }
            let comp = comps[0];
            if !parent_comps.iter().any(|c| c == comp) {
                return Err(MiliError::UnknownSvar(format!("{base}[{comp}]")));
            }
            QueryName::Plain(comp)
        }
        other => other,
    };
    let mut resolved = resolve_target(svars, parsed, filter.ips, int_points)?;

    let mut matches = collect_matching_subrecs(
        srec,
        svars,
        &resolved.base_name,
        class_name,
        state_data_start,
        filter.subrec,
    )?;

    // Bare component-of-VEC_ARRAY substitution (Slice B). When the
    // component is not carried directly and not a plain VECTOR member,
    // it may live inside a VEC_ARRAY parent (`es_<n>a`). Resolve via the
    // `int_points` linkage, mapping user `ips=` *labels* → positional
    // IP indices and naming components `f"{comp} ipt. {label}"`
    // (`miliinternal.py:1251-1270,1362-1378`). Each substituted subrec
    // gets its own `AtomPicker::Specific` (component-outer, IP-inner) so
    // the data axis matches the component-name order.
    let try_substitution = |base_name: &str| -> Result<Option<ReadPlan>> {
        let Some(sub) = try_vec_array_substitution(
            srec,
            svars,
            base_name,
            class_name,
            state_data_start,
            filter,
            int_points,
        )?
        else {
            return Ok(None);
        };
        let width = sub.num_type.width();
        let (slabs, labels) = match filter.labels {
            None => gather_all(&sub.matches, width, &AtomPicker::AllAtoms),
            Some(labels) => (
                gather_by_labels(
                    &sub.matches,
                    width,
                    &AtomPicker::AllAtoms,
                    labels,
                    class_name,
                )?,
                labels.to_vec(),
            ),
        };
        Ok(Some(ReadPlan {
            num_type: sub.num_type,
            slabs,
            state_data_start,
            labels,
            components: Some(sub.components),
        }))
    };

    // When `ips` is given the user explicitly wants the element-set
    // integration point, so the VEC_ARRAY substitution takes precedence
    // over the VECTOR-parent fallback (Slice B `basic1` `sx`/`brick`
    // `ips=4`). `ips` is meaningless for a plain VECTOR — for a
    // component that has *only* a VECTOR parent (`sx`/`brick` on
    // d3samp6) the substitution finds nothing and the fallback below
    // resolves it with `ips` silently ignored, matching the oracle.
    if matches.is_empty() && !is_subscript && filter.ips.is_some() {
        if let Some(plan) = try_substitution(&resolved.base_name)? {
            return Ok(plan);
        }
    }

    // Bare component-name fallback: if no subrec carries the named svar
    // directly, see whether it is a component of a VECTOR parent
    // (`reference/mili-python/src/mili/miliinternal.py:990-996`) and
    // retry against that parent. `ips` does *not* gate this: upstream
    // only consumes `ips` for VEC_ARRAY svars and silently ignores it
    // for everything else (scalar / VECTOR / ARRAY) — verified vs the
    // `_MiliInternal` oracle (`sx`/`brick`+`ips` on d3samp6,
    // `sand`/`brick`+`ips` on basic1).
    if matches.is_empty() && !is_subscript {
        // A component can belong to several VECTOR parents (e.g.
        // `sx` ∈ `stress`, `stress_mid`, `stress_in`, `stress_out`).
        // Upstream disambiguates by subrecord membership for the
        // queried class (`miliinternal.py:1222-1231`): the right
        // parent is the one actually carried in a subrec for that
        // class. Take the first such parent in svar-table order.
        for parent in find_vector_parents(svars, &resolved.base_name) {
            let parent_matches = collect_matching_subrecs(
                srec,
                svars,
                &parent.name,
                class_name,
                state_data_start,
                filter.subrec,
            )?;
            if !parent_matches.is_empty() {
                resolved = Resolved {
                    base_name: parent.name.clone(),
                    num_type: resolved.num_type,
                    picker: AtomPicker::Specific {
                        atom_indices: parent.comp_atom_indices,
                    },
                };
                matches = parent_matches;
                break;
            }
        }
    }

    // VEC_ARRAY substitution for the no-`ips` case (or when the VECTOR
    // fallback above found nothing) — unchanged Slice B behavior.
    if matches.is_empty() && !is_subscript {
        if let Some(plan) = try_substitution(&resolved.base_name)? {
            return Ok(plan);
        }
    }

    if matches.is_empty() {
        return Err(MiliError::NoMatchingSubrec {
            svar: svar_input.to_owned(),
            class: class_name.to_owned(),
        });
    }

    // `ips` against a svar that resolved directly (no VEC_ARRAY
    // substitution) and is not itself a vec_array is **silently
    // ignored**, exactly matching the upstream `_MiliInternal` oracle
    // (`miliinternal.py:1246-1270` only builds `matching_int_points`
    // for `__int_points`-linked svars; a scalar / VECTOR component /
    // ARRAY never consumes `ips`). Cross-validated:
    // `query("sx","brick",ips=[1])` on d3samp6 and
    // `query("sand","brick",ips=[1])` on basic1 both succeed upstream
    // with `ips` ignored. A direct VEC_ARRAY keeps its `ips` filter via
    // the picker built in `resolve_target` (`resolve_atom_picker`).

    // Inconsistent integration-point counts across subrecords on the
    // same class produce ragged output. mili-python raises
    // `ValueError` for the no-ips-filter case
    // (`test_bugfixes.py::InconsistantIntPointsForElementClassResult`);
    // mirror that here with a typed error so consumers get a clear
    // signal instead of a misleading length mismatch downstream. With
    // an `ips` filter the picker either takes the same per-IP slab
    // across all subrecs (consistent by construction) or has already
    // been rejected upstream.
    if filter.ips.is_none() && matches.len() > 1 {
        let first = matches[0].lumps.sizes[matches[0].svar_idx];
        let mut distinct: Vec<usize> = Vec::new();
        distinct.push(first);
        for m in &matches[1..] {
            let s = m.lumps.sizes[m.svar_idx];
            if !distinct.contains(&s) {
                distinct.push(s);
            }
        }
        if distinct.len() > 1 {
            return Err(MiliError::InconsistentIpCounts {
                svar: svar_input.to_owned(),
                class: class_name.to_owned(),
                counts: distinct,
            });
        }
    }

    let width = resolved.num_type.width();
    let (slabs, labels) = match filter.labels {
        None => gather_all(&matches, width, &resolved.picker),
        Some(labels) => (
            gather_by_labels(&matches, width, &resolved.picker, labels, class_name)?,
            labels.to_vec(),
        ),
    };

    Ok(ReadPlan {
        num_type: resolved.num_type,
        slabs,
        state_data_start,
        labels,
        components: None,
    })
}

/// Result of resolving a bare component against a VEC_ARRAY parent.
struct SubstitutionPlan<'a> {
    matches: Vec<SubrecMatch<'a>>,
    num_type: NumType,
    components: Vec<String>,
}

/// Resolve `comp_name` as a component of a VEC_ARRAY parent listed in
/// the `int_points` linkage, producing one [`SubrecMatch`] per carrying
/// subrecord with its own component-outer/IP-inner [`AtomPicker`].
/// Mirrors `miliinternal.py:1251-1270` (label→index via `.index(ip)`,
/// all-IPs via `range(payload[-1])`) and the cross-subrecord IP-count
/// consistency check (`miliinternal.py:1340-1349`).
// One linear pass over the srec mirroring upstream's per-subrec
// int-point resolution; extracting sub-steps would only scatter the
// shared running-offset / payload state.
#[allow(clippy::too_many_lines)]
fn try_vec_array_substitution<'a>(
    srec: &'a Srec,
    svars: &SvarTable,
    comp_name: &str,
    class_name: &str,
    state_data_start: u64,
    filter: Filter<'_>,
    int_points: &IntPoints,
) -> Result<Option<SubstitutionPlan<'a>>> {
    let parents = int_points.parents(comp_name);
    if parents.is_empty() {
        return Ok(None);
    }
    // The leaf scalar components this query expands to (a scalar maps to
    // itself; a VECTOR like `stress` maps to its leaves in order).
    let leaves = leaf_components(svars, comp_name);
    if leaves.is_empty() {
        return Ok(None);
    }
    let num_type = svars
        .get(&leaves[0])
        .map(|s| s.num_type)
        .ok_or_else(|| MiliError::UnknownSvar(leaves[0].clone()))?;

    // Walk the srec; for each subrecord on this class that carries one
    // of the candidate VEC_ARRAY parents, build its picker.
    let mut running: u64 = state_data_start;
    let mut out: Vec<SubrecMatch<'a>> = Vec::new();
    let mut comp_qtys: Vec<usize> = Vec::new();
    let mut components: Option<Vec<String>> = None;
    for sub in &srec.subrecords {
        let (atoms, widths) = atoms_and_widths(sub, svars)?;
        let lumps = derive_lumps(&atoms, &widths);
        let n = sub.object_count() as usize;
        let size = n
            .checked_mul(lumps.bytes_per_object())
            .ok_or(MiliError::MalformedDirectory("query: subrec size overflow"))?;
        let subrec_start = running;
        running = running
            .checked_add(size as u64)
            .ok_or(MiliError::MalformedDirectory(
                "query: subrec offset overflow",
            ))?;

        if sub.mclass != class_name {
            continue;
        }
        if let Some(want) = filter.subrec {
            if sub.name != want {
                continue;
            }
        }
        // Which candidate VEC_ARRAY parent does this subrec carry?
        let Some((svar_idx, parent)) = sub
            .svar_names
            .iter()
            .enumerate()
            .find_map(|(i, sn)| parents.iter().find(|p| &p.es_svar == sn).map(|p| (i, p)))
        else {
            continue;
        };

        let es = svars
            .get(&parent.es_svar)
            .ok_or_else(|| MiliError::UnknownSvar(parent.es_svar.clone()))?;
        let n_ip = *parent.payload.last().ok_or(MiliError::MalformedDirectory(
            "element set payload missing trailing count",
        ))? as usize;
        if n_ip == 0 || es.atoms % n_ip != 0 {
            return Err(MiliError::MalformedDirectory(
                "vec_array parent atoms not divisible by IP count",
            ));
        }
        let atoms_per_ip = es.atoms / n_ip;
        let ip_labels = &parent.payload[..parent.payload.len() - 1];

        // Selected IP positional indices + their labels. With `ips=`,
        // map each *label* to its position (`list.index(ip)`); else all.
        let (ip_positions, sel_labels): (Vec<usize>, Vec<i32>) = match filter.ips {
            Some(reqs) => {
                let mut pos = Vec::with_capacity(reqs.len());
                let mut labs = Vec::with_capacity(reqs.len());
                for &r in reqs {
                    let want = i32::try_from(r)
                        .map_err(|_| MiliError::IpOutOfRange { ip: r, atoms: n_ip })?;
                    let p = ip_labels
                        .iter()
                        .position(|&x| x == want)
                        .ok_or(MiliError::IpOutOfRange { ip: r, atoms: n_ip })?;
                    pos.push(p);
                    labs.push(want);
                }
                (pos, labs)
            }
            None => ((0..n_ip).collect(), ip_labels.to_vec()),
        };

        // Per-leaf atom offset within one IP block (the leaf's index in
        // the flattened component list of the parent). VEC_ARRAY inner
        // order is components-fastest, IP-slowest, so atom for
        // (leaf, ip) = ip_pos * atoms_per_ip + leaf_offset.
        let parent_leaves = leaf_components(svars, &parent.es_svar);
        let mut atom_indices = Vec::with_capacity(leaves.len() * ip_positions.len());
        for leaf in &leaves {
            let leaf_off = parent_leaves
                .iter()
                .position(|l| l == leaf)
                .ok_or_else(|| MiliError::UnknownSvar(leaf.clone()))?;
            for &ipp in &ip_positions {
                atom_indices.push(ipp * atoms_per_ip + leaf_off);
            }
        }

        // Component names: component-outer, IP-inner — matches the
        // atom-index order above (`miliinternal.py:1367`).
        if components.is_none() {
            let mut names = Vec::with_capacity(leaves.len() * sel_labels.len());
            for leaf in &leaves {
                for lab in &sel_labels {
                    names.push(format!("{leaf} ipt. {lab}"));
                }
            }
            components = Some(names);
        }
        comp_qtys.push(leaves.len() * ip_positions.len());

        out.push(SubrecMatch {
            sub,
            subrec_start,
            svar_idx,
            lumps,
            n,
            picker_override: Some(AtomPicker::Specific { atom_indices }),
        });
    }

    if out.is_empty() {
        return Ok(None);
    }

    // Cross-material / cross-subrecord IP-count consistency
    // (`miliinternal.py:1340-1349`). Inconsistent component counts are
    // unrepresentable as a single rectangular array — upstream raises
    // `ValueError`; surface the typed equivalent.
    let mut distinct: Vec<usize> = Vec::new();
    for &q in &comp_qtys {
        if !distinct.contains(&q) {
            distinct.push(q);
        }
    }
    if distinct.len() > 1 {
        return Err(MiliError::InconsistentIpCounts {
            svar: comp_name.to_owned(),
            class: class_name.to_owned(),
            counts: distinct,
        });
    }

    Ok(Some(SubstitutionPlan {
        matches: out,
        num_type,
        components: components.unwrap_or_default(),
    }))
}

/// Flatten a svar to its leaf scalar component names, in declaration
/// order. A scalar (or unknown) maps to itself; a VECTOR / VEC_ARRAY
/// recurses through its components. Mirrors the recursion upstream
/// applies via `StateVariable.svars` (`miliinternal.py:140-154`).
fn leaf_components(svars: &SvarTable, name: &str) -> Vec<String> {
    match svars.get(name).map(|s| &s.agg) {
        Some(SvarAgg::Vector { comps } | SvarAgg::VecArray { comps, .. }) => {
            let mut out = Vec::new();
            for c in comps {
                out.extend(leaf_components(svars, c));
            }
            out
        }
        _ => vec![name.to_owned()],
    }
}

/// Parsed form of a user-supplied svar query name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum QueryName<'a> {
    /// `"hx"`, `"sx"` — plain name lookup.
    Plain(&'a str),
    /// `"hx[3]"`, `"hx[1,2]"` — 1-based subscript into an ARRAY svar.
    /// Indices are signed so out-of-range / 0 / negative values can be
    /// rejected by `resolve_target` with a typed `InvalidSubscript`,
    /// matching mili-python's error contract
    /// (`reference/mili-python/src/mili/miliinternal.py:1276-1286`).
    Subscript { base: &'a str, indices: Vec<i64> },
    /// `"nodpos[ux]"`, `"stress[sy]"` — a *named* component of a
    /// VECTOR / VEC_ARRAY parent. Upstream
    /// (`miliinternal.py:976-996`) splits `parent[comp,...]` and, when
    /// the bracket content is not integer indices, treats the tokens
    /// as component svar names of `base`. The component data is gathered
    /// exactly as the bare-component path; only the result *key* (the
    /// raw input) and *title* (the parent's) differ.
    CompSubscript { base: &'a str, comps: Vec<&'a str> },
}

/// Parse a query-name string. The grammar is intentionally minimal:
/// a bare name, or `name[i,j,k,...]` where each component is a signed
/// integer literal. Whitespace inside the bracket is tolerated.
pub(crate) fn parse_query_name(input: &str) -> Result<QueryName<'_>> {
    let Some(open) = input.find('[') else {
        return Ok(QueryName::Plain(input));
    };
    if !input.ends_with(']') {
        return Err(MiliError::InvalidSubscript {
            input: input.to_owned(),
            reason: "missing closing ']'",
        });
    }
    let base = &input[..open];
    if base.is_empty() {
        return Err(MiliError::InvalidSubscript {
            input: input.to_owned(),
            reason: "missing svar name before '['",
        });
    }
    let inner = &input[open + 1..input.len() - 1];
    if inner.is_empty() {
        return Err(MiliError::InvalidSubscript {
            input: input.to_owned(),
            reason: "empty subscript",
        });
    }
    let mut toks = Vec::new();
    for tok in inner.split(',') {
        let tok = tok.trim();
        if tok.is_empty() {
            return Err(MiliError::InvalidSubscript {
                input: input.to_owned(),
                reason: "empty subscript index",
            });
        }
        toks.push(tok);
    }
    // Integer tokens -> ARRAY-svar subscript; otherwise the tokens are
    // component svar names of a VECTOR / VEC_ARRAY parent
    // (`miliinternal.py:976-996`). All-or-nothing: a single
    // non-integer token makes the whole subscript a component lookup.
    let indices: Option<Vec<i64>> = toks.iter().map(|t| t.parse::<i64>().ok()).collect();
    match indices {
        Some(indices) => Ok(QueryName::Subscript { base, indices }),
        None => Ok(QueryName::CompSubscript { base, comps: toks }),
    }
}

/// Resolution of a parsed [`QueryName`] against the svar dictionary.
/// `base_name` is the svar name to look up in `subrec.svar_names`;
/// `picker` describes how to extract from each per-object slot.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Resolved {
    base_name: String,
    num_type: NumType,
    picker: AtomPicker,
}

fn resolve_target(
    svars: &SvarTable,
    name: QueryName<'_>,
    ips: Option<&[usize]>,
    int_points: &IntPoints,
) -> Result<Resolved> {
    match name {
        // CompSubscript is rewritten to Plain(comp) by the caller
        // (`plan_state_svar_ip`) before reaching here.
        QueryName::CompSubscript { .. } => {
            unreachable!("CompSubscript must be rewritten to Plain before resolve_target")
        }
        QueryName::Plain(n) => {
            let s = svars
                .get(n)
                .ok_or_else(|| MiliError::UnknownSvar(n.to_owned()))?;
            // Only a vec_array consumes `ips` positionally here. For
            // every other agg the `ips` semantics is the element-set
            // IP-*label* path resolved later against the VEC_ARRAY
            // parent (`plan_state_svar_ip` → `try_vec_array_substitution`);
            // the genuine-misuse typed error (svar carried directly,
            // no linkage) is raised there.
            let picker = if matches!(s.agg, SvarAgg::VecArray { .. }) {
                resolve_atom_picker(s, ips, int_points)?
            } else {
                AtomPicker::AllAtoms
            };
            Ok(Resolved {
                base_name: n.to_owned(),
                num_type: s.num_type,
                picker,
            })
        }
        QueryName::Subscript { base, indices } => {
            let s = svars
                .get(base)
                .ok_or_else(|| MiliError::UnknownSvar(base.to_owned()))?;
            // `ips` is silently ignored for an ARRAY subscript — upstream
            // only consumes `ips` for VEC_ARRAY. Verified vs the oracle:
            // `query("hx[1]","brick",ips=[1])` / `query("hx","brick",
            // ips=[1])` on the th/serial corpus both succeed with `ips`
            // ignored (identical to the no-`ips` result).
            let _ = ips;
            let dims = match &s.agg {
                SvarAgg::Array { dims } => dims.clone(),
                _ => {
                    return Err(MiliError::SubscriptNotApplicable {
                        svar: base.to_owned(),
                        agg: agg_label(&s.agg),
                    });
                }
            };
            let atom_idx = ravel_subscript(base, &indices, &dims)?;
            Ok(Resolved {
                base_name: base.to_owned(),
                num_type: s.num_type,
                picker: AtomPicker::Specific {
                    atom_indices: vec![atom_idx],
                },
            })
        }
    }
}

/// Convert a 1-based subscript `indices` for an `ARRAY` svar with
/// shape `dims` into a row-major 0-based atom index. Errors mirror
/// mili-python's behaviour:
/// - `len(indices) > rank` → invalid.
/// - `len(indices) < rank` → partial-dim slice, not yet supported (the
///   only fixture exercising arrays is 1-D, so we defer multi-dim
///   partials with a typed `Unsupported`).
/// - any index `< 1` or `> dims[i]` → out-of-range (1-based).
fn ravel_subscript(base: &str, indices: &[i64], dims: &[i32]) -> Result<usize> {
    if indices.len() > dims.len() {
        return Err(MiliError::InvalidSubscript {
            input: format_subscript(base, indices),
            reason: "too many indices for array svar",
        });
    }
    if indices.len() < dims.len() {
        return Err(MiliError::Unsupported(
            "partial-dim array subscript (only full-rank indexing supported)",
        ));
    }
    let mut atom_idx: usize = 0;
    for (i, &idx) in indices.iter().enumerate() {
        let dim = dims[i];
        if idx < 1 || idx > i64::from(dim) {
            return Err(MiliError::InvalidSubscript {
                input: format_subscript(base, indices),
                reason: "subscript index out of range (must be 1..=dim)",
            });
        }
        let mut stride: usize = 1;
        for &d in &dims[i + 1..] {
            if d < 0 {
                return Err(MiliError::MalformedDirectory("svar: negative dim"));
            }
            stride = stride
                .checked_mul(d as usize)
                .ok_or(MiliError::MalformedDirectory("svar: dim product overflow"))?;
        }
        let term = ((idx - 1) as usize)
            .checked_mul(stride)
            .ok_or(MiliError::MalformedDirectory("svar: index overflow"))?;
        atom_idx = atom_idx
            .checked_add(term)
            .ok_or(MiliError::MalformedDirectory("svar: index overflow"))?;
    }
    Ok(atom_idx)
}

fn format_subscript(base: &str, indices: &[i64]) -> String {
    let parts: Vec<String> = indices.iter().map(i64::to_string).collect();
    format!("{base}[{}]", parts.join(","))
}

/// Vector-parent resolution result for a bare component name.
struct VectorParent {
    name: String,
    /// Atom indices of the component within the parent's per-object slot.
    comp_atom_indices: Vec<usize>,
}

/// Every VECTOR parent svar that carries `name` as a component, in
/// svar-table order, each with the component's atom-index range inside
/// the parent's per-object slot. VEC_ARRAY parents are skipped — their
/// component data is striped across IP slots and is handled by the
/// separate vec-array substitution path. The caller disambiguates a
/// multi-parent component by subrecord membership for the queried
/// class (upstream resolves the component via the subrec that carries
/// its parent — `miliinternal.py:1222-1231`).
fn find_vector_parents(svars: &SvarTable, name: &str) -> Vec<VectorParent> {
    let mut found = Vec::new();
    for parent in svars.iter() {
        let SvarAgg::Vector { comps } = &parent.agg else {
            continue;
        };
        let Some(idx) = comps.iter().position(|c| c == name) else {
            continue;
        };
        let mut offset = 0usize;
        for c in &comps[..idx] {
            offset += svars.get(c).map_or(0, |sv| sv.atoms);
        }
        let count = svars.get(name).map_or(1, |sv| sv.atoms);
        let indices: Vec<usize> = (offset..offset + count).collect();
        found.push(VectorParent {
            name: parent.name.clone(),
            comp_atom_indices: indices,
        });
    }
    found
}

/// One subrecord that carries the queried `(svar, class)`. Holds the
/// metadata the gather pass needs without re-walking the srec.
struct SubrecMatch<'a> {
    sub: &'a Subrecord,
    /// Byte offset of this subrec inside the state's data block.
    subrec_start: u64,
    /// Index of `svar_name` in `sub.svar_names`.
    svar_idx: usize,
    /// `derive_lumps` over every svar in this subrec, in declaration
    /// order. `lumps.sizes[svar_idx]` is the per-object byte size of
    /// the target svar; `lumps.offsets[svar_idx]` is its in-object byte
    /// offset.
    lumps: Lumps,
    /// Object count in this subrec.
    n: usize,
    /// Per-subrecord picker override. `Some` only on the VEC_ARRAY
    /// substitution path (Slice B), where each carrying subrec needs its
    /// own component-outer/IP-inner atom-index list (the element-set
    /// payload — hence the label→index map — can differ per subrec). The
    /// shared `picker` argument is used when this is `None`.
    picker_override: Option<AtomPicker>,
}

fn collect_matching_subrecs<'a>(
    srec: &'a Srec,
    svars: &SvarTable,
    svar_name: &str,
    class_name: &str,
    state_data_start: u64,
    subrec: Option<&str>,
) -> Result<Vec<SubrecMatch<'a>>> {
    let mut running: u64 = state_data_start;
    let mut out = Vec::new();
    for sub in &srec.subrecords {
        let (atoms, widths) = atoms_and_widths(sub, svars)?;
        let lumps = derive_lumps(&atoms, &widths);
        let n = sub.object_count() as usize;
        let size = n
            .checked_mul(lumps.bytes_per_object())
            .ok_or(MiliError::MalformedDirectory("query: subrec size overflow"))?;
        let subrec_start = running;
        running = running
            .checked_add(size as u64)
            .ok_or(MiliError::MalformedDirectory(
                "query: subrec offset overflow",
            ))?;

        if sub.mclass != class_name {
            continue;
        }
        if let Some(want) = subrec {
            if sub.name != want {
                continue;
            }
        }
        let Some(svar_idx) = sub.svar_names.iter().position(|n| n == svar_name) else {
            continue;
        };
        out.push(SubrecMatch {
            sub,
            subrec_start,
            svar_idx,
            lumps,
            n,
            picker_override: None,
        });
    }
    Ok(out)
}

fn gather_all(
    matches: &[SubrecMatch<'_>],
    width: usize,
    picker: &AtomPicker,
) -> (Vec<ByteSlab>, Vec<i32>) {
    let mut slabs = Vec::new();
    let mut labels = Vec::new();
    for m in matches {
        let block_labels = expand_id_blocks(&m.sub.id_blocks);
        debug_assert_eq!(block_labels.len(), m.n);
        let pk = m.picker_override.as_ref().unwrap_or(picker);
        for (j, &label) in block_labels.iter().enumerate().take(m.n) {
            push_object_rows(&mut slabs, m, width, pk, j);
            labels.push(label);
        }
    }
    (slabs, labels)
}

/// Expand `[(start, stop), ...]` inclusive ranges into the full label
/// list in declaration order. Mirrors the entity-axis enumeration that
/// [`gather_all`] produces.
fn expand_id_blocks(blocks: &[(i32, i32)]) -> Vec<i32> {
    let total: i64 = blocks
        .iter()
        .map(|&(s, e)| (e as i64 - s as i64 + 1).max(0))
        .sum();
    let mut out = Vec::with_capacity(total.max(0) as usize);
    for &(s, e) in blocks {
        if e < s {
            continue;
        }
        for id in s..=e {
            out.push(id);
        }
    }
    out
}

fn gather_by_labels(
    matches: &[SubrecMatch<'_>],
    width: usize,
    picker: &AtomPicker,
    labels: &[i32],
    class_name: &str,
) -> Result<Vec<ByteSlab>> {
    let mut slabs = Vec::with_capacity(labels.len());
    for &label in labels {
        let mut found = false;
        for m in matches {
            if let Some(ord) = ordinal_in_subrec(&m.sub.id_blocks, label) {
                if ord >= m.n {
                    return Err(MiliError::MalformedDirectory(
                        "query: label ordinal exceeds subrec object count",
                    ));
                }
                let pk = m.picker_override.as_ref().unwrap_or(picker);
                push_object_rows(&mut slabs, m, width, pk, ord);
                found = true;
                break;
            }
        }
        if !found {
            return Err(MiliError::LabelNotFound {
                label,
                class: class_name.to_owned(),
            });
        }
    }
    Ok(slabs)
}

fn push_object_rows(
    slabs: &mut Vec<ByteSlab>,
    m: &SubrecMatch<'_>,
    width: usize,
    picker: &AtomPicker,
    ordinal: usize,
) {
    let s = m.svar_idx;
    let svar_size = m.lumps.sizes[s];
    let svar_off = m.lumps.offsets[s];
    let base = match m.sub.organization {
        Organization::ResultOrdered => {
            (m.subrec_start as usize) + m.n * svar_off + ordinal * svar_size
        }
        Organization::ObjectOrdered => {
            (m.subrec_start as usize) + ordinal * m.lumps.bytes_per_object() + svar_off
        }
    };
    match picker {
        AtomPicker::AllAtoms => slabs.push(ByteSlab {
            start: base,
            len: svar_size,
        }),
        AtomPicker::PerIp { atoms_per_ip, ips } => {
            let stride = *atoms_per_ip * width;
            for &ip in ips {
                slabs.push(ByteSlab {
                    start: base + ip * stride,
                    len: stride,
                });
            }
        }
        AtomPicker::Specific { atom_indices } => {
            for &a in atom_indices {
                slabs.push(ByteSlab {
                    start: base + a * width,
                    len: width,
                });
            }
        }
    }
}

/// Map a 1-based mili object id to its 0-based ordinal inside a
/// subrec's `id_blocks`. Returns `None` if no block covers `label`.
pub(crate) fn ordinal_in_subrec(id_blocks: &[(i32, i32)], label: i32) -> Option<usize> {
    let mut base: usize = 0;
    for &(start, stop) in id_blocks {
        if label >= start && label <= stop {
            return Some(base + (label - start) as usize);
        }
        base += (stop as i64 - start as i64 + 1).max(0) as usize;
    }
    None
}

/// Per-object atom selection for the gather pass.
#[derive(Debug, Clone, PartialEq, Eq)]
enum AtomPicker {
    /// Read every atom of the svar slot — one contiguous slab per object.
    AllAtoms,
    /// `vec_array` `ips` filter: emit one slab per selected IP, each
    /// `atoms_per_ip` atoms wide.
    PerIp {
        atoms_per_ip: usize,
        ips: Vec<usize>,
    },
    /// Pick a specific list of 0-based atom indices from the per-object
    /// slot — used for array subscript (`"hx[3]"`) and for the
    /// bare-component lookup that maps a VECTOR's component name onto
    /// the parent svar's atom range.
    Specific { atom_indices: Vec<usize> },
}

fn resolve_atom_picker(
    target: &Svar,
    ips: Option<&[usize]>,
    int_points: &IntPoints,
) -> Result<AtomPicker> {
    let Some(req) = ips else {
        return Ok(AtomPicker::AllAtoms);
    };
    let dims = match &target.agg {
        SvarAgg::VecArray { dims, .. } => dims.clone(),
        SvarAgg::Scalar | SvarAgg::Vector { .. } | SvarAgg::Array { .. } => {
            return Err(MiliError::IpFilterNotApplicable {
                svar: target.name.clone(),
                agg: agg_label(&target.agg),
            });
        }
    };
    // When the VEC_ARRAY is an element set queried by its own name,
    // upstream interprets `ips=` as integration-point *labels* and maps
    // each to its position via `.index(ip)` against the element-set
    // payload (`miliinternal.py:191,1251-1270`) — the same label
    // semantics as the bare-component substitution path
    // (`try_vec_array_substitution`). Without this, `ips=` would be a
    // 0-based positional index and `query("es_3c","shell",ips=[2])`
    // (label 2 of [1,2]) would wrongly raise IpOutOfRange.
    if let Some(parent) = int_points
        .parents_of(&target.name)
        .iter()
        .find(|p| p.es_svar == target.name)
    {
        let ip_labels = &parent.payload[..parent.payload.len().saturating_sub(1)];
        let n_ip = ip_labels.len();
        if n_ip == 0 || target.atoms == 0 || !target.atoms.is_multiple_of(n_ip) {
            return Err(MiliError::MalformedDirectory(
                "vec_array svar has zero atoms or zero dims",
            ));
        }
        let atoms_per_ip = target.atoms / n_ip;
        let mut positions = Vec::with_capacity(req.len());
        for &ip in req {
            let want =
                i32::try_from(ip).map_err(|_| MiliError::IpOutOfRange { ip, atoms: n_ip })?;
            let p = ip_labels
                .iter()
                .position(|&x| x == want)
                .ok_or(MiliError::IpOutOfRange { ip, atoms: n_ip })?;
            positions.push(p);
        }
        return Ok(AtomPicker::PerIp {
            atoms_per_ip,
            ips: positions,
        });
    }
    let n_ip = dims_product(&dims)?;
    if n_ip == 0 || target.atoms == 0 {
        return Err(MiliError::MalformedDirectory(
            "vec_array svar has zero atoms or zero dims",
        ));
    }
    let atoms_per_ip = target.atoms / n_ip;
    for &ip in req {
        if ip >= n_ip {
            return Err(MiliError::IpOutOfRange { ip, atoms: n_ip });
        }
    }
    Ok(AtomPicker::PerIp {
        atoms_per_ip,
        ips: req.to_vec(),
    })
}

fn agg_label(agg: &SvarAgg) -> &'static str {
    match agg {
        SvarAgg::Scalar => "scalar",
        SvarAgg::Vector { .. } => "vector",
        SvarAgg::Array { .. } => "array",
        SvarAgg::VecArray { .. } => "vec_array",
    }
}

fn dims_product(dims: &[i32]) -> Result<usize> {
    let mut acc: usize = 1;
    for &d in dims {
        if d < 0 {
            return Err(MiliError::MalformedDirectory("svar: negative dim"));
        }
        acc = acc
            .checked_mul(d as usize)
            .ok_or(MiliError::MalformedDirectory("svar: dim product overflow"))?;
    }
    Ok(acc)
}

pub(crate) fn atoms_and_widths(
    sub: &Subrecord,
    svars: &SvarTable,
) -> Result<(Vec<usize>, Vec<usize>)> {
    let k = sub.svar_names.len();
    let mut atoms = Vec::with_capacity(k);
    let mut widths = Vec::with_capacity(k);
    for name in &sub.svar_names {
        let s = svars
            .get(name)
            .ok_or_else(|| MiliError::UnknownSvar(name.clone()))?;
        atoms.push(s.atoms);
        widths.push(s.num_type.width());
    }
    Ok((atoms, widths))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::directory::{ByteRange, DirEntry, DirEntryType, Directory, NamePool};
    use crate::header::{Endianness, Header, PartitionScheme, PrecisionLimit};

    fn h() -> Header {
        Header {
            header_version: 3,
            dir_version: 3,
            endianness: Endianness::Little,
            precision_limit: PrecisionLimit::Double,
            suffix_width: 2,
            partition_scheme: PartitionScheme::StateCount,
        }
    }

    /// Test-only svar spec: `(name, num_type, atoms_per_object)`. The
    /// agg encoding matches the actual STATE_VAR_DICT parser: atoms==1
    /// is encoded as `Scalar` (agg=0), atoms>1 as `Array` (agg=2) with
    /// a single dim. Use [`make_vec_array_svars`] when a vec_array agg
    /// is needed (for IP-filter coverage).
    fn make_svars(specs: &[(&str, NumType, usize)]) -> SvarTable {
        let mut svar_ints: Vec<i32> = Vec::new();
        let mut chars: Vec<u8> = Vec::new();
        for (name, nt, atoms) in specs {
            let code = match nt {
                NumType::Float4 => 2,
                NumType::Float8 => 4,
                NumType::Int4 => 5,
                NumType::Int8 => 7,
            };
            if *atoms == 1 {
                svar_ints.push(0);
                svar_ints.push(code);
            } else {
                svar_ints.push(2);
                svar_ints.push(code);
                svar_ints.push(1);
                svar_ints.push(*atoms as i32);
            }
            chars.extend_from_slice(name.as_bytes());
            chars.push(0);
            chars.extend_from_slice(name.as_bytes());
            chars.push(0);
        }
        finalize_svar_table(&svar_ints, &chars)
    }

    /// Build a single-svar `SvarTable` carrying a vec_array svar with
    /// the given `dims` and component names. Components are scalar
    /// svars defined inline (the recursion path inside
    /// [`crate::svar::SvarTable::build`]).
    fn make_vec_array_svars(name: &str, nt: NumType, dims: &[i32], comps: &[&str]) -> SvarTable {
        let code = match nt {
            NumType::Float4 => 2,
            NumType::Float8 => 4,
            NumType::Int4 => 5,
            NumType::Int8 => 7,
        };
        let mut svar_ints: Vec<i32> = Vec::new();
        let mut chars: Vec<u8> = Vec::new();
        svar_ints.push(3);
        svar_ints.push(code);
        svar_ints.push(dims.len() as i32);
        for &d in dims {
            svar_ints.push(d);
        }
        svar_ints.push(comps.len() as i32);
        chars.extend_from_slice(name.as_bytes());
        chars.push(0);
        chars.extend_from_slice(name.as_bytes());
        chars.push(0);
        for c in comps {
            chars.extend_from_slice(c.as_bytes());
            chars.push(0);
        }
        for c in comps {
            svar_ints.push(0);
            svar_ints.push(code);
            chars.extend_from_slice(c.as_bytes());
            chars.push(0);
            chars.extend_from_slice(c.as_bytes());
            chars.push(0);
        }
        finalize_svar_table(&svar_ints, &chars)
    }

    fn finalize_svar_table(svar_ints: &[i32], chars: &[u8]) -> SvarTable {
        let qty_int_words = (svar_ints.len() as i32) + 2;
        let qty_char_bytes = chars.len() as i32;
        let mut full_ints = vec![qty_int_words, qty_char_bytes];
        full_ints.extend_from_slice(svar_ints);
        let int_bytes: Vec<u8> = full_ints.iter().flat_map(|w| w.to_le_bytes()).collect();
        let mut payload = Vec::new();
        payload.extend_from_slice(&int_bytes);
        payload.extend_from_slice(chars);

        let pool = NamePool::parse(b"dict\0", 1).unwrap();
        let dir = Directory {
            commit_count: 1,
            qty_states: 0,
            state_map: ByteRange { start: 0, end: 0 },
            entries: vec![DirEntry {
                entry_type: DirEntryType::StateVarDict,
                modifier1: qty_int_words as i64,
                modifier2: qty_char_bytes as i64,
                string_qty: 1,
                offset: 0,
                length: payload.len() as i64,
                name_start: 0,
                name_count: 1,
            }],
            names: pool,
        };
        SvarTable::build(&payload, &dir, h()).expect("svar table builds")
    }

    fn mk_subrec(
        mclass: &str,
        org: Organization,
        svars: &[&str],
        blocks: &[(i32, i32)],
    ) -> Subrecord {
        Subrecord {
            name: format!("{mclass}_sub"),
            mclass: mclass.to_owned(),
            organization: org,
            svar_names: svars.iter().map(|s| (*s).to_owned()).collect(),
            id_blocks: blocks.to_vec(),
        }
    }

    fn no_filter() -> Filter<'static> {
        Filter {
            labels: None,
            ips: None,
            subrec: None,
        }
    }

    // ---------------------------- ordinal mapping --------------------------

    #[test]
    fn ordinal_within_single_block_is_offset_from_start() {
        assert_eq!(ordinal_in_subrec(&[(10, 20)], 10), Some(0));
        assert_eq!(ordinal_in_subrec(&[(10, 20)], 15), Some(5));
        assert_eq!(ordinal_in_subrec(&[(10, 20)], 20), Some(10));
        assert_eq!(ordinal_in_subrec(&[(10, 20)], 9), None);
        assert_eq!(ordinal_in_subrec(&[(10, 20)], 21), None);
    }

    #[test]
    fn ordinal_across_multi_blocks_accumulates_prior_block_sizes() {
        // Three blocks: [1..3]=3, [10..12]=3, [100..101]=2.
        // Label 11 → 3 + (11-10) = 4. Label 100 → 6 + 0 = 6.
        let blocks = vec![(1, 3), (10, 12), (100, 101)];
        assert_eq!(ordinal_in_subrec(&blocks, 1), Some(0));
        assert_eq!(ordinal_in_subrec(&blocks, 3), Some(2));
        assert_eq!(ordinal_in_subrec(&blocks, 11), Some(4));
        assert_eq!(ordinal_in_subrec(&blocks, 100), Some(6));
        assert_eq!(ordinal_in_subrec(&blocks, 101), Some(7));
        assert_eq!(ordinal_in_subrec(&blocks, 5), None);
    }

    // ---------------------------- RESULT_ORDERED plans ---------------------

    #[test]
    fn plan_single_scalar_result_ordered_no_filter() {
        let svars = make_svars(&[("nodpos", NumType::Float4, 1)]);
        let srec = Srec {
            srec_id: 0,
            mesh_id: 0,
            srec_size: 0,
            subrecords: vec![mk_subrec(
                "node",
                Organization::ResultOrdered,
                &["nodpos"],
                &[(1, 10)],
            )],
        };
        let plan = plan_state_svar(&srec, &svars, "nodpos", "node", 100, no_filter()).unwrap();
        assert_eq!(plan.num_type, NumType::Float4);
        // No-filter gather emits one slab per object, length 4.
        assert_eq!(plan.slabs.len(), 10);
        assert_eq!(plan.slabs[0], ByteSlab { start: 100, len: 4 });
        assert_eq!(plan.slabs[9], ByteSlab { start: 136, len: 4 });
        assert_eq!(plan.total_bytes(), 40);
    }

    #[test]
    fn plan_walks_prior_subrec_for_offset() {
        // subrec 0: 5 hex objects, scalar f32 → 20 bytes
        // subrec 1: 10 nodes,      scalar f32 → 40 bytes, target
        let svars = make_svars(&[("svA", NumType::Float4, 1), ("svB", NumType::Float4, 1)]);
        let srec = Srec {
            srec_id: 0,
            mesh_id: 0,
            srec_size: 0,
            subrecords: vec![
                mk_subrec("brick", Organization::ResultOrdered, &["svA"], &[(1, 5)]),
                mk_subrec("node", Organization::ResultOrdered, &["svB"], &[(1, 10)]),
            ],
        };
        let plan = plan_state_svar(&srec, &svars, "svB", "node", 1000, no_filter()).unwrap();
        // 1000 + 20 = 1020 is the node-subrec start; per-object slabs.
        assert_eq!(plan.slabs.len(), 10);
        assert_eq!(
            plan.slabs[0],
            ByteSlab {
                start: 1020,
                len: 4
            }
        );
        assert_eq!(
            plan.slabs[9],
            ByteSlab {
                start: 1056,
                len: 4
            }
        );
    }

    #[test]
    fn plan_picks_second_svar_in_subrec() {
        // subrec has [svA scalar f32, svB scalar f32] over 4 objects.
        // svB slab base: subrec_start + N * lump_offsets[1] = 0 + 4*4 = 16.
        let svars = make_svars(&[("svA", NumType::Float4, 1), ("svB", NumType::Float4, 1)]);
        let srec = Srec {
            srec_id: 0,
            mesh_id: 0,
            srec_size: 0,
            subrecords: vec![mk_subrec(
                "node",
                Organization::ResultOrdered,
                &["svA", "svB"],
                &[(1, 4)],
            )],
        };
        let plan = plan_state_svar(&srec, &svars, "svB", "node", 0, no_filter()).unwrap();
        assert_eq!(plan.slabs.len(), 4);
        assert_eq!(plan.slabs[0], ByteSlab { start: 16, len: 4 });
        assert_eq!(plan.slabs[3], ByteSlab { start: 28, len: 4 });
    }

    #[test]
    fn plan_handles_vector_atoms_no_filter() {
        let svars = make_svars(&[("nodpos", NumType::Float4, 3)]);
        let srec = Srec {
            srec_id: 0,
            mesh_id: 0,
            srec_size: 0,
            subrecords: vec![mk_subrec(
                "node",
                Organization::ResultOrdered,
                &["nodpos"],
                &[(1, 4)],
            )],
        };
        let plan = plan_state_svar(&srec, &svars, "nodpos", "node", 0, no_filter()).unwrap();
        assert_eq!(plan.slabs.len(), 4);
        assert_eq!(plan.slabs[0], ByteSlab { start: 0, len: 12 });
        assert_eq!(plan.slabs[3], ByteSlab { start: 36, len: 12 });
    }

    #[test]
    fn plan_concatenates_across_matching_subrecs() {
        let svars = make_svars(&[("svA", NumType::Float4, 1)]);
        let srec = Srec {
            srec_id: 0,
            mesh_id: 0,
            srec_size: 0,
            subrecords: vec![
                mk_subrec("node", Organization::ResultOrdered, &["svA"], &[(1, 3)]),
                mk_subrec("node", Organization::ResultOrdered, &["svA"], &[(100, 102)]),
            ],
        };
        let plan = plan_state_svar(&srec, &svars, "svA", "node", 0, no_filter()).unwrap();
        assert_eq!(plan.slabs.len(), 6);
        assert_eq!(plan.slabs[0], ByteSlab { start: 0, len: 4 });
        assert_eq!(plan.slabs[5], ByteSlab { start: 20, len: 4 });
    }

    #[test]
    fn plan_subrec_filter_restricts_to_named_subrecord() {
        // Two `node` subrecs both carrying svA; `subrec=` selects one.
        let svars = make_svars(&[("svA", NumType::Float4, 1)]);
        let mut first = mk_subrec("node", Organization::ResultOrdered, &["svA"], &[(1, 3)]);
        first.name = "first_rec".to_owned();
        let mut second = mk_subrec("node", Organization::ResultOrdered, &["svA"], &[(100, 102)]);
        second.name = "second_rec".to_owned();
        let srec = Srec {
            srec_id: 0,
            mesh_id: 0,
            srec_size: 0,
            subrecords: vec![first, second],
        };
        // No filter → both subrecs (6 objects).
        let all = plan_state_svar(&srec, &svars, "svA", "node", 0, no_filter()).unwrap();
        assert_eq!(all.slabs.len(), 6);
        // subrec="second_rec" → only that subrec's 3 objects, and the
        // labels come from its id_block [100..102] (offset accounts
        // for the skipped first subrec's bytes).
        let f = Filter {
            labels: None,
            ips: None,
            subrec: Some("second_rec"),
        };
        let only = plan_state_svar(&srec, &svars, "svA", "node", 0, f).unwrap();
        assert_eq!(only.slabs.len(), 3);
        assert_eq!(only.labels, vec![100, 101, 102]);
        assert_eq!(only.slabs[0], ByteSlab { start: 12, len: 4 });
        // A name that matches no subrec → no-match error (mirrors
        // upstream "No subrecords found").
        let none = Filter {
            labels: None,
            ips: None,
            subrec: Some("nope"),
        };
        assert!(plan_state_svar(&srec, &svars, "svA", "node", 0, none).is_err());
    }

    #[test]
    fn plan_label_filter_emits_only_requested_labels_in_argument_order() {
        // 4 objects, RO scalar f32. Request labels [3,1] → ordinals
        // [2,0], slabs in that order.
        let svars = make_svars(&[("svA", NumType::Float4, 1)]);
        let srec = Srec {
            srec_id: 0,
            mesh_id: 0,
            srec_size: 0,
            subrecords: vec![mk_subrec(
                "node",
                Organization::ResultOrdered,
                &["svA"],
                &[(1, 4)],
            )],
        };
        let f = Filter {
            labels: Some(&[3, 1]),
            ips: None,
            subrec: None,
        };
        let plan = plan_state_svar(&srec, &svars, "svA", "node", 0, f).unwrap();
        assert_eq!(
            plan.slabs,
            vec![ByteSlab { start: 8, len: 4 }, ByteSlab { start: 0, len: 4 },]
        );
    }

    #[test]
    fn plan_label_filter_routes_across_multi_block_subrec() {
        // Single subrec with two blocks [1..3] + [10..12]. Labels
        // [11, 2] → ordinals [4, 1].
        let svars = make_svars(&[("svA", NumType::Float4, 1)]);
        let srec = Srec {
            srec_id: 0,
            mesh_id: 0,
            srec_size: 0,
            subrecords: vec![mk_subrec(
                "node",
                Organization::ResultOrdered,
                &["svA"],
                &[(1, 3), (10, 12)],
            )],
        };
        let f = Filter {
            labels: Some(&[11, 2]),
            ips: None,
            subrec: None,
        };
        let plan = plan_state_svar(&srec, &svars, "svA", "node", 0, f).unwrap();
        // N=6, lump_size=4. ord=4 → 16; ord=1 → 4.
        assert_eq!(
            plan.slabs,
            vec![
                ByteSlab { start: 16, len: 4 },
                ByteSlab { start: 4, len: 4 },
            ]
        );
    }

    #[test]
    fn plan_label_filter_errors_when_label_absent() {
        let svars = make_svars(&[("svA", NumType::Float4, 1)]);
        let srec = Srec {
            srec_id: 0,
            mesh_id: 0,
            srec_size: 0,
            subrecords: vec![mk_subrec(
                "node",
                Organization::ResultOrdered,
                &["svA"],
                &[(1, 3)],
            )],
        };
        let f = Filter {
            labels: Some(&[5]),
            ips: None,
            subrec: None,
        };
        let err = plan_state_svar(&srec, &svars, "svA", "node", 0, f).unwrap_err();
        assert!(matches!(err, MiliError::LabelNotFound { label: 5, .. }));
    }

    // ---------------------------- OBJECT_ORDERED plans ---------------------

    #[test]
    fn plan_object_ordered_scalar_no_filter() {
        // OO subrec with one scalar f32, 3 objects. Object j starts at
        // subrec_start + j * 4; svar offset within object is 0.
        let svars = make_svars(&[("svA", NumType::Float4, 1)]);
        let srec = Srec {
            srec_id: 0,
            mesh_id: 0,
            srec_size: 0,
            subrecords: vec![mk_subrec(
                "brick",
                Organization::ObjectOrdered,
                &["svA"],
                &[(1, 3)],
            )],
        };
        let plan = plan_state_svar(&srec, &svars, "svA", "brick", 100, no_filter()).unwrap();
        assert_eq!(
            plan.slabs,
            vec![
                ByteSlab { start: 100, len: 4 },
                ByteSlab { start: 104, len: 4 },
                ByteSlab { start: 108, len: 4 },
            ]
        );
    }

    #[test]
    fn plan_object_ordered_mixed_widths_walks_per_svar_lumps() {
        // OO subrec carrying [stress(6 f32 = 24B), eps(1 f32 = 4B),
        // flag(1 i32 = 4B)] over N=5 objects. Per-object stride is 32.
        // Pulling `eps` should yield slabs at base + j*32 + 24, length 4.
        let svars = make_svars(&[
            ("stress", NumType::Float4, 6),
            ("eps", NumType::Float4, 1),
            ("flag", NumType::Int4, 1),
        ]);
        let srec = Srec {
            srec_id: 0,
            mesh_id: 0,
            srec_size: 0,
            subrecords: vec![mk_subrec(
                "shell",
                Organization::ObjectOrdered,
                &["stress", "eps", "flag"],
                &[(1, 5)],
            )],
        };
        let plan = plan_state_svar(&srec, &svars, "eps", "shell", 0, no_filter()).unwrap();
        assert_eq!(plan.slabs.len(), 5);
        for (j, slab) in plan.slabs.iter().enumerate() {
            assert_eq!(slab.start, j * 32 + 24);
            assert_eq!(slab.len, 4);
        }

        // And the i32 flag — offset 28 in each object.
        let plan2 = plan_state_svar(&srec, &svars, "flag", "shell", 0, no_filter()).unwrap();
        assert_eq!(plan2.num_type, NumType::Int4);
        for (j, slab) in plan2.slabs.iter().enumerate() {
            assert_eq!(slab.start, j * 32 + 28);
            assert_eq!(slab.len, 4);
        }
    }

    #[test]
    fn plan_object_ordered_label_filter_uses_ordinal() {
        // Same OO shell subrec as above; request labels [4, 1] → ordinals
        // [3, 0]. Slabs follow the request order.
        let svars = make_svars(&[("stress", NumType::Float4, 6), ("eps", NumType::Float4, 1)]);
        let srec = Srec {
            srec_id: 0,
            mesh_id: 0,
            srec_size: 0,
            subrecords: vec![mk_subrec(
                "shell",
                Organization::ObjectOrdered,
                &["stress", "eps"],
                &[(1, 5)],
            )],
        };
        let f = Filter {
            labels: Some(&[4, 1]),
            ips: None,
            subrec: None,
        };
        let plan = plan_state_svar(&srec, &svars, "eps", "shell", 0, f).unwrap();
        assert_eq!(
            plan.slabs,
            vec![
                ByteSlab {
                    start: 3 * 28 + 24,
                    len: 4
                },
                ByteSlab { start: 24, len: 4 },
            ]
        );
    }

    // ---------------------------- vec_array IP filter ----------------------

    #[test]
    fn plan_vec_array_ip_filter_picks_per_ip_slab_per_object() {
        // vec_array stress with dims=[3] (3 IPs) and 6 f32 components.
        // atoms = 18, per-object byte size = 72. RO over 2 objects:
        // svar slab starts at 0, length 144.
        // Requesting ips=[0,2] should emit, per object, slabs at
        //   base + 0 * (6*4) = base
        //   base + 2 * (6*4) = base + 48
        // ip stride = 6*4 = 24, len 24.
        let svars = make_vec_array_svars(
            "stress",
            NumType::Float4,
            &[3],
            &["sx", "sy", "sz", "sxy", "syz", "szx"],
        );
        let srec = Srec {
            srec_id: 0,
            mesh_id: 0,
            srec_size: 0,
            subrecords: vec![mk_subrec(
                "shell",
                Organization::ResultOrdered,
                &["stress"],
                &[(1, 2)],
            )],
        };
        let f = Filter {
            labels: None,
            ips: Some(&[0, 2]),
            subrec: None,
        };
        let plan = plan_state_svar(&srec, &svars, "stress", "shell", 0, f).unwrap();
        // obj 0 base = 0; obj 1 base = 72.
        assert_eq!(
            plan.slabs,
            vec![
                ByteSlab { start: 0, len: 24 },
                ByteSlab { start: 48, len: 24 },
                ByteSlab { start: 72, len: 24 },
                ByteSlab {
                    start: 120,
                    len: 24
                },
            ]
        );
    }

    #[test]
    fn plan_ip_filter_ignored_for_non_vec_array_svar() {
        // Upstream silently ignores `ips` for a non-VEC_ARRAY svar — the
        // result is identical to the no-`ips` query (cross-validated vs
        // the `_MiliInternal` oracle on the serial corpus). The earlier
        // `IpFilterNotApplicable` here was stricter than upstream with
        // no oracle basis.
        let svars = make_svars(&[("svA", NumType::Float4, 1)]);
        let srec = Srec {
            srec_id: 0,
            mesh_id: 0,
            srec_size: 0,
            subrecords: vec![mk_subrec(
                "node",
                Organization::ResultOrdered,
                &["svA"],
                &[(1, 3)],
            )],
        };
        let f = Filter {
            labels: None,
            ips: Some(&[0]),
            subrec: None,
        };
        let with_ips = plan_state_svar(&srec, &svars, "svA", "node", 0, f).unwrap();
        let no_ips = plan_state_svar(&srec, &svars, "svA", "node", 0, no_filter()).unwrap();
        assert_eq!(with_ips, no_ips);
    }

    #[test]
    fn plan_ip_filter_rejects_out_of_range_index() {
        let svars = make_vec_array_svars("stress", NumType::Float4, &[2], &["sx", "sy"]);
        let srec = Srec {
            srec_id: 0,
            mesh_id: 0,
            srec_size: 0,
            subrecords: vec![mk_subrec(
                "shell",
                Organization::ResultOrdered,
                &["stress"],
                &[(1, 2)],
            )],
        };
        let f = Filter {
            labels: None,
            ips: Some(&[5]),
            subrec: None,
        };
        let err = plan_state_svar(&srec, &svars, "stress", "shell", 0, f).unwrap_err();
        assert!(matches!(err, MiliError::IpOutOfRange { ip: 5, .. }));
    }

    // ---------------------------- error paths ------------------------------

    #[test]
    fn plan_errors_when_no_subrec_matches() {
        let svars = make_svars(&[("svA", NumType::Float4, 1)]);
        let srec = Srec {
            srec_id: 0,
            mesh_id: 0,
            srec_size: 0,
            subrecords: vec![mk_subrec(
                "brick",
                Organization::ResultOrdered,
                &["svA"],
                &[(1, 3)],
            )],
        };
        let err = plan_state_svar(&srec, &svars, "svA", "node", 0, no_filter()).unwrap_err();
        assert!(matches!(err, MiliError::NoMatchingSubrec { .. }));
    }

    #[test]
    fn plan_errors_on_unknown_svar() {
        let svars = make_svars(&[("svA", NumType::Float4, 1)]);
        let srec = Srec {
            srec_id: 0,
            mesh_id: 0,
            srec_size: 0,
            subrecords: Vec::new(),
        };
        let err = plan_state_svar(&srec, &svars, "missing", "node", 0, no_filter()).unwrap_err();
        assert!(matches!(err, MiliError::UnknownSvar(_)));
    }

    // ---------------------------- rebased ----------------------------------

    // ---------------------------- query-name parser ------------------------

    #[test]
    fn parse_query_name_plain_passes_through() {
        let q = parse_query_name("hx").unwrap();
        assert_eq!(q, QueryName::Plain("hx"));
    }

    #[test]
    fn parse_query_name_single_subscript() {
        let q = parse_query_name("hx[3]").unwrap();
        assert_eq!(
            q,
            QueryName::Subscript {
                base: "hx",
                indices: vec![3]
            }
        );
    }

    #[test]
    fn parse_query_name_multi_subscript_with_whitespace() {
        // Whitespace inside the bracket should be tolerated.
        let q = parse_query_name("hx[1, 2 ,3]").unwrap();
        assert_eq!(
            q,
            QueryName::Subscript {
                base: "hx",
                indices: vec![1, 2, 3]
            }
        );
    }

    #[test]
    fn parse_query_name_keeps_negative_and_zero_indices_for_later_validation() {
        // Parsing accepts any signed integer literal; range-checking
        // happens in `resolve_target`. Matches mili-python.
        let q = parse_query_name("hx[-2]").unwrap();
        assert_eq!(
            q,
            QueryName::Subscript {
                base: "hx",
                indices: vec![-2]
            }
        );
        let q = parse_query_name("hx[0]").unwrap();
        assert_eq!(
            q,
            QueryName::Subscript {
                base: "hx",
                indices: vec![0]
            }
        );
    }

    #[test]
    fn parse_query_name_rejects_unbalanced_bracket() {
        assert!(matches!(
            parse_query_name("hx[3").unwrap_err(),
            MiliError::InvalidSubscript { .. }
        ));
    }

    #[test]
    fn parse_query_name_rejects_empty_index() {
        assert!(matches!(
            parse_query_name("hx[]").unwrap_err(),
            MiliError::InvalidSubscript { .. }
        ));
        assert!(matches!(
            parse_query_name("hx[1,]").unwrap_err(),
            MiliError::InvalidSubscript { .. }
        ));
    }

    #[test]
    fn parse_query_name_non_integer_index_is_component_subscript() {
        // Non-integer bracket content is a named-component lookup of a
        // VECTOR / VEC_ARRAY parent (`nodpos[ux]`, `stress[sy]`), not
        // an error (`miliinternal.py:976-996`).
        assert_eq!(
            parse_query_name("nodpos[ux]").unwrap(),
            QueryName::CompSubscript {
                base: "nodpos",
                comps: vec!["ux"],
            }
        );
        assert_eq!(
            parse_query_name("stress[ sy ]").unwrap(),
            QueryName::CompSubscript {
                base: "stress",
                comps: vec!["sy"],
            }
        );
        // All-integer stays an ARRAY subscript.
        assert!(matches!(
            parse_query_name("hx[3]").unwrap(),
            QueryName::Subscript { .. }
        ));
    }

    #[test]
    fn parse_query_name_rejects_missing_base() {
        assert!(matches!(
            parse_query_name("[3]").unwrap_err(),
            MiliError::InvalidSubscript { .. }
        ));
    }

    // ---------------------------- array subscript ravel --------------------

    #[test]
    fn ravel_subscript_one_d_is_index_minus_one() {
        assert_eq!(ravel_subscript("hx", &[1], &[8]).unwrap(), 0);
        assert_eq!(ravel_subscript("hx", &[3], &[8]).unwrap(), 2);
        assert_eq!(ravel_subscript("hx", &[8], &[8]).unwrap(), 7);
    }

    #[test]
    fn ravel_subscript_two_d_is_row_major() {
        // dims=[3,4]; subscript [2,3] (1-based) → 0-based [1,2] → 1*4+2=6.
        assert_eq!(ravel_subscript("g", &[2, 3], &[3, 4]).unwrap(), 6);
        assert_eq!(ravel_subscript("g", &[1, 1], &[3, 4]).unwrap(), 0);
        assert_eq!(ravel_subscript("g", &[3, 4], &[3, 4]).unwrap(), 11);
    }

    #[test]
    fn ravel_subscript_rejects_out_of_range() {
        let err = ravel_subscript("hx", &[0], &[8]).unwrap_err();
        assert!(matches!(err, MiliError::InvalidSubscript { .. }));
        let err = ravel_subscript("hx", &[9], &[8]).unwrap_err();
        assert!(matches!(err, MiliError::InvalidSubscript { .. }));
        let err = ravel_subscript("hx", &[-2], &[8]).unwrap_err();
        assert!(matches!(err, MiliError::InvalidSubscript { .. }));
    }

    #[test]
    fn ravel_subscript_rejects_extra_indices() {
        let err = ravel_subscript("hx", &[1, 1], &[8]).unwrap_err();
        assert!(matches!(err, MiliError::InvalidSubscript { .. }));
    }

    #[test]
    fn ravel_subscript_defers_partial_dim_with_typed_error() {
        // dims=[3,4], only one index given → partial-dim slice (not yet
        // implemented; surface a typed Unsupported rather than silently
        // raveling as if rank-1).
        let err = ravel_subscript("g", &[1], &[3, 4]).unwrap_err();
        assert!(matches!(err, MiliError::Unsupported(_)));
    }

    // ---------------------------- subscript plan ---------------------------

    #[test]
    fn plan_array_subscript_picks_atom_from_each_object() {
        // hx as an ARRAY svar with dims=[8] of f32, in a RO subrec over
        // 5 objects. `hx[3]` should emit per-object 4-byte slabs at
        // svar_base + 2*4 = svar_base + 8.
        let svars = make_svars(&[("hx", NumType::Float4, 8)]);
        let srec = Srec {
            srec_id: 0,
            mesh_id: 0,
            srec_size: 0,
            subrecords: vec![mk_subrec(
                "brick",
                Organization::ResultOrdered,
                &["hx"],
                &[(1, 5)],
            )],
        };
        let plan = plan_state_svar(&srec, &svars, "hx[3]", "brick", 0, no_filter()).unwrap();
        assert_eq!(plan.num_type, NumType::Float4);
        // RO over N=5, svar_size = 32; svar_off = 0; per-obj base = j*32.
        // Atom 2 of each → base + 8, len 4.
        assert_eq!(plan.slabs.len(), 5);
        for (j, slab) in plan.slabs.iter().enumerate() {
            assert_eq!(slab.start, j * 32 + 8);
            assert_eq!(slab.len, 4);
        }
    }

    #[test]
    fn plan_array_subscript_in_object_ordered_subrec() {
        // Same hx[8] f32 array, but in an OO subrec — per-object stride
        // is the svar slot itself = 32 bytes. `hx[3]` slabs at j*32 + 8.
        let svars = make_svars(&[("hx", NumType::Float4, 8)]);
        let srec = Srec {
            srec_id: 0,
            mesh_id: 0,
            srec_size: 0,
            subrecords: vec![mk_subrec(
                "brick",
                Organization::ObjectOrdered,
                &["hx"],
                &[(1, 3)],
            )],
        };
        let plan = plan_state_svar(&srec, &svars, "hx[3]", "brick", 100, no_filter()).unwrap();
        assert_eq!(
            plan.slabs,
            vec![
                ByteSlab { start: 108, len: 4 },
                ByteSlab { start: 140, len: 4 },
                ByteSlab { start: 172, len: 4 },
            ]
        );
    }

    #[test]
    fn plan_array_subscript_composes_with_label_filter() {
        let svars = make_svars(&[("hx", NumType::Float4, 8)]);
        let srec = Srec {
            srec_id: 0,
            mesh_id: 0,
            srec_size: 0,
            subrecords: vec![mk_subrec(
                "brick",
                Organization::ResultOrdered,
                &["hx"],
                &[(1, 4)],
            )],
        };
        // labels [3, 1] → ordinals [2, 0]. Each emits one 4-byte slab
        // for hx[3] at svar_base + 2*4.
        let f = Filter {
            labels: Some(&[3, 1]),
            ips: None,
            subrec: None,
        };
        let plan = plan_state_svar(&srec, &svars, "hx[3]", "brick", 0, f).unwrap();
        // N=4, svar_size=32 → per-obj base = ord*32; atom2 byte off = 8.
        assert_eq!(
            plan.slabs,
            vec![
                ByteSlab { start: 72, len: 4 },
                ByteSlab { start: 8, len: 4 },
            ]
        );
    }

    #[test]
    fn plan_array_subscript_on_scalar_errors() {
        let svars = make_svars(&[("svA", NumType::Float4, 1)]);
        let srec = Srec {
            srec_id: 0,
            mesh_id: 0,
            srec_size: 0,
            subrecords: vec![mk_subrec(
                "node",
                Organization::ResultOrdered,
                &["svA"],
                &[(1, 4)],
            )],
        };
        let err = plan_state_svar(&srec, &svars, "svA[1]", "node", 0, no_filter()).unwrap_err();
        assert!(matches!(err, MiliError::SubscriptNotApplicable { .. }));
    }

    #[test]
    fn plan_bare_component_of_vec_array_substitutes_with_ip_labels() {
        // Slice B: `eps` is not a subrec svar — it is a scalar
        // component of the VEC_ARRAY `es_1a` (dims=[2], comps=[sx,eps],
        // 2 atoms/IP, 2 IPs). With the int_points linkage carrying the
        // element-set payload `[1, 2, 2]` (IP labels 1,2 + trailing
        // count), `ips=[2]` maps label 2 → positional IP index 1; eps
        // is leaf-offset 1, so atom = 1*2 + 1 = 3. Component name is
        // `eps ipt. 2` (`miliinternal.py:1367`).
        let svars = make_vec_array_svars("es_1a", NumType::Float4, &[2], &["sx", "eps"]);
        let srec = Srec {
            srec_id: 0,
            mesh_id: 0,
            srec_size: 0,
            subrecords: vec![mk_subrec(
                "shell",
                Organization::ResultOrdered,
                &["es_1a"],
                &[(1, 1)],
            )],
        };
        let mut ip = IntPoints::default();
        ip.insert("sx", "es_1a", &[1, 2, 2]);
        ip.insert("eps", "es_1a", &[1, 2, 2]);

        let labels = [1];
        let ips = [2usize];
        let f = Filter {
            labels: Some(&labels),
            ips: Some(&ips),
            subrec: None,
        };
        let plan = plan_state_svar_ip(&srec, &svars, "eps", "shell", 0, f, &ip).unwrap();
        assert_eq!(plan.slabs, vec![ByteSlab { start: 12, len: 4 }]);
        assert_eq!(
            plan.components.as_deref(),
            Some(["eps ipt. 2".to_owned()].as_slice())
        );

        // No `ips` → every IP, component-outer/IP-inner naming.
        let f_all = Filter {
            labels: Some(&labels),
            ips: None,
            subrec: None,
        };
        let plan = plan_state_svar_ip(&srec, &svars, "eps", "shell", 0, f_all, &ip).unwrap();
        assert_eq!(
            plan.slabs,
            vec![
                ByteSlab { start: 4, len: 4 },
                ByteSlab { start: 12, len: 4 },
            ]
        );
        assert_eq!(
            plan.components.as_deref(),
            Some(["eps ipt. 1".to_owned(), "eps ipt. 2".to_owned()].as_slice())
        );
    }

    #[test]
    fn plan_array_subscript_on_vec_array_errors() {
        let svars = make_vec_array_svars("stress", NumType::Float4, &[2], &["sx", "sy"]);
        let srec = Srec {
            srec_id: 0,
            mesh_id: 0,
            srec_size: 0,
            subrecords: vec![mk_subrec(
                "shell",
                Organization::ResultOrdered,
                &["stress"],
                &[(1, 2)],
            )],
        };
        let err = plan_state_svar(&srec, &svars, "stress[1]", "shell", 0, no_filter()).unwrap_err();
        assert!(matches!(err, MiliError::SubscriptNotApplicable { .. }));
    }

    #[test]
    fn plan_array_subscript_rejects_out_of_range_index() {
        let svars = make_svars(&[("hx", NumType::Float4, 8)]);
        let srec = Srec {
            srec_id: 0,
            mesh_id: 0,
            srec_size: 0,
            subrecords: vec![mk_subrec(
                "brick",
                Organization::ResultOrdered,
                &["hx"],
                &[(1, 4)],
            )],
        };
        for bad in ["hx[0]", "hx[9]", "hx[-2]"] {
            let err = plan_state_svar(&srec, &svars, bad, "brick", 0, no_filter()).unwrap_err();
            assert!(
                matches!(err, MiliError::InvalidSubscript { .. }),
                "expected InvalidSubscript for {bad}, got {err:?}"
            );
        }
    }

    #[test]
    fn plan_array_subscript_rejects_too_many_indices() {
        let svars = make_svars(&[("hx", NumType::Float4, 8)]);
        let srec = Srec {
            srec_id: 0,
            mesh_id: 0,
            srec_size: 0,
            subrecords: vec![mk_subrec(
                "brick",
                Organization::ResultOrdered,
                &["hx"],
                &[(1, 4)],
            )],
        };
        let err = plan_state_svar(&srec, &svars, "hx[1,1]", "brick", 0, no_filter()).unwrap_err();
        assert!(matches!(err, MiliError::InvalidSubscript { .. }));
    }

    #[test]
    fn plan_array_subscript_with_ips_filter_is_ignored() {
        // `ips` is silently ignored for an ARRAY subscript too —
        // upstream only consumes `ips` for VEC_ARRAY. Verified vs the
        // oracle on the th/serial corpus (`hx[1]`/`hx`+`ips`).
        let svars = make_svars(&[("hx", NumType::Float4, 8)]);
        let srec = Srec {
            srec_id: 0,
            mesh_id: 0,
            srec_size: 0,
            subrecords: vec![mk_subrec(
                "brick",
                Organization::ResultOrdered,
                &["hx"],
                &[(1, 4)],
            )],
        };
        let f = Filter {
            labels: None,
            ips: Some(&[0]),
            subrec: None,
        };
        let with_ips = plan_state_svar(&srec, &svars, "hx[1]", "brick", 0, f).unwrap();
        let no_ips = plan_state_svar(&srec, &svars, "hx[1]", "brick", 0, no_filter()).unwrap();
        assert_eq!(with_ips, no_ips);
    }

    // ---------------------------- bare component lookup --------------------

    /// Build a `SvarTable` with a single VECTOR svar `parent` of f32,
    /// whose components are inline scalar f32s named per `comps`.
    fn make_vector_svars(parent: &str, nt: NumType, comps: &[&str]) -> SvarTable {
        let code = match nt {
            NumType::Float4 => 2,
            NumType::Float8 => 4,
            NumType::Int4 => 5,
            NumType::Int8 => 7,
        };
        let mut svar_ints: Vec<i32> = Vec::new();
        let mut chars: Vec<u8> = Vec::new();
        svar_ints.push(1);
        svar_ints.push(code);
        svar_ints.push(comps.len() as i32);
        chars.extend_from_slice(parent.as_bytes());
        chars.push(0);
        chars.extend_from_slice(parent.as_bytes());
        chars.push(0);
        for c in comps {
            chars.extend_from_slice(c.as_bytes());
            chars.push(0);
        }
        for c in comps {
            svar_ints.push(0);
            svar_ints.push(code);
            chars.extend_from_slice(c.as_bytes());
            chars.push(0);
            chars.extend_from_slice(c.as_bytes());
            chars.push(0);
        }
        finalize_svar_table(&svar_ints, &chars)
    }

    #[test]
    fn plan_bare_component_resolves_to_parent_vector_atom_range() {
        // VECTOR `stress` with comps [sx, sy, sz] in a RO subrec over
        // 4 objects. Querying "sy" should resolve to the parent and
        // pick the second atom (atom_idx = 1) from each object.
        let svars = make_vector_svars("stress", NumType::Float4, &["sx", "sy", "sz"]);
        let srec = Srec {
            srec_id: 0,
            mesh_id: 0,
            srec_size: 0,
            subrecords: vec![mk_subrec(
                "brick",
                Organization::ResultOrdered,
                &["stress"],
                &[(1, 4)],
            )],
        };
        let plan = plan_state_svar(&srec, &svars, "sy", "brick", 0, no_filter()).unwrap();
        // N=4, lump_size=12 (3 atoms * 4); per-obj base = j*12. sy → atom 1 → +4.
        assert_eq!(plan.slabs.len(), 4);
        for (j, slab) in plan.slabs.iter().enumerate() {
            assert_eq!(slab.start, j * 12 + 4);
            assert_eq!(slab.len, 4);
        }
    }

    #[test]
    fn plan_bare_component_prefers_direct_match_when_subrec_carries_it() {
        // sx exists both as a top-level scalar svar (parsed via vector
        // recursion) AND in a subrec that holds it directly. The direct
        // match wins — the gather is one slab per object of svar_size.
        let svars = make_vector_svars("stress", NumType::Float4, &["sx", "sy"]);
        let srec = Srec {
            srec_id: 0,
            mesh_id: 0,
            srec_size: 0,
            subrecords: vec![mk_subrec(
                "brick",
                Organization::ResultOrdered,
                &["sx"],
                &[(1, 3)],
            )],
        };
        let plan = plan_state_svar(&srec, &svars, "sx", "brick", 0, no_filter()).unwrap();
        assert_eq!(plan.slabs.len(), 3);
        for (j, slab) in plan.slabs.iter().enumerate() {
            assert_eq!(slab.start, j * 4);
            assert_eq!(slab.len, 4);
        }
    }

    #[test]
    fn rebased_shifts_all_slabs_by_delta() {
        let plan = ReadPlan {
            num_type: NumType::Float4,
            slabs: vec![
                ByteSlab { start: 100, len: 4 },
                ByteSlab { start: 200, len: 4 },
            ],
            state_data_start: 100,
            labels: vec![1, 2],
            components: None,
        };
        let new = plan.rebased(500).unwrap();
        assert_eq!(new.state_data_start, 500);
        assert_eq!(
            new.slabs,
            vec![
                ByteSlab { start: 500, len: 4 },
                ByteSlab { start: 600, len: 4 },
            ]
        );
    }

    // ---------------------------- inconsistent IP counts -------------------

    // The cross-subrec IP-count guard fires when `matches.len() > 1`
    // and the matching subrecs disagree on `lumps.sizes[svar_idx]`.
    // Today the only way that happens is via element-set substitution
    // (Step 16 item 5), which isn't wired yet — so the guard is
    // dormant in production but the typed error variant is in place
    // as the user-facing contract. This test pins the error shape so
    // future work can rely on it.
    #[test]
    fn inconsistent_ip_counts_error_is_typed_and_informative() {
        let err = MiliError::InconsistentIpCounts {
            svar: "sx".to_owned(),
            class: "brick".to_owned(),
            counts: vec![8 * 4, 9 * 4],
        };
        let msg = err.to_string();
        assert!(msg.contains("inconsistent integration-point counts"));
        assert!(msg.contains("\"sx\""));
        assert!(msg.contains("\"brick\""));
    }
}
