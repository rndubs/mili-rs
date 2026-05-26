"""Stage 6 — assemble the v1 SFT corpus from Stage 5 teacher rollouts.

Reads one or more ``rollouts.jsonl`` files written by ``run_eval`` under
``retain="passing"``, filters ``retained == True``, dedups on
``(normalized_instruction, fixture, tool_calls_flat)`` (the
``posttraining-dataset.md`` §6 key), splits by ``(intent_id, fixture)``
cell per the **per-intent held-out policy** (held out cell = the
smaller of each intent's ``(d3samp6, cylinder)`` pair, tiebreak by
fixture name), writes:

* ``sft/train.jsonl`` + ``sft/val.jsonl`` — FunctionGemma/OpenAI-shape
  records ``{messages, tools, scenario_id, intent_id, fixture, ...}``.
* ``eval/heldout.jsonl`` — the held-out cells, same record shape.
* ``pref/train.jsonl`` + ``pref/val.jsonl`` — DPO pairs ``{chosen,
  rejected}`` for ``(intent, fixture, normalized_instruction)`` cells
  with both passing and failing rollouts. With Stage-5's K=3@T=0.7
  zero-diversity (see ``m5-sft-pipeline.md`` rev 12) these files are
  expected to be empty on the v1 corpus; emit them anyway so the SFT
  trainer's data-loader has a stable target.
* ``dataset_card.md`` — corpus counts, per-intent row counts vs floor,
  contamination matrix, the three preamble decisions, all ``TODO(v2)``
  entries.

Contracts (load-bearing):

1. **Dedup key.** ``(normalized_instruction, fixture,
   json.dumps(tool_calls_flat, sort_keys=True))``. The §6 spec; the §1
   shorthand ``(intent, fixture, tool_calls_flat)`` is instruction-blind
   and collapses paraphrase diversity that SFT relies on.
2. **Contamination control.** Scenario IDs in ``heldout.jsonl`` never
   appear in ``train.jsonl`` or ``val.jsonl``; ``(intent_id, fixture)``
   cells in heldout never appear in train.
3. **Compound family floor.** ``compound-*`` intents stay at ≥20% in
   both train and heldout (the Stage 2 / Stage 3 gate carried forward
   into Stage 6).
4. **Tools-array format.** Records emit ``tools`` in the FG/OpenAI
   shape via ``tool_format.w1_to_openai_tool`` — the same helper the
   llamacpp inference path uses, so train- and inference-time can't
   drift.
5. **Anthropic ↔ FG.** No further conversion needed at assemble time;
   the Stage 5 driver wrote each rollout's ``messages`` array in the
   canonical FG/OpenAI shape (developer / user / assistant.tool_calls /
   tool.content) when the Anthropic provider's response was parsed.
   The byte-for-byte parity test is the Stage-5-side
   ``test_providers_anthropic`` pin; assemble re-emits the messages
   verbatim minus the synthetic driver-stop markers.

Login-node safe; no GPU / Anthropic API / network calls.
"""

from __future__ import annotations

import json
import re
from collections import defaultdict
from dataclasses import dataclass, field
from pathlib import Path
from random import Random
from typing import Any, Iterable

from .harness import Registry
from .tool_format import w1_to_openai_tool


# ---------------------------------------------------------------------------
# Tunables / decision pins.
# ---------------------------------------------------------------------------


# Per-intent floor gate for the v1 SFT corpus. Originally pinned at ≥40
# in ``m5-sft-pipeline.md`` rev 4 on the assumption that scenario
# synthesis would emit a ~3× paraphrase multiplier; the actual synth
# corpus (``data/posttraining/scenarios/synth.jsonl``, 175 scenarios)
# has one instruction per scenario, so post-dedup row counts top out at
# ~20 per intent. Re-pinned to ≥10 in Stage 6 (rev 13) on the realistic
# distribution; under-floor intents are flagged in ``dataset_card.md``
# as v1 holes (regenerate-synth-with-paraphrases is the v2 lift).
DEFAULT_FLOOR_PER_INTENT = 10


# Compound family ratio. ``planning/mili-viz/mili-agent/posttraining-dataset.md``
# §"Multi-step tool calls" pins ≥20% of v1 scenarios as compound; Stage 6
# preserves this in both train and heldout so eval can measure the
# compositional tail without interpolation.
DEFAULT_COMPOUND_RATIO_MIN = 0.20


# Default 90/10 train/val split (stratified by intent_id). Val is for
# loss-curve early-stopping; per-intent eval coverage lives in
# ``eval/heldout.jsonl``.
DEFAULT_VAL_FRACTION = 0.10


HELDOUT_POLICY_PER_INTENT = "per-intent"
HELDOUT_POLICY_WHOLE_FIXTURE = "whole-fixture"
HELDOUT_POLICIES = (HELDOUT_POLICY_PER_INTENT, HELDOUT_POLICY_WHOLE_FIXTURE)


