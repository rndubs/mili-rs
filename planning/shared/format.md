# Mili on-disk format reference

This is the byte-level reverse engineering of the existing C library
that we are pinning down for compatibility. Citations are
`reference/mili/<path>:<line>`.

## File set

A mili database with root `R` is a directory containing:

- `R.A` — non-state metadata. Header is at offset 0; the **directory
  is at the trailer**, located by seeking backward from EOF (see
  "Directory" below). Body between holds node coordinates, element
  connectivity, class definitions, svar dictionary, parameters.
- `R.A00`, `R.A01`, … — state files. Suffix width is configurable in
  the header (`mili_internal.h:414-422`). **No per-state header**:
  states are written back-to-back; offsets and times live in the
  `.A` file's `state_map` (`mili_internal.h:109-115`,
  `srec.c:3456-3586`).
- `R.ATI0`, `R.ATI1`, … — **separate files** for time-independent
  parameters. Not stored in the main `.A`. Have their own directory,
  hash table, and roll-over logic (`mili_internal.h:476, 484-496`).
- `R.A.tfile` — optional time-state index file (header v3+ only).
  When present, the state count moves out of the directory header
  into this file (`direc.c:443-455, 487-490`).
- `R.mili` — JSON sidecar for VisIt. Not part of the binary format
  proper; generated separately. `mili-rs` writes it on close if
  enabled.
- `R.lock` — 128-byte advisory lock, only present when filelock is
  opted in via `mc_filelock_enable()` (`mili.c:101+`).

### State file partitioning

Two schemes (`mili_enum.h:101-104`, `mili_internal.h:404-406`):

- `STATE_COUNT` (default): roll over after `states_per_file` states.
  Default 10,000,000.
- `BYTE_COUNT`: roll over once a file's size exceeds
  `bytes_per_file_limit`. Default 10,000,000.

Both thresholds can be active; whichever trips first triggers the
roll-over (`mili.c:2862-2887`). The active scheme and thresholds are
persisted in the `.A` file as named scalar parameters
(`"states per file"`, `"max size per file"`; `mili.c:110, 2214,
2486`) and read back via `mc_read_scalar()` at open time
(`mili.c:1295-1311`).

## `.A` header (`mili.c:2059`, `mili_enum.h:414-422`)

Fixed 16-byte prefix (extensible via byte 15):

| Offset | Bytes | Meaning                                           |
|-------:|------:|---------------------------------------------------|
| 0      | 4     | Magic ASCII `"mili"`                              |
| 4      | 1     | Header version (current = 3)                      |
| 5      | 1     | Directory version (current = 3)                   |
| 6      | 1     | Endianness: `1` = big, `2` = little               |
| 7      | 1     | Precision limit (single / double / mixed)         |
| 8      | 1     | State file suffix width (digits)                  |
| 9      | 1     | Partition scheme: state-count or byte-count       |
| 10–14  | 5     | Reserved                                          |
| 15     | 1     | Extension field count (more 4-byte fields follow) |

Endianness flag is the one byte that decides whether the rest of the
file needs swapping at read time. See `mili_endian.h:74,82` for the
swap routines: `M_FLOAT4`, `M_FLOAT8`, `M_INT`, `M_INT4`, `M_INT8`
swap; `M_STRING` and `M_CHAR` do not.

## Directory (`mili_enum.h:179-191`, `mili_internal.h:157-168`, `direc.c`)

The directory lives at the trailer of the `.A` file. To read it the
parser seeks `-(QTY_DIR_HEADER_FIELDS * sizeof(int))` bytes from EOF
to find the directory header, then walks backward to recover the
entries and the name pool (`direc.c:411-415`).

Layout, in file order:

```
[ ... body bytes ... ]
[ name pool: NUL-terminated strings, total NAMES_LEN bytes ]
[ entries: QTY_ENTRIES × 6 × sizeof(int_t) bytes ]
[ directory header: QTY_DIR_HEADER_FIELDS × 4-byte ints ]   <- EOF
```

