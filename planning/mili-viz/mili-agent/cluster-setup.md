# Cluster setup — SFT pipeline on NVIDIA H100

How to stand up the post-training pipeline on an H100 cluster. The
270M base model is small (~540MB BF16); compute is **not** the
bottleneck — the doc focuses on getting the toolchain wired up
cleanly so a new user can go from a fresh login to a trained
checkpoint without trial-and-error.

This complements [`m5-sft-pipeline.md`](m5-sft-pipeline.md) (the live
SFT tracker) and the local-dev setup in `CLAUDE.md`. Reuse the same
`uv` workspace; only the training stack and llama.cpp build differ
between macOS and Linux.

---

## Hardware baseline

- NVIDIA H100 80GB (Hopper, compute capability **sm_90**).
- Single GPU is plenty for 270M full BF16 fine-tune; multi-GPU is
  irrelevant until the base model grows.

---

## What runs where

| Stage (from `posttraining-dataset.md` §2)                | Platform              | Why                                            |
| -------------------------------------------------------- | --------------------- | ---------------------------------------------- |
| Stage 2 — intent catalog                                 | macOS or Linux        | Pure authoring                                 |
| Stage 3 — scenario synthesis                             | macOS or Linux        | Light LLM paraphrase; no GPU                   |
| Stage 4 — verifier (already implemented)                 | macOS or Linux        | CPU-only Python                                |
| Stage 6.5 — Claude data smoke test                       | Anywhere with network | Bound by Anthropic API                         |
| Stage 5 — teacher rollouts (Claude)                      | Anywhere with network | Bound by Anthropic API                         |
| **SFT training itself**                                  | **H100 cluster**      | GPU, BF16, takes minutes for 270M              |
| GGUF conversion + (optional) quantize                    | H100 cluster or local | One-shot; CPU is fine                          |
| Post-SFT eval (llama-server + bench harness)             | H100 cluster          | Same harness as v5 baseline, new GGUF artifact |

The workflow: iterate Stages 1–4 / 6 / 6.5 locally on macOS against
`MockGrizSession` and Claude; ship the assembled JSONL up to the
cluster only when you're ready to train. Eval can run either side;
running it on the cluster keeps the new GGUF colocated with the
trained checkpoint.

---

## Prerequisites