QUERY_POLICY_DROP = "drop"
QUERY_POLICY_ACCEPT = "accept"
QUERY_POLICY_OVERSAMPLE = "oversample"
QUERY_POLICIES = (QUERY_POLICY_DROP, QUERY_POLICY_ACCEPT, QUERY_POLICY_OVERSAMPLE)


# ---------------------------------------------------------------------------
# Rollout loader / dedup.
# ---------------------------------------------------------------------------


def _normalize_instruction(text: str) -> str:
    """Collapse whitespace + lowercase the user instruction for the
    dedup key. The dedup hash is structural — paraphrase-similar
    rewrites (e.g. trailing whitespace, mid-sentence case) collapse;
    semantically-distinct paraphrases stay separate."""
    return re.sub(r"\s+", " ", text.strip().lower())


def _flat_calls_key(calls: list[dict[str, Any]]) -> str:
    """Stable JSON serialization of ``tool_calls_flat`` for the dedup
    key. ``sort_keys=True`` makes dict-key ordering irrelevant; the
    list order itself is meaningful (the sequence is the trajectory)
    and is preserved."""
    return json.dumps(calls, sort_keys=True)


def _strip_driver_stop_markers(
    messages: list[dict[str, Any]],
) -> list[dict[str, Any]]:
    """Drop the driver's synthetic ``{"role": "system", "content":
    "stop:..."}`` markers. They're a verifier-side bookkeeping signal
    (see ``driver._append_stop``); the trainer must never see them."""
    return [
        m
        for m in messages
        if not (
            m.get("role") == "system"
            and isinstance(m.get("content"), str)
            and m["content"].startswith("stop:")
        )
    ]


def _normalize_tool_call_arguments(
    messages: list[dict[str, Any]],
) -> list[dict[str, Any]]:
    """Parse JSON-string ``tool_calls[i].function.arguments`` into a
    dict (m5-sft-pipeline.md Risks §6 / rev 21 (4)).

    Stage 5's teacher driver wrote ``arguments`` as a JSON-encoded
    string (``'{"root": "d3samp6"}'``) instead of the canonical dict.
    The FG chat template's string-arguments branch (chat_template.jinja
    L194-197) then renders that literal between the call's curly
    braces, producing double-braced training tokens
    (``call:NAME{<whitespace>{<JSON>}}``) rather than the canonical
    FG-DSL ``call:NAME{key:<escape>value<escape>}``.

    This is the rev-21 path-(b) fix: normalize in Stage 6 so the
    existing rev-12 rollouts feed forward and the next training run
    renders canonical FG-DSL. Idempotent — dict-shaped arguments pass
    through unchanged, so the helper is safe on partially-fixed
    inputs. Malformed JSON raises (loud); the rev-12 corpus is
    well-formed per the rev-21 audit."""
    out: list[dict[str, Any]] = []
    for m in messages:
        tool_calls = m.get("tool_calls")
        if (
            m.get("role") != "assistant"
            or not tool_calls
            or not isinstance(tool_calls, list)
        ):
            out.append(m)
            continue
        new_tool_calls: list[dict[str, Any]] = []
        changed = False
        for tc in tool_calls:
            fn = tc.get("function") or {}
            args = fn.get("arguments")
            if isinstance(args, str):
                new_tool_calls.append(
                    {**tc, "function": {**fn, "arguments": json.loads(args)}}
                )
                changed = True
            else:
                new_tool_calls.append(tc)
        out.append({**m, "tool_calls": new_tool_calls} if changed else m)
    return out


@dataclass
class LoadedRollout:
    """One retained Stage-5 rollout, projected into Stage-6 dedup keys."""

    record: dict[str, Any]
    scenario_id: str
    intent_id: str
    fixture: str
    normalized_instruction: str
    tool_calls_flat_key: str
    retained: bool
    max_tier: int

    @property
    def dedup_key(self) -> tuple[str, str, str]:
        return (self.normalized_instruction, self.fixture, self.tool_calls_flat_key)

    @property
    def cell_key(self) -> tuple[str, str]:
        return (self.intent_id, self.fixture)


def load_rollouts(paths: Iterable[Path]) -> list[LoadedRollout]:
    """Load every JSON line from each rollouts file into a
    ``LoadedRollout``. Records that fail the minimum-key check
    (missing ``intent_id`` / ``fixture`` / ``tool_calls_flat``) are
    skipped with no warning — Stage 5 already validates these on the
    write side."""
    out: list[LoadedRollout] = []
    for path in paths:
        with Path(path).open() as f:
            for line in f:
                line = line.strip()
                if not line:
                    continue
                rec = json.loads(line)
                scenario_id = rec.get("id")
                intent_id = rec.get("intent_id")
                fixture = rec.get("fixture")
                instruction = rec.get("instruction", "")
                calls = rec.get("tool_calls_flat") or []
                if scenario_id is None or intent_id is None or fixture is None:
                    continue
                verifier = rec.get("verifier") or {}
                out.append(
                    LoadedRollout(
                        record=rec,
                        scenario_id=str(scenario_id),
                        intent_id=str(intent_id),
                        fixture=str(fixture),
                        normalized_instruction=_normalize_instruction(
                            str(instruction)
                        ),
                        tool_calls_flat_key=_flat_calls_key(calls),
                        retained=bool(rec.get("retained", True)),
                        max_tier=int(verifier.get("max_tier", 0)),
                    )
                )
    return out


