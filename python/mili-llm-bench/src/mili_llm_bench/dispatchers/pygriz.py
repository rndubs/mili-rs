"""``PygrizDispatcher`` — typed Commands → pygriz typed helpers.

This is the **only** file in the ``mili-llm-bench`` package that
imports ``pygriz``. ``pygriz`` is an optional dep
(``[project.optional-dependencies].pygriz``) so the always-on test
path (W4a harness tests, W2 scenarios, W3 verifier) stays GPU-free /
server-free / pygriz-free.

Lowering follows the table in baseline.md §W4a "Dispatcher":

* ``set_state`` / ``step`` → ``s.state = n`` / ``s.next()`` / ``s.prev()`` / ``s.first()`` / ``s.last()``
* ``select`` → ``s.select(class_name, range)``
* ``clrsel`` → ``s.selection.clear(class_name)``
* ``show`` → ``s.show(result[, component])``
* ``material`` → ``s.materials.enable(...)`` / ``s.materials.disable(...)``
* ``view`` → ``s.view.*`` (op-dispatched)
* ``cutplane`` / ``iso`` / ``contour`` / ``colormap`` / ``legend`` / ``named_view`` → typed helpers
* ``load`` / ``close`` → ``s.open(root)`` / ``s.close()``
* ``griz_raw`` → ``s.command(raw)``
* ``query`` / ``snapshot`` → pygriz read paths

After every successful typed call the adapter reads a fresh snapshot
(or the appropriate per-tool affordance) and projects it through the
W1 response table; the harness then runs the defensive
``_project_response`` belt on top. Argument-level failures
(``nonexistent_material``/``_class``/``_result``/``state_out_of_range``)
are tagged via best-effort substring match on the pygriz error message;
classification falls back to ``dispatch_error`` when the message
doesn't match any closed-set pattern.
"""

from __future__ import annotations

import json
import os
import sys
from dataclasses import dataclass
from pathlib import Path
from typing import Any, Callable, Iterator

from ..scenarios import Scenario, default_bootstrap_path

# Set MILI_DISPATCH_LOG=1 to print every dispatch call (name, arguments,
# response) to stderr. Goes through `print` (not logging) so it shows up
# without needing the caller to configure log levels — the root logger
# defaults to WARNING and silently ate our INFO lines on the first try.
_DISPATCH_LOG = os.environ.get("MILI_DISPATCH_LOG", "").lower() in ("1", "true", "yes")

# ``pygriz`` is intentionally imported lazily — at adapter-construction
# time, not at module-import time — so importing this module from
# ``mili_llm_bench.dispatchers.pygriz`` on a machine that does not have
# ``pygriz`` installed still yields a clear ``ImportError`` only when a
# caller actually constructs ``PygrizDispatcher``.


def _import_pygriz() -> Any:
    try:
        import griz  # type: ignore[import-not-found]
    except ImportError as exc:  # pragma: no cover — exercised on the user's box.
        raise ImportError(
            "PygrizDispatcher requires the 'pygriz' optional dependency. "
            "Install with `pip install mili-llm-bench[pygriz]` or "
            "`pip install -e python/pygriz`."
        ) from exc
    return griz


# Best-effort substring patterns for argument-level error classification.
# These mirror the closed L2 set in ``verifier._L2_ARG_FAILS``. The
# patterns intentionally match pygriz's error-message conventions
# (lowercased before matching) — drift in pygriz wording falls back to
# the generic ``dispatch_error`` (and one of the always-on tests in
# ``test_harness.py`` pins the closed set).

_ERROR_PATTERNS: tuple[tuple[str, str], ...] = (
    ("nonexistent_material", "unknown material"),
    ("nonexistent_material", "no such material"),
    ("nonexistent_class", "unknown class"),
    ("nonexistent_class", "no such class"),
    ("nonexistent_result", "unknown result"),
    ("nonexistent_result", "no such result"),
    ("nonexistent_result", "no such svar"),
    ("state_out_of_range", "state out of range"),
    ("state_out_of_range", "no such state"),
    ("state_out_of_range", "invalid state"),
)


def _classify(msg: str) -> str:
    low = msg.lower()
    for tag, pattern in _ERROR_PATTERNS:
        if pattern in low:
            return tag
    return "dispatch_error"


