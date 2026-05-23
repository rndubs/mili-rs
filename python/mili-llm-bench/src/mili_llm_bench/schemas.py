"""W1 — auto-derive ``tools.json`` from the frozen ``mili_viz.proto``.

Source of truth: ``crates/mili-viz-proto/proto/mili_viz.proto``. The
tool list, input schemas, and output projections pin the surface the
LLM is shown. Output schemas mirror the projected response shapes
pinned in ``planning/mili-viz/agent-local-llm-baseline.md`` §W1 — the
W4a harness will project to these shapes; the three harness
invariants (no ``state_times`` / ``flight_ticket`` / ``agent`` ever
reach the LLM) are pre-enforced here at the schema layer.

Proto-introspection choice: **(a) hand-parse the .proto text** rather
than (b) compile via ``grpc_tools.protoc`` and reflect. The proto is
small, regular, frozen by ``phase-4-m9.md`` (the second additive
field — ``CutPlane.slice_only`` — was the last change), and a tiny
parser keeps the v0 dep surface small (no ``grpcio-tools`` runtime
dep just for schema derivation).
"""

from __future__ import annotations

import json
import re
from dataclasses import dataclass, field
from pathlib import Path
from typing import Any

# ---------------------------------------------------------------------------
# Tool inventory — 15 typed Command variants + 2 read tools + 1 fallback.
# Excluded from v0: `raw` (the Layer-0 escape hatch — `griz_raw` exposes
# it under a dedicated tool slot with the right argument shape) and
# `render` (offscreen capture, not session manipulation).
# ---------------------------------------------------------------------------

TYPED_COMMAND_TOOLS: list[tuple[str, str, str]] = [
    # (tool_name, oneof_field_name, proto_message_name)
    ("load", "load", "Load"),
    ("close", "close", "Close"),
    ("set_state", "set_state", "SetState"),
    ("step", "step", "Step"),
    ("select", "select", "Select"),
    ("clrsel", "clrsel", "ClearSelection"),
    ("show", "show", "Show"),
    ("view", "view", "View"),
    ("iso", "iso", "Isosurface"),
    ("contour", "contour", "Contour"),
    ("material", "material", "MaterialVisibility"),
    ("cutplane", "cutplane", "CutPlane"),
    ("colormap", "colormap", "Colormap"),
    ("legend", "legend", "LegendLimits"),
    ("named_view", "named_view", "NamedView"),
]

# Commands intentionally NOT in v0:
EXCLUDED_COMMANDS: set[str] = {"raw", "render"}

# Short, model-facing descriptions. The proto comments are too verbose
# for prompts; these are tight by design.
TOOL_DESCRIPTIONS: dict[str, str] = {
    "load": "Load a Mili database by root path (e.g. 'd3samp6').",
    "close": "Close the currently loaded database.",
    "set_state": "Jump to a specific 1-based state index.",
    "step": "Step the state cursor: NEXT, PREV, FIRST, or LAST.",
    "select": "Add elements/nodes of one class to the selection by range string.",
    "clrsel": "Clear the selection for one class (empty class clears all).",
    "show": "Color the mesh by a result (primal svar or derived family).",
    "view": "Manipulate the camera (rotate / translate / scale / zoom / set / reset).",
    "iso": "Toggle isosurfaces for a result with explicit levels or a count.",
    "contour": "Draw scalar contour lines for a result with a given count.",
    "material": "Enable or disable a material (whole-class when material is omitted).",
    "cutplane": "Set or clear the cut-plane (origin + normal; relative or absolute).",
    "colormap": "Pick a named colormap (e.g. 'cool', 'jet').",
    "legend": "Set the legend's min/max range (omit a bound to autoscale that end).",
    "named_view": "Save, restore, or list named camera views.",
    "query": "Query result values for a class/labels/states subset.",
    "snapshot": "Read the current session state (state, selection, result, camera).",
    "griz_raw": "Escape hatch: run one raw griz/grizinit line (Layer-0).",
}

# Output schemas — the W1 projection table from baseline.md. The W4a
# harness will project tool responses through these shapes before they
# reach the LLM. Pinning them here pre-enforces the three harness
# invariants (no state_times / flight_ticket / agent) at the schema
# layer so the projection cannot grow back the forbidden fields.

# Default for tools whose response is just acknowledge-or-error.
_OUT_OK_ONLY: dict[str, Any] = {
    "type": "object",
    "properties": {
        "ok": {"type": "boolean"},
        "error": {"type": "string"},
    },
    "required": ["ok"],
    "additionalProperties": False,
}