def dedup_retained(rollouts: list[LoadedRollout]) -> list[LoadedRollout]:
    """Keep retained==True rollouts; for each ``(normalized_instruction,
    fixture, tool_calls_flat)`` key keep the first occurrence (rollouts
    are read in file order, scenario_id-sorted by the Stage 5 writer)."""
    seen: dict[tuple[str, str, str], LoadedRollout] = {}
    for r in rollouts:
        if not r.retained:
            continue
        if r.dedup_key in seen:
            continue
        seen[r.dedup_key] = r
    return list(seen.values())


# ---------------------------------------------------------------------------
# Split.
# ---------------------------------------------------------------------------


@dataclass
class SplitPlan:
    """The result of the (intent, fixture) cell partition."""

    train_cells: set[tuple[str, str]] = field(default_factory=set)
    heldout_cells: set[tuple[str, str]] = field(default_factory=set)
    # Per-intent reason string ("smaller cell d3samp6=3 / cylinder=4"
    # etc.) — surfaced in dataset_card.md.
    heldout_reasons: dict[str, str] = field(default_factory=dict)


def plan_per_intent_heldout(
    rollouts: list[LoadedRollout],
    policy: str,
) -> SplitPlan:
    """Pick the per-intent heldout cells.

    For ``policy == "per-intent"``: for each intent, hold out the
    ``(intent, fixture)`` cell with the FEWER rollouts; ties broken
    alphabetically by fixture name (so the partition is deterministic
    independent of insertion order). If an intent has only one fixture
    bound, no cell is held out (we'd leave train empty); that intent
    is logged in ``heldout_reasons`` with a note.

    ``policy == "whole-fixture"`` raises ``NotImplementedError`` — the
    third-fixture rerun (Stage 3 + Stage 5 against shell_mat2 / bar5)
    is documented as a separate session.
    """
    if policy == HELDOUT_POLICY_WHOLE_FIXTURE:
        raise NotImplementedError(
            "whole-fixture heldout policy requires a third fixture; "
            "rerun Stage 3 + Stage 5 against shell_mat2 or bar5 first. "
            "Tracked in m5-sft-pipeline.md Risks §1."
        )
    if policy != HELDOUT_POLICY_PER_INTENT:
        raise ValueError(
            f"unknown heldout policy {policy!r}; expected one of {HELDOUT_POLICIES}"
        )

    cell_counts: dict[tuple[str, str], int] = defaultdict(int)
    intent_fixtures: dict[str, set[str]] = defaultdict(set)
    for r in rollouts:
        cell_counts[r.cell_key] += 1
        intent_fixtures[r.intent_id].add(r.fixture)

    plan = SplitPlan()
    for intent, fixtures in sorted(intent_fixtures.items()):
        if len(fixtures) < 2:
            sole = next(iter(fixtures))
            plan.train_cells.add((intent, sole))
            plan.heldout_reasons[intent] = (
                f"only one fixture bound ({sole}); no held-out cell"
            )
            continue
        # Pick smaller; tiebreak alphabetical fixture name.
        ranked = sorted(
            fixtures,
            key=lambda f: (cell_counts[(intent, f)], f),
        )
        heldout_fixture = ranked[0]
        train_fixture = ranked[1]
        plan.heldout_cells.add((intent, heldout_fixture))
        plan.train_cells.add((intent, train_fixture))
        plan.heldout_reasons[intent] = (
            f"heldout={heldout_fixture} ({cell_counts[(intent, heldout_fixture)]} rows), "
            f"train={train_fixture} ({cell_counts[(intent, train_fixture)]} rows)"
        )
    return plan


def _stratified_train_val_split(
    rollouts: list[LoadedRollout],
    val_fraction: float,
    seed: int,
) -> tuple[list[LoadedRollout], list[LoadedRollout]]:
    """Stratified by ``intent_id``: each intent contributes
    ``round(count * val_fraction)`` to val, the rest to train; min 0,
    max ``count - 1`` so an intent always has at least one train row.
    Shuffle within intent is seeded for determinism."""
    rng = Random(seed)
    by_intent: dict[str, list[LoadedRollout]] = defaultdict(list)
    for r in rollouts:
        by_intent[r.intent_id].append(r)
    train: list[LoadedRollout] = []
    val: list[LoadedRollout] = []
    for intent in sorted(by_intent):
        rows = list(by_intent[intent])
        rng.shuffle(rows)
        n_val = max(0, min(len(rows) - 1, round(len(rows) * val_fraction)))
        val.extend(rows[:n_val])
        train.extend(rows[n_val:])
    # Stable order on output for byte-determinism.
    train.sort(key=lambda r: r.scenario_id)
    val.sort(key=lambda r: r.scenario_id)
    return train, val


def _compound_ratio(rollouts: list[LoadedRollout]) -> float:
    if not rollouts:
        return 0.0
    n_compound = sum(1 for r in rollouts if r.intent_id.startswith("compound"))
    return n_compound / len(rollouts)


