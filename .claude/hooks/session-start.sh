#!/bin/bash
set -euo pipefail

# Web-only: locally we trust the user to manage their own checkout.
if [ "${CLAUDE_CODE_REMOTE:-}" != "true" ]; then
  exit 0
fi

cd "$CLAUDE_PROJECT_DIR"

# Provision the parity environment via the same script CI uses so web
# sessions run the reference-mili-python-dependent tests rather than
# silently skipping them (which made `cargo test` look green locally
# while CI caught the regression). This inits the reference/mili-python
# AND reference/mili submodules and pip-installs the Python oracle.
# griz is referenced only by path, so we don't need its content.
scripts/setup-parity.sh

# Warm the pinned toolchain + dependency cache so the first `cargo
# test` / `cargo clippy` in the session is fast.
cargo fetch
