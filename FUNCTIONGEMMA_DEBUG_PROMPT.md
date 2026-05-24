# FunctionGemma Tool-Calling Debug Session — Start Here

## Context & Status

You are debugging why **FunctionGemma-270M via llama-server produces malformed tool calls** that fail to parse. A v0 baseline eval was run and completed with a **0% L3 pass rate** — all 50 scenarios either failed on first turn with `parse_error` (32 scenarios) or hit the step cap without ever producing valid tool calls (18 scenarios).

### The Problem in 30 Seconds

When the model is asked to call a tool (e.g., "load the d3samp6 database"), instead of outputting:
```
<start_function_call>call:load{file:<escape>d3samp6<escape>}<end_function_call>
```

It outputs broken, repeating text:
```
<start_function_call>call:load
<start_function_call><start_function_response>
<start_function_call>call:load
<start_function_call><start_function_response>call:load
<escape><start_function_response>call:load
...
[repeats endlessly]
```

Key issues:
1. No closing `<end_function_call>` tags
2. Mystery `<start_function_response>` tags (should only come from system)
3. No function arguments ever provided
4. Endless repetition / corruption

### Root Cause Hypothesis

The prompt format we manually constructed (reverse-engineered from the FunctionGemma model card) doesn't match what the model expects. The `/v1/chat/completions` endpoint doesn't support tools in llama-server, so we can't use the model's baked-in template. Manual construction failed.

### Baseline Run Results

