//! State-data read path: byte-offset math for a single (svar, class,
//! state) tuple in a `RESULT_ORDERED` subrecord.
//!
//! Step 9 of `planning/mili-rs/plan.md`. Pure metadata + offset
//! computation; the I/O (state-file mmap, byteswap, decode) lives in
//! [`crate::family::Database::state_var_values`].
//!
//! The byte-layout matrix in `planning/shared/format.md` §
//! "Subrecord byte-layout matrix" is the source of truth. For svar
//! `s` inside subrecord `k` of an srec held by a state:
//!
//! ```text
//! state_data_start  = state.offset                       (within state file)
//! subrec_k_start    = state_data_start
//!                     + sum_{i<k} N_i * bytes_per_obj_i  (across all svars in i)
//! RESULT_ORDERED:
//!     svar_s_start  = subrec_k_start + N_k * lump_offsets[s]
//!     svar_s_size   = N_k * lump_sizes[s]
//! ```
//!
//! Subrec byte size is organisation-agnostic — both `RESULT_ORDERED`
//! and `OBJECT_ORDERED` layouts pack `N * sum(per-object bytes)`
//! — so the running offset table is computable without decoding any
//! subrecord we don't actually read from.

use crate::error::{MiliError, Result};
use crate::srec::{derive_lumps, Organization, Srec, Subrecord};
use crate::svar::{NumType, SvarTable};

/// Typed return for a single-svar, single-state read. Variant chosen
/// from the svar's [`NumType`]. Values are flat in
/// `[object][atom]` row-major order — `len == N * atoms_per_object`.
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

/// One contiguous byte range to read from a single state file, owned
/// by one matching subrecord.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ByteSlab {
    /// Absolute byte offset into the state file.
    pub start: usize,
    /// Length in bytes.
    pub len: usize,
}

/// Plan for one `(svar, class, state)` read: a list of contiguous
/// byte ranges within the state file, plus the svar's numeric type.
///
/// May span multiple subrecords when more than one carries the
/// `(svar, class)` pair (typical for classes that are split across
/// id-block subrecs).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ReadPlan {
    pub num_type: NumType,
    pub slabs: Vec<ByteSlab>,
}

impl ReadPlan {
    pub fn total_bytes(&self) -> usize {
        self.slabs.iter().map(|s| s.len).sum()
    }
}

/// Build the read plan for `(svar_name, class_name)` against an srec,
/// rooted at `state_data_start` (absolute byte offset of the state's
/// data block inside its state file).
///
/// Errors on:
/// - svar name not in [`SvarTable`]
/// - subrecord references an undefined svar
/// - `OBJECT_ORDERED` for a matching subrecord (Step 10)
/// - no subrecord covers `(svar, class)`
pub(crate) fn plan_state_svar(
    srec: &Srec,
    svars: &SvarTable,
    svar_name: &str,
    class_name: &str,
    state_data_start: u64,
) -> Result<ReadPlan> {
    let target = svars
        .get(svar_name)
        .ok_or_else(|| MiliError::UnknownSvar(svar_name.to_owned()))?;
    let num_type = target.num_type;

    let mut running: u64 = state_data_start;
    let mut slabs = Vec::new();
    for sub in &srec.subrecords {
        let size = subrec_byte_size(sub, svars)?;
        let subrec_start = running;
        running = running
            .checked_add(size as u64)
            .ok_or(MiliError::MalformedDirectory(
                "query: subrec offset overflow",
            ))?;

        if sub.mclass != class_name {
            continue;
        }
        let Some(svar_idx) = sub.svar_names.iter().position(|n| n == svar_name) else {
            continue;
        };
        if sub.organization != Organization::ResultOrdered {
            return Err(MiliError::Unsupported("OBJECT_ORDERED query (Step 10)"));
        }

        let (atoms, widths) = atoms_and_widths(sub, svars)?;
        let lumps = derive_lumps(&atoms, &widths);
        let n = sub.object_count() as usize;
        let slab_off = n
            .checked_mul(lumps.offsets[svar_idx])
            .ok_or(MiliError::MalformedDirectory("query: slab offset overflow"))?;
        let slab_len = n
            .checked_mul(lumps.sizes[svar_idx])
            .ok_or(MiliError::MalformedDirectory("query: slab length overflow"))?;

        let start = (subrec_start as usize)
            .checked_add(slab_off)
            .ok_or(MiliError::MalformedDirectory("query: slab start overflow"))?;
        slabs.push(ByteSlab {
            start,
            len: slab_len,
        });
    }

    if slabs.is_empty() {
        return Err(MiliError::NoMatchingSubrec {
            svar: svar_name.to_owned(),
            class: class_name.to_owned(),
        });
    }
    Ok(ReadPlan { num_type, slabs })
}

fn subrec_byte_size(sub: &Subrecord, svars: &SvarTable) -> Result<usize> {
    let n = sub.object_count() as usize;
    let mut per_obj: usize = 0;
    for name in &sub.svar_names {
        let s = svars
            .get(name)
            .ok_or_else(|| MiliError::UnknownSvar(name.clone()))?;
        let cell = s
            .atoms
            .checked_mul(s.num_type.width())
            .ok_or(MiliError::MalformedDirectory(
                "query: per-svar byte size overflow",
            ))?;
        per_obj = per_obj
            .checked_add(cell)
            .ok_or(MiliError::MalformedDirectory(
                "query: per-object byte size overflow",
            ))?;
    }
    n.checked_mul(per_obj)
        .ok_or(MiliError::MalformedDirectory("query: subrec size overflow"))
}

