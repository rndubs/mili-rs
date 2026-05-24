# FunctionGemma-270M Tool-Calling Debug Report

## Executive Summary

**Status:** v0 baseline run completed but produced 0% L3 pass rate. All 32 L0 failures are `parse_error` — the model generates malformed tool calls that our parser cannot extract.

**Run Details:**
- Provider: `LlamaCppProvider` (llama-server backend)
- Model: `ggml-org/functiongemma-270m-it-GGUF:BF16`
- Scenarios: 50 (bootstrap.jsonl)
- Result: 32 parse_error (L0), 18 step_cap_hit (L2), 0 L3 passes
- Output directory: `data/posttraining/runs/v0-llamacpp-20260524_003947Z/`

## The Problem: Malformed Tool Call Output

### What the Model Actually Outputs

When asked to call a tool (e.g., `load d3samp6`), the model produces:

```
<start_function_call>call:load
<start_function_call><start_function_response>
<start_function_call>call:load
<start_function_call><start_function_response>call:load
<start_function_call><start_function_response>call:load
<start_function_call><start_function_response>call:load
<escape><start_function_response>call:load
<escape><start_function_response>call:load
<escape><start_function_call><start_function_response>call:load
...
```

**Key observations:**
1. The initial part (`<start_function_call>call:load`) looks like it's trying to start a tool call
2. But then it mixes in `<start_function_response>` tags (which shouldn't be in model output)
3. The output gets corrupted/repetitive, mixing `<escape>` tokens
4. No arguments are ever provided
5. No closing `<end_function_call>` tag
6. The pattern repeats endlessly until hitting token limit or step cap

### Example Scenario Details

**Scenario ID:** `bs-001`  
**Intent:** `load`  
**Instruction:** `"load the d3samp6 database"`  
**Tools available:** 18 (including `load`, `set_state`, `step`, etc.)

**Full corrupted output from scenario bs-001 (3788 chars):**
See: `data/posttraining/runs/v0-llamacpp-20260524_003947Z/rollouts.jsonl` (first entry)

## What We Expected

Based on the [FunctionGemma model card](https://huggingface.co/google/functiongemma-270m-it), we expected output like:

```
<start_function_call>call:load{file:<escape>d3samp6<escape>}<end_function_call>
```

Or possibly:

```
<start_function_declaration>
...
<end_function_call>
```

With:
- Proper opening/closing tags
- Function name and arguments in the format `call:name{arg:val,...}`
- Arguments wrapped in `<escape>...<escape>`
- No `<start_function_response>` tags (those should come from the system, not the model)

## What We Tried (and Why It Failed)

### Approach 1: `/v1/chat/completions` Endpoint
**Status:** ❌ Rejected early
- llama-server's `/v1/chat/completions` endpoint does not support `tools` parameter
- The endpoint returns a syntax error when `tools` is included
- This would have been ideal because the model's baked-in Jinja template would apply automatically

### Approach 2: Manual Prompt Construction + `/completion` Endpoint
**Status:** ✅ Code implemented, ❌ but model output broken

**What we did:**
1. Read the model card for FunctionGemma's expected input format
2. Manually constructed the prompt in the format described:
   ```
   <start_of_turn>developer
   [system message]
   <start_function_declaration>
   [tool definitions]
   <end_function_declaration>
   <end_of_turn>
   <start_of_turn>user
   [user message]
   <end_of_turn>
   <start_of_turn>model
   ```
3. Hit the `/completion` endpoint with this prompt
4. Attempted to parse the output with regex matching: `<start_function_call>call:(\w+)\{([^}]*)\}<end_function_call>`

**Why it failed:**
- The model output doesn't match the expected regex pattern at all
- The prompt format may be wrong, or the model card documentation is incomplete/inaccurate
- The model seems to be entering a loop or confusion state, outputting `<start_function_response>` (which should only come from the system)

## Key Code Sections

### Provider Implementation
**File:** `python/mili-llm-bench/src/mili_llm_bench/providers/llamacpp.py`

**Key method: `_build_functiongemma_prompt()`** (lines 195–247)
```python
def _build_functiongemma_prompt(
    self, messages: list[dict[str, Any]], tools: list[dict[str, Any]]
) -> str:
    prompt_parts = []
    dev_content = None
    user_messages = []

    for msg in messages:
        if msg.get("role") in ("developer", "system"):
            dev_content = msg.get("content", "")
        elif msg.get("role") == "user":
            user_messages.append(msg.get("content", ""))

    # Build developer turn with tool declarations
    prompt_parts.append("<start_of_turn>developer\n")
    if dev_content:
        prompt_parts.append(dev_content)
    else:
        prompt_parts.append("You are a helpful assistant.")

    # Add tool declarations if present
    if tools:
        prompt_parts.append("\n\n")
        for tool in tools:
            prompt_parts.append(self._format_tool_declaration(tool))

    prompt_parts.append("\n<end_of_turn>\n")

    # Add user messages
    for user_content in user_messages:
        prompt_parts.append(f"<start_of_turn>user\n{user_content}\n<end_of_turn>\n")

    # Prime the model to generate a response
    prompt_parts.append("<start_of_turn>model\n")

    return "".join(prompt_parts)
```

**Key method: `_format_tool_declaration()`** (lines 249–290)
- Formats individual tools with descriptions and parameter schemas
- Uses `<start_function_declaration>` / `<end_function_declaration>` tags
- Uses `<escape>` tags to wrap string values

**Key method: `_parse_functiongemma_tool_calls()`** (lines 307–342)
- Attempts to parse two formats: text-based and JSON
- Regex pattern: `r"<start_function_call>call:(\w+)\{([^}]*)\}<end_function_call>"`
- Falls back to `_parse_json_tool_calls()` from FunctionGemmaProvider

### Parser Regex
```python
call_pattern = r"<start_function_call>call:(\w+)\{([^}]*)\}<end_function_call>"
matches = re.findall(call_pattern, text)
```

This pattern expects:
- Literal `<start_function_call>call:`
- Function name (word characters)
- Literal `{`
- Arguments (anything except `}`)
- Literal `}`
- Literal `<end_function_call>`

**The actual output matches NONE of this.**

## Baseline Run Artifacts

**Location:** `data/posttraining/runs/v0-llamacpp-20260524_003947Z/`

**Files:**
- `config.yaml` — Run configuration (model, temp, seed, system prompt hash)
- `rollouts.jsonl` — 50 scenario results (one JSON line per scenario), each with:
  - `id`, `fixture`, `intent_id`, `instruction`
  - `tools` (list of tool names)
  - `messages` (conversation history with model outputs)
  - `verifier` (max_tier, failure_mode, reward)
- `report.md` — Aggregated results (tier breakdown, failure modes)
- `summary.json` — Summary stats (L3 pass rate, mean turns, wall time)

**To inspect a failed scenario:**
```python
import json
with open('data/posttraining/runs/v0-llamacpp-20260524_003947Z/rollouts.jsonl') as f:
    for line in f:
        data = json.loads(line)
        if data['verifier']['failure_mode'] == 'parse_error':
            # Find assistant message(s)
            for msg in data['messages']:
                if msg['role'] == 'assistant':
                    print(f"Scenario {data['id']}: {msg['content']}")
            break
```

## Hypotheses

### Hypothesis 1: Tool Declaration Format is Wrong
**Likelihood:** HIGH

The `_format_tool_declaration()` method constructs tool definitions using a custom format that may not match what FunctionGemma actually expects. The model card doesn't provide detailed examples of the input format for tool definitions.

**To test:** Look at actual FunctionGemma fine-tuning examples or source code to see the exact format it expects.

### Hypothesis 2: Model Card Format is Incomplete/Misleading
**Likelihood:** MEDIUM

The model card shows high-level structure but may be missing crucial details about:
- Exact tag names and nesting
- How to include tool definitions in the prompt
- How the model is fine-tuned to output tool calls
- Whether the model even supports tool-calling in the way we're attempting

**To test:** Check the FunctionGemma Hugging Face repo issues, discussion threads, or source code.

### Hypothesis 3: llama-server Quantization/Inference Issues
**Likelihood:** LOW

The model might be breaking due to:
- BF16 quantization artifacts
- llama-server version incompatibilities
- Inference parameter settings (temperature, seed, max_tokens)

**To test:** Compare against a non-quantized run or different llama-server version.

### Hypothesis 4: Model is Confused by the Prompt Format
**Likelihood:** MEDIUM

The model might be entering a loop because:
- The prompt structure doesn't match any training data the model saw
- The tool definitions are triggering unexpected behavior
- The model is trying to output a response/action dialogue but getting confused

**To test:** Try simpler prompts, fewer tools, or different prompt structures.

## Solution Found & Deployed ✅

### Root Cause
The `_format_tool_declaration()` method in `LlamaCppProvider` was constructing tool declarations with a nested, complex format that didn't match FunctionGemma's expectations:

**Wrong format (what we were sending):**
```
<start_function_declaration>
declaration:toolname
{description:<escape>desc<escape>
,parameters:{
properties:{ name:{description:...,type:...} }
}
}
<end_function_declaration>
```

**Correct format (what FunctionGemma expects):**
```
<start_function_declaration>declaration:toolname{
description:<escape>desc<escape>,
parameters:{param1:<escape>type1<escape>,param2:<escape>type2<escape>}
}<end_function_declaration>
```

The key differences:
- Tool name goes immediately after `declaration:` on the same line
- Parameters are simple `key:<escape>type<escape>` pairs, not nested objects
- No `properties:` or `required:` structures in the declaration

### Testing & Verification

Tested with curl directly against llama-server:
```bash
curl -X POST http://localhost:8080/completion \
  -d '{"prompt": "<start_of_turn>developer\n...\n<start_function_declaration>declaration:load{\ndescription:<escape>Load a database file<escape>,\nparameters:{file:<escape>string<escape>}\n}<end_function_declaration>\n...", ...}'
```

Result: Model correctly outputs `<start_function_call>call:load{file:<escape>d3samp6<escape>}<end_function_call>` which parses cleanly.

### Single Scenario Validation

Ran `bs-001` (load the d3samp6 database) after the fix:
- ✅ All 8 turns generated valid tool calls
- ✅ Parser extracted function name and arguments correctly  
- ✅ No `parse_error` failures (0% before → tool calls working now)
- ⏳ Scenario hit step_cap_hit (model behavior issue, not parsing issue)

This represents massive improvement: model went from generating unparseable corrupted output to generating valid tool calls in the expected format.

## Files to Read/Modify

### Implementation
- `python/mili-llm-bench/src/mili_llm_bench/providers/llamacpp.py` — Provider + prompt construction
- `python/mili-llm-bench/src/mili_llm_bench/providers/base.py` — Protocol definition
- `python/mili-llm-bench/src/mili_llm_bench/cli.py` — CLI integration

### Tests
- `python/mili-llm-bench/tests/test_providers_llamacpp.py` — Provider tests (currently all pass because they mock the binary/network)
- `python/mili-llm-bench/tests/test_harness.py` — Harness tests (include parse_error feedback)

### Evaluation
- `python/mili-llm-bench/src/mili_llm_bench/driver.py` — Eval driver
- `python/mili-llm-bench/src/mili_llm_bench/verifier.py` — Verifier (failure mode classification)

## Key Contacts / Documentation

- **Model card:** https://huggingface.co/google/functiongemma-270m-it
- **llama.cpp:** https://github.com/ggml-org/llama.cpp
- **mili-llm-bench design:** `planning/mili-viz/agent-local-llm-baseline.md` (§W1–W6)
- **Run configuration:** See baseline.md § "Caps and determinism"

## What's Known to Work

- The provider integrates correctly (no import errors, health checks pass)
- The harness can dispatch tool calls and handle parse errors
- The verifier correctly classifies `parse_error` as L0 failure
- Mock/replay providers work end-to-end
- The eval driver runs all 50 scenarios without crashing

**What doesn't work:**
- The model outputs malformed tool calls
- Parser cannot extract any tool calls from the output
- Result: every scenario fails on first turn with parse_error (except those that hit step_cap)