- **Output directory:** `data/posttraining/runs/v0-llamacpp-20260524_003947Z/`
- **Scenarios:** 50 (from `data/posttraining/eval/bootstrap.jsonl`)
- **Tiers:** L0=32 (parse_error), L1=0, L2=18 (step_cap_hit), L3=0
- **Mean turns:** 3.52 (scenarios that didn't fail on turn 1)
- **Wall time:** 4980 seconds (~83 minutes)

### Key Files

**Implementation:**
- `python/mili-llm-bench/src/mili_llm_bench/providers/llamacpp.py` — The provider + prompt construction
  - `_build_functiongemma_prompt()` (lines 195–247) — Constructs the prompt sent to the model
  - `_format_tool_declaration()` (lines 249–290) — Formats individual tool definitions
  - `_parse_functiongemma_tool_calls()` (lines 307–342) — Tries to parse the output
- `python/mili-llm-bench/src/mili_llm_bench/cli.py` — CLI integration (calls `build_factories()`)
- `python/mili-llm-bench/pyproject.toml` — Dependencies (only `requests` for llamacpp)

**Tests:**
- `python/mili-llm-bench/tests/test_providers_llamacpp.py` — Provider tests (all pass, but they mock the binary/network)

**Baseline data:**
- `data/posttraining/runs/v0-llamacpp-20260524_003947Z/rollouts.jsonl` — 50 raw scenario results (JSON lines)
- `data/posttraining/runs/v0-llamacpp-20260524_003947Z/report.md` — Aggregated results
- `data/posttraining/runs/v0-llamacpp-20260524_003947Z/summary.json` — Summary stats

**Debug documents (created for this session):**
- `planning/mili-viz/functiongemma-debug-report.md` — Full analysis with hypotheses
- `planning/mili-viz/functiongemma-debug-prompts.md` — Test commands & prompt format ideas

## What We Know Works

- The provider code integrates correctly (no import errors, health checks work)
- The harness can dispatch tool calls and handle parse errors
- The verifier correctly classifies failures
- Mock/replay providers work end-to-end
- The eval driver ran all 50 scenarios without crashing
- llama-server is responsive and running fine

## What Doesn't Work

- The model outputs malformed tool calls
- Parser regex cannot extract any tool calls from the output
- Every scenario fails on first turn with `parse_error` (except those that hit step cap)

## Debugging Steps for This Session

### Phase 1: Understand the Current Prompt

1. Read `planning/mili-viz/functiongemma-debug-report.md` for full context
2. Run this Python script to see the exact prompt being sent:

```bash
cd /Users/rwhit/Workspace/mili-rs
python3 << 'EOF'
import json
import sys
sys.path.insert(0, 'python/mili-llm-bench/src')

from mili_llm_bench.providers.llamacpp import LlamaCppProvider
from mili_llm_bench.harness import Registry

registry = Registry.load_from_artifact()
tools = registry.all()

messages = [
    {
        "role": "developer",
        "content": "You are an assistant that operates the Griz post-processor for the Mili finite-element format. You drive Griz by emitting JSON function calls into the supplied tool inventory. Inspect the user's request, call exactly the tools that satisfy it, and reply with one short final text message only after the request is fully complete. Do not narrate plans; emit a tool call instead. Prefer the typed tools over the `griz_raw` fallback when a typed tool exists for the task."
    },
    {"role": "user", "content": "load the d3samp6 database"}
]

provider = LlamaCppProvider()
prompt = provider._build_functiongemma_prompt(messages, tools)

print("=== FULL PROMPT ===")
print(prompt)
print(f"\n=== LENGTH: {len(prompt)} chars ===")
EOF
```

This will show you the exact structure being sent to the model.

### Phase 2: Test Model Capability

Make sure the model can do basic completions (to rule out llama-server/quantization issues):

```bash
cd /Users/rwhit/Workspace/mili-rs

# Make sure llama-server is running
llama-server -hf ggml-org/functiongemma-270m-it-GGUF:BF16 2>&1 &
sleep 5

# Test 1: Simple math (no tools)
curl -s -X POST http://localhost:8080/completion \
  -H "Content-Type: application/json" \
  -d '{
    "prompt": "Q: What is 2+2?\nA:",
    "temperature": 0.0,
    "n_predict": 50,
    "seed": 0
  }' | python3 -c "import sys, json; print(json.load(sys.stdin)['content'])"

# Test 2: Simple instruction (no tools)
curl -s -X POST http://localhost:8080/completion \
  -H "Content-Type: application/json" \
  -d '{
    "prompt": "<start_of_turn>developer\nYou are helpful.\n<end_of_turn>\n<start_of_turn>user\nHello! What can you do?\n<end_of_turn>\n<start_of_turn>model\n",
    "temperature": 0.0,
    "n_predict": 50,
    "seed": 0
  }' | python3 -c "import sys, json; print(json.load(sys.stdin)['content'])"
```

If these produce reasonable output, the model is working. If they're corrupted, we have a llama-server/quantization problem, not a prompt format problem.

### Phase 3: Inspect Actual Model Output from Baseline

Extract and examine what the model actually produced:

```python
python3 << 'EOF'
import json

# Look at first few scenarios
with open('data/posttraining/runs/v0-llamacpp-20260524_003947Z/rollouts.jsonl') as f:
    for i, line in enumerate(f):
        if i >= 3:
            break
        data = json.loads(line)
        
        print(f"\n{'='*60}")
        print(f"Scenario: {data['id']} | Intent: {data['intent_id']}")
        print(f"Failure: {data['verifier']['failure_mode']}")
        print(f"{'='*60}")
        
        # Find assistant message
        for msg in data['messages']:
            if msg['role'] == 'assistant':
                content = msg['content']
                print(f"Assistant output ({len(content)} chars):")
                print(content[:500])
                if len(content) > 500:
                    print("\n... [middle omitted] ...\n")
                    print(content[-200:])
EOF
```

This shows you the raw model output that failed to parse.

### Phase 4: Research FunctionGemma Format

Check these sources:
1. **Model card:** https://huggingface.co/google/functiongemma-270m-it
   - Look for tool-calling examples or documentation
   - Check discussions/issues for other users having similar problems

2. **GitHub:** https://github.com/google/functiongemma
   - Check README for tool-calling documentation
   - Look for example prompts

3. **llama.cpp issues:** https://github.com/ggml-org/llama.cpp/issues
   - Search for "FunctionGemma" to see if others have reported issues

### Phase 5: Test Alternative Prompt Formats

Try different tool declaration formats to see if any produce valid output. See `planning/mili-viz/functiongemma-debug-prompts.md` for the 4 hypotheses (A–D).

Start with the simplest format that still makes sense:

```python
# Hypothesis B: Simpler plain-text format
prompt = """<start_of_turn>developer
You are a helpful assistant. You can call these functions:
- load(file: string): Load a database file
- set_state(step: int): Set simulation state
<end_of_turn>
<start_of_turn>user
load the d3samp6 database
<end_of_turn>
<start_of_turn>model
"""
```

Or:

```python
# Hypothesis C: JSON format for tools
prompt = """<start_of_turn>developer
You are a helpful assistant with these tools:

{"tools": [{"name": "load", "description": "Load a database", "parameters": {"type": "object", "properties": {"file": {"type": "string"}}}}]}

When you need to call a tool, respond with: call:name{arg:val}
<end_of_turn>
<start_of_turn>user
load the d3samp6 database
<end_of_turn>
<start_of_turn>model
"""
```

Test each format and see which (if any) produces better output.

### Phase 6: If Alternative Formats Work

Once you find a format that produces valid tool calls, update:
1. `_build_functiongemma_prompt()` in `llamacpp.py`
2. `_parse_functiongemma_tool_calls()` if the output format changed
3. Re-run a small subset of scenarios to verify the fix
4. Run full baseline again

## Quick Reference: Key Commands

```bash
# Start llama-server
llama-server -hf ggml-org/functiongemma-270m-it-GGUF:BF16

# Check it's running
curl http://localhost:8080/health

# Run a single scenario manually
python3 << 'EOF'
import sys
sys.path.insert(0, 'python/mili-llm-bench/src')
from mili_llm_bench.providers.llamacpp import LlamaCppProvider
from mili_llm_bench.harness import Registry
from mili_llm_bench.scenarios import Scenario
import json

# Load first scenario
with open('data/posttraining/eval/bootstrap.jsonl') as f:
    data = json.loads(f.readline())
scenario = Scenario.from_dict(data)

# Generate
provider = LlamaCppProvider()
registry = Registry.load_from_artifact()
output = provider.generate(
    messages=[
        {"role": "developer", "content": "You are helpful."},
        {"role": "user", "content": scenario.instruction}
    ],
    tools=registry.all(),
    temperature=0.0,
    max_new_tokens=512,
    seed=0
)
print(f"Tool calls: {output.tool_calls}")
print(f"Raw output:\n{output.raw[:500]}")
EOF

# Run a small baseline (first 5 scenarios)
mili-llm-bench run --provider llamacpp \
  --scenarios data/posttraining/eval/bootstrap.jsonl \
  --out /tmp/v0-debug/ \
  --step-cap 8 \
  --per-turn-timeout-s 120 \
  --max-new-tokens 512
```

## Success Criteria

A successful fix should:
1. Produce valid tool calls that match the parsing regex (or have regex updated)
2. Extract at least some L1/L2/L3 passes (not all parse_error)
3. Get the model out of the repetition loop
4. Ideally achieve >10% L3 pass rate on the bootstrap eval

## Documents to Read

In order:
1. This file (you're reading it now)
2. `planning/mili-viz/functiongemma-debug-report.md` — Full analysis
3. `planning/mili-viz/functiongemma-debug-prompts.md` — Test commands & format ideas

## Questions to Answer

As you debug, try to answer:
1. **Can the model do basic completions without tools?** (Tests Phase 1)
2. **What exactly is in the malformed output?** (Phase 3)
3. **What does the FunctionGemma documentation actually say?** (Phase 4)
4. **Which alternative prompt format produces the best output?** (Phase 5)
5. **Is the model's Jinja template the right one?** (Compare model card to actual chat template)

## Good Luck! 🚀

You have:
- ✅ Real baseline data (50 concrete failures to analyze)
- ✅ Prompt construction code (easy to modify)
- ✅ Parsing code (easy to update if format changes)
- ✅ Test infrastructure (can re-run quickly)

The fix is probably one of these:
1. **Wrong tool declaration format** → Fix `_format_tool_declaration()`
2. **Wrong turn markers** → Fix the `<start_of_turn>` structure
3. **Model wants JSON or simpler format** → Replace the whole custom format
4. **Model card docs are incomplete** → Find the right format in GitHub/discussions

Go find it! 💪

