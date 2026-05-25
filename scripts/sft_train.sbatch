#!/usr/bin/env bash
# SFT v1 training — slurm submission entry point.
#
# Usage (from the repo root, on a login node):
#   sbatch scripts/sft_train.sbatch
#
# Or directly inside an existing GPU allocation:
#   srun scripts/sft_train.sbatch
#
# Overrides:
#   sbatch --time=02:00:00 --partition=pdebug scripts/sft_train.sbatch
#   sbatch scripts/sft_train.sbatch --learning-rate 1e-5 --num-train-epochs 4
#
# Extra args after the script name are forwarded to sft_train.py.

#SBATCH --job-name=sft-fg-v1
#SBATCH --nodes=1
#SBATCH --ntasks=1
#SBATCH --cpus-per-task=4
#SBATCH --gres=gpu:1
#SBATCH --time=01:00:00
#SBATCH --output=logs/sft-train-%j.out
#SBATCH --error=logs/sft-train-%j.err

set -euo pipefail

# Resolve repo root from this script's location so the launcher works
# regardless of cwd at submission time.
script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
repo_root="$(cd "$script_dir/.." && pwd)"
cd "$repo_root"
mkdir -p logs "$repo_root/data/posttraining/checkpoints/v1"

# Training-safe env. NOT sourcing scripts/setup-gpu-env.sh: that script
# loads `cuda/12.9.1` to keep llama.cpp's CUDA backend ABI-consistent
# with its build toolchain, which would put system CUDA libs on
# LD_LIBRARY_PATH and risk shadowing PyTorch's bundled CUDA 13 runtime
# (the nvidia-* PyPI wheels in the workspace venv). Same reasoning as
# the comment block at the top of scripts/gpu-sanity.sh. The two env
# vars below are the training-safe subset of setup-gpu-env.sh's exports.
export HF_HUB_DISABLE_TELEMETRY=1
export UV_LINK_MODE=copy

echo "=== job context ==="
echo "host:        $(hostname)"
echo "job id:      ${SLURM_JOB_ID:-<interactive>}"
echo "partition:   ${SLURM_JOB_PARTITION:-<unknown>}"
echo "repo_root:   $repo_root"
echo "cwd:         $(pwd)"
date
echo

echo "=== gpu visibility ==="
if ! nvidia-smi --query-gpu=name,driver_version,memory.total,memory.free --format=csv; then
    echo >&2
    echo "FAIL: no GPU visible from $(hostname). This script must run on a GPU node." >&2
    echo "  Submit via slurm:   sbatch $0 $*" >&2
    echo "  Or interactively:   srun --partition=pbatch --time=00:30:00 --gres=gpu:1 $0 $*" >&2
    exit 1
fi
echo

echo "=== launching sft_train.py ==="
exec uv run --directory python python scripts/sft_train.py "$@"