### Entries (`mili_internal.h:157`)

Each entry is six fields. **Field width depends on directory
version** — see "Version history" below.

| Field           | Meaning                                              |
|-----------------|------------------------------------------------------|
| `TYPE`          | NODES, ELEM_CONNS, STATE_VAR_DICT, STATE_REC_DATA, MILI_PARAM, APPLICATION_PARAM, CLASS_DEF, SURFACE_CONNS, TI_PARAM |
| `MODIFIER1`     | Mesh or object ID                                    |
| `MODIFIER2`     | Object count or type-specific modifier               |
| `STRING_QTY`    | Number of names in the pool this entry consumes      |
| `OFFSET`        | Byte offset into the owning file                     |
| `LENGTH`        | Length in bytes                                      |

### Directory header (`mili_enum.h:187-191`)

Four 4-byte ints, immediately before EOF:

| Field             | Meaning                                          |
|-------------------|--------------------------------------------------|
| `NAMES_LEN`       | Length of the name pool in bytes (rounded up)    |
| `COMMIT_COUNT`    | Commit number for this directory                 |
| `QTY_ENTRIES`     | Number of entries that follow                    |
| `QTY_STATES`      | (v2+) Total state count; (v1) absent             |

In header v3+ databases that ship a `.tfile`, `QTY_STATES` is 0 and
the real state count is read from the time-state file instead
(`direc.c:443-455`).

### Name pool (`mili_internal.h:160-168`, `direc.c:154-161, 540-545`)

Names are stored separately from entries in a compact pool of
NUL-terminated strings sized by `NAMES_LEN`. Each entry's
`STRING_QTY` says how many names from the pool it consumes, in
order. On the write side, `ios_str_dup()` appends into an
`IO_mem_store`; on the read side, the pool is loaded into a
`File_dir` (`char **names` plus the backing `IO_mem_store
*name_data`).

### Version history

Older directories exist in the wild — `reference/mili-python/tests/
data/serial/dir_version_2/` is a test fixture that exercises them.

- **Directory v1** (`direc.c:238, 255, 425-441`): writes
  `QTY_DIR_HEADER_FIELDS - 1` ints (no `QTY_STATES`). Entries are
  stored as 4-byte ints. Entry data type on disk differs
  (`direc.c:492-538`).
- **Directory v2** (`direc.c:454, 507, 520`): adds `QTY_STATES` to
  the header. Entries still 4-byte ints.
- **Directory v3** (`direc.c:507, 520, 531-536`): entries widen to
  8-byte `LONGLONG`. v2 entries are converted to LONGLONGs in
  memory after read.

A read implementation has to handle all three. A write implementation
emits v3 only.

Reading directory entries gives us the contiguous byte ranges that
`MiliBuffer` will eventually point at.

## Numeric types (`mili.h:54-60`, `mili.c:140-141`)

| Code      | On-disk size | Rust type        |
|-----------|-------------:|------------------|
| M_STRING  | variable     | `&[u8]` (NUL-term) |
| M_FLOAT4  | 4            | `f32`            |
| M_FLOAT8  | 8            | `f64`            |
| M_INT4    | 4            | `i32`            |
| M_INT8    | 8            | `i64`            |
| M_CHAR    | 1            | `u8`             |

`M_FLOAT` and `M_INT` are platform-native aliases that resolve to one
of the explicit widths at write time and are recorded as the explicit
form in the directory. We always emit explicit widths.

No packed, varint, or compressed encodings exist. Every numeric block
on disk is a plain C array of the declared width.

## Mesh entities (`mili_internal.h:262-269`, `253-260`; `mili.h:429-494`)

- **Mesh**: unstructured; holds a hash table of object classes.
- **Object class** (e.g. "Nodal" with superclass `M_NODE`, "Hex" with
  superclass `M_HEX`): name, superclass code, element count.
