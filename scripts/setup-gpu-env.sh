#!/usr/bin/env bash
# Cluster session env for the SFT pipeline (per
# planning/mili-viz/mili-agent/cluster-setup.md). Source me at the start
# of a login or compute-node shell to load modules + cache + PATH the
# same way every time.
#
# Usage:
#   source scripts/setup-gpu-env.sh
#
# Idempotent. Must be sourced (it loads lmod modules and exports env
# vars); executing it has no effect on the caller's shell.
#
# Mirrors the toolchain used by ../llama.cpp's build.sh (gcc/12.1.1 +
# cuda/12.9.1 + cmake/3.30.5), so llama-server's CUDA backend, any
# downstream source builds (e.g. flash-attn later), and the
# llama-cpp-compatible CUDA runtime see one consistent toolkit.

# Detect sourcing — `module load` only has effect in the caller's shell.
if [[ "${BASH_SOURCE[0]}" == "${0}" ]]; then
    echo "ERROR: source this script, don't execute it." >&2
    echo "  source scripts/setup-gpu-env.sh" >&2
    return 1 2>/dev/null || exit 1
fi

# ----- modules -----------------------------------------------------------
# Match cadsat/build.sh: same gcc + cuda + cmake the existing
# llama.cpp build was linked against.
source /etc/profile.d/z00_lmod.sh 2>/dev/null || true
module unload intel-classic mvapich2 2>/dev/null || true
module load gcc/12.1.1 cuda/12.9.1 cmake/3.30.5

# ----- pre-built llama.cpp on PATH ---------------------------------------
# Built by ../llama.cpp/build.sh; we don't rebuild here.
_llama_bin=/p/vast1/whitmore/cadsat/llama.cpp/build/bin
if [[ -d "$_llama_bin" ]]; then
    case ":$PATH:" in
        *":$_llama_bin:"*) : ;;
        *) export PATH="$_llama_bin:$PATH" ;;
    esac
fi
unset _llama_bin

# ----- HuggingFace settings ----------------------------------------------
# We *don't* override HF_HOME — the user's hf-cli token lives at
# ~/.cache/huggingface/token (default location), home is visible from
# compute nodes, and existing dataset caches already live there. Revisit
# if home-dir quota becomes the bottleneck (set HF_HUB_CACHE then, not
# HF_HOME, so credentials stay alongside the rest of ~/.cache).
export HF_HUB_DISABLE_TELEMETRY=1

# ----- uv link mode ------------------------------------------------------
# uv's cache (~/.cache/uv) and the workspace venv (python/.venv on
# /p/vast1) live on different filesystems, so hardlinks fail. `copy`
# is correct here; `reflink` would also work on filesystems that
# support it.
export UV_LINK_MODE=copy

# ----- summary -----------------------------------------------------------
echo "Cluster SFT env ready:"
printf "  gcc:        %s\n" "$(gcc --version 2>/dev/null | head -1)"
printf "  cuda:       %s\n" "$(nvcc --version 2>/dev/null | tail -1)"
printf "  cmake:      %s\n" "$(cmake --version 2>/dev/null | head -1)"
printf "  llama-server: %s\n" "$(command -v llama-server || echo 'not on PATH')"
printf "  HF cache:   %s\n" "${HF_HUB_CACHE:-${HF_HOME:-$HOME/.cache/huggingface}}"