# ---------------------------------------------------------------------------
# Per-tool lowering helpers. Each returns the *projected* response dict
# (the W1 table shape). On failure they raise — the harness catches the
# exception and tags ``error_kind`` via the defensive belt, but we
# pre-tag in the adapter (via the ``ok=False`` shape) when we already
# have a classified message, so the model sees the most specific label
# available.
# ---------------------------------------------------------------------------


def _proj_load(session: Any, _arguments: dict[str, Any], result: Any) -> dict[str, Any]:
    snap = session._snapshot()
    loaded = getattr(snap, "loaded", None)
    state_times = list(getattr(loaded, "state_times", []) or [])
    state_time_range: list[float] = (
        [float(state_times[0]), float(state_times[-1])] if state_times else [0.0, 0.0]
    )
    classes = list(getattr(loaded, "classes", []) or [])
    return {
        "ok": True,
        "action_complete": True,
        "num_states": int(getattr(loaded, "num_states", 0)),
        "num_classes": len(classes),
        "classes": classes,
        "state_time_range": state_time_range,
        "current_time": float(getattr(snap, "current_time", 0.0)),
    }


def _proj_state(session: Any, arguments: dict[str, Any] | None = None) -> dict[str, Any]:
    snap = session._snapshot()
    loaded = getattr(snap, "loaded", None)
    current_state = int(getattr(snap, "state", 0))

    # Check if requested state matches current state for completion signal
    action_complete = True
    requested_state = None
    if arguments:
        requested_state = arguments.get("state")
        if requested_state is not None:
            requested_state = int(requested_state)
            action_complete = (requested_state == current_state)

    result = {
        "ok": True,
        "state": current_state,
        "num_states": int(getattr(loaded, "num_states", 0)),
        "current_time": float(getattr(snap, "current_time", 0.0)),
    }

    if requested_state is not None:
        result["requested_state"] = requested_state
        result["action_complete"] = action_complete

    return result


def _proj_selection(session: Any) -> dict[str, Any]:
    snap = session._snapshot()
    sel = getattr(snap, "selection", None)
    by_class = dict(getattr(sel, "by_class", {}) or {})
    return {
        "ok": True,
        "action_complete": True,
        "selection": {k: v for k, v in by_class.items() if v},
    }


def _proj_show(_session: Any, arguments: dict[str, Any], result: Any) -> dict[str, Any]:
    rng = getattr(result, "range", (0.0, 0.0))
    return {
        "ok": True,
        "action_complete": True,
        "result": arguments.get("result", ""),
        "component": arguments.get("component", ""),
        "range": [float(rng[0]), float(rng[1])],
    }


def _proj_materials(session: Any) -> dict[str, Any]:
    snap = session._snapshot()
    mats = getattr(snap, "materials", None)
    visible_map = dict(getattr(mats, "visible", {}) or {})
    hidden = sorted(int(k) for k, v in visible_map.items() if not v)
    return {
        "ok": True,
        "action_complete": True,
        "hidden_materials": hidden,
    }


def _proj_snapshot(session: Any) -> dict[str, Any]:
    snap = session._snapshot()
    loaded = getattr(snap, "loaded", None)
    sel = getattr(snap, "selection", None)
    res = getattr(snap, "result", None)
    cam = getattr(snap, "camera", None)
    mats = getattr(snap, "materials", None)

    classes = list(getattr(loaded, "classes", []) or [])
    by_class = dict(getattr(sel, "by_class", {}) or {})
    visible_map = dict(getattr(mats, "visible", {}) or {})
    hidden_materials = sorted(int(k) for k, v in visible_map.items() if not v)

    res_dict: dict[str, Any] | None = None
    if res is not None:
        rng = getattr(res, "range", None)
        res_dict = {
            "result": str(getattr(res, "result", "")),
            "component": str(getattr(res, "component", "")),
            "range": (
                [float(rng[0]), float(rng[1])]
                if rng is not None and len(rng) == 2
                else [0.0, 0.0]
            ),
        }

    cam_dict: dict[str, Any] | None = None
    if cam is not None:
        focus = list(getattr(cam, "focus", []) or [0.0, 0.0, 0.0])
        if len(focus) != 3:
            focus = [0.0, 0.0, 0.0]
        cam_dict = {
            "azimuth": float(getattr(cam, "azimuth", 0.0)),
            "elevation": float(getattr(cam, "elevation", 0.0)),
            "distance": float(getattr(cam, "distance", 0.0)),
            "focus": [float(x) for x in focus],
        }

    out: dict[str, Any] = {
        "state": int(getattr(snap, "state", 0)),
        "num_states": int(getattr(loaded, "num_states", 0)),
        "current_time": float(getattr(snap, "current_time", 0.0)),
        "classes": classes,
        "selection": {k: v for k, v in by_class.items() if v},
        "hidden_materials": hidden_materials,
    }
    if res_dict is not None:
        out["result"] = res_dict
    if cam_dict is not None:
        out["camera"] = cam_dict
    return out


