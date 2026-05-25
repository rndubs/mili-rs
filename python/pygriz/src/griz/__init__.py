"""griz — Python scripting client for the mili-viz server.

A pure-Python second client of the frozen ``mili-viz-proto`` wire
contract (the egui client is the first). It drives a ``mili-viz``
session from any CPython >= 3.11 — VS Code, a venv, a notebook —
without the Visit-style bundled-interpreter problem.

Phase 6 M1 surface (planning/mili-viz/phase-6-m1.md Decisions 35–37,
53–55): ``griz.connect(host, port, token=...)`` + the ``Hello``
version/capability handshake (a mismatch *warns*, never raises —
the Visit "API matches the engine" guarantee bought with a wire
contract), and the Layer-0 escape hatch ``session.command(...)`` /
``session.run_script(path)`` → ``Command.raw``. The connection-model
``attach()``/``launch()``, the Layer-1 object API, live ``Subscribe``
sync, and the query payoff land in Phase 6 M2–M6; see
``planning/mili-viz/scripting.md`` for the full target surface.
"""

from __future__ import annotations

import json
import os
import pathlib
import re
import subprocess
import time
import warnings
from dataclasses import dataclass
from typing import TYPE_CHECKING

__version__ = "0.0.0"

# Mirror of crates/mili-viz-proto/src/lib.rs `PROTOCOL_VERSION`. The
# server compares MAJOR only (see mili-viz-server `Hello`); a bump of
# the major here is what the M1 gate's mismatch leg exercises.
PROTOCOL_VERSION = "1.0.0"

__all__ = [
    "__version__",
    "PROTOCOL_VERSION",
    "ProtocolMismatchWarning",
    "GuiUnavailableWarning",
    "QueryError",
    "Session",
    "SessionInfo",
    "Database",
    "Result",
    "Isosurface",
    "Contour",
    "QueryResult",
    "connect",
    "attach",
    "launch",
    "list_sessions",
]


class ProtocolMismatchWarning(UserWarning):
    """Raised (as a warning, never an exception) when the server's
    protocol version is incompatible with this client's.

    Decision 36 / the scripting.md Visit guarantee: a pip-upgraded
    client warns and keeps going instead of segfaulting. The server
    still answers; the caller decides whether to trust it.
    """


class GuiUnavailableWarning(UserWarning):
    """Emitted by ``launch(gui=True)``: the GUI is the Phase 5
    ``wgpu``/``egui`` renderer, an **independent track** Phase 6 does
    not spawn (phase-6-m2.md Decision 58). The signature is preserved;
    ``launch`` proceeds headless rather than silently ignoring the flag.
    """


class QueryError(RuntimeError):
    """Server-rejected ``Query`` RPC. Carries the verbatim
    ``QueryReply.error`` so callers can branch on the typed message
    (unknown svar, no run loaded, derived-result deferred, etc.) — the
    server returns ``ok=false`` with a typed string rather than a
    transport-level ``Status``, matching the M1 wire-surface shape
    (phase-6-m5.md Decision 67)."""


def _stubs():
    """Import the generated stubs, with an actionable error if the
    build-output ``griz._proto`` package has not been generated yet
    (Decision 36: stubs are gitignored build output)."""
    try:
        from griz._proto import mili_viz_pb2 as pb
        from griz._proto import mili_viz_pb2_grpc as pb_grpc
    except ImportError as exc:  # pragma: no cover - exercised via skip
        raise ImportError(
            "griz._proto stubs are not generated. They are build "
            "output from the canonical crates/mili-viz-proto/proto/"
            "mili_viz.proto — run scripts/gen-pygriz-stubs.sh after "
            "`pip install -e python/pygriz[dev]`."
        ) from exc
    return pb, pb_grpc


if TYPE_CHECKING:  # pragma: no cover
    from griz._proto import mili_viz_pb2 as _pb


