"""W2 — bootstrap eval scenarios; see agent-local-llm-baseline.md §W2.

Each scenario is one JSON object pinning a *single user instruction*,
a fixture root, an intent label (used for coverage rollups in the W6
report), and a closed-kind ``postcondition`` the W3 verifier dispatches
on. Scenarios are hand-authored and small enough to check in
(``data/posttraining/eval/bootstrap.jsonl``) — the W2 x W3 contract
test fabricates a perfect rollout for each and asserts the verifier
grades it L3.

Closed post-condition kinds are mirrored in ``verifier.py``; drift
between scenarios.jsonl and the verifier must fail loudly, so
``load_scenarios`` rejects unknown kinds at load time.
"""

from __future__ import annotations

import json
from dataclasses import dataclass, field
from pathlib import Path
from typing import Any

# Mirrors verifier.VALID_POSTCONDITION_KINDS; imported there for one
# source of truth. Defined here too so a scenario file that references
# a not-yet-built verifier still gets a clean load-time error.
VALID_POSTCONDITION_KINDS: frozenset[str] = frozenset(
    {
        "state_index",
        "selection_set",
        "active_result",
        "result_range",
        "materials_visible",
        "camera_named_view",
        "query_value",
    }
)


@dataclass(frozen=True)
class Postcondition:
    kind: str
    expect: dict[str, Any] = field(default_factory=dict)

    def to_json(self) -> dict[str, Any]:
        return {"kind": self.kind, "expect": dict(self.expect)}


@dataclass(frozen=True)
class Scenario:
    id: str
    fixture: str
    intent_id: str
    instruction: str
    postcondition: Postcondition
    # Stage 3 records carry their paraphrase tag here so the W4b
    # rollout writer can stamp it through verbatim (template /
    # manual-paraphrase / teacher-paraphrase). ``None`` for legacy
    # bootstrap rows; the rollout writer falls back to
    # ``INSTRUCTION_SOURCE_V0`` in that case.
    instruction_source: str | None = None

    def to_json(self) -> dict[str, Any]:
        out: dict[str, Any] = {
            "id": self.id,
            "fixture": self.fixture,
            "intent_id": self.intent_id,
            "instruction": self.instruction,
            "postcondition": self.postcondition.to_json(),
        }
        if self.instruction_source is not None:
            out["instruction_source"] = self.instruction_source
        return out


def _parse_scenario(obj: dict[str, Any]) -> Scenario:
    for key in ("id", "fixture", "intent_id", "instruction", "postcondition"):
        if key not in obj:
            raise ValueError(f"scenario missing required key {key!r}: {obj!r}")
    pc = obj["postcondition"]
    if not isinstance(pc, dict) or "kind" not in pc:
        raise ValueError(f"scenario {obj['id']!r} has malformed postcondition")
    kind = pc["kind"]
    if kind not in VALID_POSTCONDITION_KINDS:
        raise ValueError(
            f"scenario {obj['id']!r}: unknown postcondition kind {kind!r}; "
            f"expected one of {sorted(VALID_POSTCONDITION_KINDS)}"
        )
    expect = pc.get("expect", {})
    if not isinstance(expect, dict):
        raise ValueError(f"scenario {obj['id']!r}: postcondition.expect must be a dict")
    return Scenario(
        id=obj["id"],
        fixture=obj["fixture"],
        intent_id=obj["intent_id"],
        instruction=obj["instruction"],
        postcondition=Postcondition(kind=kind, expect=dict(expect)),
        instruction_source=obj.get("instruction_source"),
    )


def _parse_assembled_record(obj: dict[str, Any]) -> Scenario:
    """Parse one assembled-corpus record (rev 14 / option (a)) into a
    ``Scenario``.

    The assembled shape (from ``mili_llm_bench.assemble.project_sft_record``)
    carries ``scenario_id`` instead of ``id`` and includes the
    canonical ``messages`` + ``tools`` arrays the trainer / Stage 7 eval
    actually consumes. ``postcondition`` is lifted to a top-level field
    so the assembled corpus is self-contained — no synth.jsonl join
    required at Stage 7 load.
    """
    for key in ("scenario_id", "fixture", "intent_id", "instruction", "postcondition"):
        if key not in obj:
            raise ValueError(
                f"assembled record missing required key {key!r}: "
                f"{ {k: v for k, v in obj.items() if k != 'messages'} !r}"
            )
    pc = obj["postcondition"]
    if not isinstance(pc, dict) or "kind" not in pc:
        raise ValueError(
            f"assembled record {obj['scenario_id']!r} has malformed postcondition"
        )
    kind = pc["kind"]
    if kind not in VALID_POSTCONDITION_KINDS:
        raise ValueError(
            f"assembled record {obj['scenario_id']!r}: "
            f"unknown postcondition kind {kind!r}; expected one of "
            f"{sorted(VALID_POSTCONDITION_KINDS)}"
        )
    expect = pc.get("expect", {})
    if not isinstance(expect, dict):
        raise ValueError(
            f"assembled record {obj['scenario_id']!r}: "
            "postcondition.expect must be a dict"
        )
    return Scenario(
        id=obj["scenario_id"],
        fixture=obj["fixture"],
        intent_id=obj["intent_id"],
        instruction=obj["instruction"],
        postcondition=Postcondition(kind=kind, expect=dict(expect)),
        instruction_source=obj.get("instruction_source") or None,
    )