# ---------------------------------------------------------------------------
# Record projection (rollout → SFT/eval record).
# ---------------------------------------------------------------------------


def project_sft_record(
    rollout: LoadedRollout,
    registry: Registry,
) -> dict[str, Any]:
    """Project one retained rollout into the on-disk SFT record shape.

    Output keys:
      * ``scenario_id`` / ``intent_id`` / ``fixture`` / ``instruction``
        / ``instruction_source`` — corpus bookkeeping the trainer
        ignores but contamination tests + the data card consume.
      * ``messages`` — the FG/OpenAI canonical transcript, minus the
        driver's synthetic ``stop:`` markers.
      * ``tools`` — the rollout's tool inventory projected into the
        FG/OpenAI shape via ``w1_to_openai_tool`` (the same helper the
        llamacpp inference path uses, so train- and inference-time
        can't drift).
      * ``tool_calls_flat`` — the dedup key body, surfaced for audit
        in dataset_card.md.
      * ``postcondition`` — the verifier's grading target. Lifted
        from ``record["verifier"]["postcondition"]`` so the assembled
        corpus is self-contained for Stage 7 (the eval harness reads
        it back to reconstruct a ``Scenario`` without joining against
        ``synth.jsonl``). Self-containment is load-bearing — a future
        synth.jsonl regen must not silently rewrite the heldout
        postconditions. Decision pinned 2026-05-24 (rev 14, option a).

    Tool names referenced by the rollout but missing from the registry
    are dropped with no warning (Stage 5 already validated they
    exist; a missing entry would be a registry-vs-rollouts drift the
    Stage 5 / Stage 6.5 gates would catch first).
    """
    rec = rollout.record
    tool_names = rec.get("tools") or []
    tools_oai: list[dict[str, Any]] = []
    for name in tool_names:
        if not registry.has(name):
            continue
        tools_oai.append(w1_to_openai_tool(registry.tools[name]))

    messages = _strip_driver_stop_markers(rec.get("messages") or [])
    messages = _normalize_tool_call_arguments(messages)

    verifier = rec.get("verifier") or {}
    postcondition = verifier.get("postcondition") or {}

    return {
        "scenario_id": rollout.scenario_id,
        "intent_id": rollout.intent_id,
        "fixture": rollout.fixture,
        "instruction": rec.get("instruction", ""),
        "instruction_source": rec.get("instruction_source", ""),
        "messages": messages,
        "tools": tools_oai,
        "tool_calls_flat": rec.get("tool_calls_flat", []),
        "postcondition": dict(postcondition),
    }


# ---------------------------------------------------------------------------
# Preference pairs.
# ---------------------------------------------------------------------------


def build_preference_pairs(
    all_rollouts: list[LoadedRollout],
    cell_filter: set[tuple[str, str]],
    registry: Registry,
) -> list[dict[str, Any]]:
    """Build DPO ``(chosen, rejected)`` pairs from a list of *all*
    rollouts (retained and not), restricted to cells in
    ``cell_filter``.

    Pairing key: ``(intent_id, fixture, normalized_instruction)``.
    A pair is emitted whenever a single key has at least one
    passing (``max_tier == 3``) and at least one failing
    (``max_tier < 3``) rollout. ``chosen`` = first passing record;
    ``rejected`` = first failing record; both projected through
    ``project_sft_record``.

    Returns an empty list when the rollouts file has no mixed-tier
    keys — expected on the v1 corpus because K=3@T=0.7 produced
    zero-diversity rollouts under Claude (see ``m5-sft-pipeline.md``
    rev 12). The empty file is still emitted by the caller so the
    trainer's data-loader has a stable target.
    """
    by_key: dict[
        tuple[str, str, str],
        dict[str, list[LoadedRollout]],
    ] = defaultdict(lambda: {"passing": [], "failing": []})
    for r in all_rollouts:
        if r.cell_key not in cell_filter:
            continue
        bucket = "passing" if r.max_tier == 3 else "failing"
        by_key[
            (r.intent_id, r.fixture, r.normalized_instruction)
        ][bucket].append(r)

    pairs: list[dict[str, Any]] = []
    for (intent, fixture, _instr), bucket in sorted(by_key.items()):
        if not bucket["passing"] or not bucket["failing"]:
            continue
        chosen = bucket["passing"][0]
        rejected = bucket["failing"][0]
        pairs.append(
            {
                "scenario_id": chosen.scenario_id,
                "intent_id": intent,
                "fixture": fixture,
                "chosen": project_sft_record(chosen, registry),
                "rejected": project_sft_record(rejected, registry),
            }
        )
    return pairs


# ---------------------------------------------------------------------------
# Top-level assemble.
# ---------------------------------------------------------------------------