class Session:
    """A live connection to a ``mili-viz`` server.

    M1 carries only the Layer-0 escape hatch. ``command`` and
    ``run_script`` both lower to a single ``Command{raw}`` and let the
    server's existing dispatcher (``mili-viz-server`` ``parse_raw``)
    do all griz-line parsing — there is deliberately **no** Python-side
    griz parser, so the single-parser invariant holds and Layer-0 ≡ the
    raw stream by construction (Decisions 37 & 54).
    """

    def __init__(self, channel, stub, hello, proc=None):
        self._channel = channel
        self._stub = stub
        #: The child ``mili-viz-server`` process when this session was
        #: created by :func:`launch` (else ``None``). The launcher owns
        #: the lifecycle — the server does not self-clean its session
        #: file (phase-6-m2.md Decisions 56 & 58).
        self._proc = proc
        #: The ``HelloReply`` returned by the handshake.
        self.hello = hello

    # --- handshake-derived, read-only conveniences -------------------
    @property
    def compatible(self) -> bool:
        return bool(self.hello.compatible)

    @property
    def capabilities(self) -> list[str]:
        return list(self.hello.capabilities)

    @property
    def info(self):
        """The echoed ``SessionInfo`` (id/pid/host/port/db)."""
        return self.hello.session

    # --- Layer-0 escape hatch (Decision 37) --------------------------
    def command(self, raw: str):
        """Send a raw griz / ``;``-joined Layer-0 stream verbatim as
        ``Command{raw}`` and return the server's ``CommandReply``
        (``.ok`` / ``.error`` / ``.delta_seq``)."""
        pb, _ = _stubs()
        return self._stub.Execute(pb.Command(raw=raw))

    def run_script(self, path) -> "_pb.CommandReply":
        """Send a ``grizinit``-style batch file verbatim as a single
        ``Command{raw}``.

        The file is **not** parsed or line-split here: the server's
        ``parse_raw`` already splits on ``;``/newline and skips blank
        lines and ``#``/``//`` comments, so streaming the whole file as
        one ``raw`` keeps griz-line parsing in exactly one place
        (Decisions 37 & 54)."""
        with open(path, "r", encoding="utf-8") as fh:
            return self.command(fh.read())

    # --- Layer-1 object API (phase-6-m3.md Decisions 59–61) ----------
    #
    # Every Layer-1 call lowers to a *typed* ``Command`` oneof variant
    # (Decision 59) — never the ``raw`` arm, which stays exclusively the
    # Layer-0 escape hatch above. No griz string is ever formatted in
    # Python: the single-parser invariant (the server's ``parse_raw`` is
    # the one parser; M1 Decision 37) generalizes to "no second emitter
    # either". Camera/view is server-authoritative (scripting.md
    # Decision 1): these calls *emit*; the authoritative state is read
    # back from a one-shot ``Subscribe`` snapshot (Decision 61), never
    # client-predicted (prediction/reconciliation is Phase 5 M4).

    def _exec(self, command):
        return self._stub.Execute(command)

    def _snapshot(self) -> "_pb.Snapshot":
        """The authoritative session state, read once from the opening
        ``DELTA_SNAPSHOT`` of a short-lived ``Subscribe`` (Decision 61).
        No local model, no prediction — just the server's current truth
        on demand (the continuous ``@s.on(...)`` stream is Phase 6 M4)."""
        pb, _ = _stubs()
        stream = self._stub.Subscribe(pb.SubscribeRequest())
        try:
            delta = next(iter(stream))
        finally:
            stream.cancel()
        return delta.snapshot

    def open(self, root) -> "Database":
        """``load <root>`` → :class:`Database` handle. The ``db.query``
        payoff is Phase 6 M5; M3's handle carries only ``.root``."""
        pb, _ = _stubs()
        self._exec(pb.Command(load=pb.Load(root=str(root))))
        return Database(self, str(root))

    @property
    def state(self) -> int:
        """The authoritative 1-based state index (one-shot snapshot
        read; Decision 61). Set it to navigate: ``s.state = 10``."""
        return int(self._snapshot().state)

    @state.setter
    def state(self, n: int) -> None:
        pb, _ = _stubs()
        self._exec(pb.Command(set_state=pb.SetState(state=int(n))))

    def _step(self, which: str) -> None:
        pb, _ = _stubs()
        dir_ = {
            "next": pb.Step.NEXT,
            "prev": pb.Step.PREV,
            "first": pb.Step.FIRST,
            "last": pb.Step.LAST,
        }[which]
        self._exec(pb.Command(step=pb.Step(dir=dir_)))

    def next(self) -> None:
        self._step("next")

    def prev(self) -> None:
        self._step("prev")

    def first(self) -> None:
        self._step("first")

    def last(self) -> None:
        self._step("last")

    def select(self, class_name: str, range: str) -> None:
        """``select <class> <range>`` (griz-style range string, e.g.
        ``"1-100,150"``). Clear via ``s.selection.clear(class_name)``."""
        pb, _ = _stubs()
        self._exec(
            pb.Command(select=pb.Select(class_name=class_name, range=range))
        )

    @property
    def selection(self) -> "_Selection":
        return _Selection(self)

    def show(self, result: str, component: str = "", **opts) -> "Result":
        """``show <result> [component] [k=v...]`` → :class:`Result`
        handle (``.range`` is the authoritative ``(min, max)``)."""
        pb, _ = _stubs()
        msg = pb.Show(result=result, component=component)
        for k, v in opts.items():
            msg.opts[str(k)] = str(v)
        self._exec(pb.Command(show=msg))
        return Result(self, result, component)

    def isosurface(
        self,
        result: str,
        *,
        levels=None,
        count: int | None = None,
        vmin: float | None = None,
        vmax: float | None = None,
        on: bool = True,
    ) -> "Isosurface":
        """``iso on/off <result>`` → :class:`Isosurface` handle
        (``.remove()`` emits ``iso off``)."""
        pb, _ = _stubs()
        msg = pb.Isosurface(result=result, on=on)
        lv = [float(x) for x in levels] if levels else []
        if lv:
            msg.levels.extend(lv)
        if count is not None:
            msg.count = int(count)
        if vmin is not None:
            msg.min = float(vmin)
        if vmax is not None:
            msg.max = float(vmax)
        self._exec(pb.Command(iso=msg))
        return Isosurface(self, result, lv)

    def contour(self, result: str, *, count: int = 0) -> "Contour":
        """``contour <result> <count>`` → :class:`Contour` handle."""
        pb, _ = _stubs()
        self._exec(
            pb.Command(contour=pb.Contour(result=result, count=int(count)))
        )
        return Contour(self, result, int(count))

    @property
    def materials(self) -> "_Materials":
        return _Materials(self)

    def cutplane(
        self,
        *,
        origin=(0.0, 0.0, 0.0),
        normal=(1.0, 0.0, 0.0),
        relative: bool = False,
    ) -> None:
        """``cutpln``/``cutrpln`` (``relative=True`` → ``cutrpln``)."""
        pb, _ = _stubs()
        ox, oy, oz = (float(c) for c in origin)
        nx, ny, nz = (float(c) for c in normal)
        self._exec(
            pb.Command(
                cutplane=pb.CutPlane(
                    ox=ox,
                    oy=oy,
                    oz=oz,
                    nx=nx,
                    ny=ny,
                    nz=nz,
                    relative=bool(relative),
                )
            )
        )

    def colormap(self, name: str) -> None:
        """``cmap <name>`` (e.g. ``"cool"``, ``"jet"``)."""
        pb, _ = _stubs()
        self._exec(pb.Command(colormap=pb.Colormap(name=name)))

    @property
    def legend(self) -> "_Legend":
        return _Legend(self)

    @property
    def view(self) -> "_View":
        """The server-authoritative camera (scripting.md Decision 1).
        ``s.view.*`` *emits* the command; the GUI mirrors the broadcast
        — M3 does not predict (that is Phase 5 M4)."""
        return _View(self)

    # --- Layer-1 query (phase-6-m5.md Decisions 67–69) ---------------
    #
    # The Phase 6 "real payoff" (scripting.md "data back into Python").
    # Mirrors the Rust `Session::query` wrapper (server arm landed —
    # wireframe-parity.md #4) one-for-one: build the typed
    # `QueryRequest`, dispatch over the in-band `Query` RPC, surface a
    # server-rejected reply as `QueryError`, hand the inline carrier to
    # `QueryResult` for numpy/pandas conversion. No griz string is
    # formatted (M3's "no second emitter" invariant); empty `labels`/
    # `states` are passed verbatim so the server fills them in (all
    # labels / current state cursor — see `proto/mili_viz.proto:379`).
    # Only the inline carrier is implemented today; the
    # `flight_ticket` arm of the proto's `oneof` raises until the
    # Arrow-Flight path lands (Decision 68).
    def query(
        self,
        result: str,
        class_name: str,
        *,
        labels=None,
        states=None,
        component: str = "",
    ) -> "QueryResult":
        """Run one ``Query`` RPC and return a :class:`QueryResult` (raw
        labels/states/values + ``to_dataframe()``). Empty ``labels``
        ⇒ all labels for the class; empty ``states`` ⇒ current state."""
        pb, _ = _stubs()
        req = pb.QueryRequest(
            result=str(result),
            class_name=str(class_name),
            component=str(component),
        )
        if labels:
            req.labels.extend(int(l) for l in labels)
        if states:
            req.states.extend(int(s) for s in states)
        reply = self._stub.Query(req)
        if not reply.ok:
            raise QueryError(reply.error)
        which = reply.WhichOneof("data")
        if which == "inline":
            t = reply.inline
            return QueryResult(
                result=str(result),
                class_name=str(class_name),
                component=str(component),
                labels=list(t.labels),
                states=list(t.states),
                values=list(t.values),
                components=int(t.components),
            )
        if which == "flight_ticket":
            raise QueryError(
                "query: server returned a Flight ticket (large-result "
                "path), not implemented in pygriz M5 yet — pin the "
                "request smaller until the Arrow-Flight client lands"
            )
        raise QueryError("query: server reply carries no data")

    # --- lifecycle ---------------------------------------------------
    def close(self) -> None:
        if self._channel is not None:
            self._channel.close()
            self._channel = None
        proc = self._proc
        if proc is not None and proc.poll() is None:
            proc.terminate()
            try:
                proc.wait(timeout=10)
            except subprocess.TimeoutExpired:
                proc.kill()
        self._proc = None

    def __enter__(self) -> "Session":
        return self

    def __exit__(self, *exc) -> None:
        self.close()


