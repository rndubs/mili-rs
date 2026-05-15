#!/usr/bin/env bash
# Canonical, repeatable setup for the reference-mili-python-dependent
# tests (the `parity` feature and the xmilics / full-corpus suites).
#
# Single source of truth: CI's `test-parity` job, the Claude Code
# session-start hook, and local developers all run THIS script so the
# environment is identical everywhere. If a parity test passes here it
# passes in CI, and vice versa.
#
# What it does:
#   1. Checks out the two submodules the suites read:
#        - reference/mili-python : the Python oracle + tests/data corpus
#        - reference/mili        : the C-library xmilics multi-proc corpus
#   2. Installs the mili-python package editable, which pulls every
#      Python runtime dep (numpy, pandas, dill, psutil,
#      typing_extensions, matplotlib, ...) transitively from its
#      pyproject — never hand-maintain that list.
#
# Idempotent: safe to re-run. Usage:
#   scripts/setup-parity.sh
#   cargo test --workspace --features parity
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repo_root"

# `--depth 1` keeps the corpus checkout small; both submodules are
# data/reference only. We do NOT recurse — the nested mdgtest
# submodule under reference/mili/test/ lives on an LLNL-internal SSH
# host unreachable from CI / web runners.
echo "==> submodule: reference/mili-python"
git submodule update --init --depth 1 reference/mili-python
echo "==> submodule: reference/mili"
git submodule update --init --depth 1 reference/mili

# Editable install so `import mili` resolves to the checked-out
# submodule. Deps come from reference/mili-python/pyproject.toml.
python_bin="${PYTHON:-python3}"
echo "==> pip install -e reference/mili-python (via $python_bin)"
# Best-effort pip upgrade: skip quietly when pip is system/distro
# managed (e.g. Debian's pip 24.0 has no RECORD and can't self-
# uninstall). The editable install below is the load-bearing step.
"$python_bin" -m pip install --upgrade pip || \
  echo "    (pip self-upgrade skipped — distro-managed pip; continuing)"
"$python_bin" -m pip install -e reference/mili-python

echo "==> parity environment ready. Run:"
echo "      cargo test --workspace --features parity"
