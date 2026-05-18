"""Phase 6 M2 acceptance gate (planning/mili-viz/phase-6-m2.md
§ "M2 acceptance gate", Decisions 56–58).

Two halves, the CLAUDE.md / M1 skip-on-absent convention:

* **always-on pure logic** — ``list_sessions()`` newest-first,
  ``attach(id=)`` / ``attach()`` selection, malformed-file skip, and
  the empty-dir error, all against fabricated session JSON in a
  hermetic ``GRIZ_SESSIONS_DIR`` (no server, no cargo);
* **skip-on-absent** — the real ``mili-viz-server`` binary writing a
  valid session file + ``attach()``/``launch()`` end-to-end against
  it; skipped (never failed) when ``cargo``/the binary is absent.
"""

from __future__ import annotations

import json
import os
import pathlib
import re
import subprocess
import time

import pytest

from conftest import REPO_ROOT, server_binary

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


def _write_session(d: pathlib.Path, sid: str, *, pid=None, port=50051, mtime=None):
    pid = os.getpid() if pid is None else pid
    p = d / f"{sid}.json"
    p.write_text(
        json.dumps(
            {
                "id": sid,
                "pid": pid,
                "host": "127.0.0.1",
                "port": port,
                "token": f"tok-{sid}",
                "protocol_version": "1.0.0",
                "db": "",
            }
        )
    )
    if mtime is not None:
        os.utime(p, (mtime, mtime))
    return p


# --------------------------------------------------------------------
# Always-on pure logic
# --------------------------------------------------------------------


def test_list_sessions_newest_first_and_skips_malformed(sessions_dir):
    import griz

    _write_session(sessions_dir, "old", port=1, mtime=1000.0)
    _write_session(sessions_dir, "new", port=2, mtime=2000.0)
    # A malformed / half-written sibling must be skipped, not raised on.
    (sessions_dir / "broken.json").write_text("{ not json")
    (sessions_dir / "partial.json").write_text('{"id": "p"}')

    sessions = griz.list_sessions()
    ids = [s.id for s in sessions]
    assert ids == ["new", "old"], "newest file (mtime) must come first"
    assert sessions[0].port == 2
    assert all(isinstance(s, griz.SessionInfo) for s in sessions)


def test_attach_by_id_and_newest_selection(sessions_dir, monkeypatch):
    import griz

    _write_session(sessions_dir, "aaa", port=11, mtime=1000.0)
    _write_session(sessions_dir, "bbb", port=22, mtime=2000.0)

    captured: list = []

    def fake_connect(host, port, token="", **kw):
        captured.append((host, port, token))
        return "SESSION"

    monkeypatch.setattr(griz, "connect", fake_connect)

    # id -> that file's endpoint+token.
    assert griz.attach(id="aaa") == "SESSION"
    assert captured[-1] == ("127.0.0.1", 11, "tok-aaa")

    # no args -> newest live file (this pid is alive: os.getpid()).
    assert griz.attach() == "SESSION"
    assert captured[-1] == ("127.0.0.1", 22, "tok-bbb")

    # explicit endpoint bypasses the files entirely.
    assert griz.attach(host="h", port=99, token="t") == "SESSION"
    assert captured[-1] == ("h", 99, "t")


def test_attach_unknown_id_and_empty_dir_raise_clearly(sessions_dir):
    import griz

    with pytest.raises(FileNotFoundError, match="no readable griz session"):
        griz.attach(id="does-not-exist")

    with pytest.raises(RuntimeError, match="no live griz sessions"):
        griz.attach()


def test_attach_skips_dead_pid_for_newest(sessions_dir, monkeypatch):
    import griz

    # Newest file is a dead pid; older one is this (alive) process.
    _write_session(sessions_dir, "alive", pid=os.getpid(), port=7, mtime=1000.0)
    _write_session(sessions_dir, "dead", pid=2**31 - 1, port=8, mtime=2000.0)

    captured: list = []
    monkeypatch.setattr(
        griz, "connect", lambda h, p, t="", **k: captured.append((h, p, t))
    )
    griz.attach()
    assert captured[-1] == ("127.0.0.1", 7, "tok-alive"), "dead-pid skipped"


# --------------------------------------------------------------------
# Skip-on-absent: real server binary writes the session file
# --------------------------------------------------------------------


@pytest.fixture
def server_proc(sessions_dir):
    binary = server_binary()
    if binary is None:
        pytest.skip("mili-viz-server binary/cargo unavailable")
    env = dict(os.environ, GRIZ_SESSIONS_DIR=str(sessions_dir))
    proc = subprocess.Popen(
        [str(binary), "127.0.0.1:0"],
        stdout=subprocess.PIPE,
        stderr=subprocess.STDOUT,
        text=True,
        env=env,
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
        yield proc, port
    finally:
        proc.terminate()
        try:
            proc.wait(timeout=10)
        except subprocess.TimeoutExpired:
            proc.kill()


def test_server_writes_valid_session_file(server_proc, sessions_dir):
    import griz

    proc, port = server_proc

    # The server writes the file just after the bind print; poll.
    deadline = time.time() + 10.0
    info = None
    while time.time() < deadline:
        for s in griz.list_sessions():
            if s.pid == proc.pid:
                info = s
                break
        if info is not None:
            break
        time.sleep(0.05)
    assert info is not None, "server wrote no session file"
    assert info.host == "127.0.0.1"
    assert info.port == port, "session-file port == the bound port"
    assert info.pid == proc.pid
    assert info.token, "token must be written (Jupyter-file contract)"
    assert info.protocol_version == griz.PROTOCOL_VERSION
    assert info.path.parent == sessions_dir


def test_attach_end_to_end_against_real_server(server_proc, sessions_dir):
    import griz

    proc, _ = server_proc
    # Wait for the file, then attach with no args (newest live).
    deadline = time.time() + 10.0
    while time.time() < deadline and not griz.list_sessions():
        time.sleep(0.05)

    s = griz.attach()
    assert s.compatible is True
    root = str(FIXTURE) if FIXTURE.exists() else "nonexistent_run"
    reply = s.command(f"load {root}; state 2; show sx")
    assert reply.ok, f"Layer-0 over attach() failed: {reply.error}"
    s.close()


def test_launch_spawns_attaches_and_close_terminates(sessions_dir):
    import griz

    binary = server_binary()
    if binary is None:
        pytest.skip("mili-viz-server binary/cargo unavailable")

    with griz.launch() as s:
        assert s.compatible is True
        assert s._proc is not None and s._proc.poll() is None
        reply = s.command("load nonexistent_run; state 1")
        assert reply.ok
        proc = s._proc
    # Context exit must terminate the spawned child.
    assert proc.poll() is not None, "launch() child not terminated on close"