fn atoms_and_widths(sub: &Subrecord, svars: &SvarTable) -> Result<(Vec<usize>, Vec<usize>)> {
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

    // Build an SvarTable with a small handcrafted dict.
    fn make_svars(specs: &[(&str, NumType, usize)]) -> SvarTable {
        // Encode a single STATE_VAR_DICT payload covering the specs.
        // qty_svars = len(specs), then per-svar: 4-int header
        //   [qty_subrecs_unused, agg=SCALAR(0), num_type_code, atoms (ignored)],
        // ... actually SvarTable::build follows the real on-disk dual-stream
        // format. Reusing the parser here would be heavyweight for a unit
        // test. Instead, build the table via a private constructor.
        //
        // We don't have a public test-only ctor — so this test relies on
        // svar.rs's tests for parser coverage and instead exercises the
        // plan math against a hand-built `SvarTable` we get by feeding
        // make_dict bytes through the parser.
        use_full_parser_for_test(specs)
    }

    fn use_full_parser_for_test(specs: &[(&str, NumType, usize)]) -> SvarTable {
        // Encode a STATE_VAR_DICT payload that the real parser will
        // accept. Layout: 2-int header [qty_int_words including the
        // header, qty_char_bytes] then per-svar int slots, then the
        // char stream. SCALAR (atoms == 1) gets [agg=0, type_code];
        // anything else uses ARRAY (agg=2) with rank=1 and a single
        // dim of `atoms`.
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
        let qty_int_words = (svar_ints.len() as i32) + 2;
        let qty_char_bytes = chars.len() as i32;
        let mut full_ints = vec![qty_int_words, qty_char_bytes];
        full_ints.extend(&svar_ints);
        let int_bytes: Vec<u8> = full_ints.iter().flat_map(|w| w.to_le_bytes()).collect();
        let mut payload = Vec::new();
        payload.extend_from_slice(&int_bytes);
        payload.extend_from_slice(&chars);

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

    #[test]
    fn plan_single_scalar_result_ordered() {
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
        let plan = plan_state_svar(&srec, &svars, "nodpos", "node", 100).unwrap();
        assert_eq!(plan.num_type, NumType::Float4);
        assert_eq!(
            plan.slabs,
            vec![ByteSlab {
                start: 100,
                len: 40
            }]
        );
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
        let plan = plan_state_svar(&srec, &svars, "svB", "node", 1000).unwrap();
        // 1000 (state start) + 20 (brick subrec) = 1020
        assert_eq!(
            plan.slabs,
            vec![ByteSlab {
                start: 1020,
                len: 40
            }]
        );
    }

    #[test]
    fn plan_picks_second_svar_in_subrec() {
        // subrec has [svA scalar f32, svB scalar f32] over 4 objects.
        // svB slab starts at subrec_start + N * lump_offsets[1]
        //   = 0 + 4 * 4 = 16. Length = 4 * 4 = 16.
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
        let plan = plan_state_svar(&srec, &svars, "svB", "node", 0).unwrap();
        assert_eq!(plan.slabs, vec![ByteSlab { start: 16, len: 16 }]);
    }

    #[test]
    fn plan_handles_vector_atoms() {
        // ARRAY rank=1 atoms=3 (e.g. position vector). N=4 objects →
        // bytes = 4 * 3 * 4 = 48.
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
        let plan = plan_state_svar(&srec, &svars, "nodpos", "node", 0).unwrap();
        assert_eq!(plan.slabs, vec![ByteSlab { start: 0, len: 48 }]);
    }

    #[test]
    fn plan_concatenates_across_matching_subrecs() {
        // Two subrecs with the same (svar, class), different id ranges.
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
        let plan = plan_state_svar(&srec, &svars, "svA", "node", 0).unwrap();
        assert_eq!(
            plan.slabs,
            vec![
                ByteSlab { start: 0, len: 12 },
                ByteSlab { start: 12, len: 12 },
            ]
        );
    }

    #[test]
    fn plan_errors_on_object_ordered_match() {
        let svars = make_svars(&[("svA", NumType::Float4, 1)]);
        let srec = Srec {
            srec_id: 0,
            mesh_id: 0,
            srec_size: 0,
            subrecords: vec![mk_subrec(
                "node",
                Organization::ObjectOrdered,
                &["svA"],
                &[(1, 3)],
            )],
        };
        let err = plan_state_svar(&srec, &svars, "svA", "node", 0).unwrap_err();
        assert!(matches!(err, MiliError::Unsupported(_)));
    }

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
        let err = plan_state_svar(&srec, &svars, "svA", "node", 0).unwrap_err();
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
        let err = plan_state_svar(&srec, &svars, "missing", "node", 0).unwrap_err();
        assert!(matches!(err, MiliError::UnknownSvar(_)));
    }
}