OUTPUT_SCHEMAS: dict[str, dict[str, Any]] = {
    "load": {
        "type": "object",
        "properties": {
            "ok": {"type": "boolean"},
            "num_states": {"type": "integer"},
            "num_classes": {"type": "integer"},
            "classes": {"type": "array", "items": {"type": "string"}},
            "state_time_range": {
                "type": "array",
                "items": {"type": "number"},
                "minItems": 2,
                "maxItems": 2,
            },
            "current_time": {"type": "number"},
            "error": {"type": "string"},
        },
        "required": ["ok"],
        "additionalProperties": False,
    },
    "set_state": {
        "type": "object",
        "properties": {
            "ok": {"type": "boolean"},
            "state": {"type": "integer"},
            "num_states": {"type": "integer"},
            "current_time": {"type": "number"},
            "error": {"type": "string"},
        },
        "required": ["ok"],
        "additionalProperties": False,
    },
    "step": {
        "type": "object",
        "properties": {
            "ok": {"type": "boolean"},
            "state": {"type": "integer"},
            "num_states": {"type": "integer"},
            "current_time": {"type": "number"},
            "error": {"type": "string"},
        },
        "required": ["ok"],
        "additionalProperties": False,
    },
    "select": {
        "type": "object",
        "properties": {
            "ok": {"type": "boolean"},
            "selection": {
                "type": "object",
                "additionalProperties": {"type": "string"},
            },
            "error": {"type": "string"},
        },
        "required": ["ok"],
        "additionalProperties": False,
    },
    "clrsel": {
        "type": "object",
        "properties": {
            "ok": {"type": "boolean"},
            "selection": {
                "type": "object",
                "additionalProperties": {"type": "string"},
            },
            "error": {"type": "string"},
        },
        "required": ["ok"],
        "additionalProperties": False,
    },
    "show": {
        "type": "object",
        "properties": {
            "ok": {"type": "boolean"},
            "result": {"type": "string"},
            "component": {"type": "string"},
            "range": {
                "type": "array",
                "items": {"type": "number"},
                "minItems": 2,
                "maxItems": 2,
            },
            "error": {"type": "string"},
        },
        "required": ["ok"],
        "additionalProperties": False,
    },
    "material": {
        "type": "object",
        "properties": {
            "ok": {"type": "boolean"},
            "hidden_materials": {
                "type": "array",
                "items": {"type": "integer"},
            },
            "error": {"type": "string"},
        },
        "required": ["ok"],
        "additionalProperties": False,
    },
    "query": {
        # The harness fills `table` with a compact projection of the
        # InlineTable payload (or fetches Flight if any). W4a pins the
        # exact shape; v0 leaves it permissive so the LLM is not
        # surprised by a structurally varied response.
        "type": "object",
        "properties": {
            "ok": {"type": "boolean"},
            "table": {"type": "object"},
            "error": {"type": "string"},
        },
        "required": ["ok"],
        "additionalProperties": False,
    },
    "snapshot": {
        # Pruned LoadedState + ResultState; state_times stripped (use
        # the load tool for the range), GeometryRef dropped wholesale
        # (no flight_ticket), AgentTranscript stripped.
        "type": "object",
        "properties": {
            "state": {"type": "integer"},
            "num_states": {"type": "integer"},
            "current_time": {"type": "number"},
            "classes": {"type": "array", "items": {"type": "string"}},
            "selection": {
                "type": "object",
                "additionalProperties": {"type": "string"},
            },
            "result": {
                "type": "object",
                "properties": {
                    "result": {"type": "string"},
                    "component": {"type": "string"},
                    "range": {
                        "type": "array",
                        "items": {"type": "number"},
                        "minItems": 2,
                        "maxItems": 2,
                    },
                },
                "additionalProperties": False,
            },
            "hidden_materials": {
                "type": "array",
                "items": {"type": "integer"},
            },
            "camera": {
                "type": "object",
                "properties": {
                    "azimuth": {"type": "number"},
                    "elevation": {"type": "number"},
                    "distance": {"type": "number"},
                    "focus": {
                        "type": "array",
                        "items": {"type": "number"},
                        "minItems": 3,
                        "maxItems": 3,
                    },
                },
                "additionalProperties": False,
            },
        },
        "additionalProperties": False,
    },
    "griz_raw": {
        "type": "object",
        "properties": {
            "ok": {"type": "boolean"},
            "output": {"type": "string"},
            "error": {"type": "string"},
        },
        "required": ["ok"],
        "additionalProperties": False,
    },
}

