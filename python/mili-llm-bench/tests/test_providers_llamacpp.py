"""Tests for LlamaCppProvider — always-on (no binary, no network).

Group 1: Lazy-import gate stays green (import CLI doesn't load heavy deps).
Group 2: Provider import doesn't require the binary.
Group 3: build_factories("llamacpp", ...) returns a valid bundle.
Group 4: Response-parser unit tests.
Group 5: CLI --help mentions llamacpp.
"""

from __future__ import annotations

import sys
from pathlib import Path
from unittest.mock import MagicMock, patch

import pytest

from mili_llm_bench.cli import SUPPORTED_PROVIDERS, build_factories
from mili_llm_bench.driver import EvalConfig


class TestLazyImportGate:
    """Lazy-import gate stays green — import cli must not load heavy deps."""

    def test_cli_import_does_not_import_requests(self) -> None:
        """Importing mili_llm_bench.cli must not import requests."""
        # Remove requests from sys.modules if it's there.
        requests_imported_before = "requests" in sys.modules
        try:
            if "requests" in sys.modules:
                del sys.modules["requests"]
            # Now import cli and check that requests is still not imported.
            import mili_llm_bench.cli  # noqa: F401
            assert "requests" not in sys.modules
        finally:
            # Restore state if needed (requests may be imported by other tests).
            pass


class TestProviderImportSafety:
    """Provider import succeeds on a machine without llama-server."""

    def test_provider_import_without_binary(self) -> None:
        """Importing LlamaCppProvider succeeds without llama-server on $PATH."""
        from mili_llm_bench.providers.llamacpp import LlamaCppProvider

        # Should not raise.
        provider = LlamaCppProvider()
        assert provider is not None

    def test_generate_raises_without_binary(self) -> None:
        """generate() raises RuntimeError if llama-server is not on $PATH."""
        from mili_llm_bench.providers.llamacpp import LlamaCppProvider

        provider = LlamaCppProvider()
        # Mock shutil.which to return None (binary not found).
        with patch("mili_llm_bench.providers.llamacpp.shutil.which", return_value=None):
            with pytest.raises(RuntimeError, match="llama-server binary not found"):
                provider.generate(
                    messages=[{"role": "user", "content": "hello"}],
                    tools=[],
                    temperature=0.0,
                    max_new_tokens=256,
                    seed=0,
                )


class TestProviderFactoryBuilder:
    """build_factories("llamacpp", ...) returns a valid bundle."""

    def test_build_factories_llamacpp_valid_bundle(self) -> None:
        """build_factories returns a FactoryBundle with llamacpp provider."""
        config = EvalConfig()
        bundle = build_factories(
            "llamacpp",
            config=config,
        )
        assert bundle.provider_name == "llamacpp"
        # model_id should be the default GGUF pinned.
        assert "functiongemma" in bundle.model_id.lower()
        assert bundle.provider_factory is not None
        assert bundle.dispatcher_factory is not None

    def test_build_factories_llamacpp_creates_provider(self) -> None:
        """build_factories creates a callable provider_factory."""
        from mili_llm_bench.providers.llamacpp import LlamaCppProvider

        config = EvalConfig()
        bundle = build_factories("llamacpp", config=config)
        # The factory should be callable and return a LlamaCppProvider.
        provider = bundle.provider_factory(None)
        assert isinstance(provider, LlamaCppProvider)


class TestResponseParsing:
    """Response-parser unit tests for OpenAI format normalization."""

    def test_normalize_openai_tool_calls(self) -> None:
        """LlamaCppProvider normalizes OpenAI-format tool_calls."""
        from mili_llm_bench.providers.llamacpp import LlamaCppProvider

        provider = LlamaCppProvider()
        openai_calls = [
            {
                "type": "function",
                "function": {
                    "name": "load",
                    "arguments": '{"file": "d3samp6"}',
                },
            }
        ]
        result = provider._normalize_openai_tool_calls(openai_calls)
        assert result is not None
        assert len(result) == 1
        assert result[0]["name"] == "load"
        assert result[0]["arguments"] == {"file": "d3samp6"}

    def test_normalize_empty_tool_calls(self) -> None:
        """_normalize_openai_tool_calls handles empty lists."""
        from mili_llm_bench.providers.llamacpp import LlamaCppProvider

        provider = LlamaCppProvider()
        result = provider._normalize_openai_tool_calls([])
        assert result == []

    def test_normalize_invalid_tool_calls_returns_none(self) -> None:
        """_normalize_openai_tool_calls returns None for non-list input."""
        from mili_llm_bench.providers.llamacpp import LlamaCppProvider

        provider = LlamaCppProvider()
        result = provider._normalize_openai_tool_calls("not a list")
        assert result is None

    def test_normalize_tool_call_with_dict_arguments(self) -> None:
        """_normalize_openai_tool_calls handles both string and dict arguments."""
        from mili_llm_bench.providers.llamacpp import LlamaCppProvider

        provider = LlamaCppProvider()
        # Test with dict arguments (not stringified)
        openai_calls = [
            {
                "type": "function",
                "function": {
                    "name": "load",
                    "arguments": {"file": "d3samp6"},
                },
            }
        ]
        result = provider._normalize_openai_tool_calls(openai_calls)
        assert result is not None
        assert len(result) == 1
        assert result[0]["name"] == "load"
        assert result[0]["arguments"] == {"file": "d3samp6"}


