# Stage 3 synthesis report

- seed: `42`
- total scenarios: **163**
- compound scenarios: **41** (ratio 25.15%; ≥20% gate)

## paraphrase source breakdown

- `manual-paraphrase`: 98
- `template`: 65

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
| `query` | `cylinder` | 0 |
| `query` | `d3samp6` | 0 |
| `compound-material-then-show` | `cylinder` | 6 |
| `compound-material-then-show` | `d3samp6` | 7 |
| `compound-select-then-show` | `cylinder` | 7 |
| `compound-select-then-show` | `d3samp6` | 7 |
| `compound-state-then-show` | `cylinder` | 7 |
| `compound-state-then-show` | `d3samp6` | 7 |

## fixture-fact confirmation

- d3samp6: num_states=101 ✓, classes ⊇ ['brick', 'beam', 'shell', 'node'] ✓, mat_ids [1, 2, 3] ✓
- cylinder: num_states=11 ✓, classes ⊇ ['brick', 'node'] ✓, mat_ids [1, 2] ✓

## skipped rows

- query/d3samp6: resolution failed: AttributeError("'Session' object has no attribute 'query'")
- query/d3samp6: resolution failed: AttributeError("'Session' object has no attribute 'query'")
- query/d3samp6: resolution failed: AttributeError("'Session' object has no attribute 'query'")
- query/d3samp6: resolution failed: AttributeError("'Session' object has no attribute 'query'")
- query/d3samp6: resolution failed: AttributeError("'Session' object has no attribute 'query'")
- query/d3samp6: resolution failed: AttributeError("'Session' object has no attribute 'query'")
- query/cylinder: resolution failed: AttributeError("'Session' object has no attribute 'query'")
- query/cylinder: resolution failed: AttributeError("'Session' object has no attribute 'query'")
- query/cylinder: resolution failed: AttributeError("'Session' object has no attribute 'query'")
- query/cylinder: resolution failed: AttributeError("'Session' object has no attribute 'query'")
- query/cylinder: resolution failed: AttributeError("'Session' object has no attribute 'query'")
- query/cylinder: resolution failed: AttributeError("'Session' object has no attribute 'query'")

## notes

- query oracle: live pygriz capture (per fixture)
