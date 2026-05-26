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

## §0. Pre-flight checklist — gate before any `trainer.train()` call

These items were resolved off-GPU (✅) or must be resolved **on the
cluster, before the first real training run** (🛑). See
[`sft-preflight-gpu.md`](sft-preflight-gpu.md) for the runnable scripts
matching the 🛑 items.

### Resolved off-GPU (2026-05-24)

- ✅ **HF model id is `google/functiongemma-270m-it`.** Confirmed via
  the GGUF README at `ggml-org/functiongemma-270m-it-GGUF`
  (`base_model: google/functiongemma-270m-it`). The model is **gated**
  on Hugging Face — a direct anonymous fetch returns HTTP 401; cluster
  users must `huggingface-cli login` with a token that has accepted
  the Gemma license before §5 will succeed.
- ✅ **Reference hyperparameters pinned** to Google's published recipe
  (LR 5e-5, 8 epochs, batch 4, constant LR, `max_length=512`,
  `packing=False`, `adamw_torch_fused`). Any deviation from these is a
  deliberate, justified change — see §6.
- ✅ **TRL API contract pinned** to `processing_class=` (not the
  deprecated `tokenizer=`) and `trl>=1.0,<2`; transformers pinned
  to `>=4.50,<5` and `dtype=` (not the deprecated `torch_dtype=`).
  The original rev-2 pin (`trl>=0.11,<0.13`) was bumped on
  2026-05-25 — see `m5-sft-pipeline.md` rev 16: `assistant_only_loss`
  (rev-2's stated justification for the pin) did not actually exist
  on trl 0.11/0.12; it landed in trl 0.20+. The `<2` upper bound is
  a defensive ceiling against a hypothetical trl 2.x API break.
- ✅ **`tools.json` shape is `{name, description, input_schema,
  output_schema}`** (W1 proto-derived). FunctionGemma's chat template
  expects OpenAI-style `{"type": "function", "function": {"name",
  "description", "parameters"}}`. The conversion is the same one
  already used by `providers/llamacpp.py::_convert_to_openai_tool`
  (`input_schema` → `parameters`); Stage 6 of the SFT assembly reuses
  it verbatim. See §6 below.
- ✅ **v1 pilot uses `attn_implementation="eager"`** (matches Google's
  validated recipe). Move to `flash_attention_2` only after the v1
  pilot clears the regression tripwire.
- ✅ **Assistant-only loss masking** is wired via a custom data
  collator (`mili_llm_bench.assistant_only_collator.MaskAssistantOnlyCollator`),
  not TRL's native `assistant_only_loss=True` kwarg. The original
  rev-12 seam claim ("`assistant_only_loss=True` is pinned in
  `SFTConfig`") was vacuous: the kwarg didn't exist on the rev-2
  pinned TRL 0.12.x (rev 15 / 16), and on rev-16's bumped TRL 1.5.0
  the kwarg exists but `SFTTrainer.__init__` raises on FG's chat
  template (lacks `{% generation %}` markers and is too macro-heavy
  for TRL's auto-patch). See `m5-sft-pipeline.md` rev 17 and report
  `data/posttraining/sft/preflight-4-loss-mask.md`. The custom
  collator masks: developer / user / tool turns + the assistant-turn
  `<start_of_turn>model\n` header + tool-response payloads inside
  model turns. Without this masking, the 270M model gets gradient
  signal on user prompts and tool stdout — actively harmful at this
  scale.

### Must be resolved on-GPU before training (see `sft-preflight-gpu.md`)

- 🛑 **`SFTTrainer` + `tools` field test.** TRL's default collator may
  not pass `row["tools"]` into `apply_chat_template`. Dump one
  tokenized sample and grep for `start_function_declaration`; if
  absent, use the explicit `formatting_func` in §6.
- ✅ **Train-vs-inference chat-template parity.** Resolved 2026-05-24
  (m5-sft-pipeline.md rev 8) via Path A — `_build_functiongemma_prompt`
  deleted; `LlamaCppProvider.generate` now POSTs
  `/v1/chat/completions` with llama-server in `--jinja` mode. Single
  template source: the FG jinja baked into the GGUF (and mirrored on
  the HF tokenizer). Test pin in
  `python/mili-llm-bench/tests/test_providers_llamacpp.py::
  TestChatCompletionsPath`. **v5 floor re-baseline pending on a GPU
  node** — see `sft-preflight-gpu.md` §2 "Required follow-on".
- ✅ **`assistant_only_loss` mask check.** Cleared 2026-05-25 on
  `matrix41` H100 via custom data collator (option B) — see above and
  `m5-sft-pipeline.md` rev 17. TRL's native path failed on FG's
  template; `MaskAssistantOnlyCollator` is the in-tree replacement.
- 🛑 **GGUF chat-template baking.** `convert_hf_to_gguf.py` carries
  the tokenizer's `chat_template` into the GGUF. After conversion,
  `diff` the `tokenizer_config.json` between source HF model and saved
  final checkpoint to detect any silent template drift introduced by
  TRL.
- 🛑 **Effective `max_length` audit.** Render the longest assembled
  rollout (compound multi-step + full tools array) through
  `apply_chat_template(..., tokenize=True)` and confirm length <
  `max_length`. Multi-step compound scenarios with the full ~16-tool
  inventory can blow past 512 tokens silently.

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
llama-server -hf ggml-org/functiongemma-270m-it-GGUF:BF16 --jinja
```

This downloads the BF16 GGUF (~540MB) into `~/.cache/llama.cpp/`
(or wherever `LLAMA_CACHE` points) and serves on `localhost:8080`.

`--jinja` is **required** — `LlamaCppProvider` (m5-sft-pipeline.md
rev 8) drives the server through `/v1/chat/completions` and relies
on the server applying the FunctionGemma chat template baked into
the GGUF. Without `--jinja`, the server returns plain text and the
tool-calls field stays empty.

Smoke-test it:

```bash
curl -s http://localhost:8080/health
# /v1/chat/completions exercises the --jinja path used in production:
curl -s http://localhost:8080/v1/chat/completions \
  -H 'Content-Type: application/json' \
  -d '{"model": "fg", "messages": [{"role": "user", "content": "hello"}],
       "max_tokens": 20}'
```

If a JSON response with `choices[0].message.content` comes back,
inference works. **Kill the server before SFT** — training wants
the GPU.

On air-gapped clusters, download the GGUF on a login node and serve
with `llama-server -m /path/to/local.gguf --jinja` instead.

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
#
# Version pins are upper-bounded because both TRL and transformers
# have churned APIs across minors (tokenizer→processing_class,
# torch_dtype→dtype, SFTTrainer collator behavior on `tools`). Bump
# one library at a time and re-run §0's pre-flight before training.
train = [
  "transformers>=4.50,<5",     # `dtype=` arg; chat_template w/ tools
  "torch>=2.5,<3",             # CUDA 12.4-compatible build
  "accelerate>=0.34,<2",
  "trl>=1.0,<2",               # SFTTrainer w/ assistant_only_loss
                                # (added in trl 0.20+; the original
                                #  trl>=0.11,<0.13 pin pre-dated the
                                #  feature — see m5-sft-pipeline.md
                                #  rev 16)
  "peft>=0.13,<0.14",          # optional; we full-FT at 270M, see §4
  "datasets>=3.0,<5",
  "sentencepiece",             # Gemma tokenizer
  "protobuf",                  # Gemma tokenizer dep
  # "flash-attn>=2.6",         # add after torch is installed; see §3a
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
**`google/functiongemma-270m-it`** (confirmed via the GGUF repo's
`base_model:` metadata; see §0).

**Gated model — log in once per cluster session.** FunctionGemma is
license-gated; anonymous `from_pretrained` fetches return HTTP 401.
Accept the license on huggingface.co under your account, mint a
read token, then:

```bash
huggingface-cli login --token "$HF_TOKEN"
```

```python
from transformers import AutoModelForCausalLM, AutoTokenizer

model_id = "google/functiongemma-270m-it"
tok = AutoTokenizer.from_pretrained(model_id)
model = AutoModelForCausalLM.from_pretrained(
    model_id,
    dtype="auto",                       # transformers ≥4.50: `dtype` (not `torch_dtype`)
    attn_implementation="eager",        # v1 pilot: match Google's validated recipe
                                        # post-pilot: switch to "flash_attention_2"
    device_map="cuda",
)
```

Rationale for `eager`: Google's reference fine-tune recipe was
validated with `attn_implementation="eager"`. Flash-attn 2 is faster
on H100 but introduces a second variable to debug if the v1 pilot
yields garbage; defer the switch until after the regression tripwire
is cleared.

On clusters with a shared filesystem, point `HF_HOME` at a fast
scratch dir so multiple jobs don't redownload:

```bash
export HF_HOME=/scratch/$USER/hf-cache
```

---

## 6. SFT training (recipe)

The dataset format from Stage 6 (`sft/{train,val}.jsonl`) has one
row per rollout in canonical FunctionGemma shape:

```json
{
  "messages": [{"role": "developer", "content": "..."}, ...],
  "tools":    [{"type": "function", "function": {"name": "...", "description": "...", "parameters": {...}}}, ...]
}
```

The `tools` array is **the OpenAI/FunctionGemma shape**, *not* the
W1 `{name, description, input_schema, output_schema}` shape from
`data/posttraining/grammar/tools.json`. Stage 6 of assembly applies
the conversion (same as `providers/llamacpp.py::_convert_to_openai_tool`:
`name`→`function.name`, `description`→`function.description`,
`input_schema`→`function.parameters`); train-time data must not
require the trainer to do that conversion.

### Hyperparameters — pinned to Google's reference

These match Google's published FunctionGemma fine-tuning guide
(16/20 success on the reference task). **Deviate only with a one-line
justification in the changelog** — silent drift is the failure mode
that produces a worse-than-baseline model with no obvious cause.

| Knob | Value | Source |
|---|---|---|
| `learning_rate` | `5e-5` | Google reference |
| `num_train_epochs` | `8` | Google reference |
| `per_device_train_batch_size` | `4` | Google reference |
| `gradient_accumulation_steps` | `1` | (no accumulation — Google reference uses `bs=4` raw) |
| `lr_scheduler_type` | `"constant"` | Google reference |
| `max_length` | `4096` | **Deviation from Google's `512`** — preflight #5 audit observed `max=3341` on the rev-13 corpus's 82 rows; 4096 is the next power-of-2 ceiling. See `m5-sft-pipeline.md` rev 14. |
| `packing` | `False` | Google reference |
| `optim` | `"adamw_torch_fused"` | Google reference |
| `bf16` | `True` | H100 native |
| `eval_strategy` | `"epoch"` | Google reference |
| `assistant_only_loss` | `False` at the TRL level | TRL 1.5.0's native path raises on FG's chat template (preflight #4, m5-sft-pipeline.md rev 17). Loss masking is supplied by `MaskAssistantOnlyCollator` via `data_collator=`. The training *intent* (loss only on assistant turns) is unchanged — see §0. |

### Training script

```python
from transformers import DataCollatorForLanguageModeling
from trl import SFTConfig, SFTTrainer
from datasets import load_dataset

from mili_llm_bench.assistant_only_collator import MaskAssistantOnlyCollator

train_ds = load_dataset("json", data_files="data/posttraining/sft/train.jsonl")["train"]
val_ds   = load_dataset("json", data_files="data/posttraining/sft/val.jsonl")["train"]

# Belt-and-suspenders: render the chat template ourselves so the
# `tools` field is guaranteed to reach apply_chat_template. TRL's
# default collator has historically dropped non-`messages` columns
# in some minor versions; the explicit formatting_func removes that
# variable. The §0 pre-flight test verifies whether this is needed,
# but applying it unconditionally costs nothing.
def formatting_func(row):
    return tok.apply_chat_template(
        row["messages"],
        tools=row["tools"],
        add_generation_prompt=False,
        tokenize=False,
    )

cfg = SFTConfig(
    output_dir="data/posttraining/checkpoints/v1",
    # Google reference recipe (see table above)
    num_train_epochs=8,
    per_device_train_batch_size=4,
    gradient_accumulation_steps=1,
    learning_rate=5e-5,
    lr_scheduler_type="constant",
    max_length=4096,  # preflight #5 bumped from 512 — m5-sft-pipeline.md rev 14
    packing=False,
    optim="adamw_torch_fused",
    bf16=True,
    # Tool-calling SFT specifics — TRL's native path raises on FG's template
    # (preflight #4, m5-sft-pipeline.md rev 17). Mask via the custom collator
    # passed below; keep this False to bypass TRL's broken auto-patch.
    assistant_only_loss=False,
    # Reporting / checkpoints
    eval_strategy="epoch",
    save_strategy="epoch",
    logging_steps=1,
    report_to="none",       # add "wandb" if you want a run dashboard
)
trainer = SFTTrainer(
    model=model,
    args=cfg,
    train_dataset=train_ds,
    eval_dataset=val_ds,
    processing_class=tok,           # TRL ≥0.11: replaces deprecated `tokenizer=`
    formatting_func=formatting_func,  # mandatory on trl 0.12.x (KeyError otherwise);
                                      # optional on trl 1.x (auto-detect) but kept for drift-proofing
    data_collator=MaskAssistantOnlyCollator(
        DataCollatorForLanguageModeling(tokenizer=tok, mlm=False),
    ),
)
trainer.train()
trainer.save_model("data/posttraining/checkpoints/v1/final")
tok.save_pretrained("data/posttraining/checkpoints/v1/final")  # belt + suspenders
```

Expected wall-clock for 270M on one H100: minutes per epoch at v1
corpus size (~200 scenarios → a few hundred filtered SFT records).

**Important constraints:**

- Train on the `messages` field; `tools` is a **context prefix** shown
  to the model at inference, never a target. `MaskAssistantOnlyCollator`
  enforces this at the token level (loss masked everywhere except
  assistant turns; see §0 and `m5-sft-pipeline.md` rev 17 for why
  TRL's native `assistant_only_loss=True` path doesn't work on FG's
  template).
- **Do not change the tokenizer.** Any modification to
  `tokenizer_config.json` (chat template, special tokens, vocabulary)
  between source HF model and saved checkpoint will silently break
  GGUF conversion or inference. The §0 pre-flight diffs these to
  catch drift.

### Launching the run

The recipe above is realized as `python/scripts/sft_train.py`; submit
via the slurm wrapper `scripts/sft_train.sbatch`. The launcher
short-circuits with a recovery hint if invoked from a non-GPU host
(catches the common "ran it from `matrix2`" failure mode where
`device_map="cuda"` would otherwise crash several seconds into model
loading).

```bash
# Queued, from a login node — 1h H100 job, logs to logs/sft-train-<JOBID>.{out,err}
sbatch scripts/sft_train.sbatch

# SBATCH overrides
sbatch --partition=pdebug --time=00:30:00 scripts/sft_train.sbatch

# Forwarded sft_train.py overrides (anything after the script name)
sbatch scripts/sft_train.sbatch \
  --num-train-epochs 4 \
  --output-dir data/posttraining/checkpoints/v1-4ep

# Interactive smoke — 1 epoch is ≈ 1–2 min on H100
srun --partition=pbatch --time=00:15:00 --gres=gpu:1 \
  ./scripts/sft_train.sbatch --num-train-epochs 1 --output-dir /tmp/sft-smoke

# Direct execution from an existing GPU shell (matrix41 etc.)
./scripts/sft_train.sbatch
```

`./scripts/sft_train.sbatch` as a direct command works because
`#SBATCH` lines are bash comments outside of slurm; the script `exec`s
into `uv run --directory python python scripts/sft_train.py "$@"` at
the end. Defaults to the rev-4 §6 hyperparameter table; override only
with a one-line justification in the run's `dataset_card.md` so silent
drift is impossible.

The launcher **does not** source `scripts/setup-gpu-env.sh` — that
script loads `cuda/12.9.1` to keep llama.cpp's CUDA backend
ABI-consistent with its build toolchain, which would shadow PyTorch's
bundled CUDA 13 runtime via `LD_LIBRARY_PATH`. Same reasoning as the
top-of-file comment in `scripts/gpu-sanity.sh`. The launcher inlines
the training-safe subset (`HF_HUB_DISABLE_TELEMETRY=1`,
`UV_LINK_MODE=copy`) instead.

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

Two paths, gated on whether GGUF conversion has happened yet:

### 8a. HF checkpoint — `--provider transformers` (canonical for v1 sweep)

Grade an HF checkpoint directly. The provider re-uses
`tokenizer.apply_chat_template(messages, tools=tools, …)` — the same
call SFTTrainer rendered against — so the prompt distribution at eval
matches training byte-for-byte; no chat-template mutation risk to debug.

```bash
uv run --directory python mili-llm-bench run \
  --provider transformers \
  --model-path /p/vast1/whitmore/cadsat/mili-rs/data/posttraining/checkpoints/v1/checkpoint-21 \
  --scenarios /p/vast1/whitmore/cadsat/mili-rs/data/posttraining/sft/eval/heldout.jsonl \
  --out /p/vast1/whitmore/cadsat/mili-rs/data/posttraining/runs/v1-sft-checkpoint-21-$(date +%Y%m%d-%H%M%S) \
  --step-cap 8 --per-turn-timeout-s 120 --max-new-tokens 256
```

- Use absolute paths for `--model-path`, `--scenarios`, `--out` (the
  `uv run --directory python` cwd-shift makes relative paths break in
  1 s; see memory `bench-cli-uv-cwd`).
- Do **not** source `scripts/setup-gpu-env.sh` for this path — the
  `cuda/12.9.1` module-load shadows torch's bundled CUDA 13 runtime on
  `LD_LIBRARY_PATH` (same reasoning as `gpu-sanity.sh`'s top-of-file
  comment).
- The training stack already covers the runtime — `uv sync --directory
  python --extra train` (`transformers`, `torch`, `accelerate`) is the
  whole dependency set.

The 8-checkpoint sweep is just the above command in a `for` loop over
`data/posttraining/checkpoints/v1/checkpoint-*`. One H100 grades the
full 81-row heldout split in ~3–5 s per scenario, so the entire 8 × 81
sweep finishes in roughly 30 minutes.

### 8b. GGUF checkpoint — `--provider llamacpp` (post-winner only)

After the sweep picks a winner, convert that one HF checkpoint to GGUF
(§7) and re-grade through llama-server to confirm the conversion didn't
mutate the chat template or the model output. Same eval, different
provider:

```bash
llama-server -m data/posttraining/checkpoints/v1/functiongemma-v1.bf16.gguf \
  --port 8080 --jinja &

uv run --directory python mili-llm-bench run \
  --provider llamacpp \
  --scenarios /p/vast1/whitmore/cadsat/mili-rs/data/posttraining/sft/eval/heldout.jsonl \
  --out /p/vast1/whitmore/cadsat/mili-rs/data/posttraining/runs/v1-sft-winner-gguf-$(date +%Y%m%d-%H%M%S) \
  --step-cap 8 --per-turn-timeout-s 120 --max-new-tokens 256
```

For the llamacpp path, *do* source `scripts/setup-gpu-env.sh` (same
Bash invocation as `uv run`; see memory
`bench-gpu-env-source-per-bash`) — the `llama-server` binary needs the
matched CUDA runtime that torch must not see.

### Gates

- **Regression tripwire:** ≥ 40 % L3 (the GEPA-only ceiling). Below
  that means SFT is *harming* — stop and diagnose.
- **v1 target:** ≥ 62 % L3 (half the FunctionGemma↔Claude gap).
- **Per-intent floor:** ≥ 50 % L3 on the four 0-rate intents (`colormap`,
  `select`, `show-primal`, `show-derived` per the v7 baseline). The
  rev-2 list (`material`, `select`, `clrsel`, `view-reset`) was from
  an earlier baseline; see `m5-sft-pipeline.md` v7 per-intent
  breakdown for the live floor.

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
- **FunctionGemma is gated.** Anonymous `from_pretrained` returns
  HTTP 401. `huggingface-cli login` with a token from an account that
  accepted the Gemma license. See §5.
- **Tokenizer drift on save.** `SFTTrainer.save_model()` saves the
  HF model; explicitly call `tok.save_pretrained(...)` to the same
  directory so the GGUF converter finds the tokenizer next to the
  weights. A missing/mismatched tokenizer manifests as "the model
  speaks gibberish" post-conversion. The §0 pre-flight diff catches
  this before it's a debugging mystery.
- **Train-vs-inference chat template parity.** The runtime serving
  path (`providers/llamacpp.py`) hand-rolls the FunctionGemma prompt
  rather than using the GGUF's baked-in jinja template. Training via
  HF `apply_chat_template` will likely *not* produce a byte-identical
  prompt to what llama-server feeds the trained model. §0 pre-flight
  forces a decision: switch llama-server to `--jinja`, or back-port
  the bespoke format into a custom HF chat template. **Skipping this
  decision is the most likely way to ship a model that scores 0 % L3
  post-SFT for reasons that have nothing to do with training.**
- **Do not train on `tools`.** See §6. `assistant_only_loss=True`
  enforces this.

---

## References

- llama.cpp: <https://github.com/ggml-org/llama.cpp>
- TRL (SFTTrainer): <https://huggingface.co/docs/trl>
- Flash Attention 2: <https://github.com/Dao-AILab/flash-attention>
- HF FunctionGemma checkpoint: verify via the
  `ggml-org/functiongemma-270m-it-GGUF` model card on
  huggingface.co.

---

**Last updated:** 2026-05-25 (rev 6). Treat the version-pinned ranges
as ceilings-at-time-of-writing; bump and re-test as the upstream
libraries move.

## Changelog

- **2026-05-25 (rev 6).** Eval path landed for HF checkpoints. §8 split
  into 8a (HF via `--provider transformers`, canonical for the v1 sweep)
  and 8b (GGUF via `--provider llamacpp`, post-winner only). The
  `transformers` provider re-uses `tokenizer.apply_chat_template(...)`
  — the same call SFTTrainer rendered against — so the prompt
  distribution at eval matches training byte-for-byte; no
  chat-template-mutation risk for the seven losing checkpoints in the
  sweep. See `m5-sft-pipeline.md` rev 21 for the provider's landing,
  the deleted `FunctionGemmaProvider` (broken parser, no functional
  callers — replaced wholesale), and the v1-SFT-corpus JSON-literal
  arguments-shape discovery (chat template's string-branch produced
  double-braced tool-call bodies in the training tokens; parser now
  handles both shapes — `TODO(v2)` to re-render the corpus with
  dict-shaped arguments and retire the JSON branch). Per-intent
  floor list corrected to match the v7 baseline's actual 0-rate
  intents (colormap/select/show-primal/show-derived) — the rev-2
  list (material/select/clrsel/view-reset) was from an earlier
  baseline.

- **2026-05-25 (rev 5).** Training entry point landed.
  `python/scripts/sft_train.py` realizes the §6 recipe (rev 4) as a
  runnable argparse-driven script with defaults matching the pinned
  hyperparameter table. `scripts/sft_train.sbatch` is the slurm
  submission wrapper — works either via `sbatch`, `srun`, or direct
  execution from an existing GPU shell (since `#SBATCH` lines are
  bash comments outside slurm, the script `exec`s into the `uv run`
  invocation at the end). Fails loud via `nvidia-smi` gate + recovery
  hint if invoked from a non-GPU host (`matrix2` etc.), with the
  exact sbatch / srun fallback commands printed inline. Inlines the
  training-safe subset of `setup-gpu-env.sh`'s exports
  (`HF_HUB_DISABLE_TELEMETRY=1`, `UV_LINK_MODE=copy`) but does not
  source it — avoids the `cuda/12.9.1` ↔ torch-bundled-CUDA-13
  `LD_LIBRARY_PATH` shadow the `gpu-sanity.sh` comment block warns
  about. Drive-by drift fix: §6 "Important constraints" first bullet
  was still attributing assistant-only loss masking to TRL's native
  `assistant_only_loss=True` kwarg — corrected to point at
  `MaskAssistantOnlyCollator` per rev 4.

- **2026-05-25 (rev 4).** Preflight #4 cleared via custom collator
  (option B). TRL 1.5.0's native `assistant_only_loss=True` fails at
  `SFTTrainer.__init__` on FG's chat template (`ValueError: not
  training-compatible (missing prefix-preservation or {% generation %}
  markers)`). The rev-2 §0 seam claim is now correctly attributed to
  `MaskAssistantOnlyCollator` (`mili_llm_bench.assistant_only_collator`)
  rather than the kwarg. §6 recipe updated: `assistant_only_loss=False`
  at the TRL level; loss masking via `data_collator=` instead. Hyperparam
  table row reworded; no other hyperparameters changed. New always-on
  test pin
  (`mili-llm-bench/tests/test_assistant_only_collator.py`, 8 cases).
  Full mili-llm-bench suite: 229 / 229 + 1 skip (+8 from rev 3's
  221 / 221). See `m5-sft-pipeline.md` rev 17 and report
  `data/posttraining/sft/preflight-4-loss-mask.md`.

- **2026-05-25 (rev 3).** Bumped `trl` pin from `>=0.11,<0.13` to
  `>=1.0,<2`. The rev-2 pin was self-contradicting: its stated
  justification was `assistant_only_loss`, but that kwarg was added
  in trl 0.20+ — neither 0.11 nor 0.12 supported it. Verified TRL
  1.5.0 against this repo: full `mili-llm-bench` test suite still
  passes (221 / 221 + 1 skip — same as rev 14 baseline);
  preflight #3 still PASSes (with `formatting_func`, and *also*
  without — TRL 1.x auto-detects chat-shape rows and dispatches
  `apply_chat_template`, which trl 0.12.1 did not). The recipe in §6
  keeps `formatting_func` for drift-proofing against a hypothetical
  TRL 2.x change in auto-detect behavior. Stop-loss option (if a
  future bump breaks something): revert to `trl>=0.20,<0.21`, which
  is the oldest release that still carries `assistant_only_loss`.
  Also corrected `max_length=512` → `4096` to reflect the deliberate
  preflight #5 bump that was already recorded in
  `m5-sft-pipeline.md` rev 14 but not yet mirrored here.
- **2026-05-24 (rev 2).** Added §0 pre-flight checklist (split into
  off-GPU ✅ and on-GPU 🛑 items). Pinned hyperparameters to Google's
  reference recipe (LR 5e-5 / 8 epochs / bs=4 / constant LR /
  `max_length=512` / `packing=False` / `adamw_torch_fused`); the prior
  draft (LR 2e-5 / 3 epochs / effective bs=32) is dropped — it
  deviated without justification. Fixed `tokenizer=` →
  `processing_class=`; `torch_dtype=` → `dtype="auto"`;
  `attn_implementation="flash_attention_2"` → `"eager"` for v1 pilot.
  Added `assistant_only_loss=True`. Pinned `transformers<5`,
  `trl<0.13`. Documented `tools.json` →
  FunctionGemma-shape conversion. Confirmed `google/functiongemma-270m-it`
  via GGUF `base_model:` metadata; documented the gated-model login
  requirement. Flagged train-vs-inference chat-template parity as a
  day-1 cluster decision.
- **2026-05-24 (rev 1).** Initial draft.