def _proj_griz_raw(reply: Any) -> dict[str, Any]:
    output = getattr(reply, "output", "") or ""
    ok = bool(getattr(reply, "ok", True))
    out: dict[str, Any] = {"ok": ok, "output": str(output)}
    if not ok:
        err = getattr(reply, "error", "") or ""
        out["error"] = str(err)
        out["error_kind"] = _classify(str(err))
    return out


def _project_query_result(qr: Any) -> dict[str, Any]:
    """Project a ``griz.QueryResult`` into the JSON-stable dict shape
    the verifier compares against ``expect.table``. Row-major
    ``[state][label][component]`` reshape so plain ``==`` works."""
    labels = [int(x) for x in qr.labels]
    states = [int(x) for x in qr.states]
    components = int(qr.components)
    flat = [float(x) for x in qr.values]
    rows: list[list[list[float]]] = []
    if states and labels and components:
        stride = len(labels) * components
        for si in range(len(states)):
            row = []
            base = si * stride
            for li in range(len(labels)):
                cell = flat[base + li * components : base + (li + 1) * components]
                row.append(cell)
            rows.append(row)
    return {
        "ok": True,
        "result": str(qr.result),
        "class_name": str(qr.class_name),
        "labels": labels,
        "states": states,
        "components": components,
        "values": rows,
    }


def _step_dir(arguments: dict[str, Any]) -> Callable[[Any], None]:
    direction = str(arguments.get("dir", "")).upper()
    if direction == "NEXT":
        return lambda s: s.next()
    if direction == "PREV":
        return lambda s: s.prev()
    if direction == "FIRST":
        return lambda s: s.first()
    if direction == "LAST":
        return lambda s: s.last()
    raise ValueError(f"unknown step direction {direction!r}")


