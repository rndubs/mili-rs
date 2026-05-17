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

import warnings
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
    "Session",
    "connect",
]


class ProtocolMismatchWarning(UserWarning):
    """Raised (as a warning, never an exception) when the server's
    protocol version is incompatible with this client's.

    Decision 36 / the scripting.md Visit guarantee: a pip-upgraded
    client warns and keeps going instead of segfaulting. The server
    still answers; the caller decides whether to trust it.
    """


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

    def __init__(self, channel, stub, hello):
        self._channel = channel
        self._stub = stub
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

    # --- lifecycle ---------------------------------------------------
    def close(self) -> None:
        if self._channel is not None:
            self._channel.close()

    def __enter__(self) -> "Session":
        return self

    def __exit__(self, *exc) -> None:
        self.close()


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
