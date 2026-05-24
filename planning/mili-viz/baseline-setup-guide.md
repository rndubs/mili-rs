# FunctionGemma v0 Baseline — Setup & Run Guide

Complete guide for setting up and running the FunctionGemma-270M v0 baseline evaluation.

## Prerequisites

- **Rust toolchain** — `cargo build` must work
- **Python 3.11+** — with `uv` package manager
- **llama.cpp** — `llama-server` binary on `$PATH`
- **~1-2 hours** — for full 50-scenario baseline (100-200 seconds per scenario)

## One-Time Setup

Run these once per environment:

```bash
cd /Users/rwhit/Workspace/mili-rs

# 1. Build Rust components
cargo build -p mili-viz-server --release

# 2. Generate Python protobuf stubs
uv run --directory python bash scripts/gen-pygriz-stubs.sh

# 3. Sync Python workspace with all extras
uv sync --directory python --extra llamacpp --extra pygriz
```

## Running the Baseline

Use **two separate terminal windows**:

### **Window 1: Start llama-server**

```bash
llama-server -hf ggml-org/functiongemma-270m-it-GGUF:BF16
```

Wait until you see:
```
main: model loaded
```

### **Window 2: Run the baseline**

From the repo root:

```bash
uv run --directory python/mili-llm-bench mili-llm-bench run \
  --provider llamacpp \
  --scenarios ../../data/posttraining/eval/bootstrap.jsonl \
  --out ../../data/posttraining/runs/v0-llamacpp-baseline \
  --step-cap 8 \
  --per-turn-timeout-s 120 \
  --max-new-tokens 256
```

The progress bar will show real-time progress:

```
Running scenarios:  42%|████▏     | 21/50 [12:34<18:15, 22.32s/scenario, last=bs-021:step_cap_hit, L3=3/21]
```

Showing:
- `42%` — Progress through 50 scenarios
- `21/50` — 21 completed, 29 remaining
- `22.32s/scenario` — Average time per scenario
- `last=bs-021:step_cap_hit` — Last scenario result
- `L3=3/21` — Running count of L3 passes

## Results

After completion, check the results:

```bash
# Summary
cat data/posttraining/runs/v0-llamacpp-baseline/report.md

# Per-scenario details
python3 << 'EOF'
import json
with open('data/posttraining/runs/v0-llamacpp-baseline/rollouts.jsonl') as f:
    for line in f:
        data = json.loads(line)
        print(f"{data['id']}: {data['verifier']['failure_mode']} (L{data['verifier']['max_tier']})")
EOF

# Summary stats
cat data/posttraining/runs/v0-llamacpp-baseline/summary.json | python3 -m json.tool
```

## Troubleshooting

### **"llama-server not found"**
```bash
# Install llama.cpp: https://github.com/ggml-org/llama.cpp#build
# Then ensure llama-server is on $PATH:
which llama-server
```

### **"requests module not found"**
The llamacpp extra wasn't installed. Fix with:
```bash
uv sync --directory python --extra llamacpp
```

### **"griz._proto stubs are not generated"**
Protobuf stubs missing. Generate them:
```bash
uv run --directory python bash scripts/gen-pygriz-stubs.sh
```

### **"mili-viz-server binary not found"**
Rust server wasn't built. Build it:
```bash
cargo build -p mili-viz-server --release
```

### **Baseline runs but all scenarios fail instantly**
FakeDispatcher is being used (pygriz isn't available). This means:
- ✅ Tool format is working (no parse errors)
- ❌ Real scenarios aren't running (no verification)

Ensure pygriz extra is installed:
```bash
uv sync --directory python --extra pygriz
```

### **Progress bar shows thousands of scenarios/second**
Scenarios are running through FakeDispatcher (not real griz). See above.

## Python Workspace Notes

The `python/` directory is a **uv workspace** with two members:

- `mili-llm-bench` — Baseline harness (main CLI entry point)
- `pygriz` — Python bindings to Rust griz dispatcher

Key commands:

```bash
# Sync base dependencies only
uv sync --directory python

# Add optional extras
uv sync --directory python --extra llamacpp        # llama.cpp provider
uv sync --directory python --extra pygriz          # griz dispatcher
uv sync --directory python --extra dev             # pytest, development tools

# Run command in workspace environment
uv run --directory python <command>

# Run specific script in workspace
uv run --directory python python3 script.py
```

**Important:** Never run `pip install` directly. Always use `uv` to ensure
all packages stay in sync within the workspace.

## Next Steps (Post-v0)

Per `planning/mili-viz/agent-local-llm-baseline.md` § "After v0":

- **If L3 ≥10%:** Baseline is adequate; post-training is optional
- **If L0 ≈0% but L3 ≈0%:** Consider post-training (SFT, DPO, GRPO)
- **If L0 is high:** Prompt format still wrong; debug further
- **If compound multi-turn fails:** Add richer tool response shapes

Current status:
- L0 (parse_error): 0% ✅ — format fix validated
- L3: 0% — model behavior, not implementation issue