@dataclass
class AssembleReport:
    """Surface counts + decisions for the data card writer."""

    total_input: int
    total_retained: int
    total_unique: int
    train_count: int
    val_count: int
    heldout_count: int
    pref_count: int
    train_intent_counts: dict[str, int]
    val_intent_counts: dict[str, int]
    heldout_intent_counts: dict[str, int]
    pre_split_intent_counts: dict[str, int]
    train_cell_counts: dict[tuple[str, str], int]
    heldout_cell_counts: dict[tuple[str, str], int]
    train_compound_ratio: float
    heldout_compound_ratio: float
    split_plan: SplitPlan
    under_floor_intents: list[str]
    floor_per_intent: int
    compound_ratio_min: float
    heldout_policy: str
    query_policy: str
    seed: int
    contamination_clean: bool


def assemble(
    rollouts_paths: list[Path],
    out_dir: Path,
    *,
    registry: Registry | None = None,
    heldout_policy: str = HELDOUT_POLICY_PER_INTENT,
    query_policy: str = QUERY_POLICY_ACCEPT,
    seed: int = 42,
    floor_per_intent: int = DEFAULT_FLOOR_PER_INTENT,
    compound_ratio_min: float = DEFAULT_COMPOUND_RATIO_MIN,
    val_fraction: float = DEFAULT_VAL_FRACTION,
) -> AssembleReport:
    """End-to-end Stage 6 assemble pipeline.

    Reads ``rollouts_paths``, dedups, splits, projects, writes
    ``out_dir/sft/{train,val}.jsonl`` + ``out_dir/eval/heldout.jsonl``
    + ``out_dir/pref/{train,val}.jsonl``. The data card is written by
    the caller (``write_dataset_card``) so this function stays a pure
    transform — easy to test record-by-record.

    Raises ``RuntimeError`` if the compound-ratio gate or the
    contamination check fails. The floor gate is soft: under-floor
    intents are recorded in the report and the data card flags them,
    but the assembler still writes the corpus (the gate is the SFT
    pipeline's call to make, not the assembler's — see
    ``m5-sft-pipeline.md`` rev 13).
    """
    if heldout_policy not in HELDOUT_POLICIES:
        raise ValueError(
            f"unknown heldout policy {heldout_policy!r}; "
            f"expected one of {HELDOUT_POLICIES}"
        )
    if query_policy not in QUERY_POLICIES:
        raise ValueError(
            f"unknown query policy {query_policy!r}; "
            f"expected one of {QUERY_POLICIES}"
        )

    reg = registry if registry is not None else Registry.load_from_artifact()

    all_rollouts = load_rollouts(rollouts_paths)
    total_input = len(all_rollouts)

    # query_policy=="drop" removes the intent entirely from the corpus
    # (training + eval + pref). "accept" lets the small cells land and
    # be flagged in dataset_card.md. "oversample" assumes the caller
    # has already merged an oversample run into rollouts_paths — Stage
    # 6 does not re-roll the teacher itself.
    if query_policy == QUERY_POLICY_DROP:
        all_rollouts = [r for r in all_rollouts if r.intent_id != "query"]

    retained_all = [r for r in all_rollouts if r.retained]
    total_retained = len(retained_all)

    unique = dedup_retained(all_rollouts)
    total_unique = len(unique)

    plan = plan_per_intent_heldout(unique, heldout_policy)

    heldout_rollouts = [r for r in unique if r.cell_key in plan.heldout_cells]
    train_pool = [r for r in unique if r.cell_key in plan.train_cells]

    train_rollouts, val_rollouts = _stratified_train_val_split(
        train_pool, val_fraction, seed
    )

    # Compound-ratio gate. Fail-loud per the user-confirmed decision:
    # the split partitioner does NOT silently let either side fall
    # below 20 %. If the per-intent heldout cell pick violates the
    # gate, the operator must either accept the violation in
    # dataset_card.md (rerun with a relaxed gate) or change the policy.
    train_cr = _compound_ratio(train_rollouts)
    held_cr = _compound_ratio(heldout_rollouts)
    if train_cr < compound_ratio_min:
        raise RuntimeError(
            f"compound ratio gate failed in train: {train_cr:.3f} "
            f"< {compound_ratio_min:.3f}. Inspect the per-intent split "
            f"in heldout_reasons + per-cell counts."
        )
    if held_cr < compound_ratio_min:
        raise RuntimeError(
            f"compound ratio gate failed in heldout: {held_cr:.3f} "
            f"< {compound_ratio_min:.3f}. Inspect the per-intent split."
        )

    # ----- Project + write the JSONL files. -----
    out_dir = Path(out_dir)
    (out_dir / "sft").mkdir(parents=True, exist_ok=True)
    (out_dir / "eval").mkdir(parents=True, exist_ok=True)
    (out_dir / "pref").mkdir(parents=True, exist_ok=True)

    def write_jsonl(path: Path, recs: list[dict[str, Any]]) -> None:
        with path.open("w") as f:
            for rec in recs:
                f.write(json.dumps(rec))
                f.write("\n")

    train_recs = [project_sft_record(r, reg) for r in train_rollouts]
    val_recs = [project_sft_record(r, reg) for r in val_rollouts]
    heldout_recs = [project_sft_record(r, reg) for r in heldout_rollouts]

    write_jsonl(out_dir / "sft" / "train.jsonl", train_recs)
    write_jsonl(out_dir / "sft" / "val.jsonl", val_recs)
    write_jsonl(out_dir / "eval" / "heldout.jsonl", heldout_recs)

    pref_pairs = build_preference_pairs(
        all_rollouts=all_rollouts,
        cell_filter=plan.train_cells,
        registry=reg,
    )
    pref_train, pref_val = (
        (pref_pairs, []) if len(pref_pairs) < 5 else (pref_pairs[:-1], pref_pairs[-1:])
    )
    write_jsonl(out_dir / "pref" / "train.jsonl", pref_train)
    write_jsonl(out_dir / "pref" / "val.jsonl", pref_val)

    # ----- Contamination check (hard gate). -----
    train_val_ids = {r.scenario_id for r in train_rollouts} | {
        r.scenario_id for r in val_rollouts
    }
    heldout_ids = {r.scenario_id for r in heldout_rollouts}
    contamination_clean = train_val_ids.isdisjoint(heldout_ids)
    if not contamination_clean:
        overlap = sorted(train_val_ids & heldout_ids)
        raise RuntimeError(
            f"contamination: {len(overlap)} scenario ids appear in both "
            f"train/val and heldout (e.g. {overlap[:5]})"
        )
    train_cells_used = {(r.intent_id, r.fixture) for r in train_rollouts} | {
        (r.intent_id, r.fixture) for r in val_rollouts
    }
    if train_cells_used & plan.heldout_cells:
        bad = sorted(train_cells_used & plan.heldout_cells)
        raise RuntimeError(
            f"contamination: cells {bad} appear in both train/val and heldout"
        )

    # ----- Per-intent counts + floor analysis. -----
    def count_by_intent(recs: list[LoadedRollout]) -> dict[str, int]:
        out: dict[str, int] = defaultdict(int)
        for r in recs:
            out[r.intent_id] += 1
        return dict(out)

    pre_split_intent_counts = count_by_intent(unique)
    train_intent_counts = count_by_intent(train_rollouts)
    val_intent_counts = count_by_intent(val_rollouts)
    heldout_intent_counts = count_by_intent(heldout_rollouts)

    train_cell_counts: dict[tuple[str, str], int] = defaultdict(int)
    for r in train_rollouts:
        train_cell_counts[r.cell_key] += 1
    heldout_cell_counts: dict[tuple[str, str], int] = defaultdict(int)
    for r in heldout_rollouts:
        heldout_cell_counts[r.cell_key] += 1

    under_floor_intents = sorted(
        intent
        for intent, count in train_intent_counts.items()
        if count < floor_per_intent
    )

    return AssembleReport(
        total_input=total_input,
        total_retained=total_retained,
        total_unique=total_unique,
        train_count=len(train_rollouts),
        val_count=len(val_rollouts),
        heldout_count=len(heldout_rollouts),
        pref_count=len(pref_pairs),
        train_intent_counts=train_intent_counts,
        val_intent_counts=val_intent_counts,
        heldout_intent_counts=heldout_intent_counts,
        pre_split_intent_counts=pre_split_intent_counts,
        train_cell_counts=dict(train_cell_counts),
        heldout_cell_counts=dict(heldout_cell_counts),
        train_compound_ratio=train_cr,
        heldout_compound_ratio=held_cr,
        split_plan=plan,
        under_floor_intents=under_floor_intents,
        floor_per_intent=floor_per_intent,
        compound_ratio_min=compound_ratio_min,
        heldout_policy=heldout_policy,
        query_policy=query_policy,
        seed=seed,
        contamination_clean=True,
    )


