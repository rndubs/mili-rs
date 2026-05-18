"""Shared fixtures for the pygriz gate.

Stub generation mirrors the frozen M1 ``test_m1_connect.py`` autouse
fixture (kept independent so the frozen M1 gate is untouched), plus the
M2 binary-discovery + hermetic ``GRIZ_SESSIONS_DIR`` helpers
(phase-6-m2.md Decision 56). The skip-on-absent rule (CLAUDE.md): skip,
never fail, when ``grpcio-tools``/``cargo``/the binary is unavailable.
"""

from __future__ import annotations

import os
import pathlib
import subprocess

import pytest

REPO_ROOT = pathlib.Path(__file__).resolve().parents[3]


@pytest.fixture(scope="session", autouse=True)
def _stubs_generated():
    """Generate the gitignored ``griz._proto`` build output from the
    one canonical proto before any test imports ``griz`` (Decision 53;
    skip-on-absent when grpcio-tools is missing — CLAUDE.md)."""
    try:
        import grpc_tools  # noqa: F401
    except ImportError:
        pytest.skip("grpcio-tools absent (pip install -e python/pygriz[dev])")
    gen = REPO_ROOT / "scripts" / "gen-pygriz-stubs.sh"
    subprocess.run([str(gen)], check=True, cwd=REPO_ROOT)


def server_binary() -> pathlib.Path | None:
    """Prebuilt ``target/{release,debug}/mili-viz-server`` else
    ``cargo build`` it; ``None`` when cargo/the binary is unavailable
    (skip-on-absent, exactly the M1 gate's approach / CLAUDE.md)."""
    import shutil

    for profile in ("release", "debug"):
        p = REPO_ROOT / "target" / profile / "mili-viz-server"
        if p.exists():
            return p
    cargo = shutil.which("cargo")
    if cargo is None:
        return None
    build = subprocess.run([cargo, "build", "-p", "mili-viz-server"], cwd=REPO_ROOT)
    if build.returncode != 0:
        return None
    p = REPO_ROOT / "target" / "debug" / "mili-viz-server"
    return p if p.exists() else None


@pytest.fixture
def sessions_dir(tmp_path, monkeypatch):
    """A hermetic ``GRIZ_SESSIONS_DIR`` (Decision 56) — the spawned
    server writes here, the ``griz`` reader reads here, the real
    ``~/.griz`` is never touched by the gate."""
    d = tmp_path / "griz-sessions"
    d.mkdir()
    monkeypatch.setenv("GRIZ_SESSIONS_DIR", str(d))
    return d
