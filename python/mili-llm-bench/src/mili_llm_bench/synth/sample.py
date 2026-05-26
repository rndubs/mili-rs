"""Param sampling + budget allocation for Stage 3.

One generator function per intent_id. Each generator takes a fixture
fact record + a seeded RNG and yields ``(bound, seed_idx, seed_text)``
tuples — fully grounded parameter dicts paired with the paraphrase seed
the synthesizer should render.

Per the design (see ``planning/mili-viz/mili-agent/m5-sft-pipeline.md``
Stage 3 row), the first paraphrase seed in the catalog is tagged
``template``; the rest are ``manual-paraphrase``.

Two intent-specific bindings the generators encode in code rather than
the catalog:

* ``material`` — paraphrase seed text carries the verb polarity
  (``disable/hide`` vs. ``enable/turn-on``). The generator pairs each
  ``enable_verb`` bool with the seed indices whose text matches.
* ``clrsel`` — Stage 3 only emits the class-set variants. The
  ``clear all`` paraphrases would hit the pre-existing pygriz
  ``selection.clear_all()`` gap (logged in the synth report and
  catalog ``todo_v2``).
"""

from __future__ import annotations

import random
from dataclasses import dataclass
from typing import Any, Callable, Iterator

from .catalog import Catalog, FixtureFacts, IntentRow

# Per-fixture-cell quotas. Tuned so atomic_total + compound_total
# ≈ 200 and compound / total ≥ 0.20. See the Stage 3 design proposal
# in the conversation history for the derivation.
ATOMIC_QUOTAS: dict[str, int] = {
    "load": 3,            # 1 fixture × 3 paraphrase seeds
    "set-state": 9,       # 3 state samples × 3 seeds
    "step": 9,            # 3 dirs (NEXT/FIRST/LAST) × 3 seeds
    "select": 8,          # 2 classes × 2 ranges × 2 seeds
    "clrsel": 6,          # 2 classes × 3 seeds (class-set variant only)
    "show-primal": 9,     # up to 3 svars × 3 seeds
    "show-derived": 6,    # up to 2 results × 3 seeds
    "material": 12,       # 3 mat_ids × 2 polarities × 2 seeds (clamped)
    "view-reset": 2,      # only 2 seeds, no params
    "colormap": 9,        # 3 colormaps × 3 seeds
    "query": 6,           # 2 (result, class) × 3 paraphrases
}
COMPOUND_QUOTA_PER_CELL = 7  # 3 compounds × 2 fixtures × 7 = 42

# Safe per-fixture select ranges, mined from bootstrap.jsonl + confirmed
# by Stage 3's live-load probe. Each entry is (class_name, range_spec)
# pairs known to dispatch cleanly under pygriz.
SELECT_SAFE_RANGES: dict[str, tuple[tuple[str, str], ...]] = {
    "d3samp6": (
        ("brick", "1-10"),
        ("brick", "1"),
        ("beam", "1"),
        ("shell", "1"),
    ),
    "cylinder": (
        ("brick", "1-5"),
        ("brick", "7"),
        ("brick", "1"),
        ("node", "1"),
    ),
}

# Polarity → catalog seed indices for ``material``. Index 0 ("disable")
# is the canonical template; index 2 ("enable") matches the affirm
# polarity. The catalog seeds the synthesizer reads:
#   [0] "disable material {mat_id}"   -> enable=False (template)
#   [1] "hide material {mat_id}"      -> enable=False (manual-paraphrase)
#   [2] "enable material {mat_id}"    -> enable=True  (manual-paraphrase)
#   [3] "turn material {mat_id} on"   -> enable=True  (manual-paraphrase)
MATERIAL_SEED_BY_POLARITY: dict[bool, tuple[int, ...]] = {
    False: (0, 1),
    True: (2, 3),
}

