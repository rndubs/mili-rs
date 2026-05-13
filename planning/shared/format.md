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
- `<root>_TI_A`, `<root>_TI_B`, … — **separate files** for
  time-independent parameters, written **only in directory v1**
  databases (`mili_util.c:908-911`, base26 suffix). v2+ databases
  store `TI_PARAM` entries inline in the main `.A` directory; see
  the "TI_PARAM location" note below. Filenames follow the
  `<root>_TI_<base26_upper>` convention, **not** the `R.ATI*`
  pattern an earlier version of this doc claimed.
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

### State file suffix width (header byte 8)

`HDR_SFILE_SUFFIX_WIDTH_IDX` is a one-byte field giving the number
of digits in the state-file suffix. Width 2 → `R.A00`..`R.A99`.
Legal range `1..=255` (`mili.c:2396`, error
`INVALID_SUFFIX_WIDTH`). Overflow is **not** handled automatically:
writing past the suffix range is an error rather than a silent
expansion to a wider suffix. The Rust writer must surface this
error explicitly. Filename printf format is `%0*d`
(`mili_util.c:881`).

### Directory commit count (header field `COMMIT_COUNT_IDX`)

Incremented on every directory flush (`direc.c:204, 215`). The
reader tracks the highest commit number observed so subsequent
writes start one higher (`direc.c:562-564`). No formal validation;
it imposes a temporal order on directory contents and lets a
writer detect that another commit happened concurrently.

### Header extension fields (header byte 15)

`HDR_NUM_EXT_FIELDS_IDX` declares the count of trailing 4-byte
extension fields after the 16-byte char header (`mili_enum.h:422`).
Forward-compatibility hook; **always 0** in the reference corpus.
Treat any non-zero value as a hard error in the Rust port for now
— if a real fixture surfaces extension fields, we widen the parser
then.

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

## Numeric types (`mili.h:54-60`, `mili.c:140-141`, `dep.c:84-181`)

| Code      | On-disk size | Rust type        |
|-----------|-------------:|------------------|
| M_STRING  | variable     | `&[u8]` (NUL-term) |
| M_FLOAT4  | 4            | `f32`            |
| M_FLOAT8  | 8            | `f64`            |
| M_INT4    | 4            | `i32`            |
| M_INT8    | 8            | `i64`            |
| M_CHAR    | 1            | `u8`             |

`M_FLOAT` and `M_INT` are platform-native aliases. Their resolution
to a concrete width is **global to the database**, set by the
header's `MILI_PRECISION_LIMIT_IDX` byte (byte 7) and applied via
`fam->external_type[]` function pointers configured in
`set_default_io_routines()` (`dep.c:84-181`). Legal values
(`mili_enum.h:68-74`):

| Value | Name                | Effect on `M_FLOAT` / `M_INT`            |
|------:|---------------------|------------------------------------------|
| 0     | `PREC_LIMIT_NULL`   | undefined; treat as error                |
| 1     | `PREC_LIMIT_SINGLE` | `M_FLOAT` = 4 bytes, `M_INT` = 4 bytes   |
| 2     | `PREC_LIMIT_DOUBLE` | `M_FLOAT` = 4 bytes, `M_INT` = 4 bytes (resolved — see below) |
| 3     | `PREC_LIMIT_QUAD`   | reserved; not in use                     |
| 4     | `PREC_LIMIT_NONE`   | strict-explicit-types mode               |

**Resolved**: under `PREC_LIMIT_DOUBLE` the C lib still resolves
`M_FLOAT` to 4 bytes and `M_INT` to 4 bytes — the SINGLE and DOUBLE
arms of `set_default_io_routines` (`dep.c:100-244`) populate
`fam->external_size[]` identically. Double precision is opt-in per
svar via an explicit `M_FLOAT8` (or `M_INT8`) `num_type` in
`STATE_VAR_DICT`, not via promotion of the `M_FLOAT` alias. Verified
on the `dbl_nodtang` fixture: header byte 7 is `0x02`
(PREC_LIMIT_DOUBLE), and `db.nodes().dtype` is `float32` while the
explicit `nodtang` svar (declared `M_FLOAT8`) reads back as
`float64`.