def _is_assembled_record(obj: dict[str, Any]) -> bool:
    """Distinguish the assembled-corpus shape (Stage 6 / Stage 7) from
    the synth/bootstrap shape (Stages 0–3) by structural keys.

    Assembled rows carry ``scenario_id`` (not ``id``) and the
    ``messages`` array the trainer consumes; synth rows carry ``id``
    and no ``messages``. The discriminator is intentionally robust
    against partial overlaps — an assembled row could theoretically
    grow an ``id`` field, so we key on ``scenario_id`` AND
    ``messages``.
    """
    return "scenario_id" in obj and "messages" in obj


def load_scenarios_from_assembled(path: Path | str) -> list[Scenario]:
    """Load scenarios from an assembled SFT/eval JSONL (Stage 6
    output: ``sft/{train,val}.jsonl`` or ``eval/heldout.jsonl``).

    Strict variant of ``load_scenarios``: rejects any non-assembled
    row with a clear error. Use ``load_scenarios`` to auto-detect
    between shapes.
    """
    p = Path(path)
    out: list[Scenario] = []
    with p.open() as f:
        for lineno, raw in enumerate(f, start=1):
            raw = raw.strip()
            if not raw:
                continue
            try:
                obj = json.loads(raw)
            except json.JSONDecodeError as exc:
                raise ValueError(f"{p}:{lineno}: invalid JSON ({exc})") from exc
            if not _is_assembled_record(obj):
                raise ValueError(
                    f"{p}:{lineno}: expected assembled-corpus shape "
                    "(scenario_id + messages); got "
                    f"keys={sorted(obj.keys())}"
                )
            out.append(_parse_assembled_record(obj))
    return out


def load_scenarios(path: Path | str) -> list[Scenario]:
    """Load and validate scenarios from a JSONL file.

    Auto-detects between two shapes:

    * **synth / bootstrap shape** — ``{id, fixture, intent_id,
      instruction, postcondition, ...}`` — the canonical
      hand-authored / synthesized scenarios consumed by Stage 3+.
    * **assembled shape** — ``{scenario_id, fixture, intent_id,
      instruction, messages, tools, postcondition, ...}`` — emitted
      by ``mili_llm_bench.assemble.project_sft_record``; used by
      Stage 7's eval harness to grade against
      ``eval/heldout.jsonl``.

    Drift between the file's post-condition ``kind`` and the closed
    set raises ``ValueError`` here so the operator sees the file path,
    not a downstream verifier traceback. Mixed-shape files raise
    too — the loader picks the shape per-row by structure but the
    operator should pass a homogeneous file.
    """
    p = Path(path)
    out: list[Scenario] = []
    with p.open() as f:
        for lineno, raw in enumerate(f, start=1):
            raw = raw.strip()
            if not raw:
                continue
            try:
                obj = json.loads(raw)
            except json.JSONDecodeError as exc:
                raise ValueError(f"{p}:{lineno}: invalid JSON ({exc})") from exc
            if _is_assembled_record(obj):
                out.append(_parse_assembled_record(obj))
            else:
                out.append(_parse_scenario(obj))
    return out


def dump_scenarios(scenarios: list[Scenario]) -> str:
    """Canonical one-object-per-line JSONL with a trailing newline.

    Used by the W2 round-trip test; field order is fixed by the
    ``to_json()`` builder so a re-serialize is byte-identical to the
    checked-in file.
    """
    return "\n".join(json.dumps(s.to_json()) for s in scenarios) + "\n"


def default_bootstrap_path(start: Path | None = None) -> Path:
    """``data/posttraining/eval/bootstrap.jsonl`` relative to the repo root."""
    p = (start or Path(__file__)).resolve()
    for parent in [p, *p.parents]:
        if (parent / "crates" / "mili-viz-proto" / "proto" / "mili_viz.proto").exists():
            return parent / "data" / "posttraining" / "eval" / "bootstrap.jsonl"
    raise FileNotFoundError("could not locate repo root from " + str(p))
