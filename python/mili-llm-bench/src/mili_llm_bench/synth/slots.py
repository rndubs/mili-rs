"""Slot resolution for Stage 3 scenario synthesis.

Takes a sampled ``bound`` parameter dict (e.g. ``{"mat_id": 3,
"enable_verb": False}``) plus a fixture-fact record and a paraphrase
seed, and produces the concrete ``(instruction, arguments_per_step,
expect)`` triple a scenario record needs.

Three derived-token kinds are recognised; see ``catalog.KNOWN_DERIVED_KINDS``:

* ``from_dir_and_fixture`` — ``step`` postcondition state index
  derived from the sampled direction and the fixture's ``num_states``.
* ``from_enable_and_mat_id`` — ``material`` postcondition
  ``hidden_materials`` list derived from the sampled enable_verb +
  material id, assuming the fixture starts with the material visible.
* ``from_parity_suite`` — ``query`` postcondition ``expect.table`` —
  captured live from pygriz at synth time (see the Stage 3 design note
  in ``planning/mili-viz/mili-agent/m5-sft-pipeline.md``; the catalog's
  ``query.todo_v2`` queues a future parity-suite cross-check).
"""

from __future__ import annotations

import re
from dataclasses import dataclass, field
from typing import Any, Callable

from .catalog import FixtureFacts, IntentRow

_PARAM_TOKEN = re.compile(r"^<(?:param|int|str|bool|enum|list\[int\]):([A-Za-z_][A-Za-z0-9_]*)>$")
_DERIVED_TOKEN = re.compile(r"^<derived:([A-Za-z_][A-Za-z0-9_]*)>$")


# Type alias for the optional live-oracle hook the synthesizer hands in
# for ``query`` scenarios. Stage 3 implementation passes a pygriz-backed
# closure; the round-trip test passes a deterministic stub.
QueryOracle = Callable[[FixtureFacts, dict[str, Any]], dict[str, Any]]


@dataclass(frozen=True)
class ResolvedScenario:
    """All the pieces ``run.py`` needs to assemble one JSONL record."""

    intent_id: str
    fixture: str
    instruction: str
    instruction_source: str
    postcondition_kind: str
    expect: dict[str, Any]
    # Diagnostic — surfaced in the report. Tool-call ground truth for
    # the per-step argument dicts (the rollout / SFT projection will
    # rebuild messages from these). Not consumed by the verifier.
    steps: tuple[dict[str, Any], ...] = field(default_factory=tuple)


def substitute(tree: Any, bound: dict[str, Any]) -> Any:
    """Walk a template tree, substituting ``<param:name>``-style tokens.

    Leaves non-token strings untouched and recurses into nested
    dicts/lists. ``<derived:...>`` tokens are *not* resolved here —
    callers route those through ``resolve_derived``.
    """
    if isinstance(tree, dict):
        return {k: substitute(v, bound) for k, v in tree.items()}
    if isinstance(tree, list):
        return [substitute(v, bound) for v in tree]
    if not isinstance(tree, str):
        return tree
    m = _PARAM_TOKEN.match(tree)
    if m is None:
        return tree
    name = m.group(1)
    if name not in bound:
        raise KeyError(f"slot {tree!r} requires bound[{name!r}] which is unset")
    return bound[name]


def resolve_derived(
    kind: str,
    *,
    bound: dict[str, Any],
    intent: IntentRow,
    fixture: FixtureFacts,
    query_oracle: QueryOracle | None = None,
) -> Any:
    if kind == "from_dir_and_fixture":
        direction = str(bound.get("dir", "")).upper()
        return {
            "FIRST": 1,
            "NEXT": 2,  # after a fresh load+step, cursor at 2
            "LAST": int(fixture.num_states),
        }[direction]

    if kind == "from_enable_and_mat_id":
        enable = bool(bound["enable_verb"])
        mat_id = int(bound["mat_id"])
        return [] if enable else [mat_id]

    if kind == "from_parity_suite":
        if query_oracle is None:
            raise RuntimeError(
                f"intent {intent.intent_id!r} requires a query oracle to "
                "resolve <derived:from_parity_suite>; none was provided"
            )
        return query_oracle(fixture, bound)

    raise ValueError(f"unknown derived kind {kind!r}")


def resolve_expect(
    template: dict[str, Any],
    *,
    bound: dict[str, Any],
    intent: IntentRow,
    fixture: FixtureFacts,
    query_oracle: QueryOracle | None = None,
) -> dict[str, Any]:
    """Resolve ``expect_template`` into the concrete ``expect`` dict.

    Walks the template tree exactly like ``substitute``, but additionally
    replaces ``<derived:kind>`` leaves with the resolver's output.
    """

    def walk(node: Any) -> Any:
        if isinstance(node, dict):
            return {k: walk(v) for k, v in node.items()}
        if isinstance(node, list):
            return [walk(v) for v in node]
        if not isinstance(node, str):
            return node
        m_d = _DERIVED_TOKEN.match(node)
        if m_d is not None:
            return resolve_derived(
                m_d.group(1),
                bound=bound,
                intent=intent,
                fixture=fixture,
                query_oracle=query_oracle,
            )
        m_p = _PARAM_TOKEN.match(node)
        if m_p is not None:
            return bound[m_p.group(1)]
        return node

    out = walk(template)
    if not isinstance(out, dict):
        raise TypeError(f"resolved expect must be a dict, got {type(out).__name__}")
    return out


def format_paraphrase(seed: str, bound: dict[str, Any]) -> str:
    """Apply ``str.format`` to a paraphrase seed using the bound params.

    Returns the rendered instruction. Catalog seeds that do not
    reference a sampled param (e.g. ``"reset the view"``) pass through
    unchanged. Missing keys raise ``KeyError`` so a malformed seed
    fails loudly at synth time, not silently.
    """
    # Seeds may legitimately include literal braces; the catalog
    # author's responsibility is to escape those as ``{{`` / ``}}``.
    # All current catalog seeds use the plain ``{name}`` form.
    rendered = seed.format(**_str_view(bound))
    return rendered


def _str_view(bound: dict[str, Any]) -> dict[str, str]:
    """Render bound values as user-facing strings for ``str.format``.

    Booleans render as the catalog-author intended (``"enable"`` /
    ``"hide"``) only via the seed-template choice, not here — this
    helper just stringifies. Lists render as comma-joined to keep
    paraphrases readable.
    """
    out: dict[str, str] = {}
    for k, v in bound.items():
        if isinstance(v, bool):
            out[k] = str(v).lower()
        elif isinstance(v, list):
            out[k] = ", ".join(str(x) for x in v)
        else:
            out[k] = str(v)
    return out
