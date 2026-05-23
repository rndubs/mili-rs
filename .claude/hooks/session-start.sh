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
# while CI caught the regression). This inits the reference/mili-python,
# reference/mili AND reference/griz submodules (explicitly by path, not
# recursively) and pip-installs the Python oracle.
scripts/setup-parity.sh

# Software Vulkan (Mesa lavapipe) so the GPU-gated mili-viz-client
# tests actually run instead of `skip-on-absent` returning silently
# when `wgpu::Instance::request_adapter` finds no ICD. The container
# ships `libvulkan1` (the loader) but no driver, so wgpu's Vulkan
# backend has nothing to enumerate. `mesa-vulkan-drivers` installs
# lavapipe (a software rasteriser) and registers it at
# /usr/share/vulkan/icd.d/lvp_icd.*.json; wgpu picks it up via the
# loader with no env-var poking.
#
# Web-only — local devs and CI nodes that already have a real GPU (or
# their own ICD) are unaffected. Idempotent: skips if the package is
# already installed.
install_software_vulkan() {
  if ! command -v apt-get >/dev/null 2>&1; then
    echo "==> apt-get unavailable; skipping software Vulkan install"
    return 0
  fi
  if dpkg -s mesa-vulkan-drivers >/dev/null 2>&1; then
    echo "==> mesa-vulkan-drivers already installed"
    return 0
  fi
  local sudo=""
  if [ "$(id -u)" != "0" ] && command -v sudo >/dev/null 2>&1; then
    sudo="sudo"
  fi
  echo "==> installing mesa-vulkan-drivers (lavapipe software ICD)"
  # `apt-get update` can fail on third-party PPAs whose signatures /
  # mirrors are unreachable from the web container (e.g. deadsnakes,
  # ondrej returning 403). The main archive's package list is usually
  # fresh enough that the install still works, so swallow update
  # failures and try the install anyway.
  $sudo apt-get update -qq || \
    echo "    apt-get update had errors (likely 3rd-party PPAs); attempting install with cached lists"
  if ! $sudo apt-get install -y --no-install-recommends mesa-vulkan-drivers; then
    echo "    mesa-vulkan-drivers install failed; GPU tests will skip"
    return 0
  fi
}
install_software_vulkan

# Warm the pinned toolchain + dependency cache so the first `cargo
# test` / `cargo clippy` in the session is fast.
cargo fetch