# Forbidden field names — pre-enforced anywhere in the output_schema
# tree by the W1 invariant tests. Mirrors the W4a harness invariants
# (baseline.md §W1 "Harness invariants").
FORBIDDEN_OUTPUT_FIELDS: tuple[str, ...] = ("state_times", "flight_ticket", "agent")


# ---------------------------------------------------------------------------
# Proto parser — small, focused, regular-language only. Handles just
# what `mili_viz.proto` actually uses: messages with scalar / enum /
# nested-message / repeated / optional / oneof / map fields. NOT a
# general protobuf parser.
# ---------------------------------------------------------------------------

_SCALAR_TO_JSON: dict[str, dict[str, Any]] = {
    "string": {"type": "string"},
    "bytes": {"type": "string", "contentEncoding": "base64"},
    "bool": {"type": "boolean"},
    "double": {"type": "number"},
    "float": {"type": "number"},
    "int32": {"type": "integer"},
    "int64": {"type": "integer"},
    "uint32": {"type": "integer", "minimum": 0},
    "uint64": {"type": "integer", "minimum": 0},
    "sint32": {"type": "integer"},
    "sint64": {"type": "integer"},
    "fixed32": {"type": "integer", "minimum": 0},
    "fixed64": {"type": "integer", "minimum": 0},
}


@dataclass
class ProtoField:
    name: str
    proto_type: str  # e.g. "string", "Load", "map<string, string>"
    is_repeated: bool = False
    is_optional: bool = False
    is_map: bool = False
    map_key: str | None = None
    map_value: str | None = None


@dataclass
class ProtoOneof:
    name: str
    fields: list[ProtoField] = field(default_factory=list)


@dataclass
class ProtoEnum:
    name: str
    values: list[str] = field(default_factory=list)


@dataclass
class ProtoMessage:
    name: str
    fields: list[ProtoField] = field(default_factory=list)
    oneofs: list[ProtoOneof] = field(default_factory=list)
    enums: list[ProtoEnum] = field(default_factory=list)


def _strip_comments(text: str) -> str:
    text = re.sub(r"//.*", "", text)
    text = re.sub(r"/\*.*?\*/", "", text, flags=re.S)
    return text


def _parse_field_line(line: str) -> ProtoField | None:
    """Parse one top-of-block field line. Returns None if not a field."""
    line = line.strip().rstrip(";").strip()
    if not line:
        return None
    # map<K, V> name = N;
    m = re.match(
        r"^map\s*<\s*([\w.]+)\s*,\s*([\w.]+)\s*>\s+(\w+)\s*=\s*\d+$",
        line,
    )
    if m:
        return ProtoField(
            name=m.group(3),
            proto_type=f"map<{m.group(1)}, {m.group(2)}>",
            is_map=True,
            map_key=m.group(1),
            map_value=m.group(2),
        )
    # [repeated|optional] Type name = N;
    m = re.match(
        r"^(?:(repeated|optional)\s+)?([\w.]+)\s+(\w+)\s*=\s*\d+$",
        line,
    )
    if m:
        modifier, ptype, name = m.group(1), m.group(2), m.group(3)
        return ProtoField(
            name=name,
            proto_type=ptype,
            is_repeated=(modifier == "repeated"),
            is_optional=(modifier == "optional"),
        )
    return None


def _parse_messages(text: str) -> dict[str, ProtoMessage]:
    """Walk top-level + nested messages out of stripped proto text."""
    messages: dict[str, ProtoMessage] = {}

    # Find every "message Name { ... }" with balanced braces.
    # The proto file is small enough to walk char-by-char.
    i = 0
    while i < len(text):
        m = re.search(r"\bmessage\s+(\w+)\s*\{", text[i:])
        if not m:
            break
        name = m.group(1)
        start = i + m.end()  # position just after '{'
        depth = 1
        j = start
        while j < len(text) and depth > 0:
            if text[j] == "{":
                depth += 1
            elif text[j] == "}":
                depth -= 1
            j += 1
        body = text[start : j - 1]
        messages[name] = _parse_message_body(name, body)
        # nested messages live inside body — recurse for completeness.
        for nested_name, nested_msg in _parse_messages(body).items():
            messages.setdefault(nested_name, nested_msg)
        i = j
    return messages