# Direction → catalog seed index for ``step``. Catalog seeds:
#   [0] "step to the next state"      -> NEXT (template)
#   [1] "go to the last state"        -> LAST
#   [2] "rewind to the first state"   -> FIRST
#   [3] "advance to the next state"   -> NEXT (manual-paraphrase)
STEP_SEED_BY_DIR: dict[str, tuple[int, ...]] = {
    "NEXT": (0, 3),
    "LAST": (1,),
    "FIRST": (2,),
}

# ``clrsel`` class-set seeds — drop the catalog seeds that read
# "deselect everything" / "clear all selections" because the underlying
# dispatcher path (``selection.clear_all()``) is missing in pygriz today
# (the same gap that produced the bootstrap Claude 2× clrsel
# dispatch_error). Only seed 0 ("clear the {class_or_empty} selection")
# uses the class slot.
CLRSEL_CLASS_SEEDS: tuple[int, ...] = (0,)

# Colormap palette names — narrow to the trio that produces the cleanest
# paraphrases at this scale; sampler picks 3 per cell.
COLORMAP_SAMPLE_VALUES: tuple[str, ...] = ("cool", "jet", "viridis")

# Query sampling: pick (result, class) pairs that probe the read path
# without depending on labels beyond what bootstrap's fixture probes
# exercised. Labels stay at [1] and states at the current state ([1]).
QUERY_PROBES: dict[str, tuple[tuple[str, str, str], ...]] = {
    "d3samp6": (
        ("sx", "brick", "1"),
        ("vx", "brick", "1"),
    ),
    "cylinder": (
        ("sx", "brick", "1"),
        ("vy", "brick", "1"),
    ),
}


@dataclass(frozen=True)
class SampledTuple:
    """One sampled scenario seed: bound params + paraphrase + tag."""

    intent_id: str
    fixture: str
    bound: dict[str, Any]
    seed_idx: int
    seed_text: str
    instruction_source: str  # "template" or "manual-paraphrase"


# Per-intent generator signature:
#   (intent, fixture, rng, quota) -> iterator of (bound, seed_idx)
_TupleGen = Callable[
    [IntentRow, FixtureFacts, random.Random, int],
    Iterator[tuple[dict[str, Any], int]],
]


def _gen_load(_intent, fixture, _rng, quota):
    for idx in range(quota):
        yield ({"fixture_name": fixture.name}, idx)


def _gen_set_state(intent, fixture, rng, quota):
    # 3 distinct state samples × 3 paraphrase seeds = 9 per cell.
    state_pool = _state_pool(fixture)
    states = _rng_sample(rng, state_pool, k=min(3, len(state_pool)))
    n_seeds = len(intent.paraphrase_seeds)
    emitted = 0
    for st in states:
        for sidx in range(n_seeds):
            if emitted >= quota:
                return
            yield ({"state_num": st}, sidx)
            emitted += 1


def _gen_step(intent, _fixture, _rng, quota):
    emitted = 0
    for direction, seed_idxs in STEP_SEED_BY_DIR.items():
        for sidx in seed_idxs:
            if emitted >= quota:
                return
            yield ({"dir": direction}, sidx)
            emitted += 1


def _gen_select(_intent, fixture, _rng, quota):
    pairs = SELECT_SAFE_RANGES.get(fixture.name, ())
    n_seeds = 2  # seed indices 0 and 1
    emitted = 0
    for class_name, range_spec in pairs:
        for sidx in range(n_seeds):
            if emitted >= quota:
                return
            yield ({"class": class_name, "range_spec": range_spec}, sidx)
            emitted += 1


def _gen_clrsel(_intent, fixture, _rng, quota):
    emitted = 0
    for cls in fixture.classes:
        for sidx in CLRSEL_CLASS_SEEDS:
            if emitted >= quota:
                return
            yield ({"class_or_empty": cls}, sidx)
            emitted += 1


def _gen_show_primal(intent, fixture, _rng, quota):
    n_seeds = len(intent.paraphrase_seeds)
    emitted = 0
    for result in fixture.primal_svars:
        for sidx in range(n_seeds):
            if emitted >= quota:
                return
            yield ({"result_name": result}, sidx)
            emitted += 1


