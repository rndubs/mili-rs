# FunctionGemma Debug: Prompt Analysis & Test Commands

## Current Prompt Structure (What We're Sending)

This is what `_build_functiongemma_prompt()` currently constructs:

```
<start_of_turn>developer
You are an assistant that operates the Griz post-processor for the Mili finite-element format. You drive Griz by emitting JSON function calls into the supplied tool inventory. Inspect the user's request, call exactly the tools that satisfy it, and reply with one short final text message only after the request is fully complete. Do not narrate plans; emit a tool call instead. Prefer the typed tools over the `griz_raw` fallback when a typed tool exists for the task.

<start_function_declaration>
declaration:load
{description:<escape>Load a Griz database from a file.<escape>
,parameters:{
properties:{ file:{description:<escape>Path to database file (basename; no ext).<escape>,type:<escape>STRING<escape>}}
}
}
<end_function_declaration>

[... 17 more tool declarations follow same pattern ...]

<end_of_turn>
<start_of_turn>user
load the d3samp6 database
<end_of_turn>
<start_of_turn>model

```

### What We Built Tool Declarations To Look Like

**Example: the `load` tool**

```
<start_function_declaration>
declaration:load
{description:<escape>Load a Griz database from a file.<escape>
,parameters:{
properties:{ file:{description:<escape>Path to database file (basename; no ext).<escape>,type:<escape>STRING<escape>}}
}
}
<end_function_declaration>
```

**Format breakdown:**
- `declaration:<name>` — tool name
- `{description:<escape>...<escape>` — description in escaped format
- `,parameters:{` — parameters object
- `properties:{` — parameter definitions
- `<name>:{description:<escape>...<escape>,type:<escape>TYPE<escape>}}` — each parameter
- `}<end_function_declaration>` — closing

## Problems We Know

1. **The model outputs `<start_function_response>` tags** — These should ONLY come from the system (when reporting tool results), not from the model. The fact that the model is outputting them suggests it's confused about its role or the prompt structure.

2. **No `<end_function_call>` tags** — The model never closes the tool call block, just repeats opening tags.

3. **No arguments ever provided** — The model starts with `<start_function_call>call:load` but never provides `{file:...}`.

4. **Endless repetition** — The output loops endlessly (or until token limit), recycling the same malformed tags.

## Debugging Commands for Fresh Session

### 1. Test Basic Model Capability (No Tools)

```bash
#!/bin/bash
cd /Users/rwhit/Workspace/mili-rs

# Start llama-server if not running
llama-server -hf ggml-org/functiongemma-270m-it-GGUF:BF16 2>&1 > /tmp/llama-server.log &
LLAMA_PID=$!
sleep 5

# Test 1: Simple completion
echo "=== TEST 1: Simple completion ==="
curl -s -X POST http://localhost:8080/completion \
  -H "Content-Type: application/json" \
  -d '{
    "prompt": "Q: What is 2+2?\nA:",
    "temperature": 0.0,
    "n_predict": 100,
    "seed": 0
  }' | python3 -m json.tool | grep -A 5 '"content"'

# Test 2: Simple instruction
echo -e "\n=== TEST 2: Simple instruction ==="
curl -s -X POST http://localhost:8080/completion \
  -H "Content-Type: application/json" \
  -d '{
    "prompt": "<start_of_turn>developer\nYou are helpful.\n<end_of_turn>\n<start_of_turn>user\nHello, what can you do?\n<end_of_turn>\n<start_of_turn>model\n",
    "temperature": 0.0,
    "n_predict": 100,
    "seed": 0
  }' | python3 -m json.tool | grep -A 5 '"content"'

# Test 3: With a simple tool (no parameters)
echo -e "\n=== TEST 3: Simple tool (no params) ==="
curl -s -X POST http://localhost:8080/completion \
  -H "Content-Type: application/json" \
  -d '{
    "prompt": "<start_of_turn>developer\nYou can call tools.\n<start_function_declaration>\ndeclaration:help\n<end_function_declaration>\n<end_of_turn>\n<start_of_turn>user\nCall the help tool.\n<end_of_turn>\n<start_of_turn>model\n",
    "temperature": 0.0,
    "n_predict": 100,
    "seed": 0
  }' | python3 -m json.tool | grep -A 10 '"content"'

# Kill llama-server
kill $LLAMA_PID 2>/dev/null
```

