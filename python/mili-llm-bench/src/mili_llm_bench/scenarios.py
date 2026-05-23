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

    def to_json(self) -> dict[str, Any]:
        return {
            "id": self.id,
            "fixture": self.fixture,
            "intent_id": self.intent_id,
            "instruction": self.instruction,
            "postcondition": self.postcondition.to_json(),
        }


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
    )


def load_scenarios(path: Path | str) -> list[Scenario]:
    """Load and validate scenarios from a JSONL file.

    Drift between the file's post-condition ``kind`` and the closed
    set raises ``ValueError`` here so the operator sees the file path,
    not a downstream verifier traceback.
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