# ---------------------------------------------------------------------
# Layer-1 typed handles + server-authoritative view/selection helpers
# (phase-6-m3.md Decisions 59–61). Handles are thin: they re-emit typed
# Commands through their owning Session and read authoritative state
# from the one-shot Subscribe snapshot — no client-side scene model.
# query()/to_dataframe() is Phase 6 M5; render() is M6 (out of M3).
# ---------------------------------------------------------------------


class Database:
    """The loaded run (``s.open(root)``). M5: :meth:`query` delegates
    to the owning :meth:`Session.query` so the ``scripting.md`` sketch
    (``db.query("sx", "brick", labels=[1,2,3], states=[10,20])``) and
    the Rust-side ``Session::query`` parity shape are both spelled the
    same way."""

    def __init__(self, session: "Session", root: str):
        self._session = session
        self.root = root

    def query(
        self,
        result: str,
        class_name: str,
        *,
        labels=None,
        states=None,
        component: str = "",
    ) -> "QueryResult":
        """Alias for :meth:`Session.query` — scoped to this loaded run
        for readability in scripts (``db.query`` reads more obviously
        than ``s.query`` once ``s.open(...)`` returned the handle)."""
        return self._session.query(
            result,
            class_name,
            labels=labels,
            states=states,
            component=component,
        )

    def __repr__(self) -> str:
        return f"Database({self.root!r})"


