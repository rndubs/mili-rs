"""Phase 6 M1 acceptance gate (planning/mili-viz/phase-6-m1.md
§ "M1 acceptance gate", Decisions 35–37 & 53–55).

Two halves, mirroring the CLAUDE.md skip-on-absent convention:

* **always-on pure logic** — stub generation + import, and the
  Decision-54 invariant that ``run_script`` lowers a grizinit batch to
  a *single* verbatim ``Command{raw}`` with no Python-side griz parser
  (verified against a fake stub, no server);
* **skip-on-absent** — the connect/handshake/Layer-0 leg, which spawns
  the real ``mili-viz-server`` TCP binary; skipped (never failed) when
  ``cargo``/the binary is unavailable, exactly like the Rust suites'
  corpus skip.
"""

from __future__ import annotations

import pathlib
import re
import shutil
import subprocess
import time

import pytest

REPO_ROOT = pathlib.Path(__file__).resolve().parents[3]
PROTO = REPO_ROOT / "crates" / "mili-viz-proto" / "proto" / "mili_viz.proto"
FIXTURE = (
    REPO_ROOT
    / "reference"
    / "mili-python"
    / "tests"
    / "data"
    / "serial"
    / "basic1"
    / "basic1.pltA"
)


@pytest.fixture(scope="session", autouse=True)
def _stubs_generated():
    """Generate the gitignored ``griz._proto`` build output from the
    one canonical proto before any test imports ``griz``. Skip (not
    fail) the whole module if grpcio-tools is absent — that mirrors the
    parity suites' skip-on-absent (CLAUDE.md)."""
    try:
        import grpc_tools  # noqa: F401
    except ImportError:
        pytest.skip("grpcio-tools absent (pip install -e python/pygriz[dev])")
    gen = REPO_ROOT / "scripts" / "gen-pygriz-stubs.sh"
    subprocess.run([str(gen)], check=True, cwd=REPO_ROOT)


# --------------------------------------------------------------------
# Always-on pure logic
# --------------------------------------------------------------------


def test_import_and_proto_pinned():
    """`import griz` works on CPython >= 3.11; the client's pinned
    protocol version mirrors the canonical proto crate."""
    import griz

    assert griz.__version__
    # Decision 53/36: the stubs come from the ONE canonical proto.
    rust_const = (REPO_ROOT / "crates" / "mili-viz-proto" / "src" / "lib.rs").read_text()
    m = re.search(r'PROTOCOL_VERSION:\s*&str\s*=\s*"([^"]+)"', rust_const)
    assert m, "could not find PROTOCOL_VERSION in mili-viz-proto"
    assert griz.PROTOCOL_VERSION == m.group(1)


def test_run_script_is_one_verbatim_raw(tmp_path):
    """Decision 54: ``run_script`` sends the *entire* grizinit file as
    one ``Command{raw}`` — no Python-side splitting/parsing. Verified
    against a fake stub so it needs no server (always-on)."""
    import griz

    script = "# a grizinit batch\n\nload run\n// comment\nstate 2; show sx\n"
    path = tmp_path / "legacy_grizinit"
    path.write_text(script)

    sent: list = []

    class FakeStub:
        def Execute(self, command):  # noqa: N802 - gRPC stub casing
            sent.append(command)
            return object()

    s = griz.Session(channel=None, stub=FakeStub(), hello=None)
    s.run_script(path)

    assert len(sent) == 1, "run_script must emit exactly one Command"
    assert sent[0].raw == script, "file must be sent byte-verbatim as raw"
    assert sent[0].WhichOneof("cmd") == "raw", "must use the Layer-0 raw hatch"


# --------------------------------------------------------------------
# Skip-on-absent: connect / handshake / Layer-0 against a real server
# --------------------------------------------------------------------


