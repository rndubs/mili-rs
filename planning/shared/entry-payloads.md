# Directory entry payloads — byte-level reference

For every directory entry type, what's in the bytes that
`OFFSET..OFFSET+LENGTH` points at. Keep this open while writing
`directory.rs` and the entry-type-specific parsers.

Conventions: `M_INT` and `M_FLOAT` resolve to a concrete width
based on the header's precision-limit byte (see
`format.md` § "Numeric types"). All integers and floats are
endianness-sensitive per the header's endianness byte.

---

## `NODES` (`mesh_u.c:710-737`, reader `mesh_u.c:3971-3989`)

```
[ M_INT ]      start_node
[ M_INT ]      stop_node
[ N × M_FLOAT ] coordinates       — N = (stop_node - start_node + 1) × fam->dimensions
```

Directory entry: `MODIFIER1` = mesh_id, `MODIFIER2` = node count,
`STRING_QTY` = 1 (the nodal class short_name).

`fam->dimensions` is **not** stored in this entry; it lives in a
named scalar param `"mesh dimensions"` (`mesh_u.c:296-299, 3900`).
A reader must load that param before it can compute N.

---

## `ELEM_CONNS` (`mesh_u.c:1026-1039` for the contiguous form,
`mesh_u.c:1130-1146` for the arbitrary form)

Both forms share a header and differ only in the block list:

```
[ M_INT ]                 superclass code         — index 0
[ M_INT ]                 qty_blocks              — index 1
[ qty_blocks × 2 × M_INT ] block ranges (start, stop pairs)
[ K × M_INT ]             connectivity            — K = (Σ block_lengths) × conn_words[superclass]
```

- Contiguous form (`mc_def_conn`): `qty_blocks` = 1.
- Arbitrary form (`mc_def_conn_arb`): `qty_blocks` ≥ 1, derived
  from `list_to_blocks` coalescing a user-supplied id list.

**Material ids, part numbers, and labels are NOT stored in this
payload.** The misleading comment at `mesh_u.c:1052-1056` suggests
otherwise; the actual payload is only superclass + block ranges +
connectivity. Materials and labels live in TI_PARAM entries (see
`format.md` § "TI_PARAM-as-storage pattern").

Directory entry: `MODIFIER1` = mesh_id, `MODIFIER2` = element
count, `STRING_QTY` = 1 (element class short_name).

---

## `CLASS_DEF` (`mesh_u.c:480-548`, reader `mesh_u.c:4138-4165`)

**Empty payload** — `LENGTH` is `DONT_CARE`. All the information is
in the directory entry itself:

- `MODIFIER1` = superclass code (`M_NODE`, `M_HEX`, …).
- `MODIFIER2` = superclass code (redundant).
- `STRING_QTY` = 2 → consumes two names from the name pool:
  short_name (first), long_name (second).

A reader builds an `ObjectClass` from these four fields plus the
element count, which comes from the matching `CLASS_IDENTS` entry.

---

## `CLASS_IDENTS` (`mesh_u.c:559-637`, reader `mesh_u.c:4062-4095`)

```
[ M_INT ]   superclass
[ M_INT ]   start_id
[ M_INT ]   stop_id
```

Defines the `[start_id, stop_id]` inclusive range for one class
within one mesh. **Required** to know the element count for a
class — `CLASS_DEF` alone is not enough.

Directory entry: `MODIFIER1` = mesh_id, `MODIFIER2` = element
count `(stop_id - start_id + 1)`, `STRING_QTY` = 1 (class
short_name).