# ---------------------------------------------------------------------------
# Data card writer.
# ---------------------------------------------------------------------------


_DATA_CARD_PREAMBLE_TEMPLATE = """# v1 SFT corpus — dataset card

Generated by `mili-llm-bench assemble` from Stage 5 teacher rollouts.
See `planning/mili-viz/mili-agent/m5-sft-pipeline.md` Stage 6 row and
`planning/mili-viz/mili-agent/posttraining-dataset.md` §6 for the
build-order context.

## Stage 6 decisions (preamble)

Three decisions were locked before the assembler ran. They live here
so future-you can reproduce the corpus without re-deriving them.

1. **Per-intent ≥{floor} floor (revised from the rev-4 ≥40).** The
   original ≥40 gate assumed scenario synthesis would emit a ~3×
   paraphrase multiplier. The actual `data/posttraining/scenarios/synth.jsonl`
   carries one canonical instruction per scenario, so after K=3@T=0.7
   dedup the retained corpus is **{total_unique} unique trajectories
   distributed across 14 intents (p50 ≈ 14, p25 ≈ 6, p75 ≈ 18)** —
   no intent can clear ≥40 without re-running synth. Re-pinned to
   ≥{floor} as the realistic v1 floor; under-floor intents flagged
   below as **v1 holes** with `TODO(v2)` to regenerate synth with
   `paraphrase_count > 1`.
2. **Per-intent held-out split (rather than whole-fixture).** For
   each intent, the smaller of its two `(intent, fixture)` cells is
   held out in full (tiebreak by alphabetical fixture name). Cleaner
   coverage of intents in eval at the cost of fixture coverage; the
   whole-fixture alternative needs a third fixture (shell_mat2 or
   bar5) bound through Stage 3 + Stage 5, which is a separate session.
3. **Compound family ratio enforced in both train AND heldout** at
   ≥{compound_min:.0%}. The assembler fails loud if either side drops
   below the gate.

`query_policy={query_policy!r}`. The 4 v6/v7 stage-6.5 `query`
failures (synth-00124/00127/00130/00133, all `wrong_final_state`,
parked under catalog `query.todo_v2`) are present in the upstream
rollouts only via the 8 passing scenarios; the assembler does not
re-roll. Setting `query_policy=drop` removes the intent entirely;
`accept` lands it as an under-floor cell.

`seed={seed}` controls the stratified train/val split. Heldout cell
choice is deterministic from the per-cell row counts (with an
alphabetical fixture tiebreak), independent of seed.
"""