**Implication for `mili-rs`:** resolve `M_FLOAT`/`M_INT` to a concrete
width once at open time based on the precision-limit byte, and store
the resolved width in `Database` so downstream code never sees the
ambiguous aliases. We will not emit `M_FLOAT`/`M_INT` on write; we
always serialize the explicit form to keep produced files
unambiguous.

No packed, varint, or compressed encodings exist. Every numeric block
on disk is a plain C array of the declared width.

## Superclass table (`mili.h:65-81, 88`)

| Code | Name        | conn_words | Notes                                  |
|-----:|-------------|-----------:|----------------------------------------|
| 0    | M_UNIT      | 0          | sentinel; no geometry                  |
| 1    | M_NODE      | 0          | nodal "elements" — coordinates only    |
| 2    | M_TRUSS     | 4          |                                        |
| 3    | M_BEAM      | 5          |                                        |
| 4    | M_TRI       | 5          |                                        |
| 6    | M_TET       | 6          | (5 is intentionally skipped upstream)  |
| 7    | M_PYRAMID   | 7          |                                        |
| 8    | M_WEDGE     | 8          |                                        |
| 9    | M_HEX       | 10         | 8 corner nodes + 2 metadata words      |
| 10   | M_MAT       | 0          | material pseudo-class                  |
| 11   | M_MESH      | 0          | mesh-level pseudo-class                |
| 12   | M_SURFACE   | 0          | surfaces; connectivity in SURFACE_CONNS|
| 13   | M_PARTICLE  | 3          |                                        |
| 14   | M_TET10     | 12         | quadratic tet                          |
| 15   | M_INODE     | 3          | interface node                         |

The `conn_words` counts above include any per-element trailing
metadata that the C library stores alongside the bare node IDs. The
"`Glob`" class the Python API exposes uses the M_MESH or M_MAT
superclass (one element representing the global state).

Node-ordering conventions are not documented in code; we preserve
whatever order we read.

## Entry types in the directory

This is the complete set of values that can appear in a directory
entry's `TYPE` field. Byte-level payload schemas for each one are
in [`entry-payloads.md`](entry-payloads.md).

| Type             | Payload contents (high level)                       |
|------------------|-----------------------------------------------------|
| `NODES`          | `start_node`, `stop_node`, then dense coords        |
| `ELEM_CONNS`     | `superclass`, `qty_blocks`, block ranges, conn data |
| `CLASS_DEF`      | **empty payload**; data is in MODIFIER1 + name pool |
| `CLASS_IDENTS`   | `superclass`, `start_id`, `stop_id` — the id-range table for one class within a mesh |
| `STATE_VAR_DICT` | dual integer+character streams describing svars     |
| `STATE_REC_DATA` | dual streams describing srecs and their subrecords  |
| `MILI_PARAM`     | scalar/array/string named parameter                 |
| `APPLICATION_PARAM` | same encoding as MILI_PARAM, semantically distinct |
| `TI_PARAM`       | same encoding, lives in the separate `R.ATI*` files |
| `SURFACE_CONNS`  | `superclass`, facet-block list, facet conn data     |

**We initially missed `CLASS_IDENTS`** in the survey — it is a
separate entry type that defines the `[start_id, stop_id]` range for
each object class within a mesh. The CLASS_DEF entry alone is not
enough to know how many elements exist; CLASS_IDENTS is required.

### TI_PARAM location (directory-version-dependent)

`TI_PARAM` is a directory entry **type**, not a separate file
section. Where the entries physically live depends on the directory
version:

- **Directory v2 and v3** — every fixture in our corpus — write
  `TI_PARAM` entries inline in the main `.A` directory. They share
  the same param hash table as `MILI_PARAM` and `APPLICATION_PARAM`
  (`direc.c:653-689`: the param-table inclusion condition is
  `etype == MILI_PARAM || etype == APPLICATION_PARAM ||
  (DIR_VERSION_IDX >= 2 && etype == TI_PARAM)`). The TI read API
  short-circuits to the regular reader whenever the directory
  version is `> 1` (`ti.c:179-212, 298-341`).
  Verified against the `basic1.pltA` fixture: TI_PARAM entries
  (indices 21, 22, 25, 26, …) have offsets within the main `.A`
  file and no `*_TI_*` companion file exists alongside the database.