- Linux (your cluster's distro).
- CUDA Toolkit ≥ **12.4** (recent `nvcc` for H100 codegen). On HPC
  clusters: `module load cuda/12.x`.
- Git, CMake ≥ 3.18, gcc ≥ 11 (or clang ≥ 14).
- Python 3.11+ via `uv` (mirrors the macOS workflow in `CLAUDE.md`).
- Outbound network from compute nodes for Hugging Face / `-hf`
  downloads, or pre-staged model files on a shared filesystem.

---

## 1. Build llama.cpp with CUDA

```bash
git clone https://github.com/ggml-org/llama.cpp
cd llama.cpp
cmake -B build \
  -DGGML_CUDA=ON \
  -DCMAKE_CUDA_ARCHITECTURES=90 \
  -DLLAMA_CURL=ON
cmake --build build --config Release -j
```

Verify and put the binaries on `PATH`:

```bash
./build/bin/llama-server --version
nvidia-smi   # confirm CUDA visible
export PATH="$PWD/build/bin:$PATH"
```

**Why these flags:**

- `GGML_CUDA=ON` — enables the CUDA backend.
- `CMAKE_CUDA_ARCHITECTURES=90` — H100 is sm_90. Pinning to one
  arch on a homogeneous cluster cuts build time and binary size; on
  a mixed cluster, use `"80;86;89;90"`.
- `LLAMA_CURL=ON` — required for the `-hf ggml-org/...:BF16`
  auto-download path used by `llama-server`.

---

## 2. Smoke-test inference

```bash
llama-server -hf ggml-org/functiongemma-270m-it-GGUF:BF16
```

This downloads the BF16 GGUF (~540MB) into `~/.cache/llama.cpp/`
(or wherever `LLAMA_CACHE` points) and serves on `localhost:8080`.
Test it:

```bash
curl -s http://localhost:8080/health
curl -s http://localhost:8080/completion \
  -H 'Content-Type: application/json' \
  -d '{"prompt": "<start_of_turn>user\nhello<end_of_turn>\n<start_of_turn>model\n", "n_predict": 20}'
```

If text comes back, inference works. **Kill the server before SFT**
— training wants the GPU.

On air-gapped clusters, download the GGUF on a login node and serve
with `llama-server -m /path/to/local.gguf` instead.

---

## 3. Python training environment

Use the same `uv` workspace as macOS (`python/` directory). The
existing `functiongemma` extra (`transformers`, `torch`, `accelerate`)
covers inference deps; for training add a new `train` extra to
`python/mili-llm-bench/pyproject.toml` alongside it:

```toml
# Optional — SFT training pipeline. Heavier than `functiongemma`
# (which is inference-only); kept separate so eval-only setups don't
# pull TRL/PEFT.
train = [
  "transformers>=4.50",
  "torch>=2.5",         # CUDA 12.4-compatible build
  "accelerate>=0.34",
  "trl>=0.11",          # SFTTrainer
  "peft>=0.13",         # optional, for LoRA
  "datasets>=3.0",
  "sentencepiece",      # Gemma tokenizer
  # "flash-attn>=2.6",  # add after torch is installed; see §3a
]
```

Then on the cluster:

```bash
# Install matching CUDA torch wheel BEFORE flash-attn:
uv sync --directory python --extra train --extra llamacpp \
  --index-url https://download.pytorch.org/whl/cu124 \
  --index-strategy unsafe-best-match
```

### 3a. Flash Attention 2 (large H100 speedup)

Flash-attn ships source-only wheels gated on a torch ABI match;
install **after** `torch` is in place and outside uv's build
isolation:

```bash
uv run --directory python pip install flash-attn --no-build-isolation
```

In the training code, pass
`attn_implementation="flash_attention_2"` to
`AutoModelForCausalLM.from_pretrained(...)`. On H100 this is
typically **1.5–2× faster** than the default SDPA path. If the
wheel build fails (toolkit-mismatch or missing CUDA headers), fall
back to `attn_implementation="sdpa"` — slower but always works.

---

## 4. Memory budget — full fine-tune, not LoRA

For 270M params on one H100 80GB:

| Component                    | Size (BF16 weights)                        |
| ---------------------------- | ------------------------------------------ |
| Weights                      | 270M × 2 B = **540 MB**                    |
| Gradients                    | 540 MB                                     |
| AdamW state (FP32 m + v)     | 270M × 4 B × 2 = **2.2 GB**                |
| Activations + batch          | a few GB at sensible batch size            |
| **Total**                    | **≤ 5 GB** — fits trivially                |

No reason to use LoRA at this scale. It mainly adds complexity for
no memory or speed gain. Revisit if the base model grows past ~3B.

---

## 5. Source model — HF weights, not GGUF

You train on the original Hugging Face checkpoint. The GGUF artifact
at `ggml-org/functiongemma-270m-it-GGUF` is the **inference output**,
converted from upstream HF weights. The source is
**`google/functiongemma-270m-it`** ⚠ verify the exact id from the
`ggml-org/functiongemma-270m-it-GGUF` README before pinning, in case
the upstream repo path differs.

```python
import torch
from transformers import AutoModelForCausalLM, AutoTokenizer

model_id = "google/functiongemma-270m-it"  # verify
tok = AutoTokenizer.from_pretrained(model_id)
model = AutoModelForCausalLM.from_pretrained(
    model_id,
    torch_dtype=torch.bfloat16,
    attn_implementation="flash_attention_2",  # fallback "sdpa"
    device_map="cuda",
)
```

On clusters with a shared filesystem, point `HF_HOME` at a fast
scratch dir so multiple jobs don't redownload:

```bash
export HF_HOME=/scratch/$USER/hf-cache
```

---

## 6. SFT training (skeleton)

The dataset format from Stage 6 (`sft/{train,val}.jsonl`) already
matches HF chat-template shape (`messages` array), so `SFTTrainer`
consumes it directly. The TRL API has churned across versions — pin
the `trl` version once a recipe works.

```python
from trl import SFTConfig, SFTTrainer
from datasets import load_dataset

train_ds = load_dataset("json", data_files="data/posttraining/sft/train.jsonl")["train"]
val_ds   = load_dataset("json", data_files="data/posttraining/sft/val.jsonl")["train"]

cfg = SFTConfig(
    output_dir="data/posttraining/checkpoints/v1",
    num_train_epochs=3,
    per_device_train_batch_size=8,
    gradient_accumulation_steps=4,
    learning_rate=2e-5,
    bf16=True,
    eval_strategy="epoch",
    save_strategy="epoch",
    logging_steps=10,
    report_to="none",       # add "wandb" if you want a run dashboard
)
trainer = SFTTrainer(
    model=model,
    args=cfg,
    train_dataset=train_ds,
    eval_dataset=val_ds,
    tokenizer=tok,
)
trainer.train()
trainer.save_model("data/posttraining/checkpoints/v1/final")
tok.save_pretrained("data/posttraining/checkpoints/v1/final")  # belt + suspenders
```

This is the canonical-but-skeletal recipe; tighten hyperparameters
after a first successful run. Expected wall-clock for 270M on one
H100: minutes per epoch at v1 corpus size (~200 scenarios → a few
hundred filtered SFT records).

**Important:** train on the `messages` field of the rollout records
— **do not** train on the `tools` array. `tools` is a per-turn
context prefix shown to the model at inference; making it a target
breaks the runtime contract.

---

## 7. Convert checkpoint → GGUF

`llama.cpp` ships a Python converter:

```bash
python llama.cpp/convert_hf_to_gguf.py \
  data/posttraining/checkpoints/v1/final \
  --outfile data/posttraining/checkpoints/v1/functiongemma-v1.bf16.gguf \
  --outtype bf16
```

Optional quantization (BF16 is fine on H100; quantize mainly for
edge/CPU serving):

```bash
llama-quantize \
  data/posttraining/checkpoints/v1/functiongemma-v1.bf16.gguf \
  data/posttraining/checkpoints/v1/functiongemma-v1.q4_k_m.gguf \
  Q4_K_M
```

---

## 8. Eval the new checkpoint

Same harness as the v5 baseline, just pointed at the new GGUF:

```bash
llama-server -m data/posttraining/checkpoints/v1/functiongemma-v1.bf16.gguf \
  --port 8080 &

uv run --directory python/mili-llm-bench mili-llm-bench run \
  --provider llamacpp \
  --scenarios ../../data/posttraining/eval/bootstrap.jsonl \
  --out ../../data/posttraining/runs/v6-sft-pilot-$(date +%Y%m%d-%H%M%S) \
  --step-cap 8 --per-turn-timeout-s 120 --max-new-tokens 256
```

Grade against the M5 gates:

- **Regression tripwire:** ≥ 40 % L3 (the GEPA-only ceiling). Below
  that means SFT is *harming* — stop and diagnose.
- **v1 target:** ≥ 62 % L3 (half the FunctionGemma↔Claude gap).
- **Per-intent floor:** ≥ 50 % L3 on `material`, `select`, `clrsel`,
  `view-reset` (the four 0 %-L3 intents at floor).

---

## Gotchas

- **CUDA version triple-match.** llama.cpp's CUDA build, the `torch`
  CUDA build, and the installed CUDA toolkit need to agree (or be
  ABI-compatible). Pick one toolkit version (12.4 / 12.6) and stay
  there for both binaries.
- **flash-attn wheel failures.** Almost always a toolkit-mismatch or
  missing CUDA headers. Fall back to SDPA; correctness is identical.
- **`-hf …:BF16` requires network from the compute node.** If your
  cluster blocks outbound network from compute nodes, pre-stage the
  GGUF on shared storage and use `-m <path>`.
- **HF model id is unverified above.** `google/functiongemma-270m-it`
  is the natural guess but should be confirmed from the
  `ggml-org/functiongemma-270m-it-GGUF` README before pinning.
- **Tokenizer drift on save.** `SFTTrainer.save_model()` saves the
  HF model; explicitly call `tok.save_pretrained(...)` to the same
  directory so the GGUF converter finds the tokenizer next to the
  weights. A missing/mismatched tokenizer manifests as "the model
  speaks gibberish" post-conversion.
- **Do not train on `tools`.** See §6.

---

## References

- llama.cpp: <https://github.com/ggml-org/llama.cpp>
- TRL (SFTTrainer): <https://huggingface.co/docs/trl>
- Flash Attention 2: <https://github.com/Dao-AILab/flash-attention>
- HF FunctionGemma checkpoint: verify via the
  `ggml-org/functiongemma-270m-it-GGUF` model card on
  huggingface.co.

---

**Last updated:** 2026-05-24. Treat the version-pinned ranges as
ceilings-at-time-of-writing; bump and re-test as the upstream
libraries move.