def write_dataset_card(
    out_dir: Path,
    report: AssembleReport,
    *,
    rollouts_paths: list[Path],
) -> Path:
    """Render ``dataset_card.md`` from an ``AssembleReport``."""
    out_path = Path(out_dir) / "dataset_card.md"
    lines: list[str] = []
    lines.append(
        _DATA_CARD_PREAMBLE_TEMPLATE.format(
            floor=report.floor_per_intent,
            total_unique=report.total_unique,
            compound_min=report.compound_ratio_min,
            query_policy=report.query_policy,
            seed=report.seed,
        )
    )

    lines.append("\n## Inputs\n")
    for p in rollouts_paths:
        lines.append(f"- `{p}`")
    lines.append("")

    lines.append("## Corpus counts")
    lines.append("")
    lines.append("| Bucket | Rows |")
    lines.append("| --- | ---: |")
    lines.append(f"| input rollouts (post `query_policy`) | {report.total_input} |")
    lines.append(f"| retained (max_tier == 3) | {report.total_retained} |")
    lines.append(
        f"| unique trajectories (instruction-aware dedup) | {report.total_unique} |"
    )
    lines.append(f"| sft/train.jsonl | {report.train_count} |")
    lines.append(f"| sft/val.jsonl | {report.val_count} |")
    lines.append(f"| eval/heldout.jsonl | {report.heldout_count} |")
    lines.append(f"| pref/*.jsonl (DPO pairs) | {report.pref_count} |")
    lines.append("")

    lines.append("## Compound ratio (≥{:.0%} gate)".format(report.compound_ratio_min))
    lines.append("")
    lines.append(f"- train: **{report.train_compound_ratio:.1%}**")
    lines.append(f"- heldout: **{report.heldout_compound_ratio:.1%}**")
    lines.append("")

    lines.append("## Per-intent row counts (train + val + heldout)")
    lines.append("")
    lines.append(
        f"Floor = **≥{report.floor_per_intent}** on `sft/train.jsonl`. "
        f"Under-floor cells are tagged as v1 holes."
    )
    lines.append("")
    lines.append("| Intent | Pre-split | Train | Val | Heldout | Under floor |")
    lines.append("| --- | ---: | ---: | ---: | ---: | :---: |")
    all_intents = sorted(report.pre_split_intent_counts)
    for intent in all_intents:
        tr = report.train_intent_counts.get(intent, 0)
        flag = "**yes**" if tr < report.floor_per_intent else "no"
        lines.append(
            f"| {intent} "
            f"| {report.pre_split_intent_counts.get(intent, 0)} "
            f"| {tr} "
            f"| {report.val_intent_counts.get(intent, 0)} "
            f"| {report.heldout_intent_counts.get(intent, 0)} "
            f"| {flag} |"
        )
    lines.append("")

    lines.append("## Held-out split (per-intent policy)")
    lines.append("")
    lines.append("| Intent | Choice |")
    lines.append("| --- | --- |")
    for intent in all_intents:
        lines.append(
            f"| {intent} | {report.split_plan.heldout_reasons.get(intent, '—')} |"
        )
    lines.append("")

    lines.append("## (intent, fixture) cell coverage")
    lines.append("")
    lines.append("| Intent | Fixture | Train rows | Heldout rows |")
    lines.append("| --- | --- | ---: | ---: |")
    all_cells = sorted(
        set(report.train_cell_counts) | set(report.heldout_cell_counts)
    )
    for intent, fixture in all_cells:
        lines.append(
            f"| {intent} | {fixture} "
            f"| {report.train_cell_counts.get((intent, fixture), 0)} "
            f"| {report.heldout_cell_counts.get((intent, fixture), 0)} |"
        )
    lines.append("")

    lines.append("## Contamination check")
    lines.append("")
    lines.append(
        "Scenario IDs in `eval/heldout.jsonl` are disjoint from "
        "`sft/train.jsonl` ∪ `sft/val.jsonl`: **{}**.".format(
            "clean" if report.contamination_clean else "VIOLATED"
        )
    )
    lines.append(
        "`(intent_id, fixture)` cells in heldout are disjoint from "
        "train+val: **clean** (enforced by the split partitioner)."
    )
    lines.append("")

    lines.append("## Under-floor intents (v1 holes)")
    lines.append("")
    if not report.under_floor_intents:
        lines.append("None — every intent clears the ≥{} floor.".format(report.floor_per_intent))
    else:
        for intent in report.under_floor_intents:
            lines.append(
                f"- **{intent}** — train rows = "
                f"{report.train_intent_counts.get(intent, 0)} "
                f"(< {report.floor_per_intent}). `TODO(v2)`: regenerate "
                "synth with `paraphrase_count > 1` so this intent has "
                "more retained trajectories under the same K=1 reroll."
            )
    lines.append("")

    lines.append("## TODO(v2) — deferred for v1 pilot")
    lines.append("")
    lines.append(
        "- **Paraphrase multiplier in synth.** Re-run `mili-llm-bench "
        "synth` with `paraphrase_count > 1` per (intent, fixture, params) "
        "tuple to push every intent over the ≥40 floor. Cheapest path "
        "to lifting the under-floor intents."
    )
    lines.append(
        "- **K policy.** Drop K to 1 for the next sweep against a "
        "frontier teacher (rev-12 finding: 0/175 K=3 diversity at "
        "T=0.7). Reserve K > 1 for genuinely stochastic teachers "
        "(local 7B, T ≥ 1.0)."
    )
    lines.append(
        "- **Whole-fixture held-out.** Bind a third fixture "
        "(`shell_mat2` or `bar5`) through Stage 3 + Stage 5 so heldout "
        "measures cross-fixture generalization, not the smaller "
        "intra-fixture cell."
    )
    lines.append(
        "- **Near-dup instruction filter.** Embedding / MinHash filter "
        "on `normalized_instruction` to catch paraphrase collapse. "
        "Not needed at 171 rows; becomes useful at v2 scale (~1k)."
    )
    lines.append(
        "- **`query.todo_v2`.** The 4 `wrong_final_state` failures need "
        "either verifier leniency on default state, instruction pinning "
        "`state 1`, or a tool-schema default. Carried from rev 7."
    )
    lines.append(
        "- **DPO data.** v1 pref/*.jsonl is "
        + ("empty" if report.pref_count == 0 else f"{report.pref_count} pair(s)")
        + " because K=3@T=0.7 produced zero mixed-tier scenarios. "
        "v2: rerun a subset at T ≥ 1.0 specifically to harvest "
        "(chosen, rejected) pairs."
    )
    lines.append("")

    out_path.write_text("\n".join(lines))
    return out_path


