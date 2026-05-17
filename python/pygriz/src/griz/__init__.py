"""griz — Python scripting client for the mili-viz server.

A pure-Python second client of the frozen ``mili-viz-proto`` wire
contract (the egui client is the first). It drives a ``mili-viz``
session from any CPython >= 3.11 — VS Code, a venv, a notebook —
without the Visit-style bundled-interpreter problem.

This is the Phase 6 package scaffold. The connection entry points
(``attach`` / ``launch`` / ``connect``), the Layer-1 object API, and
the generated gRPC stubs land milestone by milestone — see
``planning/mili-viz/phase-6-m1.md`` for the buildable scope and the
``planning/mili-viz/scripting.md`` API sketch for the target surface.
"""

__version__ = "0.0.0"

__all__ = ["__version__"]