class TestChatCompletionsPath:
    """Path A pin — generate() POSTs to /v1/chat/completions, not /completion.

    Stage 6.5 preflight #2 (2026-05-24) resolved the train-vs-inference
    chat-template divergence by switching to llama-server's --jinja path:
    the FunctionGemma jinja baked into the GGUF is the single source of
    truth for both training (HF apply_chat_template) and inference (this
    provider). The bespoke ``_build_functiongemma_prompt`` + ``/completion``
    raw-text path was deleted; this test pins that decision so the
    rewrite is not silently reverted.
    """

    def _stub_response(self, monkeypatch, payload: dict) -> list[dict]:
        """Patch requests.post + binary/health checks so generate() runs
        synchronously without llama-server. Returns the call-args list
        the mocked post records, for the test to inspect."""
        from mili_llm_bench.providers import llamacpp as mod

        captured: list[dict] = []

        class _StubResp:
            def __init__(self, body: dict) -> None:
                self._body = body

            def raise_for_status(self) -> None:
                return None

            def json(self) -> dict:
                return self._body

        class _StubRequests:
            class RequestException(Exception):
                pass

            @staticmethod
            def post(url: str, json: dict, timeout: int) -> "_StubResp":
                captured.append({"url": url, "json": json, "timeout": timeout})
                return _StubResp(payload)

        monkeypatch.setattr(mod, "_import_requests", lambda: _StubRequests)
        monkeypatch.setattr(mod.shutil, "which", lambda _: "/usr/bin/llama-server")
        # Force _health_check to claim "up" without touching the network.
        monkeypatch.setattr(
            mod.LlamaCppProvider,
            "_health_check",
            lambda self, max_retries=30, retry_delay=1.0: True,
        )
        return captured

    def test_generate_posts_to_chat_completions_endpoint(self, monkeypatch) -> None:
        """generate() hits /v1/chat/completions (not /completion)."""
        from mili_llm_bench.providers.llamacpp import LlamaCppProvider

        captured = self._stub_response(
            monkeypatch,
            {"choices": [{"message": {"content": "done", "tool_calls": None}}]},
        )
        provider = LlamaCppProvider()
        provider.generate(
            messages=[{"role": "user", "content": "hello"}],
            tools=[],
            temperature=0.0,
            max_new_tokens=64,
            seed=0,
        )
        assert len(captured) == 1
        url = captured[0]["url"]
        assert url.endswith("/v1/chat/completions"), url

    def test_generate_sends_tools_in_openai_shape(self, monkeypatch) -> None:
        """Tools are converted from Anthropic shape (name+input_schema)
        to OpenAI shape ({type, function: {name, parameters}}) before
        being sent to llama-server."""
        from mili_llm_bench.providers.llamacpp import LlamaCppProvider

        captured = self._stub_response(
            monkeypatch,
            {"choices": [{"message": {"content": "", "tool_calls": []}}]},
        )
        provider = LlamaCppProvider()
        provider.generate(
            messages=[{"role": "user", "content": "load cylinder"}],
            tools=[
                {
                    "name": "load",
                    "description": "Open a database.",
                    "input_schema": {
                        "type": "object",
                        "properties": {"root": {"type": "string"}},
                    },
                }
            ],
            temperature=0.0,
            max_new_tokens=64,
            seed=0,
        )
        sent = captured[0]["json"]
        assert "tools" in sent
        assert sent["tools"][0]["type"] == "function"
        assert sent["tools"][0]["function"]["name"] == "load"
        assert sent["tools"][0]["function"]["parameters"]["type"] == "object"

    def test_generate_parses_openai_tool_calls(self, monkeypatch) -> None:
        """A tool_calls payload from llama-server (server applied
        --jinja and parsed the FG <start_function_call> blocks) gets
        normalized into the canonical [{name, arguments}] form."""
        from mili_llm_bench.providers.llamacpp import LlamaCppProvider

        self._stub_response(
            monkeypatch,
            {
                "choices": [
                    {
                        "message": {
                            "content": None,
                            "tool_calls": [
                                {
                                    "type": "function",
                                    "function": {
                                        "name": "load",
                                        "arguments": '{"root": "cylinder"}',
                                    },
                                }
                            ],
                        }
                    }
                ]
            },
        )
        provider = LlamaCppProvider()
        out = provider.generate(
            messages=[{"role": "user", "content": "load cylinder"}],
            tools=[],
            temperature=0.0,
            max_new_tokens=64,
            seed=0,
        )
        assert out.tool_calls == [{"name": "load", "arguments": {"root": "cylinder"}}]

    def test_bespoke_renderer_is_gone(self) -> None:
        """The bespoke FG prompt builder and tool-text parser are
        removed. If a future revert re-introduces them, this test
        forces the discussion before the dual-template world returns."""
        from mili_llm_bench.providers.llamacpp import LlamaCppProvider

        for forbidden in (
            "_build_functiongemma_prompt",
            "_format_tool_call_for_history",
            "_format_tool_declaration",
            "_parse_functiongemma_tool_calls",
            "_parse_function_arguments",
        ):
            assert not hasattr(LlamaCppProvider, forbidden), (
                f"{forbidden} was removed in Stage 6.5 preflight #2 (Path A); "
                f"if it returned, re-read sft-preflight-gpu.md §2 before re-adding."
            )


class TestCliHelp:
    """CLI --help mentions llamacpp."""

    def test_cli_help_mentions_llamacpp(self, capsys) -> None:
        """mili-llm-bench --help lists llamacpp among supported providers."""
        from mili_llm_bench.cli import main

        with pytest.raises(SystemExit):
            main(["run", "--help"])
        captured = capsys.readouterr()
        help_text = captured.out
        assert "llamacpp" in help_text

    def test_supported_providers_contains_llamacpp(self) -> None:
        """SUPPORTED_PROVIDERS includes llamacpp."""
        assert "llamacpp" in SUPPORTED_PROVIDERS
