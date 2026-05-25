"""Catalog loader for ``data/posttraining/intents/catalog.yaml``.

The catalog is the single intent source for Stage 3. Drift between the
catalog and the verifier's closed kind set
(``scenarios.VALID_POSTCONDITION_KINDS``) or between the catalog's
``fixture_bindings`` and ``dispatchers.pygriz._FIXTURE_PATHS`` fails
loudly at load time — Stage 3 will not silently silently emit scenarios
the harness can't grade.
"""

from __future__ import annotations

import re
from dataclasses import dataclass, field
from pathlib import Path
from typing import Any

import yaml

from ..scenarios import VALID_POSTCONDITION_KINDS

# Slot-token grammar. Catalog ``arguments_template`` and ``expect_template``
# values are either literal scalars or one of:
#
#   <param:name>              — substitute the sampled value of params[name]
#   <int:name>                — same as <param:name> but typed
#   <str:name>
#   <bool:name>
#   <enum:name>
#   <list[int]:name>
#   <derived:kind>            — call resolve_derived(kind, ...) at synth time
#
# Synthesis rejects any other token shape so a typo in the catalog
# surfaces as a clean error, not a silent skip.

_PARAM_TOKEN = re.compile(r"^<(?:param|int|str|bool|enum|list\[int\]):([A-Za-z_][A-Za-z0-9_]*)>$")
_DERIVED_TOKEN = re.compile(r"^<derived:([A-Za-z_][A-Za-z0-9_]*)>$")

# The closed set of derived-token kinds the slot resolver recognises.
# Adding a new derived token is a deliberate slots.py edit — drift here
# fails synthesis at catalog-load time.
KNOWN_DERIVED_KINDS: frozenset[str] = frozenset(
    {
        "from_dir_and_fixture",
        "from_enable_and_mat_id",
        "from_parity_suite",
    }
)


@dataclass(frozen=True)
class ParamSpec:
    """One parameter slot the synthesizer must bind before emitting a record."""

    name: str
    type: str
    binding: Any
    values: tuple[Any, ...] | None = None
    range_from: str | None = None


@dataclass(frozen=True)
class StepTemplate:
    """One step of an intent. Atomic intents have exactly one of these."""

    tool: str
    arguments_template: dict[str, Any]


@dataclass(frozen=True)
class FixtureFacts:
    """Catalog-declared facts for one fixture; confirmed at synth time."""

    name: str
    num_states: int
    classes: tuple[str, ...]
    material_ids: tuple[int, ...]
    primal_svars: tuple[str, ...]
    derived_results: tuple[str, ...]


@dataclass(frozen=True)
class IntentRow:
    """One catalog intent row, atomic or compound."""

    intent_id: str
    shape: str
    prose: str
    steps: tuple[StepTemplate, ...]
    params: tuple[ParamSpec, ...]
    postcondition_kind: str
    expect_template: dict[str, Any]
    fixture_bindings: tuple[str, ...]
    paraphrase_seeds: tuple[str, ...]


@dataclass(frozen=True)
class Catalog:
    schema_version: int
    fixtures: dict[str, FixtureFacts]
    intents: tuple[IntentRow, ...] = field(default_factory=tuple)


def load_catalog(path: Path | str) -> Catalog:
    """Parse and validate ``catalog.yaml``.

    Raises ``ValueError`` if any intent's ``postcondition_kind`` is
    outside ``VALID_POSTCONDITION_KINDS``, if any ``<param:name>`` token
    in a template lacks a matching ``params:`` entry, or if any
    ``<derived:kind>`` token is outside ``KNOWN_DERIVED_KINDS``.
    """
    p = Path(path)
    raw = yaml.safe_load(p.read_text())

    schema_version = int(raw.get("schema_version", 0))
    if schema_version != 1:
        raise ValueError(f"{p}: unsupported schema_version {schema_version!r}; expected 1")

    declared_kinds = frozenset(raw.get("postcondition_kinds") or ())
    drift = declared_kinds.symmetric_difference(VALID_POSTCONDITION_KINDS)
    if drift:
        raise ValueError(
            f"{p}: postcondition_kinds drift from "
            f"scenarios.VALID_POSTCONDITION_KINDS: symmetric diff = {sorted(drift)!r}"
        )

    fixtures: dict[str, FixtureFacts] = {}
    for fname, fbody in (raw.get("fixtures") or {}).items():
        fixtures[fname] = FixtureFacts(
            name=fname,
            num_states=int(fbody["num_states"]),
            classes=tuple(fbody.get("classes") or ()),
            material_ids=tuple(int(x) for x in (fbody.get("material_ids") or ())),
            primal_svars=tuple(fbody.get("primal_svars") or ()),
            derived_results=tuple(fbody.get("derived_results") or ()),
        )

    intents: list[IntentRow] = []
    for intent_id, body in (raw.get("intents") or {}).items():
        intents.append(_parse_intent(intent_id, body, fixtures, p))

    return Catalog(schema_version=schema_version, fixtures=fixtures, intents=tuple(intents))


