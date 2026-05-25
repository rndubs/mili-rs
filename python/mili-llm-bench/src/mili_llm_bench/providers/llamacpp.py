"""PR-6 — ``LlamaCppProvider`` (llama.cpp via llama-server HTTP).

Uses llama.cpp's llama-server to drive FunctionGemma-270M-it locally.
Launches llama-server once per provider instance (lazy, on first
generate), keeps it alive across scenarios, and hits its
``/v1/chat/completions`` endpoint with the canonical messages + tools
list.

**llama-server must be started with ``--jinja``** so the server applies
the FunctionGemma chat template baked into the GGUF and exposes
structured ``tool_calls`` in the OpenAI-shaped response. This is the
single template path: the same FG jinja governs training (HF
``apply_chat_template`` against ``google/functiongemma-270m-it``) and
inference (this provider via ``--jinja``). See
``planning/mili-viz/mili-agent/sft-preflight-gpu.md`` §2 for the
parity story.

Earlier versions of this provider hand-rolled the prompt via a bespoke
``_build_functiongemma_prompt`` and hit ``/completion``. That path
diverged from the HF template — system prompt content was discarded,
parameter schemas were flattened, tool-call argument JSON was
re-serialized with Python-style ``True``/``False``/``None`` — so any
SFT trained against ``apply_chat_template`` would not see what the
inference path emits. Stage 6.5 preflight #2 (2026-05-24) measured the
divergence and resolved Path A: delete the bespoke renderer, let
``--jinja`` do the work.

Deterministic: ``--temp 0`` (greedy) + ``seed`` (from the LlmProvider
call). The v0 baseline is taken against the GGUF quantization BF16
(full-precision GGUF, no quantization noise).

The binary check + first launch happens inside ``generate`` (lazy
startup); the module imports without requiring llama-cli/llama-server
on ``$PATH``.
"""

from __future__ import annotations

import json
import shutil
import time
from dataclasses import dataclass, field
from typing import Any

from .base import ProviderOutput

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
        """Generate a response via llama-server's ``/v1/chat/completions``
        endpoint. The server must be started with ``--jinja`` so the
        FunctionGemma chat template (baked into the GGUF) is applied
        server-side.

        Posts the canonical messages + OpenAI-shape tools list; reads
        structured ``tool_calls`` from ``choices[0].message`` via
        ``_normalize_openai_tool_calls``. No bespoke prompt building,
        no text-mode parsing — the server-applied template is the
        single source of truth for both training (HF
        ``apply_chat_template``) and inference (this provider).
        """
        requests = _import_requests()
        self._check_binary()

        # Verify the server is up and ready.
        if not self._health_check():
            raise RuntimeError(
                f"llama-server at {self.server_url} is not responding. "
                f"Start it manually with --jinja:\n\n"
                f"  llama-server -hf {DEFAULT_GGUF_REPO}:{DEFAULT_GGUF_QUANT} --jinja\n"
            )

        payload: dict[str, Any] = {
            "model": self.model_id,
            "messages": messages,
            "temperature": temperature,
            "max_tokens": max_new_tokens,
            "seed": seed,
        }
        if tools:
            payload["tools"] = [self._convert_to_openai_tool(t) for t in tools]

        try:
            resp = requests.post(
                f"{self.server_url}/v1/chat/completions",
                json=payload,
                timeout=180,
            )
            resp.raise_for_status()
        except requests.RequestException as exc:
            raise RuntimeError(
                f"llama-server chat completions request failed: {exc}\n"
                f"Ensure the server is running with --jinja:\n\n"
                f"  llama-server -hf {DEFAULT_GGUF_REPO}:{DEFAULT_GGUF_QUANT} --jinja\n"
            ) from exc

        result = resp.json()

        usage = result.get("usage") or {}
        tokens_used = int(
            usage.get("prompt_tokens", 0) + usage.get("completion_tokens", 0)
        )

        choices = result.get("choices") or []
        message = (choices[0] or {}).get("message", {}) if choices else {}
        raw_tool_calls = message.get("tool_calls")
        tool_calls = self._normalize_openai_tool_calls(raw_tool_calls)
        if tool_calls:
            self._last_tool_call_parsing_approach = "a1-chat-completions"
            return ProviderOutput(
                tool_calls=tool_calls,
                tokens_used=tokens_used,
                raw=result,
            )

        # No tool_calls field (or empty) — treat the message content as final text.
        self._last_tool_call_parsing_approach = "final-text"
        content = message.get("content") or ""
        return ProviderOutput(
            final_text=str(content).strip(),
            tokens_used=tokens_used,
            raw=result,
        )

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