@dataclass(frozen=True)
class QueryResult:
    """One ``Query`` RPC reply, with the milox/``mili.utils``
    DataFrame shape one method-call away (``to_dataframe()``).

    The proto's ``InlineTable`` is row-major
    ``[len(states) × len(labels) × components]`` — preserved verbatim
    so callers can reshape on numpy if the pandas default doesn't fit
    (multi-component results, for example, are exposed as one-cell
    arrays in the DataFrame and a 3-D ndarray on ``.values_3d``).
    """

    result: str
    class_name: str
    component: str
    labels: list  # int (server returns int64)
    states: list  # int (1-based, server-resolved)
    values: list  # float (row-major [states × labels × components])
    components: int

    def __len__(self) -> int:
        return len(self.values)

    @property
    def values_3d(self):
        """The values reshaped to ``(len(states), len(labels), components)``
        as a ``numpy.ndarray`` of ``float64`` — the milox `Query`
        layout (``query_data_to_dataframe`` expects this exact shape).
        Empty when any axis is zero."""
        import numpy as np

        arr = np.asarray(self.values, dtype=np.float64)
        ns = len(self.states)
        nl = len(self.labels)
        nc = max(int(self.components), 1)
        if ns == 0 or nl == 0:
            return arr.reshape((ns, nl, nc))
        return arr.reshape((ns, nl, nc))

    def to_dataframe(self):
        """``pandas.DataFrame`` shaped like
        ``mili.utils.query_data_to_dataframe`` — index = states,
        columns = labels. Scalar (``components==1``) results lay flat
        ``(states × labels)``; multi-component results put a 1-D
        ``ndarray`` of length ``components`` in each cell (matches
        mili-python's ``DataFrame.from_records`` arm — same types so
        viz and analysis mix freely, ``scripting.md`` "the real
        payoff")."""
        import numpy as np
        import pandas as pd

        a = self.values_3d
        if a.shape[2] == 1:
            return pd.DataFrame(
                a.reshape(a.shape[:-1]),
                index=np.asarray(self.states),
                columns=np.asarray(self.labels),
            )
        # Multi-component: one ndarray per cell. mili-python's
        # `from_records` arm builds a DataFrame whose cells each hold
        # the per-component vector; preserve that exact shape.
        records = [
            [a[i, j, :].copy() for j in range(a.shape[1])]
            for i in range(a.shape[0])
        ]
        df = pd.DataFrame(
            records,
            index=np.asarray(self.states),
            columns=np.asarray(self.labels),
        )
        return df