@dataclass
class PygrizDispatcher:
    """Dispatcher backed by a live ``griz.Session``.

    Construction does not import ``pygriz`` — only the first
    ``dispatch`` call does (lazy via ``_import_pygriz()``). This keeps
    test-only imports of ``mili_llm_bench.dispatchers.pygriz`` cheap on
    a machine that does not have ``pygriz`` installed.

    Carries a ``close()`` method the W4b driver calls in a ``finally``
    block after each scenario; see ``pygriz_dispatcher_factory``'s
    teardown story.
    """

    session: Any  # griz.Session — typed as Any so the import stays lazy.

    def close(self) -> None:
        s = self.session
        if s is None:
            return
        try:
            s.close()
        except Exception:
            pass
        self.session = None

    def dispatch(self, name: str, arguments: dict[str, Any]) -> dict[str, Any]:
        try:
            response = self._dispatch_inner(name, arguments)
        except Exception as exc:
            msg = str(exc)
            response = {
                "ok": False,
                "error": msg,
                "error_kind": _classify(msg),
            }
        if _DISPATCH_LOG:
            try:
                args_json = json.dumps(arguments, default=str)
                resp_json = json.dumps(response, default=str)
            except Exception:
                args_json, resp_json = repr(arguments), repr(response)
            print(
                f"[dispatch] {name} args={args_json} resp={resp_json}",
                file=sys.stderr,
                flush=True,
            )
        return response

    def _dispatch_inner(
        self, name: str, arguments: dict[str, Any]
    ) -> dict[str, Any]:
        s = self.session

        if name == "load":
            # Resolve the bare fixture name to the absolute .A path the
            # mili-viz-server's ``Database::open`` actually opens.
            # Without this, a model-emitted ``load(root="d3samp6")``
            # would replace the factory-preloaded real fixture with the
            # empty M1 stub (see ``_resolve_fixture``).
            resolved = _resolve_fixture(arguments["root"])
            result = s.open(resolved)
            return _proj_load(s, arguments, result)

        if name == "close":
            s.close()
            return {"ok": True, "action_complete": True}

        if name == "set_state":
            s.state = int(arguments["state"])
            return _proj_state(s, arguments)

        if name == "step":
            _step_dir(arguments)(s)
            snap = s._snapshot()
            loaded = getattr(snap, "loaded", None)
            result = {
                "ok": True,
                "action_complete": True,
                "state": int(getattr(snap, "state", 0)),
                "num_states": int(getattr(loaded, "num_states", 0)),
                "current_time": float(getattr(snap, "current_time", 0.0)),
                "direction": str(arguments.get("dir", "")).lower(),
            }
            return result

        if name == "select":
            s.select(
                class_name=arguments["class_name"],
                range=arguments.get("range", ""),
            )
            return _proj_selection(s)

        if name == "clrsel":
            class_name = arguments.get("class_name", "") or ""
            sel = s.selection
            if class_name:
                sel.clear(class_name)
            else:
                sel.clear_all()
            return _proj_selection(s)

        if name == "show":
            result_handle = s.show(
                arguments["result"], arguments.get("component", "") or ""
            )
            return _proj_show(s, arguments, result_handle)

        if name == "material":
            target = arguments.get("material")
            enable = bool(arguments.get("enable", True))
            mats = s.materials
            (mats.enable if enable else mats.disable)(mat=target)
            return _proj_materials(s)

        if name == "view":
            op = str(arguments.get("op", "")).lower()
            view = s.view
            if op == "reset" or arguments.get("reset"):
                view.reset()
            elif op in ("rotate", "translate", "scale", "zoom", "set"):
                # The L1 typed helper resolves the op; we just lower.
                getattr(view, op)(**{k: v for k, v in arguments.items() if k != "op"})
            else:
                raise ValueError(f"unknown view op {op!r}")
            return {"ok": True, "action_complete": True}

        if name == "named_view":
            op = str(arguments.get("op", "")).upper()
            nv_name = arguments.get("name", "")
            if op == "SAVE":
                s.view.save(nv_name)
            elif op == "RESTORE":
                s.view.restore(nv_name)
            elif op == "LIST":
                pass  # read-only; result is in the snapshot
            else:
                raise ValueError(f"unknown named_view op {op!r}")
            return {"ok": True, "action_complete": True}

        if name == "colormap":
            s.colormap(arguments["name"])
            return {"ok": True, "action_complete": True}

        if name == "legend":
            legend = s.legend
            if "min" in arguments:
                legend.min = float(arguments["min"])
            if "max" in arguments:
                legend.max = float(arguments["max"])
            return {"ok": True, "action_complete": True}

        if name == "iso":
            s.isosurface(arguments["result"], **{
                k: v for k, v in arguments.items() if k != "result"
            })
            return {"ok": True, "action_complete": True}

        if name == "contour":
            s.contour(
                arguments["result"],
                count=int(arguments.get("count", 0)),
            )
            return {"ok": True, "action_complete": True}

        if name == "cutplane":
            s.cutplane(**arguments)
            return {"ok": True, "action_complete": True}

        if name == "query":
            import griz

            try:
                qr = s.query(**arguments)
            except griz.QueryError as exc:
                return {"ok": False, "error": str(exc)}
            return {"ok": True, "table": _project_query_result(qr)}

        if name == "snapshot":
            return _proj_snapshot(s)

        if name == "griz_raw":
            reply = s.command(arguments["line"])
            return _proj_griz_raw(reply)

        return {
            "ok": False,
            "error": f"unknown tool {name!r}",
            "error_kind": "unknown_tool",
        }


# ---------------------------------------------------------------------------
# Factory for the W6 CLI — one live ``griz.Session`` per scenario, opened
# on the scenario's fixture, torn down after the harness loop completes.
# ---------------------------------------------------------------------------


# ---------------------------------------------------------------------------
# Bench fixture resolver — bare names → absolute paths to the .A file.
#
# The scenarios in ``data/posttraining/eval/bootstrap.jsonl`` carry bare
# fixture names (``"d3samp6"`` / ``"cylinder"``); mili-viz-server's
# ``Database::open`` takes an absolute path to the .A file and silently
# falls back to an empty M1 stub on lookup failure
# (``crates/mili-viz-server/src/lib.rs:698``). The bench used to pass the
# bare name straight through, so the entire eval ran against the stub —
# ``num_states: 0`` in every response, ``step NEXT``/``LAST`` never
# advanced, and L3 grades on multi-state postconditions were noise.
#
# This resolver maps known bench fixtures to their checked-in serial-form
# .A files and raises if a name is unknown — loud failure beats silent
# stub fallback.
# ---------------------------------------------------------------------------