- **Node coordinates**: dense `[num_nodes * dims]` `f32` block;
  dimensionality (2 or 3) recorded in `fam->dimensions`. Directory
  entry type `NODES`.
- **Element connectivity**: integer node-ID lists. Two formats:
  `M_LIST_OBJ_FMT` (arbitrary IDs) or `M_BLOCK_OBJ_FMT` (id ranges).
  Each element superclass has a fixed `conn_words` (`mili.h:88`):
  `M_HEX` = 8, `M_QUAD` = 4, `M_TET` = 4, etc.
- **Labels**: optional integer per-element / per-node arrays mapping
  local ordinals → global IDs.

## State records, subrecords, svars (`mili.h:169-196`, `srec.c:1821-1921`)

- **State variable (svar)**: scalar, vector, array, or vec-array.
  Carries `num_type`, `agg_type`, `rank`, `dims[]`, optional
  `components[]`.
- **Subrecord**: a grouping of svars over a particular object class.
  Has an `organization`:
  - `RESULT_ORDERED` — all values of svar 0 across all objects, then
    svar 1, etc. (column-major).
  - `OBJECT_ORDERED` — all svars for object 0, then object 1, etc.
    (row-major).
- **Srec** (state record format): the schema for a state. Multiple
  srec formats can coexist; each state is tagged with one.
- **State**: per-state file offset + time + srec format ID
  (`State_descriptor`, `mili_internal.h:109-115`). Inside the state
  file, each subrecord is a contiguous byte range described by
  `lump_offsets[]` and `lump_sizes[]` (`Sub_srec`,
  `mili_internal.h:215-250`).

## Error handling philosophy (`mili.c:1518`, `mili_statemap.c:602-610`, `direc.c:418-440`, `io_mem.c:292`)

The C library is **defensive on I/O** — every `read`, `seek`,
length check returns a `Return_value` error code (`mili_enum.h:210+`,
e.g. `SHORT_READ`, `CORRUPTED_FILE`, `SEEK_FAILED`, `BAD_LOAD_READ`).
It is **permissive on header validation** — the magic bytes are
read but not explicitly checked; a garbage header reads as garbage
metadata rather than failing fast.

`mili-rs` will be defensive on both: malformed magic, bad version
bytes, or directory entries that point past EOF all return a typed
`MiliError`. No `abort`, no panic on bad input.

## Zero-copy implications

A read of one svar for one state in `RESULT_ORDERED` is a single
contiguous byte range and can be exposed directly as a typed slice
(after byteswap if needed). `OBJECT_ORDERED` and multi-svar queries
require a gather pass; the gather still happens in Rust into a single
output buffer, but it is not zero-copy from disk. See `buffer.md`.

## Public C API surface to mirror

Roughly 110 entry points across `reference/mili/src/`. Coarse split:

- Family I/O: `mc_open`, `mc_close`, `mc_delete_family`,
  `mc_quick_open`, `mc_partition_state_data`, `mc_restart_at_*`.
- Metadata definition: `mc_make_umesh`, `mc_def_class`, `mc_def_nodes`,
  `mc_def_conn_*`, `mc_load_nodes`, `mc_load_conns`, `mc_def_svars`,
  `mc_def_vec_svar`, `mc_def_arr_svar`, `mc_get_svar_def`.
- State records: `mc_open_srec`, `mc_close_srec`, `mc_def_subrec`,
  `mc_get_subrec_def`.
- State data write: `mc_new_state`, `mc_wrt_stream`, `mc_wrt_subrec`,
  `mc_end_state`.
- State data read: `mc_read_results`.
- Parameters: `mc_wrt_scalar`, `mc_read_scalar`, `mc_wrt_array`,
  `mc_read_param_array`, `mc_wrt_string`, `mc_read_string` and their
  `mc_ti_*` time-independent variants.
- Query: `mc_query_family` (≈ 20 query types).

The `mili-rs` API does not have to be a 1:1 translation; the C surface
is a checklist of capabilities we need to cover, not a target shape.