# ---------------------------------------------------------------------------
# CLI entry point.
# ---------------------------------------------------------------------------


def run_assemble_cli(args: Any) -> int:
    """``mili-llm-bench assemble`` subcommand handler (see ``cli.py``)."""
    import sys

    rollouts_paths = [Path(p) for p in args.rollouts]
    if args.extra_rollouts:
        rollouts_paths.extend(Path(p) for p in args.extra_rollouts)
    out_dir = Path(args.out)

    registry: Registry | None = None
    if args.tools is not None:
        registry = Registry.load_from_artifact(Path(args.tools))

    try:
        report = assemble(
            rollouts_paths,
            out_dir,
            registry=registry,
            heldout_policy=args.heldout_policy,
            query_policy=args.query_policy,
            seed=args.seed,
            floor_per_intent=args.floor_per_intent,
            compound_ratio_min=args.compound_ratio_min,
            val_fraction=args.val_fraction,
        )
    except Exception as exc:
        sys.stderr.write(f"assemble failed: {exc!r}\n")
        return 1

    card_path = write_dataset_card(out_dir, report, rollouts_paths=rollouts_paths)
    print(
        f"assemble complete: train={report.train_count} "
        f"val={report.val_count} heldout={report.heldout_count} "
        f"pref={report.pref_count}; "
        f"compound train={report.train_compound_ratio:.1%} "
        f"heldout={report.heldout_compound_ratio:.1%}; "
        f"under-floor={len(report.under_floor_intents)}; "
        f"card={card_path}"
    )
    return 0


__all__ = [
    "AssembleReport",
    "DEFAULT_COMPOUND_RATIO_MIN",
    "DEFAULT_FLOOR_PER_INTENT",
    "DEFAULT_VAL_FRACTION",
    "HELDOUT_POLICIES",
    "HELDOUT_POLICY_PER_INTENT",
    "HELDOUT_POLICY_WHOLE_FIXTURE",
    "LoadedRollout",
    "QUERY_POLICIES",
    "QUERY_POLICY_ACCEPT",
    "QUERY_POLICY_DROP",
    "QUERY_POLICY_OVERSAMPLE",
    "SplitPlan",
    "assemble",
    "build_preference_pairs",
    "dedup_retained",
    "load_rollouts",
    "plan_per_intent_heldout",
    "project_sft_record",
    "run_assemble_cli",
    "write_dataset_card",
]