- **Directory v1** (deferred in the Rust port) writes `TI_PARAM`
  entries to a separate `<root>_TI_<base26>` family of files
  (`mili_util.c:908-911`), each with its own trailer-style
  directory parsed by `tidirc.c:380-643`. The trailer omits
  `QTY_STATES` (one fewer header word) but is otherwise identical
  in shape to the main directory.

Consequence for the Rust port: the param-decode and TI accessor
code paths can target the main directory exclusively for v2+
databases. A v1-only `ti.rs` path enumerates `<root>_TI_*` files
and loads their directories via the same parser, but is not
exercised until v1 support lands.

### TI_PARAM-as-storage pattern

Several high-level concepts that the Python API exposes as
first-class objects do **not** have their own entry type. They are
named TI_PARAM payloads, identified by naming convention:

| Concept            | TI_PARAM name pattern                                            |
|--------------------|------------------------------------------------------------------|
| Node labels        | `Node Labels<descriptor>`                                        |
| Per-class labels   | `Element Labels<descriptor>` (filter out names containing `ElemIds`) |
| Per-class elem ids | `Element Labels-ElemIds<descriptor>` (written but unused by mili-python) |
| Materials          | `MAT_NAME_<n>` (one entry per material)                          |
| Element sets       | `IntLabel_es_<setname>`                                          |
| Mesh dimensions    | scalar param named `"mesh dimensions"`                           |
| Partition limits   | scalars `"states per file"`, `"max size per file"`               |

`<descriptor>` is the fixed-format suffix built by
`ti_make_label_description` (`ti.c:879-884`):

```
[/Mesh-<meshid>/Sname-<classname>/Scls-<superclass_name>/Mat-<matid>/]
```

`matid` is a per-class monotonic counter (`label_class_list[].last_matid`),
starting at 0 and bumped on every `mc_def_conn_labels` call for the
same class (`mesh_u.c:1602-1627`). It is **not** a material number in
the physics sense — it just serializes repeated label-definition
calls for the same class.

The reader recipe (`miliinternal.py:96-106`) is therefore:

- For each TI_PARAM whose name starts with `"Node Labels"`, attach
  the payload to the `"node"` class.
- For each TI_PARAM whose name starts with `"Element Labels"` and
  does **not** contain `"ElemIds"`, parse `Sname-(\w+)` out of the
  descriptor to identify the class, then **concatenate** all matching
  payloads (in directory order) into one label array for that class.
- TI_PARAMs whose name contains `"Element Labels-ElemIds"` are
  ignored on read — they exist only for Xmilics/griz compatibility.

### Label-material trailing convention (resolved)