### 2. Inspect Actual Prompt Being Sent

```python
#!/usr/bin/env python3
import json
import sys
sys.path.insert(0, '/Users/rwhit/Workspace/mili-rs/python/mili-llm-bench/src')

from mili_llm_bench.providers.llamacpp import LlamaCppProvider
from mili_llm_bench.harness import Registry

# Load registry and get tools
registry = Registry.load_from_artifact()
tools = registry.all()

# Create a sample scenario's messages
messages = [
    {
        "role": "developer",
        "content": "You are an assistant that operates the Griz post-processor for the Mili finite-element format. You drive Griz by emitting JSON function calls into the supplied tool inventory. Inspect the user's request, call exactly the tools that satisfy it, and reply with one short final text message only after the request is fully complete. Do not narrate plans; emit a tool call instead. Prefer the typed tools over the `griz_raw` fallback when a typed tool exists for the task."
    },
    {
        "role": "user",
        "content": "load the d3samp6 database"
    }
]

# Build the prompt
provider = LlamaCppProvider()
prompt = provider._build_functiongemma_prompt(messages, tools)

print("=== FULL PROMPT BEING SENT ===")
print(prompt)
print("\n=== PROMPT LENGTH ===")
print(f"{len(prompt)} characters")
print("\n=== FIRST 500 CHARS ===")
print(prompt[:500])
print("\n=== LAST 500 CHARS ===")
print(prompt[-500:])
```

### 3. Run a Single Scenario with Full Debug Output

```python
#!/usr/bin/env python3
import json
import sys
sys.path.insert(0, '/Users/rwhit/Workspace/mili-rs/python/mili-llm-bench/src')

from mili_llm_bench.scenarios import Scenario
from mili_llm_bench.providers.llamacpp import LlamaCppProvider
from mili_llm_bench.driver import EvalConfig

# Load first scenario
with open('data/posttraining/eval/bootstrap.jsonl') as f:
    scenario_data = json.loads(f.readline())
scenario = Scenario.from_dict(scenario_data)

# Set up provider and config
provider = LlamaCppProvider()
config = EvalConfig()

# Build messages
messages = [
    {"role": "developer", "content": config.system_prompt},
    {"role": "user", "content": scenario.instruction},
]

# Get tools
from mili_llm_bench.harness import Registry
registry = Registry.load_from_artifact()
tools = registry.all()

print(f"=== Scenario: {scenario.id} ===")
print(f"Intent: {scenario.intent_id}")
print(f"Fixture: {scenario.fixture}")
print(f"Instruction: {scenario.instruction}\n")

print("=== PROMPT BEING SENT ===")
prompt = provider._build_functiongemma_prompt(messages, tools)
print(prompt)
print(f"\nPrompt length: {len(prompt)} chars\n")

print("=== CALLING PROVIDER ===")
try:
    output = provider.generate(
        messages=messages,
        tools=tools,
        temperature=config.temperature,
        max_new_tokens=config.max_new_tokens,
        seed=config.seed
    )
    print(f"Output type: {type(output)}")
    print(f"Tool calls: {output.tool_calls}")
    print(f"Final text: {output.final_text}")
    if output.raw:
        print(f"\n=== RAW OUTPUT ({len(output.raw)} chars) ===")
        print(output.raw[:1000])
        print("\n...")
        print(output.raw[-500:])
except Exception as e:
    print(f"ERROR: {e}")
    import traceback
    traceback.print_exc()
```

### 4. Extract & Display Sample Outputs from Baseline Run