Multiple `CLASS_IDENTS` for the same class are allowed and represent
a non-contiguous id range (analogous to `ELEM_CONNS`'s block list).

---

## `STATE_VAR_DICT` (`svar.c:920-997`, reader `svar.c:1134-1175`)

Two streams written consecutively in one payload: an integer
stream then a character stream. The directory entry's
`MODIFIER1` and `MODIFIER2` are `DONT_CARE`, `STRING_QTY` = 0.

### Integer stream

```
[ M_INT ]   SVAR_QTY_INT_WORDS          — total int words in this stream
[ M_INT ]   SVAR_QTY_BYTES              — total char bytes in the char stream
{ for each svar definition:
   [ M_INT ]   agg_type     — 0=SCALAR, 1=VECTOR, 2=ARRAY, 3=VEC_ARRAY
   [ M_INT ]   data_type    — M_INT4 / M_INT8 / M_FLOAT4 / M_FLOAT8
   SCALAR:
       — no further integers
   VECTOR:
       [ M_INT ]   list_size       — component count
   ARRAY:
       [ M_INT ]   rank
       [ rank × M_INT ]  dims
   VEC_ARRAY:
       [ M_INT ]   rank
       [ rank × M_INT ]  dims
       [ M_INT ]   list_size       — component count of the inner vector
}
```

### Character stream

Concatenated NUL-terminated strings in this order, padded out to
`SVAR_QTY_BYTES`:

```
for each svar:
    svar_name\0
    svar_title\0
    if VECTOR or VEC_ARRAY:
        component_name_0\0 component_name_1\0 … component_name_{list_size-1}\0
```

**Component names live here, not in the file-level name pool.**
The Rust reader must keep the character stream alongside the
integer stream while parsing — the streams are read in lockstep.

A subrecord that references a vector svar component (e.g. user
queries `"sx"`) must resolve the component name against this
embedded stream, not against the global name pool.

---

## `STATE_REC_DATA` (`srec.c:1298-1379`, reader `srec.c:1429+`)

Same two-stream pattern as STATE_VAR_DICT, but per srec format.

Directory entry: `MODIFIER1` = qty_int_words, `MODIFIER2` =
qty_char_bytes (rounded up to a 4-byte boundary), `STRING_QTY` = 0.

### Integer stream

```
[ M_INT ]   srec_id
[ M_INT ]   mesh_id
[ M_INT ]   srec size              — total bytes per state for this srec
[ M_INT ]   qty_subrecs
{ for each subrecord:
    [ M_INT ]   organization                  — 0=RESULT_ORDERED, 1=OBJECT_ORDERED
    [ M_INT ]   qty_svars                     — count of svar names that follow in char stream
    [ M_INT ]   qty_id_blks                   — count of object id ranges
    [ qty_id_blks × 2 × M_INT ]   id blocks   — (start, stop) pairs
    if surface_variable_flag is present:
        [ M_INT ]   svar_atoms
        [ svar_atoms × M_INT ]   surface_variable_flag
}
```

### Character stream

```
for each subrecord:
    subrec_name\0
    mclass_name\0
    svar_name_0\0 svar_name_1\0 … svar_name_{qty_svars-1}\0
```

The subrecord's mesh class and svar list are referenced **by name
only**. At read time the names are resolved through:
- the class table (`subrec.mclass` → class id)
- the svar hash table (`subrec.svars[i]` → svar id)

`lump_offsets`, `lump_sizes`, and `lump_atoms` are **derived** at
load time from the subrecord's id-block list, the resolved svars'
atom counts, and the organization. A reader must replicate this
derivation; see `srec.c:1409+` for the canonical algorithm. We do
not write these to disk.

---

## `MILI_PARAM`, `APPLICATION_PARAM`, `TI_PARAM` (`param.c:219-333,
515-650, 986-1066`)

Three entry types, **identical encoding**. The distinction is
semantic:

- `MILI_PARAM` — internal mili bookkeeping (partition limits,
  mesh dimensions, …).
- `APPLICATION_PARAM` — caller-defined parameters.
- `TI_PARAM` — same encoding but lives in the separate `R.ATI*`
  files, not in the main `.A` file. Labels, materials, element
  sets are TI_PARAMs.

Directory entry common fields: `MODIFIER1` = data type code (M_INT,
M_FLOAT, M_STRING, …), `MODIFIER2` = aggregation code (SCALAR,
ARRAY, or `DONT_CARE` for strings), `STRING_QTY` = 1 (the param
name).

### Scalar (`MODIFIER2 = SCALAR`)

```
[ M_INT or M_FLOAT or … ]   one value of width = sizeof(MODIFIER1 type)
```

### String (`MODIFIER1 = M_STRING`, `MODIFIER2 = DONT_CARE`)

```
[ rounded_length × M_CHAR ]   NUL-terminated, total length rounded up to an 8-byte boundary (param.c:530)
```

The 8-byte padding is purely for alignment of the next directory
entry's data; reader code must trim the trailing NULs to recover
the logical string.

### Array (`MODIFIER2 = ARRAY`)

```
[ M_INT ]              order               — rank, number of dims
[ order × M_INT ]      dims
[ atoms × type ]       data                — atoms = prod(dims), type = MODIFIER1
```

---

## `SURFACE_CONNS` (`mesh_u.c:1793-1804`, reader `mesh_u.c:4167-4205`)

Same shape as `ELEM_CONNS`:

```
[ M_INT ]                superclass code
[ M_INT ]                qty_blocks               — = qty_of_facets
[ K × M_INT ]            connectivity data
```

`MODIFIER1` = mesh_id, `MODIFIER2` = qty_of_facets, `STRING_QTY` = 1
(surface class short_name).

Rare in current databases. Defer behind a feature flag in the Rust
reader until a real fixture exercises it; emit an explicit
`MiliError::UnsupportedSurfaceConns` rather than partial parsing.

---

## Directory-entry summary table

| TYPE             | MODIFIER1    | MODIFIER2     | STRING_QTY | LENGTH semantics                              |
|------------------|--------------|---------------|-----------:|-----------------------------------------------|
| NODES            | mesh_id      | node count    | 1          | `2 * sizeof(M_INT) + N * sizeof(M_FLOAT)`     |
| ELEM_CONNS       | mesh_id      | element count | 1          | header + block list + connectivity            |
| CLASS_DEF        | superclass   | superclass    | 2          | 0 — empty payload                             |
| CLASS_IDENTS     | mesh_id      | element count | 1          | `3 * sizeof(M_INT)`                           |
| STATE_VAR_DICT   | DONT_CARE    | DONT_CARE     | 0          | int stream bytes + char stream bytes          |
| STATE_REC_DATA   | int words    | char bytes    | 0          | computed from the two stream lengths          |
| MILI_PARAM       | data type    | SCALAR/ARRAY  | 1          | scalar/array/string by case (above)           |
| APPLICATION_PARAM| data type    | SCALAR/ARRAY  | 1          | same as MILI_PARAM                            |
| TI_PARAM         | data type    | SCALAR/ARRAY  | 1          | same as MILI_PARAM (separate file)            |
| SURFACE_CONNS    | mesh_id      | facet count   | 1          | header + facet block list + facet conn data   |