The file-header comment at `mesh_u.c:29-31` ("material numbers added
to end of label list") is **stale**. The current writer
(`mc_def_conn_labels`, `mesh_u.c:1556-1678`) emits labels and local
element ids as two **separate** TI arrays of equal length `qty`, not
one concatenated `2 * qty` array with a split point. The `qty * 2`
allocation at `mesh_u.c:1196` and `mesh_u.c:1473` is a leftover; only
the first `qty` slots are populated and only the first `qty` slots
are read back (the `mc_ti_wrt_array` call sets `dims[0] = qty`).
Physical material numbers live entirely in separate `MAT_NAME_<n>`
TI params.

The Rust port therefore does **not** need to split label payloads.
The `Element Labels` array for a given class is the full label
sequence, no trailer to strip.

The Rust reader exposes these as typed accessors
(`db.labels(class)`, `db.materials()`, …) that look up the relevant
TI param name and parse the payload. The Python API's
`materials()`, `material_numbers()`, `element_sets()`,
`integration_points()`, and `labels()` are all built on this
pattern (`reference/mili-python/src/mili/miliinternal.py:96-119,
198-211, 463-474`).

Element-set values have an internal convention: the payload is an
`i32` array whose last entry is a count and whose preceding entries
are the integration-point IDs for that set
(`miliinternal.py:113-115`). This format is **not enforced by the
on-disk layout** — it is a contract the Python lib relies on. The
Rust reader has to honor the same convention.

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
  Carries `num_type`, `agg_type` (`SCALAR`=0, `VECTOR`=1, `ARRAY`=2,
  `VEC_ARRAY`=3), `rank`, `dims[]`, optional `components[]`.
  Components of vector svars are stored as a concatenated stream of
  NUL-terminated names **inside** the STATE_VAR_DICT character
  payload, not as separate svar entries and not in the file-level
  name pool (`svar.c:341-352`).
- **Subrecord**: a grouping of svars over a particular object class.
  Has an `organization` byte (`subrec_layout` enum,
  `mili_enum.h:41-45`): `RESULT_ORDERED` = 0, `OBJECT_ORDERED` = 1.
- **Srec** (state record format): the schema for a state. Multiple
  srec formats can coexist; each state is tagged with one.
- **State**: per-state file offset + time + srec format ID
  (`State_descriptor`, `mili_internal.h:109-115`). Inside the state
  file, each subrecord is a contiguous byte range. The
  `lump_offsets[]`, `lump_sizes[]`, `lump_atoms[]` arrays on
  `Sub_srec` (`mili_internal.h:215-250`) are **derived at read
  time** from svar atom counts and the subrecord's object id blocks
  (`srec.c:1409+`) — they are not written to disk.

### Subrecord byte-layout matrix

For a subrecord with `K` svars over `N` objects, the byte order
depends on `organization` and on each svar's `agg_type`. The
following enumerates every cell of the matrix because this is the
single biggest place where a port can silently produce
wrong-shaped arrays.

**RESULT_ORDERED (organization = 0):** the K svars are laid out
serially; within each svar's region, the N objects are serial;
within each object's slot, the svar's atoms are serial.

```
[ svar_0: obj_0_atoms | obj_1_atoms | … | obj_{N-1}_atoms ]
[ svar_1: obj_0_atoms | obj_1_atoms | … | obj_{N-1}_atoms ]
…
[ svar_{K-1}: … ]
```

Per-svar atom counts by `agg_type`:

| `agg_type`    | atoms per object                                     |
|---------------|------------------------------------------------------|
| `SCALAR`      | 1                                                    |
| `VECTOR`      | `list_size` (number of components)                   |
| `ARRAY`       | `prod(dims)`                                         |
| `VEC_ARRAY`   | `prod(dims) * list_size`                             |

For `VEC_ARRAY` the inner order is: array-dim indices vary fastest,
then component (vector) index, then integration-point index — i.e.
"IP axis is the slowest-varying after the array dims, before
components" (`srec.c:1908, 3018, 4263`). **TODO**: confirm against
the `vecarray` test fixture before we lock this in. The C code's
own offset math is the ground truth; a unit test against
`vecarray` is on the M5 checklist.

**OBJECT_ORDERED (organization = 1):** the N objects are laid out
serially; within each object's region, the K svars are serial in
declaration order; within each svar slot, atoms are serial per the
table above.

```
[ obj_0: svar_0_atoms | svar_1_atoms | … | svar_{K-1}_atoms ]
[ obj_1: svar_0_atoms | svar_1_atoms | … | svar_{K-1}_atoms ]
…
[ obj_{N-1}: … ]
```

The mixed-svar case the Python tests pin down
(`reference/mili-python/tests/test_bugfixes.py:119-172`) is a
VEC_ARRAY whose components are not all the same width — a stress
tensor plus a scalar plastic-strain in the same vec-array. The
layout is still: per object → per IP → per component, with each
component contributing its own width. The reader has to walk the
component list to compute per-IP byte size, not multiply
component-count by a uniform width.

### State end marker (`mili.c:276`, `mili_statemap.c:535, 602-603, 779, 811`)

A single ASCII `~` byte (0x7E) is written to the time-state file
(`R.A.tfile`) after the state map entries to mark EOF. On read, a
mismatch triggers `rebuild_state_tfile`. The marker is per-file,
not per-state; it is **not** written into the `.A0N` state files
themselves. The Rust writer must emit it when finalizing the
tfile.

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