```python
#!/usr/bin/env python3
import json

def show_scenario(num, show_full=False):
    with open('data/posttraining/runs/v0-llamacpp-20260524_003947Z/rollouts.jsonl') as f:
        for i, line in enumerate(f):
            if i == num:
                data = json.loads(line)
                print(f"=== Scenario {data['id']} ===")
                print(f"Intent: {data['intent_id']}")
                print(f"Fixture: {data['fixture']}")
                print(f"Failure mode: {data['verifier']['failure_mode']}")
                print(f"Max tier: {data['verifier']['max_tier']}\n")
                
                # Show messages
                for msg_idx, msg in enumerate(data['messages']):
                    print(f"Message {msg_idx}: role={msg['role']}")
                    if msg['role'] == 'user':
                        print(f"  {msg['content']}")
                    elif msg['role'] == 'developer':
                        print(f"  {msg['content'][:100]}...")
                    elif msg['role'] == 'assistant':
                        content = msg.get('content', '')
                        if show_full:
                            print(f"  {content}")
                        else:
                            print(f"  {content[:200]}...")
                            print(f"  ... ({len(content)} total chars)")
                    else:
                        print(f"  {msg['content'][:100]}...")
                print()
                return
    print(f"Scenario {num} not found")

print("=== First scenario (index 0) ===")
show_scenario(0)

print("\n=== Scenario 5 (different intent?) ===")
show_scenario(5)

print("\n=== Full raw output from scenario 0 ===")
show_scenario(0, show_full=True)
```

### 5. Check if Model Outputs Match Any Expected Pattern

```python
#!/usr/bin/env python3
import json
import re

# Load all scenarios and check what patterns are present
with open('data/posttraining/runs/v0-llamacpp-20260524_003947Z/rollouts.jsonl') as f:
    for i, line in enumerate(f):
        if i >= 5:  # Just check first 5
            break
        data = json.loads(line)
        
        # Find assistant message
        for msg in data['messages']:
            if msg['role'] == 'assistant':
                content = msg['content']
                
                print(f"\n=== Scenario {data['id']} ===")
                
                # Check various patterns
                patterns = {
                    'start_function_call': r'<start_function_call>',
                    'start_function_declaration': r'<start_function_declaration>',
                    'end_function_call': r'<end_function_call>',
                    'start_function_response': r'<start_function_response>',
                    'escape': r'<escape>',
                    'call_with_args': r'call:\w+\{[^}]+\}',
                    'call_no_args': r'call:\w+(?!\{)',
                }
                
                for name, pattern in patterns.items():
                    matches = re.findall(pattern, content)
                    if matches:
                        print(f"  {name}: {len(matches)} matches")
                        print(f"    First 3: {matches[:3]}")
                
                # Show first 300 chars
                print(f"\n  First 300 chars:")
                print(f"  {repr(content[:300])}")
                break
```

## Prompt Format Hypotheses to Test

### Hypothesis A: Different Tool Declaration Format

Try this instead:

```
<start_function_call>
name:load
description:Load a Griz database from a file.
parameters:
  file:
    type:string
    description:Path to database file
<end_function_call>
```

### Hypothesis B: No Special Tags, Just Function List

```
Available functions:
- load(file: str) — Load a Griz database from a file
- set_state(step: int) — Set the simulation state to the given step
...
```

### Hypothesis C: JSON Format for Tool Definitions

```json
{
  "tools": [
    {
      "name": "load",
      "description": "Load a Griz database from a file.",
      "parameters": {
        "type": "object",
        "properties": {
          "file": {
            "type": "string",
            "description": "Path to database file"
          }
        },
        "required": ["file"]
      }
    }
  ]
}
```

### Hypothesis D: Standard OpenAI format

```
System: You are a helpful assistant with access to the following tools:

[{"name": "load", "description": "Load a database", "parameters": {...}}]

When you need to call a tool, respond with:
{"tool": "load", "args": {"file": "d3samp6"}}
```

## Contacts & References

**To investigate:**
1. FunctionGemma GitHub: https://github.com/google/functiongemma — look for README, examples, or issues about tool calling
2. Model card discussion threads: https://huggingface.co/google/functiongemma-270m-it/discussions
3. llama.cpp issues: https://github.com/ggml-org/llama.cpp/issues — search for "FunctionGemma"
4. llama-server documentation: https://github.com/ggml-org/llama.cpp/blob/master/examples/server/README.md

**Key files to have ready:**
- `python/mili-llm-bench/src/mili_llm_bench/providers/llamacpp.py` — The implementation
- `python/mili-llm-bench/src/mili_llm_bench/providers/functiongemma.py` — Similar provider (for reference)
- `data/posttraining/runs/v0-llamacpp-20260524_003947Z/rollouts.jsonl` — Raw baseline results