def _parse_intent(
    intent_id: str,
    body: dict[str, Any],
    fixtures: dict[str, FixtureFacts],
    path: Path,
) -> IntentRow:
    shape = str(body.get("shape", ""))
    if shape not in ("atomic", "compound"):
        raise ValueError(f"{path}: intent {intent_id!r} shape must be atomic|compound")

    raw_steps = body.get("steps") or []
    steps = tuple(
        StepTemplate(
            tool=str(step["tool"]),
            arguments_template=dict(step.get("arguments_template") or {}),
        )
        for step in raw_steps
    )
    if shape == "atomic" and len(steps) != 1:
        raise ValueError(f"{path}: atomic intent {intent_id!r} must have exactly 1 step")
    if shape == "compound" and len(steps) < 2:
        raise ValueError(f"{path}: compound intent {intent_id!r} must have >=2 steps")

    params: list[ParamSpec] = []
    raw_params = body.get("params") or {}
    for pname, pbody in raw_params.items():
        params.append(
            ParamSpec(
                name=pname,
                type=str(pbody.get("type", "")),
                binding=pbody.get("binding"),
                values=tuple(pbody["values"]) if "values" in pbody else None,
                range_from=pbody.get("range_from"),
            )
        )
    param_names = {p.name for p in params}

    kind = str(body.get("postcondition_kind", ""))
    if kind not in VALID_POSTCONDITION_KINDS:
        raise ValueError(
            f"{path}: intent {intent_id!r} unknown postcondition_kind "
            f"{kind!r}; expected one of {sorted(VALID_POSTCONDITION_KINDS)}"
        )

    expect_template = dict(body.get("expect_template") or {})
    _validate_tokens(expect_template, param_names, intent_id, "expect_template", path)
    for step in steps:
        _validate_tokens(
            step.arguments_template, param_names, intent_id,
            f"step({step.tool}).arguments_template", path,
        )

    fixture_bindings = tuple(body.get("fixture_bindings") or ())
    unknown_fixtures = set(fixture_bindings) - set(fixtures.keys())
    if unknown_fixtures:
        raise ValueError(
            f"{path}: intent {intent_id!r} fixture_bindings reference "
            f"unknown fixtures {sorted(unknown_fixtures)!r}; "
            f"known: {sorted(fixtures.keys())!r}"
        )

    paraphrase_seeds = tuple(body.get("paraphrase_seeds") or ())
    if not paraphrase_seeds:
        raise ValueError(f"{path}: intent {intent_id!r} has no paraphrase_seeds")

    return IntentRow(
        intent_id=intent_id,
        shape=shape,
        prose=str(body.get("prose", "")),
        steps=steps,
        params=tuple(params),
        postcondition_kind=kind,
        expect_template=expect_template,
        fixture_bindings=fixture_bindings,
        paraphrase_seeds=paraphrase_seeds,
    )


def _validate_tokens(
    tree: Any, param_names: set[str], intent_id: str, where: str, path: Path
) -> None:
    if isinstance(tree, dict):
        for v in tree.values():
            _validate_tokens(v, param_names, intent_id, where, path)
        return
    if isinstance(tree, list):
        for v in tree:
            _validate_tokens(v, param_names, intent_id, where, path)
        return
    if not isinstance(tree, str):
        return
    m_param = _PARAM_TOKEN.match(tree)
    if m_param is not None:
        name = m_param.group(1)
        if name not in param_names:
            raise ValueError(
                f"{path}: intent {intent_id!r} {where} references "
                f"<param:{name}> but params:{name} is undeclared"
            )
        return
    m_derived = _DERIVED_TOKEN.match(tree)
    if m_derived is not None:
        kind = m_derived.group(1)
        if kind not in KNOWN_DERIVED_KINDS:
            raise ValueError(
                f"{path}: intent {intent_id!r} {where} references unknown "
                f"<derived:{kind}>; known: {sorted(KNOWN_DERIVED_KINDS)!r}"
            )
        return
    # Literal — only flag if it *looks* like a slot token. Real string
    # values (e.g. "reset", "viridis") pass through untouched.
    if tree.startswith("<") and tree.endswith(">"):
        raise ValueError(
            f"{path}: intent {intent_id!r} {where} has slot-shaped string "
            f"{tree!r} that matches no known token form"
        )
