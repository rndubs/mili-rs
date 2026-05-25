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

**Response-side fallback (option b, rev 10).** On `llama.cpp` builds
without a FunctionGemma response parser (the autoparser refactor in
PR #18675 / master `566059a` cannot infer FG's `<escape>` arg wrapping
and bare-key dict syntax — see m5-sft-pipeline.md rev 9 "Option (a)
status"), the server returns FG's `<start_function_call>…
<end_function_call>` markers as literal `message.content` and leaves
`tool_calls` empty. To unblock Stage 5 without waiting on upstream,
this provider runs a client-side fallback after each chat-completions
response: probe `/props` once, cache
`chat_template_caps.supports_tool_calls`, and when that's false (or
unknown) AND `tool_calls` is empty AND content contains the FG
envelope, extract `(name, args_dict)` pairs via the shared
``parse_fg_envelopes`` helper (``providers._fg_envelope``) and
synthesize the canonical normalized tool_calls. The same helper is
the response parser inside ``TransformersProvider`` for SFT
checkpoint eval — single regex set, no drift. Caller behavior is
identical to a server that does parse FG natively.

Earlier versions of this provider hand-rolled the prompt via a bespoke
``_build_functiongemma_prompt`` and hit ``/completion``. That path
diverged from the HF template — system prompt content was discarded,
parameter schemas were flattened, tool-call argument JSON was
re-serialized with Python-style ``True``/``False``/``None`` — so any
SFT trained against ``apply_chat_template`` would not see what the
inference path emits. Stage 6.5 preflight #2 (2026-05-24) measured the
divergence and resolved Path A: delete the bespoke renderer, let
``--jinja`` do the work. The rev-10 fallback re-adds the response-side
parser only; the prompt path stays on `/v1/chat/completions` +
`--jinja`.

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
from ._fg_envelope import parse_fg_envelopes
from ..tool_format import w1_to_openai_tool

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
    # /props caps cache. _caps_probed flips True after the first probe
    # attempt (success or failure); _caps_supports_tool_calls is the
    # cached value — True/False if /props returned a bool, None if the
    # probe failed or the field was missing (treated as "unknown",
    # which still allows the defensive envelope-detection branch to
    # activate the fallback).
    _caps_probed: bool = field(default=False, init=False, repr=False)
    _caps_supports_tool_calls: bool | None = field(
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
        content = message.get("content") or ""
        if tool_calls:
            self._last_tool_call_parsing_approach = "a1-chat-completions"
            return ProviderOutput(
                tool_calls=tool_calls,
                tokens_used=tokens_used,
                raw=result,
            )

        # Fallback: llama.cpp b9307 / master 549b9d843 has no FunctionGemma
        # response parser (autoparser refactor PR #18675 cannot infer
        # FG's <escape>...<escape> args). When /props caps say
        # supports_tool_calls=false (or the probe failed) AND the model
        # emitted an FG envelope in content, parse it client-side here
        # so the rest of the bench harness sees structured tool_calls.
        if self._should_run_fg_fallback(content):
            parsed = parse_fg_envelopes(content)
            if parsed:
                self._last_tool_call_parsing_approach = "b-fg-content-fallback"
                return ProviderOutput(
                    tool_calls=parsed,
                    tokens_used=tokens_used,
                    raw=result,
                )

        # No tool_calls field (or empty) — treat the message content as final text.
        self._last_tool_call_parsing_approach = "final-text"
        return ProviderOutput(
            final_text=str(content).strip(),
            tokens_used=tokens_used,
            raw=result,
        )

    def _should_run_fg_fallback(self, content: str) -> bool:
        """Decide whether to run the client-side FG content→tool_calls
        fallback.

        Gate:
        - If `/props` caps say `supports_tool_calls == True`, trust the
          server: the fallback is *not* invoked even if an FG envelope
          shows up in content (caller bug if that happens).
        - Otherwise (`supports_tool_calls == False`, or the probe failed
          / the field is missing — treated as "unknown"), activate the
          fallback iff content contains an FG envelope. The defensive
          envelope-detection guards against a future build that lies on
          `/props` but still emits FG markers.
        """
        if not content or "<start_function_call>" not in content:
            return False
        caps = self._fetch_caps_supports_tool_calls()
        if caps is True:
            return False
        return True

    def _fetch_caps_supports_tool_calls(self) -> bool | None:
        """GET `/props` once, cache `chat_template_caps.supports_tool_calls`.

        Returns True/False per `/props`, or None if the probe failed or
        the field was missing. Cached on the provider instance — one
        probe per provider, not per request.
        """
        if self._caps_probed:
            return self._caps_supports_tool_calls
        # Flip _caps_probed *before* the request: if the request itself
        # raises, we don't want to retry on every subsequent generate.
        self._caps_probed = True
        requests = _import_requests()
        try:
            resp = requests.get(f"{self.server_url}/props", timeout=5)
            resp.raise_for_status()
            body = resp.json()
        except Exception:
            self._caps_supports_tool_calls = None
            return None
        caps = (body or {}).get("chat_template_caps") or {}
        val = caps.get("supports_tool_calls")
        self._caps_supports_tool_calls = val if isinstance(val, bool) else None
        return self._caps_supports_tool_calls

    def _convert_to_openai_tool(self, tool: dict[str, Any]) -> dict[str, Any]:
        """Delegate to the shared ``tool_format`` helper so train- and
        inference-time can't drift on the W1 → FG/OpenAI projection."""
        return w1_to_openai_tool(tool)

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