class Result:
    """An active result (``s.show(...)``). ``.range`` is the
    authoritative ``(min, max)`` from the server's ``ResultState``,
    read on demand (Decision 61) — not a cached client guess."""

    def __init__(self, session: "Session", result: str, component: str = ""):
        self._session = session
        self.result = result
        self.component = component

    @property
    def range(self) -> tuple[float, float]:
        r = self._session._snapshot().result
        return (r.min, r.max)

    def __repr__(self) -> str:
        c = f", component={self.component!r}" if self.component else ""
        return f"Result({self.result!r}{c})"


class Isosurface:
    """An isosurface (``s.isosurface(...)``). ``.remove()`` emits the
    typed ``iso off <result>`` (Decision 59 — never a raw string)."""

    def __init__(self, session: "Session", result: str, levels: list[float]):
        self._session = session
        self.result = result
        self.levels = levels

    def remove(self) -> None:
        pb, _ = _stubs()
        self._session._exec(
            pb.Command(iso=pb.Isosurface(result=self.result, on=False))
        )

    def __repr__(self) -> str:
        return f"Isosurface({self.result!r}, levels={self.levels})"


class Contour:
    """A contour (``s.contour(...)``)."""

    def __init__(self, session: "Session", result: str, count: int):
        self._session = session
        self.result = result
        self.count = count

    def __repr__(self) -> str:
        return f"Contour({self.result!r}, count={self.count})"


class _Selection:
    """``s.selection.clear(class_name)`` → typed ``clrsel <class>``."""

    def __init__(self, session: "Session"):
        self._session = session

    def clear(self, class_name: str) -> None:
        pb, _ = _stubs()
        self._session._exec(
            pb.Command(clrsel=pb.ClearSelection(class_name=class_name))
        )


class _Materials:
    """``s.materials.disable(3)`` / ``.enable("brick", mat=2)`` → typed
    ``MaterialVisibility``. A positional ``int`` is a material id, a
    positional ``str`` is a class name (the scripting.md sketch)."""

    def __init__(self, session: "Session"):
        self._session = session

    def _emit(self, enable: bool, target, mat) -> None:
        pb, _ = _stubs()
        class_name = ""
        material = mat
        if isinstance(target, str):
            class_name = target
        elif target is not None:
            material = target
        msg = pb.MaterialVisibility(enable=enable, class_name=class_name)
        if material is not None:
            msg.material = int(material)
        self._session._exec(pb.Command(material=msg))

    def enable(self, target=None, *, mat: int | None = None) -> None:
        self._emit(True, target, mat)

    def disable(self, target=None, *, mat: int | None = None) -> None:
        self._emit(False, target, mat)


