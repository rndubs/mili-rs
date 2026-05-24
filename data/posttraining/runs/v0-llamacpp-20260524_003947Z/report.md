# mili-llm-bench v0 baseline report

**L3 pass rate: 0.000 (0.0%) — 0 / 50 scenarios.**

* provider: `llamacpp`
* model: `ggml-org/functiongemma-270m-it-GGUF:BF16`
* system_prompt_sha256: `1697d2f0444adbf5`
* scenarios_sha256: `216a54d0cd36a47c1785527bd1ba7e8b07617d5933efb7ba06dd9d3b2260eb45`

## by_max_tier

| tier | count | pct |
|------|-------|-----|
| 0 | 32 | 64.0% |
| 1 | 0 | 0.0% |
| 2 | 18 | 36.0% |
| 3 | 0 | 0.0% |

## by_failure_mode

Sorted by count desc, then name asc. Every closed-set entry appears (zero-init) so a missing mode is structurally impossible.

| failure_mode | count |
|--------------|-------|
| parse_error | 32 |
| step_cap_hit | 18 |
| dispatch_error | 0 |
| nonexistent_class | 0 |
| nonexistent_material | 0 |
| nonexistent_result | 0 |
| schema_mismatch | 0 |
| state_out_of_range | 0 |
| timeout | 0 |
| token_cap_hit | 0 |
| unknown_tool | 0 |
| wrong_final_state | 0 |
| wrong_materials | 0 |
| wrong_range | 0 |
| wrong_result | 0 |
| wrong_selection | 0 |

## timing

* mean turns to completion: **3.52**
* total wall clock: **4980388 ms** (4980.39 s)

## per_intent

L3 pass rate broken down by intent_id; the post-v0 decision tree (baseline.md §"After v0") branches on this.

| intent_id | count | l3 | l3_rate |
|-----------|-------|----|---------|
| clrsel | 4 | 0 | 0.0% |
| colormap | 4 | 0 | 0.0% |
| compound | 1 | 0 | 0.0% |
| load | 6 | 0 | 0.0% |
| material | 6 | 0 | 0.0% |
| select | 4 | 0 | 0.0% |
| set-state | 6 | 0 | 0.0% |
| show-derived | 4 | 0 | 0.0% |
| show-primal | 6 | 0 | 0.0% |
| step | 6 | 0 | 0.0% |
| view-reset | 3 | 0 | 0.0% |

## raw_fallback_rate

Rollouts containing at least one `griz_raw` call: **2 / 50** (4.0%). The v0 verifier treats `griz_raw` as a fair pass; this rate tells us how often the model bypassed the typed-tool surface.

## artifacts

* rollouts: `data/posttraining/runs/v0-llamacpp-20260524_003947Z/rollouts.jsonl`
* summary: `data/posttraining/runs/v0-llamacpp-20260524_003947Z/summary.json`
* config: `data/posttraining/runs/v0-llamacpp-20260524_003947Z/config.yaml`
