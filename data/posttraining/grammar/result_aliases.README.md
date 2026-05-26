# `result_aliases.json` — natural-language aliases for queriable result names

## Purpose

Maps each canonical svar exposed by `Database::queriable_svars` /
`Database::derived_variables_of_class` (the names the model is expected
to use in `show` / `query` / `iso` / `contour`) to:

- a short human-readable **description** (one line, plain English),
- a list of **aliases** — natural-language phrasings a user might say
  ("first principal stress" → `prin_stress1`).

Loaded at compile time by the Rust agent
([`crates/mili-viz-server/src/lib.rs`](../../../crates/mili-viz-server/src/lib.rs)
`result_alias_table()`) and surfaced through the agent-local
`list_results` tool (m7 Delta 5).

Also consumed by the M8 generation pipeline
([`m8-corpus-distillation.md` §"Artifact 1"](../../../planning/mili-viz/mili-agent/m8-corpus-distillation.md))
as the source of paraphrase phrasings the teacher uses when
constructing intermediate-tier SFT scenarios.

## Schema

```jsonc
[
  {
    "name": "prin_stress1",
    "type": "derived",
    "description": "First principal stress eigenvalue",
    "aliases": [
      "first principal stress",
      "principal stress 1",
      "max principal stress",
      "σ₁",
      "sigma_1"
    ]
  },
  {
    "name": "vel_x",
    "type": "primal",
    "description": "Velocity x component",
    "aliases": ["x velocity", "velocity x", "vx", "u-velocity"]
  }
]
```

- `name`: REQUIRED. The canonical svar name as it appears in the
  database catalog. Must be unique across the table.
- `type`: REQUIRED. One of `"primal"` or `"derived"`. Surfaced verbatim
  to the model in the `list_results` response so it can disambiguate.
- `description`: optional, defaults to `""`. One short sentence.
- `aliases`: optional, defaults to `[]`. List of natural-language
  phrasings. Lower-case is conventional; the lookup is exact on the
  user's instruction so include the casings users actually type.

## Current state

**Empty.** The file ships as `[]` so the Rust agent compiles and the
`list_results` tool surfaces canonical names without descriptions /
aliases. Populating it is **M8 Stage B1**:

1. Enumerate the canonical svar set across all test corpora (`d3samp6`,
   `basic1`, `cylinder`, `bar71`, etc.) by walking
   `db.queriable_svars(false, false)` and `db.derived_variables_of_class`
   for each fixture. Union → ~155 entries.
2. One-shot Gemma-4-31B (or Claude) draft pass: feed the canonical
   names and ask for ~5 plausible aliases each.
3. Human review with mili / griz domain knowledge — aliases are the
   seed for the corpus; bad aliases poison every scenario that uses
   them.
4. Unit test: every key unique; every name is in the test-corpus
   catalog (no orphans).

## Updating safely

The Rust agent reads this file with `include_str!` at compile time and
parses it lazily on first `list_results` emit. A malformed file is
**loud at startup** — the agent surface is load-bearing and silently
degrading to an empty table would hide regressions. Run
`cargo test -p mili-viz-server` after edits.

The Python bench's M8 synth pipeline parses the same file at runtime;
its validation layer will reject scenarios that reference aliases the
file does not declare.
