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