class _Legend:
    """``s.legend.limits = (lo, hi)`` → typed ``LegendLimits``. A
    ``None`` bound autoscales that end (the proto's unset semantics)."""

    def __init__(self, session: "Session"):
        self._session = session

    @property
    def limits(self) -> tuple[float, float]:
        r = self._session._snapshot().result
        return (r.min, r.max)

    @limits.setter
    def limits(self, lohi) -> None:
        pb, _ = _stubs()
        lo, hi = lohi
        msg = pb.LegendLimits()
        if lo is not None:
            msg.min = float(lo)
        if hi is not None:
            msg.max = float(hi)
        self._session._exec(pb.Command(legend=msg))


class _View:
    """The server-authoritative camera (scripting.md Decision 1). Every
    method *emits* a typed ``View``/``NamedView`` command; the server
    broadcasts the authoritative ``DELTA_CAMERA`` to all peers. M3 does
    not predict locally — that is Phase 5 M4."""

    def __init__(self, session: "Session"):
        self._session = session

    def _emit(self, view) -> None:
        pb, _ = _stubs()
        self._session._exec(pb.Command(view=view))

    def rotate(self, x: float = 0.0, y: float = 0.0, z: float = 0.0) -> None:
        pb, _ = _stubs()
        self._emit(
            pb.View(rotate=pb.Rotate(x=float(x), y=float(y), z=float(z)))
        )

    def translate(self, dx: float, dy: float, dz: float) -> None:
        pb, _ = _stubs()
        self._emit(
            pb.View(
                translate=pb.Translate(
                    dx=float(dx), dy=float(dy), dz=float(dz)
                )
            )
        )

    def scale(self, factor: float) -> None:
        pb, _ = _stubs()
        self._emit(pb.View(scale=pb.Scale(factor=float(factor))))

    def zoom(self, factor: float) -> None:
        pb, _ = _stubs()
        self._emit(pb.View(zoom=pb.Zoom(factor=float(factor))))

    def set(
        self,
        *,
        azimuth: float,
        elevation: float,
        distance: float,
        fx: float | None = None,
        fy: float | None = None,
        fz: float | None = None,
    ) -> None:
        pb, _ = _stubs()
        cam = pb.SetCamera(
            azimuth=float(azimuth),
            elevation=float(elevation),
            distance=float(distance),
        )
        if fx is not None:
            cam.fx = float(fx)
        if fy is not None:
            cam.fy = float(fy)
        if fz is not None:
            cam.fz = float(fz)
        self._emit(pb.View(set=cam))

    def reset(self) -> None:
        pb, _ = _stubs()
        self._emit(pb.View(reset=True))

    def _named(self, op, name: str = "") -> None:
        pb, _ = _stubs()
        self._session._exec(
            pb.Command(named_view=pb.NamedView(op=op, name=name))
        )

    def save(self, name: str) -> None:
        pb, _ = _stubs()
        self._named(pb.NamedView.SAVE, name)

    def restore(self, name: str) -> None:
        pb, _ = _stubs()
        self._named(pb.NamedView.RESTORE, name)

    def list(self) -> None:
        pb, _ = _stubs()
        self._named(pb.NamedView.LIST)


def connect(
    host: str,
    port: int,
    token: str = "",
    *,
    client_id: str | None = None,
    protocol_version: str = PROTOCOL_VERSION,
    timeout: float = 10.0,
) -> Session:
    """Connect to a ``mili-viz`` server over gRPC and complete the
    ``Hello`` handshake.

    A protocol/capability mismatch is reported by the server, not
    crashed on: this raises a :class:`ProtocolMismatchWarning` (a
    warning, never an exception) and still returns a usable
    :class:`Session` — the scripting.md Visit guarantee (Decision 36).

    ``attach()`` / ``launch()`` / ``list_sessions()`` are Phase 6 M2;
    this M1 path is the explicit remote-style connect.
    """
    import grpc

    pb, pb_grpc = _stubs()
    channel = grpc.insecure_channel(f"{host}:{port}")
    stub = pb_grpc.MiliVizStub(channel)
    request = pb.HelloRequest(
        protocol_version=protocol_version,
        session_token=token,
        client_id=client_id or f"griz-py/{__version__}",
    )
    reply = stub.Hello(request, timeout=timeout)
    if not reply.compatible:
        warnings.warn(
            "mili-viz protocol mismatch (client "
            f"{protocol_version} vs server "
            f"{reply.server_protocol_version}): "
            f"{reply.mismatch_detail or 'no detail'}. "
            "Proceeding; behavior may be undefined.",
            ProtocolMismatchWarning,
            stacklevel=2,
        )
    return Session(channel, stub, reply)


