# Stage 3 synthesis report

- seed: `42`
- total scenarios: **175**
- compound scenarios: **41** (ratio 23.43%; ≥20% gate)

## paraphrase source breakdown

- `manual-paraphrase`: 106
- `template`: 69

## per-cell count

| intent_id | fixture | count |
| --- | --- | --- |
| `load` | `cylinder` | 3 |
| `load` | `d3samp6` | 3 |
| `set-state` | `cylinder` | 9 |
| `set-state` | `d3samp6` | 9 |
| `step` | `cylinder` | 4 |
| `step` | `d3samp6` | 4 |
| `select` | `cylinder` | 8 |
| `select` | `d3samp6` | 8 |
| `clrsel` | `cylinder` | 2 |
| `clrsel` | `d3samp6` | 4 |
| `show-primal` | `cylinder` | 9 |
| `show-primal` | `d3samp6` | 9 |
| `show-derived` | `cylinder` | 3 |
| `show-derived` | `d3samp6` | 5 |
| `material` | `cylinder` | 8 |
| `material` | `d3samp6` | 12 |
| `view-reset` | `cylinder` | 2 |
| `view-reset` | `d3samp6` | 2 |
| `colormap` | `cylinder` | 9 |
| `colormap` | `d3samp6` | 9 |
| `query` | `cylinder` | 6 |
| `query` | `d3samp6` | 6 |
| `compound-material-then-show` | `cylinder` | 6 |
| `compound-material-then-show` | `d3samp6` | 7 |
| `compound-select-then-show` | `cylinder` | 7 |
| `compound-select-then-show` | `d3samp6` | 7 |
| `compound-state-then-show` | `cylinder` | 7 |
| `compound-state-then-show` | `d3samp6` | 7 |

## fixture-fact confirmation

- d3samp6: num_states=101 ✓, classes ⊇ ['brick', 'beam', 'shell', 'node'] ✓, mat_ids [1, 2, 3] ✓
- cylinder: num_states=11 ✓, classes ⊇ ['brick', 'node'] ✓, mat_ids [1, 2] ✓

## notes

- query oracle: live pygriz capture (per fixture)
