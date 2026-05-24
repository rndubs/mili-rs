"""PR-6 — ``LlamaCppProvider`` (llama.cpp via llama-server HTTP).

Uses llama.cpp's llama-server to drive FunctionGemma-270M-it locally.
Launches llama-server once per provider instance (lazy, on first
generate), keeps it alive across scenarios, and hits its
/v1/chat/completions endpoint with the canonical messages + tools list.

Approach (a1): tool-calling endpoint with --jinja. If llama-server's
tool-use parsing aligns with FunctionGemma's chat template (the server
applies the model's baked-in jinja template via --jinja and parses
<start_function_call> blocks into OpenAI-shaped tool_calls), the
provider maps the response. If the endpoint returns plain text instead
of structured tool_calls, falls back to (a2) — raw completion parsing
with the existing _parse_tool_call_block helper.

Deterministic: --temp 0 (greedy) + --seed (from the LlmProvider call).
The v0 baseline is taken against the GGUF quantization BF16
(full-precision GGUF, no quantization noise).

The binary check + first launch happens inside generate (lazy startup);
the module imports without requiring llama-cli/llama-server on $PATH.
"""

from __future__ import annotations

import json
import shutil
import subprocess
from dataclasses import dataclass, field
from pathlib import Path
from typing import Any
import time
import signal

from .base import ProviderOutput
from .functiongemma import _parse_tool_call_block as _parse_json_tool_calls

DEFAULT_MODEL_ID = "ggml-org/functiongemma-270m-it-GGUF:BF16"
DEFAULT_GGUF_REPO = "ggml-org/functiongemma-270m-it-GGUF"
DEFAULT_GGUF_QUANT = "BF16"
DEFAULT_SERVER_URL = "http://localhost:8080"


def _import_requests() -> Any:
    """Lazy-load requests; raise a friendly ImportError if the llamacpp
    extra is not installed."""
    try:
        import requests  # type: ignore[import-not-found]
    except ImportError as exc:
        raise ImportError(
            "LlamaCppProvider requires the 'llamacpp' optional dependency. "
            "Install with `pip install mili-llm-bench[llamacpp]`."
        ) from exc
    return requests