# ---------------------------------------------------------------------
# Phase 6 M2 — connection model (phase-6-m2.md Decisions 56–58)
# ---------------------------------------------------------------------


@dataclass(frozen=True)
class SessionInfo:
    """One parsed ``~/.griz/sessions/<id>.json`` session/connection
    file (the Jupyter-connection-file pattern; scripting.md). Written
    by the ``mili-viz-server`` binary on startup (phase-6-m2.md
    Decision 56)."""

    id: str
    pid: int
    host: str
    port: int
    token: str
    protocol_version: str
    db: str
    #: Absolute path of the JSON file this was parsed from.
    path: pathlib.Path
    #: The file's mtime — "newest" for ``attach()`` is the max of this.
    mtime: float


def _sessions_dir() -> pathlib.Path:
    """``$GRIZ_SESSIONS_DIR`` (hermetic tests / redirection) else
    ``~/.griz/sessions`` — must match the server writer (Decision 56)."""
    env = os.environ.get("GRIZ_SESSIONS_DIR")
    if env:
        return pathlib.Path(env)
    return pathlib.Path.home() / ".griz" / "sessions"


def _parse_session_file(path: pathlib.Path) -> SessionInfo | None:
    """Parse one session file, returning ``None`` (never raising) for a
    missing/partial/malformed file so a stale or half-written sibling
    can never break ``list_sessions()``/``attach()`` (Decision 56:
    staleness is handled read-side)."""
    try:
        raw = json.loads(path.read_text(encoding="utf-8"))
        return SessionInfo(
            id=str(raw["id"]),
            pid=int(raw["pid"]),
            host=str(raw["host"]),
            port=int(raw["port"]),
            token=str(raw.get("token", "")),
            protocol_version=str(raw.get("protocol_version", "")),
            db=str(raw.get("db", "")),
            path=path,
            mtime=path.stat().st_mtime,
        )
    except (OSError, ValueError, KeyError, TypeError):
        return None


def _pid_alive(pid: int) -> bool:
    """Best-effort liveness for the newest-*live* pick (Decision 57).
    Unknown → treat as alive (never hide a session we can't disprove)."""
    if pid <= 0:
        return False
    try:
        os.kill(pid, 0)
    except ProcessLookupError:
        return False
    except (PermissionError, OSError):
        return True
    return True


def list_sessions() -> list[SessionInfo]:
    """All parseable session/connection files under the sessions dir,
    **newest file first** (mtime; the Jupyter pattern / scripting.md
    "the newest local session"). Malformed/partial files are skipped,
    never raised on (Decision 56)."""
    d = _sessions_dir()
    if not d.is_dir():
        return []
    out = [
        s
        for p in d.glob("*.json")
        if (s := _parse_session_file(p)) is not None
    ]
    out.sort(key=lambda s: s.mtime, reverse=True)
    return out


def attach(
    id: str | None = None,
    *,
    host: str | None = None,
    port: int | None = None,
    token: str | None = None,
    **connect_kwargs,
) -> Session:
    """Attach to a running ``mili-viz`` session (the priority
    interactive path; scripting.md "open the GUI, attach a VS Code
    script to it"). Precedence (phase-6-m2.md Decision 57):

    1. explicit ``host`` **and** ``port`` → connect there directly
       (the ``attach``-spelled alias of :func:`connect`);
    2. ``id`` → ``<sessions dir>/<id>.json``;
    3. otherwise → the **newest live** local session file.

    Every branch lowers to the one M1 :func:`connect` transport — this
    is a session-file resolver, not a parallel client."""
    if host is not None and port is not None:
        return connect(host, port, token or "", **connect_kwargs)

    d = _sessions_dir()
    if id is not None:
        path = d / f"{id}.json"
        info = _parse_session_file(path) if path.is_file() else None
        if info is None:
            raise FileNotFoundError(
                f"no readable griz session {id!r} in {d} "
                "(is the server running and is GRIZ_SESSIONS_DIR set "
                "the same as the server's?)"
            )
    else:
        live = [s for s in list_sessions() if _pid_alive(s.pid)]
        if not live:
            raise RuntimeError(
                f"no live griz sessions in {d}. Start one with "
                "`griz.launch()`, run `mili-viz-server`, or pass an "
                "explicit host/port (attach(host=..., port=...))."
            )
        info = live[0]

    return connect(
        info.host,
        info.port,
        token if token is not None else info.token,
        **connect_kwargs,
    )