# Repo-relative paths (relative to repo root, the parent of ``crates/``).
# ``cylinder`` uses the xmilics single-file ``.plt_cA`` variant —
# ``Database::open`` is serial-only (no ``DatabaseSet`` support in
# mili-viz-server), so a parallel-only fixture is unusable here.
_FIXTURE_PATHS: dict[str, tuple[str, ...]] = {
    "d3samp6": ("reference", "mili-python", "tests", "data", "v3", "serial_t", "d3samp6.pltA"),
    "cylinder": ("reference", "mili", "test", "xmilics", "cylinder", "cylinder.plt_cA"),
}


def _repo_root() -> Path:
    # ``default_bootstrap_path()`` already walks upward to the repo root
    # via the ``crates/mili-viz-proto`` sentinel; piggyback on it instead
    # of duplicating the search. The returned bootstrap path is
    # ``<repo>/data/posttraining/eval/bootstrap.jsonl`` → four ``parents``
    # up.
    return default_bootstrap_path().parents[3]


def _resolve_fixture(name: str) -> str:
    rel = _FIXTURE_PATHS.get(name)
    if rel is None:
        raise ValueError(
            f"unknown bench fixture {name!r}; expected one of "
            f"{sorted(_FIXTURE_PATHS)}. Add the name and its repo-relative "
            f"path to _FIXTURE_PATHS in dispatchers/pygriz.py."
        )
    path = _repo_root().joinpath(*rel)
    if not path.exists():
        raise FileNotFoundError(
            f"bench fixture {name!r} resolved to {path} but the file is "
            f"absent. Check the relevant submodule is checked out "
            f"(see scripts/setup-parity.sh)."
        )
    return str(path)


def pygriz_dispatcher_factory(
    session_factory: Callable[[], Any] | None = None,
) -> Callable[[Scenario], "PygrizDispatcher"]:
    """Return a ``dispatcher_factory(scenario) -> PygrizDispatcher`` the
    W4b driver consumes.

    The returned factory opens a fresh ``griz.Session`` per scenario
    (so per-scenario fixture state never leaks across the eval run),
    opens the scenario's fixture, and hands a ``PygrizDispatcher``
    back. ``session_factory`` defaults to ``griz.launch()`` — pass an
    override for tests or to drive a remote server.

    The scenario carries a bare fixture name; the factory resolves it
    to an absolute .A path via ``_resolve_fixture`` before calling
    ``session.open`` so mili-viz-server's ``Database::open`` actually
    finds the corpus instead of silently falling back to the empty M1
    stub.

    Teardown story (load-bearing — a 50-scenario eval leaks one session
    per scenario without explicit teardown and OOMs on a long run, per
    baseline.md §"Acceptance gate" cost note):

    * Each ``PygrizDispatcher`` returned by the factory carries a
      ``close()`` method (added below) that closes the live session.
    * The driver invokes it after ``run_one_scenario`` returns; the CLI
      wires that via ``run_eval``'s post-scenario callback.
    """
    if session_factory is None:
        def _default_factory() -> Any:
            griz = _import_pygriz()
            return griz.launch()
        session_factory = _default_factory

    def factory(scenario: Scenario) -> "PygrizDispatcher":
        resolved = _resolve_fixture(scenario.fixture)
        session = session_factory()  # type: ignore[misc]
        try:
            session.open(resolved)
        except Exception:
            try:
                session.close()
            except Exception:
                pass
            raise
        return PygrizDispatcher(session=session)

    return factory


def _close_dispatcher(dispatcher: Any) -> None:
    """Best-effort teardown of a ``PygrizDispatcher`` returned by
    ``pygriz_dispatcher_factory``. Swallows exceptions so one failing
    teardown does not abort the rest of an eval run.
    """
    session = getattr(dispatcher, "session", None)
    if session is None:
        return
    try:
        session.close()
    except Exception:
        pass


__all__ = ["PygrizDispatcher", "pygriz_dispatcher_factory"]