@dataclass
class LlamaCppProvider:
    """Stock FunctionGemma-270M-it driven through llama.cpp's llama-server.

    Lazy-loads the server on the first generate call; subsequent calls
    reuse the running server. The binary check happens on first generate
    and raises RuntimeError if llama-server is not on $PATH.

    The v0 baseline run sets temperature=0 (greedy). The server port is
    pinned to 8080 (the default llama-server port); callers should start
    the server manually before use:

        llama-server -hf ggml-org/functiongemma-270m-it-GGUF:BF16

    Or the provider will raise RuntimeError with instructions.

    Approach: Try (a1) first — hit /v1/chat/completions with the model's
    baked-in jinja template, parse tool_calls from OpenAI-shaped response.
    If the endpoint returns plain text instead of structured tool_calls,
    fall back to (a2) — raw completion parsing.
    """

    server_url: str = DEFAULT_SERVER_URL
    model_id: str = DEFAULT_MODEL_ID
    _server_process: Any = field(default=None, init=False, repr=False)
    _requests: Any = field(default=None, init=False, repr=False)
    _last_tool_call_parsing_approach: str | None = field(
        default=None, init=False, repr=False
    )

    def _check_binary(self) -> None:
        """Check that llama-server is on $PATH; raise RuntimeError if not."""
        if shutil.which("llama-server") is None:
            raise RuntimeError(
                "llama-server binary not found on $PATH. "
                "Install llama.cpp from https://github.com/ggml-org/llama.cpp "
                "and ensure llama-server is in your PATH, or start the server "
                "manually:\n\n"
                f"  llama-server -hf {DEFAULT_GGUF_REPO}:{DEFAULT_GGUF_QUANT}\n"
            )

    def _health_check(self, max_retries: int = 30, retry_delay: float = 1.0) -> bool:
        """Poll the server's health endpoint until it's ready."""
        requests = _import_requests()
        for attempt in range(max_retries):
            try:
                resp = requests.get(f"{self.server_url}/health", timeout=2)
                if resp.status_code == 200:
                    return True
            except (requests.RequestException, Exception):
                pass
            if attempt < max_retries - 1:
                time.sleep(retry_delay)
        return False

    def generate(
        self,
        messages: list[dict[str, Any]],
        tools: list[dict[str, Any]],
        *,
        temperature: float,
        max_new_tokens: int,
        seed: int,
    ) -> ProviderOutput:
        """Generate a response via llama-server's /completion endpoint.

        Uses the raw completion path (a2): manually construct the FunctionGemma
        prompt (since /apply-template doesn't support tools in llama-server),
        send to /completion, parse raw text with _parse_tool_call_block.
        """
        requests = _import_requests()
        self._check_binary()

        # Verify the server is up and ready.
        if not self._health_check():
            raise RuntimeError(
                f"llama-server at {self.server_url} is not responding. "
                f"Start it manually:\n\n"
                f"  llama-server -hf {DEFAULT_GGUF_REPO}:{DEFAULT_GGUF_QUANT}\n"
            )

        # Manually construct FunctionGemma prompt (llama-server /apply-template
        # doesn't support tools, so we build it ourselves per the model card).
        prompt = self._build_functiongemma_prompt(messages, tools)

        # Hit /completion with the formatted prompt.
        completion_payload = {
            "prompt": prompt,
            "temperature": temperature,
            "n_predict": max_new_tokens,
            "seed": seed,
        }

        try:
            resp = requests.post(
                f"{self.server_url}/completion",
                json=completion_payload,
                timeout=180,
            )
            resp.raise_for_status()
        except requests.RequestException as exc:
            raise RuntimeError(
                f"llama-server completion request failed: {exc}\n"
                f"Ensure the server is running:\n\n"
                f"  llama-server -hf {DEFAULT_GGUF_REPO}:{DEFAULT_GGUF_QUANT}\n"
            ) from exc

        result = resp.json()

        # Extract token counts.
        tokens_used = 0
        if "tokens_evaluated" in result:
            tokens_used += result["tokens_evaluated"]
        if "tokens_predicted" in result:
            tokens_used += result["tokens_predicted"]

        # Raw completion text parsing (a2).
        raw_text = result.get("content", "")

        # Parse tool calls from the text.
        # Try FunctionGemma text format first (call:name{args}), then JSON format.
        tool_calls = self._parse_functiongemma_tool_calls(raw_text)
        if tool_calls is not None:
            self._last_tool_call_parsing_approach = "a2-raw-completion"
            return ProviderOutput(
                tool_calls=tool_calls,
                tokens_used=tokens_used,
                raw=raw_text,
            )

        # No tool calls found; treat as final text.
        self._last_tool_call_parsing_approach = "final-text"
        return ProviderOutput(
            final_text=raw_text.strip(),
            tokens_used=tokens_used,
            raw=raw_text,
        )

    def _build_functiongemma_prompt(
        self, messages: list[dict[str, Any]], tools: list[dict[str, Any]]
    ) -> str:
        """Build FunctionGemma-format prompt manually (llama-server /apply-template
        doesn't support tools).

        Format per the model card:
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
        """
        prompt_parts = []

        # Extract developer/system message and user messages separately
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

    def _format_tool_declaration(self, tool: dict[str, Any]) -> str:
        """Format a single tool for the <start_function_declaration> block."""
        parts = ["<start_function_declaration>\n"]
        parts.append(f"declaration:{tool.get('name', '')}\n")

        # Description
        desc = tool.get("description", "")
        parts.append(f"{{description:<escape>{desc}<escape>\n")

        # Parameters
        input_schema = tool.get("input_schema", {})
        if input_schema:
            properties = input_schema.get("properties", {})
            required = input_schema.get("required", [])

            if properties:
                parts.append(",parameters:{\n")
                parts.append("properties:{ ")

                prop_parts = []
                for prop_name, prop_schema in properties.items():
                    prop_type = prop_schema.get("type", "string").upper()
                    prop_desc = prop_schema.get("description", "")
                    prop_parts.append(
                        f"{prop_name}:{{description:<escape>{prop_desc}<escape>,type:<escape>{prop_type}<escape>}}"
                    )
                parts.append(", ".join(prop_parts))
                parts.append(" }")

                if required:
                    parts.append(",\n")
                    parts.append("required:[")
                    parts.append(
                        ",".join(f"<escape>{r}<escape>" for r in required)
                    )
                    parts.append("]\n")

                parts.append("}\n")

        parts.append("}\n")
        parts.append("<end_function_declaration>")
        return "".join(parts)

    def _convert_to_openai_tool(self, tool: dict[str, Any]) -> dict[str, Any]:
        """Convert our tool format to OpenAI format for llama-server.

        Our format: {"name", "description", "input_schema", "output_schema"}
        OpenAI format: {"type": "function", "function": {"name", "description", "parameters"}}
        """
        return {
            "type": "function",
            "function": {
                "name": tool.get("name", ""),
                "description": tool.get("description", ""),
                "parameters": tool.get("input_schema", {}),
            },
        }

    def _parse_functiongemma_tool_calls(
        self, text: str
    ) -> list[dict[str, Any]] | None:
        """Parse FunctionGemma text-based tool call format.

        Format: <start_function_call>call:name{arg1:value1,arg2:value2}<end_function_call>
        Values are wrapped in <escape>...<escape> for special chars.
        """
        import re

        # First try JSON format (fallback).
        # Only return JSON results if they're non-empty (empty list means malformed JSON).
        json_result = _parse_json_tool_calls(text)
        if json_result:  # Non-empty list means valid JSON was found
            return json_result

        # Parse FunctionGemma text format: call:name{key:value,key:value}
        call_pattern = r"<start_function_call>call:(\w+)\{([^}]*)\}<end_function_call>"
        matches = re.findall(call_pattern, text)

        if not matches:
            return None

        calls = []
        for name, args_str in matches:
            args = {}
            if args_str:
                # Parse key:value pairs, handling <escape>...</escape> wrapped values
                arg_pattern = r"(\w+):<escape>([^<]*)<escape>"
                arg_matches = re.findall(arg_pattern, args_str)
                for arg_name, arg_value in arg_matches:
                    args[arg_name] = arg_value

            calls.append({"name": name, "arguments": args})

        return calls if calls else None

    def _normalize_openai_tool_calls(
        self, tool_calls_raw: Any
    ) -> list[dict[str, Any]] | None:
        """Normalize OpenAI-format tool_calls to canonical form.

        Returns None if the input is not a list of tool_calls.
        Returns [] if the list is empty.
        Returns the canonical [{"name", "arguments": dict}, ...] list otherwise.
        """
        if not isinstance(tool_calls_raw, list):
            return None

        if not tool_calls_raw:
            return []

        out: list[dict[str, Any]] = []
        for call in tool_calls_raw:
            if not isinstance(call, dict):
                continue
            # OpenAI format: {"type": "function", "function": {"name": "...", "arguments": "..."}}
            func_info = call.get("function", {})
            if not isinstance(func_info, dict):
                continue
            name = func_info.get("name", "")
            args_str = func_info.get("arguments", "{}")
            try:
                arguments = json.loads(args_str) if isinstance(args_str, str) else args_str
            except json.JSONDecodeError:
                arguments = {}
            if not isinstance(arguments, dict):
                arguments = {}
            out.append({"name": str(name), "arguments": arguments})

        return out if out else []


__all__ = [
    "DEFAULT_MODEL_ID",
    "DEFAULT_GGUF_REPO",
    "DEFAULT_GGUF_QUANT",
    "DEFAULT_SERVER_URL",
    "LlamaCppProvider",
]