def _discover_server_bin() -> pathlib.Path | None:
    """``$GRIZ_SERVER_BIN`` → ``target/{release,debug}/mili-viz-server``
    → ``mili-viz-server`` on ``PATH`` (phase-6-m2.md Decision 58 —
    same discovery shape as the M1 gate's)."""
    env = os.environ.get("GRIZ_SERVER_BIN")
    if env:
        p = pathlib.Path(env)
        return p if p.exists() else None
    # python/pygriz/src/griz/__init__.py -> repo root is parents[4].
    repo_root = pathlib.Path(__file__).resolve().parents[4]
    for profile in ("release", "debug"):
        p = repo_root / "target" / profile / "mili-viz-server"
        if p.exists():
            return p
    import shutil

    found = shutil.which("mili-viz-server")
    return pathlib.Path(found) if found else None


def launch(
    gui: bool = False,
    *,
    server_bin: str | pathlib.Path | None = None,
    timeout: float = 30.0,
    **connect_kwargs,
) -> Session:
    """Spawn ``mili-viz-server`` on a free port and attach to it via
    the session file it writes (phase-6-m2.md Decision 58) — the
    ``visit -cli`` equivalent. The returned :class:`Session` owns the
    child; ``close()`` / ``with`` terminates it.

    ``gui=True`` is accepted but the GUI is the Phase 5 renderer, an
    independent track Phase 6 does not spawn: a
    :class:`GuiUnavailableWarning` is emitted and ``launch`` proceeds
    headless (Decision 58)."""
    if gui:
        warnings.warn(
            "launch(gui=True): the mili-viz GUI is the Phase 5 "
            "renderer (an independent track); Phase 6 M2 does not "
            "spawn it. Proceeding headless.",
            GuiUnavailableWarning,
            stacklevel=2,
        )

    binary = pathlib.Path(server_bin) if server_bin else _discover_server_bin()
    if binary is None or not pathlib.Path(binary).exists():
        raise FileNotFoundError(
            "mili-viz-server binary not found. Build it "
            "(`cargo build -p mili-viz-server`), set $GRIZ_SERVER_BIN, "
            "or pass server_bin=..."
        )

    proc = subprocess.Popen(
        [str(binary), "127.0.0.1:0"],
        stdout=subprocess.PIPE,
        stderr=subprocess.STDOUT,
        text=True,
    )
    port: int | None = None
    deadline = time.time() + timeout
    try:
        while time.time() < deadline:
            line = proc.stdout.readline() if proc.stdout else ""
            if not line:
                if proc.poll() is not None:
                    break
                continue
            m = re.search(r"tcp://127\.0\.0\.1:(\d+)", line)
            if m:
                port = int(m.group(1))
                break
        if port is None:
            raise RuntimeError(
                "mili-viz-server did not report a bound port within "
                f"{timeout}s"
            )

        # Attach via the session file this child just wrote (matched by
        # its pid), so launch exercises the Decision-56 file path and
        # inherits its token. Poll: the bind print and the file write
        # are ordered in main but the fs is async.
        info: SessionInfo | None = None
        while time.time() < deadline:
            for s in list_sessions():
                if s.pid == proc.pid and s.port == port:
                    info = s
                    break
            if info is not None:
                break
            time.sleep(0.05)
        if info is None:
            raise RuntimeError(
                "mili-viz-server bound but wrote no matching session "
                f"file in {_sessions_dir()}"
            )

        session = connect(
            info.host, info.port, info.token, **connect_kwargs
        )
        session._proc = proc
        return session
    except BaseException:
        proc.terminate()
        try:
            proc.wait(timeout=10)
        except subprocess.TimeoutExpired:
            proc.kill()
        raise
