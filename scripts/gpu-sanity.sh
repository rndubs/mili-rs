#!/usr/bin/env bash
# Minimal GPU sanity check for the SFT pipeline. Run via:
#   srun -N1 -n1 -t 5 -p pdebug --gres=gpu:1 scripts/gpu-sanity.sh
#
# Confirms:
#   1. nvidia driver visible from the allocated node
#   2. PyTorch's bundled CUDA runtime works (matmul on the GPU)
#   3. The H100 reports sm_90 + supports BF16 (the dtype training uses)
#
# Does *not* load the cuda/12.9.1 module: PyTorch's wheel ships its
# own CUDA 13 runtime via the nvidia-* PyPI packages and we don't
# want LD_LIBRARY_PATH from the module to shadow them.

set -uo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repo_root/python"

echo "=== nvidia-smi ==="
nvidia-smi --query-gpu=name,driver_version,memory.total,memory.free --format=csv

echo
echo "=== PyTorch CUDA sanity ==="
uv run python -c "
import torch
print('torch:           ', torch.__version__)
print('cuda available:  ', torch.cuda.is_available())
print('device count:    ', torch.cuda.device_count())
print('device name:     ', torch.cuda.get_device_name(0))
print('capability:      ', torch.cuda.get_device_capability(0), '(want (9, 0) for H100)')
print('bf16 supported:  ', torch.cuda.is_bf16_supported())
print('cuda runtime ver:', torch.version.cuda)
print('cudnn version:   ', torch.backends.cudnn.version())

# Real op on the GPU — not just init.
x = torch.randn(2048, 2048, device='cuda', dtype=torch.bfloat16)
y = (x @ x.T)
torch.cuda.synchronize()
print('bf16 2048x2048 matmul: shape=', tuple(y.shape), 'dtype=', y.dtype)
print('result sum:      ', float(y.sum()))
print('peak alloc MB:   ', torch.cuda.max_memory_allocated() // (1024*1024))
"
