//! `mili_rs::write` — the on-disk A/T/S byte-layout writer (Phase 3.1).
//!
//! Decision 22 (`planning/mili-py/m4.md` § "Phase 3"): the
//! parity-sensitive byte-layout writer lives in the Rust core, gated by
//! `crates/mili-rs/tests/parity_write_append.rs`. It reproduces the
//! **output** of upstream `mili.afileIO.AFileWriter` (which
//! *renormalises* the parsed model — it is **not** a byte-identity
//! round-trip of the original `.A`).
//!
//! Empirical bound (`planning/mili-py/phase-3.md` + `status.md` §
//! Surprises): for the d3samp6 corpus upstream emits payload bytes
//! **byte-identical to the original `.A` mmap ranges** for every
//! directory except `STATE_VAR_DICT` (params / class-def / class-idents
//! / nodes / elem-conns + `STATE_REC_DATA`), and a verbatim 16-byte
//! header. Only `STATE_VAR_DICT` is rebuilt (upstream renormalises its
//! int/char stream), plus the directory string pool, the dir-decl
//! table, the state-map block, and the footer.

use std::collections::HashSet;
use std::io::{Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

use crate::directory::DirEntryType;
use crate::error::{MiliError, Result};
use crate::header::Endianness;
use crate::mesh::MeshId;
use crate::query::{plan_state_svar_ip, Filter, IntPoints};
use crate::srec::Srec;
use crate::state::StateMeta;
use crate::svar::{SvarAgg, SvarTable};
use crate::{Database, QueryArgs};

/// `mili.afileIO.AFileWriter`'s `dir_order` /
/// `__write_dir_decls.dir_decl_order` (`afileIO.py:520-526, 729-737`):
/// the seven payload-bearing directory types, then `STATE_VAR_DICT`,
/// then `STATE_REC_DATA`.
const DIR_ORDER: [DirEntryType; 7] = [
    DirEntryType::MiliParam,
    DirEntryType::ApplicationParam,
    DirEntryType::TiParam,
    DirEntryType::ClassDef,
    DirEntryType::ClassIdents,
    DirEntryType::Nodes,
    DirEntryType::ElemConns,
];

fn put_i32(out: &mut Vec<u8>, end: Endianness, v: i32) {
    match end {
        Endianness::Big => out.extend_from_slice(&v.to_be_bytes()),
        Endianness::Little => out.extend_from_slice(&v.to_le_bytes()),
    }
}

fn put_i64(out: &mut Vec<u8>, end: Endianness, v: i64) {
    match end {
        Endianness::Big => out.extend_from_slice(&v.to_be_bytes()),
        Endianness::Little => out.extend_from_slice(&v.to_le_bytes()),
    }
}

fn put_word(out: &mut Vec<u8>, end: Endianness, word: usize, v: i64) {
    if word == 8 {
        put_i64(out, end, v);
    } else {
        put_i32(out, end, v as i32);
    }
}

fn f32_bytes(end: Endianness, v: f32) -> [u8; 4] {
    match end {
        Endianness::Big => v.to_be_bytes(),
        Endianness::Little => v.to_le_bytes(),
    }
}

/// What to do with the per-fragment `state_count` APPLICATION_PARAM
/// scalar. `copy_non_state_data` resets it to 0
/// (`datatypes.py:546-547`); `append_state` increments the current
/// value by one (`miliinternal.py:1494-1495`); otherwise it is copied
/// verbatim.
#[derive(Clone, Copy, PartialEq)]
enum StateCountOp {
    Keep,
    Zero,
    Increment,
}

/// A merged directory declaration to emit in the dir-decl table.
struct EmitDecl {
    type_code: i64,
    modifier1: i64,
    modifier2: i64,
    string_qty: i64,
    offset: i64,
    length: i64,
}

impl Database {
    /// Re-serialise the `.A` file exactly as upstream
    /// `mili.afileIO.AFileWriter.write` would, given an explicit
    /// state-map list (`copy_non_state_data` passes `&[]`;
    /// `append_state` passes the existing maps plus the new one) and
    /// whether the `APPLICATION_PARAM` `state_count` scalar should be
    /// bumped by one (`append_state`).
    #[allow(clippy::too_many_lines)]
    fn serialize_afile(
        &self,
        smaps: &[StateMeta],
        state_count_op: StateCountOp,
    ) -> Result<Vec<u8>> {
        let a = self.a_bytes();
        let end = self.header().endianness;
        let dir = self.directory();
        let word = if self.header().dir_version == 3 { 8 } else { 4 };

        let mut out: Vec<u8> = Vec::with_capacity(a.len());
        // Header: verbatim original 16-byte range (proven bit-identical
        // to upstream `__write_header` for the corpus).
        out.extend_from_slice(&a[..crate::header::Header::SIZE]);

        let mut strings_pool: Vec<&str> = Vec::new();
        // Decls grouped by the dir-decl write order. Index parallels
        // `DIR_ORDER` + [StateVarDict, StateRecData].
        let mut groups: Vec<Vec<EmitDecl>> = (0..DIR_ORDER.len() + 2).map(|_| Vec::new()).collect();

        let names = &dir.names;

        // ---- payload-bearing directories (dir_order) ----------------
        for (gi, &dt) in DIR_ORDER.iter().enumerate() {
            for entry in dir.entries.iter().filter(|e| e.entry_type == dt) {
                let new_off = out.len();
                let payload_start = usize::try_from(entry.offset)
                    .map_err(|_| MiliError::MalformedDirectory("write: negative dir offset"))?;
                let payload_len = usize::try_from(entry.length)
                    .map_err(|_| MiliError::MalformedDirectory("write: negative dir length"))?;

                // The single per-fragment `state_count` APPLICATION_PARAM
                // scalar is bumped on append (`miliinternal.py:1494`).
                let is_state_count = dt == DirEntryType::ApplicationParam
                    && entry.name_count == 1
                    && names.get(entry.name_start as usize) == "state_count";
                if is_state_count && payload_len == 4 && state_count_op != StateCountOp::Keep {
                    let cur = end.read_i32(
                        a[payload_start..payload_start + 4]
                            .try_into()
                            .expect("4 bytes"),
                    );
                    let v = match state_count_op {
                        StateCountOp::Zero => 0,
                        StateCountOp::Increment => cur + 1,
                        StateCountOp::Keep => cur,
                    };
                    put_i32(&mut out, end, v);
                } else {
                    out.extend_from_slice(&a[payload_start..payload_start + payload_len]);
                }
                let new_len = (out.len() - new_off) as i64;
                for i in 0..entry.name_count {
                    strings_pool.push(names.get((entry.name_start + i) as usize));
                }
                groups[gi].push(EmitDecl {
                    type_code: dt as i64,
                    modifier1: entry.modifier1,
                    modifier2: entry.modifier2,
                    string_qty: entry.string_qty,
                    offset: new_off as i64,
                    length: new_len,
                });
            }
        }

        // ---- STATE_VAR_DICT (rebuilt) -------------------------------
        // Upstream picks `dir_decls[STATE_VAR_DICT][0]` and rewrites the
        // whole svar dict from the parsed model (`afileIO.py:532-535`).
        if let Some(entry) = dir
            .entries
            .iter()
            .find(|e| e.entry_type == DirEntryType::StateVarDict)
        {
            let new_off = out.len();
            let payload = build_svar_payload(self.svars(), end);
            out.extend_from_slice(&payload);
            groups[DIR_ORDER.len()].push(EmitDecl {
                type_code: DirEntryType::StateVarDict as i64,
                modifier1: entry.modifier1,
                modifier2: entry.modifier2,
                string_qty: entry.string_qty,
                offset: new_off as i64,
                length: (out.len() - new_off) as i64,
            });
            for i in 0..entry.name_count {
                strings_pool.push(names.get((entry.name_start + i) as usize));
            }
        }

        // ---- STATE_REC_DATA (raw payload copy) ----------------------
        // Proven byte-identical to the original payload for the corpus
        // (upstream rebuilds it but to the same bytes); modifiers are
        // therefore unchanged.
        if let Some(entry) = dir
            .entries
            .iter()
            .find(|e| e.entry_type == DirEntryType::StateRecData)
        {
            let new_off = out.len();
            let ps = entry.offset as usize;
            let pl = entry.length as usize;
            out.extend_from_slice(&a[ps..ps + pl]);
            groups[DIR_ORDER.len() + 1].push(EmitDecl {
                type_code: DirEntryType::StateRecData as i64,
                modifier1: entry.modifier1,
                modifier2: entry.modifier2,
                string_qty: entry.string_qty,
                offset: new_off as i64,
                length: (out.len() - new_off) as i64,
            });
            for i in 0..entry.name_count {
                strings_pool.push(names.get((entry.name_start + i) as usize));
            }
        }

        // ---- directory string pool ----------------------------------
        let str_region = out.len();
        for s in &strings_pool {
            out.extend_from_slice(s.as_bytes());
            out.push(0);
        }
        let raw_str_bytes = out.len() - str_region;
        let pad = (4 - raw_str_bytes % 4) % 4;
        out.resize(out.len() + pad, 0u8);
        let string_bytes = (raw_str_bytes + pad) as i32;

        // ---- dir-decl table -----------------------------------------
        let mut dir_count: i32 = 0;
        for grp in &groups {
            for d in grp {
                put_word(&mut out, end, word, d.type_code);
                put_word(&mut out, end, word, d.modifier1);
                put_word(&mut out, end, word, d.modifier2);
                put_word(&mut out, end, word, d.string_qty);
                put_word(&mut out, end, word, d.offset);
                put_word(&mut out, end, word, d.length);
                dir_count += 1;
            }
        }

        // ---- state-map block (inline; `file_version < 3` or
        // `not allow_tfile` — always inline for the corpus) -----------
        for sm in smaps {
            put_i32(&mut out, end, sm.file);
            put_i64(&mut out, end, sm.offset);
            out.extend_from_slice(&f32_bytes(end, sm.time));
            put_i32(&mut out, end, sm.srec_format);
        }

        // ---- footer -------------------------------------------------
        put_i32(&mut out, end, string_bytes);
        put_i32(&mut out, end, dir.commit_count);
        put_i32(&mut out, end, dir_count);
        put_i32(&mut out, end, smaps.len() as i32);

        Ok(out)
    }

    /// Upstream `_MiliInternal.copy_non_state_data` — write a new `.A`
    /// with **no** states. The source basename's trailing `(\d+)$`
    /// digits are appended to `new_base` so an uncombined family's
    /// per-proc files stay distinct (`miliinternal.py:1542-1558`).
    pub fn copy_non_state_data(&self, new_base: &str) -> Result<()> {
        let digits = trailing_proc_digits(self.a_path());
        let target = format!("{new_base}{digits}");
        let bytes = self.serialize_afile(&[], StateCountOp::Zero)?;
        std::fs::write(format!("{target}A"), bytes)?;
        Ok(())
    }

    /// Upstream `_MiliInternal.append_state` (`miliinternal.py:1433`):
    /// append one new state. Returns the new state count.
    pub fn append_state(
        &self,
        new_state_time: f64,
        zero_out: bool,
        limit_states_per_file: Option<i64>,
        limit_bytes_per_file: Option<i64>,
    ) -> Result<usize> {
        let n = self.states().len();
        if n > 0 && (new_state_time as f32) <= self.states()[n - 1].time {
            return Err(MiliError::Unsupported(
                "append_state: new state time must exceed the current last state",
            ));
        }
        if self.srecs().is_empty() {
            return Err(MiliError::Unsupported("append_state: no subrecords exist"));
        }
        let state_size: i64 = self
            .srecs()
            .iter()
            .map(|s| i64::from(s.srec_size))
            .sum::<i64>()
            + 8;

        let (file_number, file_offset, creating_new) = if n == 0 {
            (0i32, 0i64, true)
        } else {
            let last = self.states()[n - 1];
            let mut fnum = last.file;
            let mut off = last.offset + state_size;
            let mut newfile = false;
            if let Some(lim) = limit_states_per_file {
                let in_file = self.states().iter().filter(|s| s.file == fnum).count() as i64;
                if in_file + 1 > lim {
                    newfile = true;
                    fnum += 1;
                    off = 0;
                }
            }
            if let Some(lb) = limit_bytes_per_file {
                if off + state_size > lb {
                    newfile = true;
                    fnum += 1;
                    off = 0;
                }
            }
            (fnum, off, newfile)
        };

        let new_smap = StateMeta {
            file: file_number,
            offset: file_offset,
            time: new_state_time as f32,
            srec_format: 0,
        };
        let mut smaps = self.states().to_vec();
        smaps.push(new_smap);

        // Rewrite the `.A` (smap appended, state_count bumped).
        // Atomic write-then-rename: the database's own `a_mmap` is a
        // live `MAP_SHARED` mapping of this very path — truncating it in
        // place under that mapping corrupts every subsequent
        // `a_bytes()` read in this same call (the nodal-position read
        // below) and the in-process re-parse (`reload`), surfacing as
        // "NODES: payload shorter than declared body" (the Phase-3.1
        // surprise). Renaming a fresh inode over the path leaves the
        // old mapping valid (original bytes) and gives `reload` a clean
        // new file. Bytes are identical to a direct write, so the
        // Phase-3.1 byte gate is unaffected.
        let abytes = self.serialize_afile(&smaps, StateCountOp::Increment)?;
        atomic_write(self.a_path(), &abytes)?;

        // State file `<base><suffix>` (`"{:02}".format(file_number)`).
        let state_path = self.state_file_path(file_number);
        let body_len = (state_size - 8) as usize;
        let mut buf = vec![0u8; state_size as usize];
        buf[0..4].copy_from_slice(&f32_bytes(self.header().endianness, new_state_time as f32));
        // state_map_id = 0; bytes already zero.
        if !zero_out && n > 0 {
            // Copy the previous state's body (`__get_state_byte_data`).
            let prev = self.states()[n - 1];
            let prev_path = self.state_file_path(prev.file);
            let prev_bytes = std::fs::read(&prev_path)?;
            let s = (prev.offset + 8) as usize;
            buf[8..8 + body_len].copy_from_slice(&prev_bytes[s..s + body_len]);
        }

        if creating_new {
            std::fs::write(&state_path, &buf)?;
        } else {
            let mut f = std::fs::OpenOptions::new()
                .read(true)
                .write(true)
                .open(&state_path)?;
            f.seek(SeekFrom::Start(file_offset as u64))?;
            f.write_all(&buf)?;
        }

        // On a zeroed / first state, upstream writes the nodal
        // positions and sand flags so the state is visualisable
        // (`miliinternal.py:1518-1538`).
        if n == 0 || zero_out {
            let data_start = (file_offset + 8) as u64;
            // Upstream (`miliinternal.py:1518-1538`): on the **first**
            // state write the initial nodal positions (`db.nodes()`);
            // on a later `zero_out` state copy the **previous state's**
            // nodal positions (`query("nodpos", states=[prev])`), not
            // the initial coords. Both are then scattered exactly as a
            // `query(write_data=)` would (decision 23) — the same
            // single-svar primitive.
            let nodpos_vals: Vec<f32> = if n == 0 {
                let mesh = self.canonical_mesh_id();
                self.node_coords(mesh)?.map(|(c, _)| c).unwrap_or_default()
            } else {
                let qa = QueryArgs {
                    svar: "nodpos",
                    class: "node",
                    labels: None,
                    states: &[n - 1],
                    materials: None,
                    ips: None,
                    subrec: None,
                };
                match self.query_full(&qa)?.values {
                    crate::query::StateValues::F32(v) => v,
                    _ => Vec::new(),
                }
            };
            if !nodpos_vals.is_empty() {
                self.scatter_state_field(&state_path, data_start, "nodpos", "node", &nodpos_vals)?;
            }
            if let Some(classes) = self.classes_of_state_variable("sand") {
                for class in classes {
                    self.scatter_state_field_fill(&state_path, data_start, "sand", &class, 1.0)?;
                }
            }
        }

        Ok(smaps.len())
    }

    fn canonical_mesh_id(&self) -> MeshId {
        self.meshes()
            .meshes()
            .map(|m| m.id)
            .min()
            .unwrap_or(MeshId(0))
    }

    fn state_file_path(&self, file_number: i32) -> PathBuf {
        let a = self.a_path();
        let dir = a.parent().unwrap_or_else(|| Path::new("."));
        let fname = a.file_name().and_then(|s| s.to_str()).unwrap_or("");
        // base = filename minus the trailing 'A'.
        let base = fname.strip_suffix('A').unwrap_or(fname);
        dir.join(format!("{base}{file_number:02}"))
    }

    /// Scatter a flat f32 value array into one `(svar, class)`'s byte
    /// slabs at `data_start`, inverting the read gather
    /// (`query::ReadPlan`). Used for the `nodpos` write.
    fn scatter_state_field(
        &self,
        state_path: &Path,
        data_start: u64,
        svar: &str,
        class: &str,
        values: &[f32],
    ) -> Result<()> {
        let srec = self.srec_for_new_state()?;
        let plan = plan_state_svar_ip(
            srec,
            self.svars(),
            svar,
            class,
            data_start,
            Filter {
                labels: None,
                ips: None,
                subrec: None,
            },
            &IntPoints::default(),
        )?;
        let end = self.header().endianness;
        let mut f = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(state_path)?;
        let mut vi = 0usize;
        for slab in &plan.slabs {
            let n = slab.len / 4;
            let mut bytes = Vec::with_capacity(slab.len);
            for _ in 0..n {
                let v = *values.get(vi).ok_or(MiliError::MalformedDirectory(
                    "write: value array shorter than the gather plan",
                ))?;
                bytes.extend_from_slice(&f32_bytes(end, v));
                vi += 1;
            }
            f.seek(SeekFrom::Start(slab.start as u64))?;
            f.write_all(&bytes)?;
        }
        Ok(())
    }

    /// Scatter a constant f32 fill into one `(svar, class)`'s slabs.
    /// Used for the `sand = 1.0` write.
    fn scatter_state_field_fill(
        &self,
        state_path: &Path,
        data_start: u64,
        svar: &str,
        class: &str,
        fill: f32,
    ) -> Result<()> {
        let srec = self.srec_for_new_state()?;
        let plan = plan_state_svar_ip(
            srec,
            self.svars(),
            svar,
            class,
            data_start,
            Filter {
                labels: None,
                ips: None,
                subrec: None,
            },
            &IntPoints::default(),
        )?;
        let end = self.header().endianness;
        let fb = f32_bytes(end, fill);
        let mut f = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(state_path)?;
        for slab in &plan.slabs {
            let mut bytes = Vec::with_capacity(slab.len);
            for _ in 0..slab.len / 4 {
                bytes.extend_from_slice(&fb);
            }
            f.seek(SeekFrom::Start(slab.start as u64))?;
            f.write_all(&bytes)?;
        }
        Ok(())
    }

    /// The srec a freshly appended state writes into (`state_map_id =
    /// 0`, `miliinternal.py:1489`).
    fn srec_for_new_state(&self) -> Result<&Srec> {
        self.srecs()
            .get(0)
            .or_else(|| self.srecs().iter().next())
            .ok_or(MiliError::MalformedDirectory(
                "append_state: no srec for the new state",
            ))
    }
}

/// `AFileWriter.__collect_svar_data` (`afileIO.py:665-680`): append
/// this svar's `[agg, type]` ints + `[name, title]` strings, the
/// ARRAY/VEC_ARRAY `[order, dims…]`, and the VECTOR/VEC_ARRAY
/// `[list_size]` + comp names, then recurse into not-yet-seen comps.
fn collect_svar_data(
    svars: &SvarTable,
    name: &str,
    int_data: &mut Vec<i32>,
    str_data: &mut Vec<String>,
    processed: &mut HashSet<String>,
) {
    let Some(sv) = svars.get(name) else {
        return;
    };
    processed.insert(sv.name.clone());
    let agg_code: i32 = match &sv.agg {
        SvarAgg::Scalar => 0,
        SvarAgg::Vector { .. } => 1,
        SvarAgg::Array { .. } => 2,
        SvarAgg::VecArray { .. } => 3,
    };
    int_data.push(agg_code);
    int_data.push(sv.type_code);
    str_data.push(sv.name.clone());
    str_data.push(sv.title.clone());
    match &sv.agg {
        SvarAgg::Array { dims } | SvarAgg::VecArray { dims, .. } => {
            int_data.push(dims.len() as i32);
            int_data.extend_from_slice(dims);
        }
        _ => {}
    }
    if let SvarAgg::Vector { comps } | SvarAgg::VecArray { comps, .. } = &sv.agg {
        int_data.push(comps.len() as i32);
        for c in comps {
            str_data.push(c.clone());
        }
        for c in comps.clone() {
            if !processed.contains(&c) {
                collect_svar_data(svars, &c, int_data, str_data, processed);
            }
        }
    }
}

/// Port of `AFileWriter.__write_svars` (`afileIO.py:637-663`). The
/// two header ints + the trailing alignment use the file endian; the
/// int stream mirrors upstream's prefix-less `struct.pack('{n}i', …)`
/// (native order, == the file endian on the little-endian corpus + CI).
fn build_svar_payload(svars: &SvarTable, end: Endianness) -> Vec<u8> {
    let mut int_data: Vec<i32> = Vec::new();
    let mut str_data: Vec<String> = Vec::new();
    let mut processed: HashSet<String> = HashSet::new();

    for sv in svars.iter() {
        if !processed.contains(&sv.name) {
            let name = sv.name.clone();
            collect_svar_data(svars, &name, &mut int_data, &mut str_data, &mut processed);
        }
    }

    let mut body: Vec<u8> = Vec::new();
    for v in &int_data {
        match end {
            Endianness::Big => body.extend_from_slice(&v.to_be_bytes()),
            Endianness::Little => body.extend_from_slice(&v.to_le_bytes()),
        }
    }
    let int_bytes = body.len();
    for s in &str_data {
        body.extend_from_slice(s.as_bytes());
        body.push(0);
    }
    let raw_str = body.len() - int_bytes;
    let pad = (4 - raw_str % 4) % 4;
    body.resize(body.len() + pad, 0u8);
    let str_bytes = raw_str + pad;

    let int_cnt = (int_bytes / 4 + 2) as i32;
    let mut out: Vec<u8> = Vec::with_capacity(8 + body.len());
    put_i32(&mut out, end, int_cnt);
    put_i32(&mut out, end, str_bytes as i32);
    out.extend_from_slice(&body);
    out
}

/// Write `bytes` to `path` atomically (write a sibling temp file, then
/// `rename` it over `path`). The rename swaps in a **new inode**, so a
/// pre-existing `MAP_SHARED` mapping of `path` keeps observing the old
/// inode's original bytes — required because the database rewrites its
/// own live-mmapped `.A` (Phase 3.2; see `append_state`).
fn atomic_write(path: &Path, bytes: &[u8]) -> Result<()> {
    let tmp = match path.file_name().and_then(|s| s.to_str()) {
        Some(name) => path.with_file_name(format!(".{name}.milox-tmp")),
        None => return Err(MiliError::MalformedDirectory("write: bad .A path")),
    };
    std::fs::write(&tmp, bytes)?;
    std::fs::rename(&tmp, path)?;
    Ok(())
}

/// The trailing `(\d+)$` of a `<base>A` path's filename
/// (`miliinternal.py:1555` `re.compile(r'(\d+)$')`).
fn trailing_proc_digits(a_path: &Path) -> String {
    let fname = a_path.file_name().and_then(|s| s.to_str()).unwrap_or("");
    let base = fname.strip_suffix('A').unwrap_or(fname);
    let digits: Vec<char> = base
        .chars()
        .rev()
        .take_while(char::is_ascii_digit)
        .collect();
    digits.into_iter().rev().collect()
}