def _gen_show_derived(intent, fixture, _rng, quota):
    # Catalog seed 1 is slot-free ("color the mesh by effective stress")
    # — it teaches the prose↔symbol mapping for ``eff_stress``. Pair
    # it only with that specific result, never with ``pressure`` etc.,
    # so the rendered instruction always agrees with the postcondition.
    n_seeds = len(intent.paraphrase_seeds)
    emitted = 0
    for result in fixture.derived_results:
        for sidx in range(n_seeds):
            if emitted >= quota:
                return
            seed_text = intent.paraphrase_seeds[sidx]
            if "{result_name}" not in seed_text and result != "eff_stress":
                continue
            yield ({"result_name": result}, sidx)
            emitted += 1


def _gen_material(_intent, fixture, _rng, quota):
    emitted = 0
    for mat_id in fixture.material_ids:
        for enable, seed_idxs in MATERIAL_SEED_BY_POLARITY.items():
            for sidx in seed_idxs:
                if emitted >= quota:
                    return
                yield ({"mat_id": mat_id, "enable_verb": enable}, sidx)
                emitted += 1


def _gen_view_reset(intent, _fixture, _rng, quota):
    n = min(quota, len(intent.paraphrase_seeds))
    for sidx in range(n):
        yield ({}, sidx)


def _gen_colormap(_intent, _fixture, _rng, quota):
    emitted = 0
    for cm in COLORMAP_SAMPLE_VALUES:
        for sidx in range(3):  # 3 paraphrase seeds per colormap
            if emitted >= quota:
                return
            yield ({"colormap_name": cm}, sidx)
            emitted += 1


def _gen_query(intent, fixture, _rng, quota):
    n_seeds = len(intent.paraphrase_seeds)
    probes = QUERY_PROBES.get(fixture.name, ())
    emitted = 0
    for result, class_name, label in probes:
        for sidx in range(n_seeds):
            if emitted >= quota:
                return
            yield (
                {
                    "result_name": result,
                    "class": class_name,
                    "labels": [int(label)],
                    "states": [1],
                },
                sidx,
            )
            emitted += 1


def _gen_compound_material_then_show(_intent, fixture, _rng, quota):
    emitted = 0
    n_seeds = 3
    for mat_id in fixture.material_ids:
        for result in fixture.derived_results:
            for sidx in range(n_seeds):
                if emitted >= quota:
                    return
                yield ({"mat_id": mat_id, "result_name": result}, sidx)
                emitted += 1


def _gen_compound_select_then_show(_intent, fixture, _rng, quota):
    emitted = 0
    n_seeds = 2
    pairs = SELECT_SAFE_RANGES.get(fixture.name, ())
    result_pool = tuple(fixture.primal_svars) + tuple(fixture.derived_results)
    for (class_name, range_spec) in pairs:
        for result in result_pool:
            for sidx in range(n_seeds):
                if emitted >= quota:
                    return
                yield (
                    {
                        "class": class_name,
                        "range_spec": range_spec,
                        "result_name": result,
                    },
                    sidx,
                )
                emitted += 1


def _gen_compound_state_then_show(intent, fixture, rng, quota):
    state_pool = _state_pool(fixture)
    states = _rng_sample(rng, state_pool, k=min(2, len(state_pool)))
    result_pool = tuple(fixture.primal_svars) + tuple(fixture.derived_results)
    n_seeds = 3
    emitted = 0
    for st in states:
        for result in result_pool:
            for sidx in range(n_seeds):
                if emitted >= quota:
                    return
                yield (
                    {"state_num": st, "result_name": result},
                    sidx,
                )
                emitted += 1


_GENERATORS: dict[str, _TupleGen] = {
    "load": _gen_load,
    "set-state": _gen_set_state,
    "step": _gen_step,
    "select": _gen_select,
    "clrsel": _gen_clrsel,
    "show-primal": _gen_show_primal,
    "show-derived": _gen_show_derived,
    "material": _gen_material,
    "view-reset": _gen_view_reset,
    "colormap": _gen_colormap,
    "query": _gen_query,
    "compound-material-then-show": _gen_compound_material_then_show,
    "compound-select-then-show": _gen_compound_select_then_show,
    "compound-state-then-show": _gen_compound_state_then_show,
}