def _server_binary() -> pathlib.Path | None:
    for profile in ("release", "debug"):
        p = REPO_ROOT / "target" / profile / "mili-viz-server"
        if p.exists():
            return p
    cargo = shutil.which("cargo")
    if cargo is None:
        return None
    build = subprocess.run(
        [cargo, "build", "-p", "mili-viz-server"], cwd=REPO_ROOT
    )
    if build.returncode != 0:
        return None
    p = REPO_ROOT / "target" / "debug" / "mili-viz-server"
    return p if p.exists() else None


@pytest.fixture
def server():
    binary = _server_binary()
    if binary is None:
        pytest.skip("mili-viz-server binary/cargo unavailable")
    proc = subprocess.Popen(
        [str(binary), "127.0.0.1:0"],
        stdout=subprocess.PIPE,
        stderr=subprocess.STDOUT,
        text=True,
    )
    port = None
    deadline = time.time() + 30.0
    while time.time() < deadline:
        line = proc.stdout.readline()
        if not line:
            break
        m = re.search(r"tcp://127\.0\.0\.1:(\d+)", line)
        if m:
            port = int(m.group(1))
            break
    if port is None:
        proc.terminate()
        pytest.skip("mili-viz-server did not report a bound port")
    try:
        yield port
    finally:
        proc.terminate()
        try:
            proc.wait(timeout=10)
        except subprocess.TimeoutExpired:
            proc.kill()


def test_connect_handshake_and_layer0(server):
    import griz

    # Matching protocol_version -> compatible, no warning.
    with _no_warning(griz):
        s = griz.connect("127.0.0.1", server)
    assert s.compatible is True
    assert s.hello.server_protocol_version == griz.PROTOCOL_VERSION

    # Layer-0: load (server `load` never errors — graceful M1/M2, so
    # this is corpus-independent; the real fixture is used when present
    # so the assertion exercises the realistic path too) + state + show.
    root = str(FIXTURE) if FIXTURE.exists() else "nonexistent_run"
    reply = s.command(f"load {root}; state 2; show sx")
    assert reply.ok, f"Layer-0 command failed: {reply.error}"

    # A bad verb is a parse error surfaced as ok == False, not a crash.
    bad = s.command("definitely_not_a_griz_verb")
    assert bad.ok is False
    assert bad.error, "parse error must populate `error`"
    s.close()

    # run_script streams a grizinit-style batch (comments/blank lines
    # skipped by the SERVER's parse_raw, not Python) to the same
    # dispatcher, all via Command.raw (Decisions 37 & 54).
    s2 = griz.connect("127.0.0.1", server)
    script = REPO_ROOT / "python" / "pygriz" / "tests" / "_grizinit.tmp"
    script.write_text(
        f"# grizinit-style batch\n\nload {root}\n"
        "// trailing comment\nstate 2; show sx\n"
    )
    try:
        r = s2.run_script(script)
        assert r.ok, f"run_script failed: {r.error}"
    finally:
        script.unlink(missing_ok=True)
        s2.close()


def test_handshake_mismatch_warns_not_raises(server):
    """A deliberately bumped client major -> ``compatible == False``
    with a non-empty ``mismatch_detail`` and a Python *warning*, never
    an exception (Decision 36 / the Visit guarantee)."""
    import griz

    with pytest.warns(griz.ProtocolMismatchWarning) as record:
        s = griz.connect("127.0.0.1", server, protocol_version="9999.0.0")
    assert s.compatible is False
    assert s.hello.mismatch_detail, "mismatch must carry a human detail"
    assert any("mismatch" in str(w.message).lower() for w in record)
    s.close()


class _no_warning:
    """Assert ``connect`` on a matching version emits no
    ProtocolMismatchWarning (the positive leg of Decision 36)."""

    def __init__(self, griz_mod):
        self._griz = griz_mod

    def __enter__(self):
        import warnings

        self._ctx = warnings.catch_warnings(record=True)
        self._log = self._ctx.__enter__()
        warnings.simplefilter("always")
        return self

    def __exit__(self, *exc):
        offending = [
            w
            for w in self._log
            if issubclass(w.category, self._griz.ProtocolMismatchWarning)
        ]
        self._ctx.__exit__(*exc)
        assert not offending, f"unexpected mismatch warning: {offending}"
        return False