def _parse_message_body(name: str, body: str) -> ProtoMessage:
    msg = ProtoMessage(name=name)
    i = 0
    while i < len(body):
        # Skip whitespace
        while i < len(body) and body[i] in " \t\n\r":
            i += 1
        if i >= len(body):
            break
        rest = body[i:]
        # Nested message — skip; collected by the outer walker.
        m = re.match(r"message\s+\w+\s*\{", rest)
        if m:
            depth = 1
            j = i + m.end()
            while j < len(body) and depth > 0:
                if body[j] == "{":
                    depth += 1
                elif body[j] == "}":
                    depth -= 1
                j += 1
            i = j
            continue
        # Enum.
        m = re.match(r"enum\s+(\w+)\s*\{([^}]*)\}", rest)
        if m:
            enum = ProtoEnum(name=m.group(1))
            for ev in m.group(2).split(";"):
                ev = ev.strip()
                if not ev:
                    continue
                em = re.match(r"^(\w+)\s*=\s*\d+$", ev)
                if em:
                    enum.values.append(em.group(1))
            msg.enums.append(enum)
            i += m.end()
            continue
        # Oneof.
        m = re.match(r"oneof\s+(\w+)\s*\{", rest)
        if m:
            oneof = ProtoOneof(name=m.group(1))
            depth = 1
            j = i + m.end()
            start = j
            while j < len(body) and depth > 0:
                if body[j] == "{":
                    depth += 1
                elif body[j] == "}":
                    depth -= 1
                j += 1
            inner = body[start : j - 1]
            for line in inner.split(";"):
                f = _parse_field_line(line)
                if f is not None:
                    oneof.fields.append(f)
            msg.oneofs.append(oneof)
            i = j
            continue
        # Field — read up to next semicolon.
        semi = body.find(";", i)
        if semi == -1:
            break
        line = body[i:semi]
        f = _parse_field_line(line)
        if f is not None:
            msg.fields.append(f)
        i = semi + 1
    return msg


# ---------------------------------------------------------------------------
# Proto → JSON Schema for INPUT shapes.
# ---------------------------------------------------------------------------


def _type_to_schema(ptype: str, messages: dict[str, ProtoMessage]) -> dict[str, Any]:
    if ptype in _SCALAR_TO_JSON:
        return dict(_SCALAR_TO_JSON[ptype])
    if ptype in messages:
        return _message_to_input_schema(messages[ptype], messages)
    # Unknown — treat as opaque object. We do not expect this for the
    # 15 typed Command variants pinned in TYPED_COMMAND_TOOLS.
    return {"type": "object"}


def _field_to_schema(
    f: ProtoField, messages: dict[str, ProtoMessage]
) -> tuple[str, dict[str, Any]]:
    if f.is_map:
        assert f.map_value is not None
        return f.name, {
            "type": "object",
            "additionalProperties": _type_to_schema(f.map_value, messages),
        }
    base = _type_to_schema(f.proto_type, messages)
    if f.is_repeated:
        return f.name, {"type": "array", "items": base}
    return f.name, base


def _message_to_input_schema(
    msg: ProtoMessage, messages: dict[str, ProtoMessage]
) -> dict[str, Any]:
    # Enums-as-strings: nested enum becomes a string with `enum`.
    enum_index = {e.name: e for e in msg.enums}

    properties: dict[str, dict[str, Any]] = {}

    for f in msg.fields:
        if f.proto_type in enum_index:
            properties[f.name] = {
                "type": "string",
                "enum": list(enum_index[f.proto_type].values),
            }
        else:
            _, sub = _field_to_schema(f, messages)
            properties[f.name] = sub

    # Oneofs become a JSON Schema `oneOf` of single-key objects: each
    # arm declares exactly one of the variant fields. This mirrors the
    # proto semantics (exactly one set at a time) cleanly for the LLM.
    one_of_clauses: list[dict[str, Any]] = []
    for oneof in msg.oneofs:
        clauses: list[dict[str, Any]] = []
        for vf in oneof.fields:
            if vf.proto_type in enum_index:
                arm_schema: dict[str, Any] = {
                    "type": "string",
                    "enum": list(enum_index[vf.proto_type].values),
                }
            else:
                _, arm_schema = _field_to_schema(vf, messages)
            properties[vf.name] = arm_schema
            clauses.append({"required": [vf.name]})
        if clauses:
            one_of_clauses.append({"oneOf": clauses})

    schema: dict[str, Any] = {
        "type": "object",
        "properties": properties,
        "additionalProperties": False,
    }
    if one_of_clauses and len(one_of_clauses) == 1:
        schema["oneOf"] = one_of_clauses[0]["oneOf"]
    elif one_of_clauses:
        schema["allOf"] = one_of_clauses
    return schema


# ---------------------------------------------------------------------------
# Public derivation entry point.
# ---------------------------------------------------------------------------