def _state_pool(fixture: FixtureFacts) -> tuple[int, ...]:
    """Safe state-index samples per fixture, biased toward variety.

    Always includes 1, the last index, and ``num_states // 2``. For
    fixtures with ``num_states >= 25`` also includes 25 and 50 (so
    d3samp6's set-state scenarios overlap bootstrap.jsonl's bs-006
    "set the cursor to 25" and bs-004 "jump to state 50").
    """
    n = int(fixture.num_states)
    pool = {1, n, max(1, n // 2)}
    if n >= 25:
        pool.add(25)
    if n >= 50:
        pool.add(50)
    if n >= 5:
        pool.add(5)
    if n >= 3:
        pool.add(3)
    return tuple(sorted(pool))


def _rng_sample(rng: random.Random, pool: tuple[Any, ...], *, k: int) -> tuple[Any, ...]:
    if k >= len(pool):
        return tuple(pool)
    return tuple(sorted(rng.sample(list(pool), k=k)))


def sample_tuples(
    catalog: Catalog,
    *,
    seed: int = 42,
    target_total: int = 200,
    compound_ratio: float = 0.20,
) -> list[SampledTuple]:
    """Sample all (intent, fixture, bound, seed_idx) tuples for one run.

    Deterministic for a given ``seed``. Compound rows are emitted last
    so a caller iterating the result sees atomic-first; the per-cell
    quota tables guarantee the resulting list satisfies
    ``compound_count / total >= compound_ratio``.

    ``target_total`` is informational — actual total depends on the
    available combinatorics per cell. The current quotas yield ~200
    rows; ``target_total`` does not currently re-tune the per-cell caps.
    """
    out: list[SampledTuple] = []

    by_id = {row.intent_id: row for row in catalog.intents}
    atomic_ids = [r.intent_id for r in catalog.intents if r.shape == "atomic"]
    compound_ids = [r.intent_id for r in catalog.intents if r.shape == "compound"]

    for intent_id in atomic_ids:
        out.extend(_sample_intent(by_id[intent_id], catalog, seed))

    for intent_id in compound_ids:
        out.extend(_sample_intent(by_id[intent_id], catalog, seed))

    return out


def _sample_intent(
    intent: IntentRow, catalog: Catalog, seed: int
) -> list[SampledTuple]:
    gen = _GENERATORS.get(intent.intent_id)
    if gen is None:
        raise ValueError(
            f"no tuple generator registered for intent_id {intent.intent_id!r}; "
            f"add it to synth.sample._GENERATORS"
        )
    quota = (
        ATOMIC_QUOTAS.get(intent.intent_id, 0)
        if intent.shape == "atomic"
        else COMPOUND_QUOTA_PER_CELL
    )
    if quota == 0 and intent.shape == "atomic":
        raise ValueError(
            f"intent {intent.intent_id!r} has no entry in ATOMIC_QUOTAS"
        )

    out: list[SampledTuple] = []
    for fixture_name in intent.fixture_bindings:
        fixture = catalog.fixtures[fixture_name]
        # Per-cell RNG so swapping the seed perturbs every cell equally.
        rng = random.Random(f"{seed}|{intent.intent_id}|{fixture_name}")
        for bound, seed_idx in gen(intent, fixture, rng, quota):
            seed_text = intent.paraphrase_seeds[seed_idx]
            tag = "template" if seed_idx == 0 else "manual-paraphrase"
            out.append(
                SampledTuple(
                    intent_id=intent.intent_id,
                    fixture=fixture_name,
                    bound=dict(bound),
                    seed_idx=seed_idx,
                    seed_text=seed_text,
                    instruction_source=tag,
                )
            )
    return out
