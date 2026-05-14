#!/bin/bash
set -euo pipefail

# Web-only: locally we trust the user to manage their own checkout.
if [ "${CLAUDE_CODE_REMOTE:-}" != "true" ]; then
  exit 0
fi

cd "$CLAUDE_PROJECT_DIR"

# Fixture-parity tests skip-on-absent against
# reference/mili-python/tests/data/serial/...; without the submodule a
# local `cargo test` looks green while CI catches the regression. mili
# and griz are referenced only by path, so we don't need their content.
git submodule update --init --depth 1 reference/mili-python

# Warm the pinned toolchain + dependency cache so the first `cargo
# test` / `cargo clippy` in the session is fast.
cargo fetch
