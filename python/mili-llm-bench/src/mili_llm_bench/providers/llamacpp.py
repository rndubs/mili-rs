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
        # Stop sequences: <start_function_response> (FunctionGemma handoff token per Google docs)
        # and <end_of_turn> (natural boundary if model doesn't call a tool).
        completion_payload = {
            "prompt": prompt,
            "temperature": temperature,
            "n_predict": max_new_tokens,
            "seed": seed,
            "stop": ["<start_function_response>", "<end_of_turn>"],
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

        Format per the model card and Google's FunctionGemma documentation.
        Includes full conversation history (assistant tool calls and tool responses)
        so the model can complete the official loop: tool call → tool response → final answer.

        Key: the developer phrase "You are a model that can do function calling with
        the following functions" activates the model's function calling logic.
        """
        prompt_parts = []
        dev_sent_once = False

        # Build developer turn with tool declarations (only once, at the start)
        prompt_parts.append("<start_of_turn>developer\n")

        # Use the exact trigger phrase from Google's documentation
        dev_content = (
            "You are a model that can do function calling with the following functions"
        )
        prompt_parts.append(dev_content)

        # Add tool declarations if present
        if tools:
            prompt_parts.append("\n\n")
            for tool in tools:
                prompt_parts.append(self._format_tool_declaration(tool))

        prompt_parts.append("\n<end_of_turn>\n")
        dev_sent_once = True

        # Process messages in order, including assistant/tool history for multi-turn
        for msg in messages:
            role = msg.get("role")
            content = msg.get("content", "")

            if role in ("developer", "system"):
                # Skip developer/system after first one (already added above)
                if not dev_sent_once:
                    prompt_parts.append("<start_of_turn>developer\n")
                    prompt_parts.append(content)
                    prompt_parts.append("\n<end_of_turn>\n")
                    dev_sent_once = True
            elif role == "user":
                prompt_parts.append(f"<start_of_turn>user\n{content}\n<end_of_turn>\n")
            elif role == "assistant":
                # Include assistant tool calls and final text in the conversation
                prompt_parts.append("<start_of_turn>model\n")

                # If there are tool_calls, emit them in FunctionGemma format
                tool_calls = msg.get("tool_calls") or []
                if tool_calls:
                    for tc in tool_calls:
                        func = tc.get("function", {})
                        name = func.get("name", "")
                        args = func.get("arguments", {})
                        if isinstance(args, str):
                            import json
                            try:
                                args = json.loads(args)
                            except json.JSONDecodeError:
                                args = {}

                        # Format: <start_function_call>call:name{key:value,...}<end_function_call>
                        prompt_parts.append(
                            self._format_tool_call_for_history(name, args)
                        )

                # Add final text if present
                if content:
                    prompt_parts.append(content)

                prompt_parts.append("\n<end_of_turn>\n")
            elif role == "tool":
                # Tool response: append in FunctionGemma format before next model turn
                tool_name = msg.get("tool_use_id", "").split(":")[-1] or "unknown"
                prompt_parts.append(
                    f"<start_function_response>response:{tool_name}{{{content}}}<end_function_response>\n"
                )

        # Prime the model to generate the next response
        prompt_parts.append("<start_of_turn>model\n")

        return "".join(prompt_parts)

    def _format_tool_call_for_history(self, name: str, args: dict[str, Any]) -> str:
        """Format a tool call for inclusion in conversation history.

        Format: <start_function_call>call:name{key:<escape>value<escape>,...}<end_function_call>
        """
        parts = [f"<start_function_call>call:{name}{{"]
        arg_parts = []
        for key, value in args.items():
            arg_parts.append(f"{key}:<escape>{value}<escape>")
        parts.append(",".join(arg_parts))
        parts.append("}<end_function_call>")
        return "".join(parts)

    def _format_tool_declaration(self, tool: dict[str, Any]) -> str:
        """Format a single tool for the <start_function_declaration> block.

        FunctionGemma expects the format:
        <start_function_declaration>declaration:toolname{
        description:<escape>desc<escape>,
        parameters:{param1:<escape>type1<escape>,param2:<escape>type2<escape>}
        }<end_function_declaration>
        """
        name = tool.get('name', '')
        desc = tool.get("description", "")

        parts = [f"<start_function_declaration>declaration:{name}{{\n"]
        parts.append(f"description:<escape>{desc}<escape>")

        # Format parameters as simple key:type pairs
        input_schema = tool.get("input_schema", {})
        properties = input_schema.get("properties", {})

        if properties:
            parts.append(",\nparameters:{")
            param_parts = []
            for param_name, param_schema in properties.items():
                param_type = param_schema.get("type", "string")
                param_parts.append(f"{param_name}:<escape>{param_type}<escape>")
            parts.append(",".join(param_parts))
            parts.append("}")

        parts.append("\n}<end_function_declaration>\n")
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

        Tolerant parser per research report: accepts both fully closed and partially
        open tool calls. Format: <start_function_call>call:name{arg1:value1,...}<end_function_call>
        Values are wrapped in <escape>...<escape> for special chars.

        Tolerates malformed tool names (e.g., material.disable → material) by extracting
        the base tool name (first component before any dot).
        """
        import re

        # First try JSON format (fallback).
        # Only return JSON results if they're non-empty (empty list means malformed JSON).
        json_result = _parse_json_tool_calls(text)
        if json_result:  # Non-empty list means valid JSON was found
            return json_result

        calls = []

        # Try fully closed calls first: <start_function_call>call:name{...}<end_function_call>
        # Accept tool names with dots, dashes, underscores (e.g., material.disable, show-primal)
        closed_pattern = (
            r"<start_function_call>call:([\w.-]+)\{(.*?)\}<end_function_call>"
        )
        closed_matches = re.findall(closed_pattern, text, re.DOTALL)
        for name, args_str in closed_matches:
            # Extract base tool name (before any dot): material.disable → material
            base_name = name.split('.')[0]
            args = self._parse_function_arguments(args_str)
            calls.append({"name": base_name, "arguments": args})

        # If no fully closed calls found, try bare format: call:name{...}
        if not calls:
            bare_pattern = r"call:([\w.-]+)\{(.*?)\}(?:\s|$|<)"
            bare_matches = re.findall(bare_pattern, text, re.DOTALL)
            for name, args_str in bare_matches:
                # Extract base tool name (before any dot)
                base_name = name.split('.')[0]
                args = self._parse_function_arguments(args_str)
                calls.append({"name": base_name, "arguments": args})

        return calls if calls else None

    def _parse_function_arguments(self, args_str: str) -> dict[str, Any]:
        """Parse function arguments from escape-wrapped format.

        Handles: key:<escape>value<escape> or key:value (unescaped scalar)
        Pattern based on Google's FunctionGemma example parser.
        """
        import re

        args = {}
        if not args_str:
            return args

        # Parse escape-wrapped and scalar values: key:<escape>val<escape> or key:val
        # Matches: (word):(escaped content or scalar)
        pattern = r"(\w+):<escape>(.*?)<escape>|(\w+):([^,}<\s]+)"
        matches = re.findall(pattern, args_str)

        for match in matches:
            key, escaped_val, scalar_key, scalar_val = match
            if key and escaped_val is not None:
                args[key] = escaped_val
            elif scalar_key and scalar_val:
                args[scalar_key] = scalar_val

        return args

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