def find_proto_path(start: Path | None = None) -> Path:
    """Locate ``crates/mili-viz-proto/proto/mili_viz.proto`` by walking up."""
    p = (start or Path(__file__)).resolve()
    for parent in [p, *p.parents]:
        candidate = parent / "crates" / "mili-viz-proto" / "proto" / "mili_viz.proto"
        if candidate.exists():
            return candidate
    raise FileNotFoundError(
        "could not locate crates/mili-viz-proto/proto/mili_viz.proto"
    )


def derive_tools(proto_path: Path | None = None) -> list[dict[str, Any]]:
    """Parse the proto, build the 18-tool table (input + output schema).

    Output: a list of ``{name, description, input_schema, output_schema}``
    entries, sorted by ``name`` so the artifact diff is stable.
    """
    path = proto_path or find_proto_path()
    text = _strip_comments(path.read_text())
    messages = _parse_messages(text)

    # Verify the Command oneof covers exactly TYPED_COMMAND_TOOLS plus
    # EXCLUDED_COMMANDS. A drift here means the proto changed without
    # this artifact catching up — flag it loudly so the honest-diff
    # test points the operator at the right file.
    command = messages.get("Command")
    if command is None:
        raise RuntimeError("Command message not found in proto")
    if not command.oneofs:
        raise RuntimeError("Command.cmd oneof not found")
    command_oneof = command.oneofs[0]
    proto_names = {f.name for f in command_oneof.fields}
    declared = {oneof_name for _, oneof_name, _ in TYPED_COMMAND_TOOLS}
    missing = declared - proto_names
    unexpected = proto_names - declared - EXCLUDED_COMMANDS
    if missing or unexpected:
        raise RuntimeError(
            "Command oneof drift vs TYPED_COMMAND_TOOLS: "
            f"missing={sorted(missing)} unexpected={sorted(unexpected)}"
        )

    tools: list[dict[str, Any]] = []
    for tool_name, _oneof_name, message_name in TYPED_COMMAND_TOOLS:
        msg = messages.get(message_name)
        if msg is None:
            raise RuntimeError(f"proto message {message_name!r} missing")
        input_schema = _message_to_input_schema(msg, messages)
        output_schema = OUTPUT_SCHEMAS.get(tool_name, _OUT_OK_ONLY)
        tools.append(
            {
                "name": tool_name,
                "description": TOOL_DESCRIPTIONS[tool_name],
                "input_schema": input_schema,
                "output_schema": output_schema,
            }
        )

    # Read tools — hand-registered. `query` mirrors the QueryRequest
    # proto fields; `snapshot` takes no inputs (one-shot Subscribe
    # opening Snapshot projection).
    query_msg = messages.get("QueryRequest")
    if query_msg is None:
        raise RuntimeError("QueryRequest message not found in proto")
    tools.append(
        {
            "name": "query",
            "description": TOOL_DESCRIPTIONS["query"],
            "input_schema": _message_to_input_schema(query_msg, messages),
            "output_schema": OUTPUT_SCHEMAS["query"],
        }
    )
    tools.append(
        {
            "name": "snapshot",
            "description": TOOL_DESCRIPTIONS["snapshot"],
            "input_schema": {
                "type": "object",
                "properties": {},
                "additionalProperties": False,
            },
            "output_schema": OUTPUT_SCHEMAS["snapshot"],
        }
    )

    # Fallback — `griz_raw` is the Layer-0 escape hatch; lowers to the
    # excluded `Command.raw` (a single griz/grizinit line).
    tools.append(
        {
            "name": "griz_raw",
            "description": TOOL_DESCRIPTIONS["griz_raw"],
            "input_schema": {
                "type": "object",
                "properties": {"line": {"type": "string"}},
                "required": ["line"],
                "additionalProperties": False,
            },
            "output_schema": OUTPUT_SCHEMAS["griz_raw"],
        }
    )

    tools.sort(key=lambda t: t["name"])
    return tools


def dump_tools_json(tools: list[dict[str, Any]]) -> str:
    """Canonical pretty-print: 2-space indent, sorted by name, trailing
    newline. Matches what the honest-diff test re-derives and diffs."""
    return json.dumps(tools, indent=2, sort_keys=False) + "\n"


def default_artifact_path(start: Path | None = None) -> Path:
    """``data/posttraining/grammar/tools.json`` relative to the repo root."""
    p = (start or Path(__file__)).resolve()
    for parent in [p, *p.parents]:
        if (parent / "crates" / "mili-viz-proto" / "proto" / "mili_viz.proto").exists():
            return parent / "data" / "posttraining" / "grammar" / "tools.json"
    raise FileNotFoundError("could not locate repo root from " + str(p))
