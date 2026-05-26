"""Tests for TransformersProvider — always-on (no torch, no weights).

Mirrors the structure of ``test_providers_llamacpp.py``:

  * Group 1: Lazy-import gate — `import cli` stays torch-free.
  * Group 2: Provider import doesn't require torch/transformers.
  * Group 3: ``build_factories("transformers", …)`` returns a valid
    bundle; missing ``--model-path`` raises.
  * Group 4: generate() round-trips through apply_chat_template +
    parse_fg_envelopes when torch/transformers are stubbed.
  * Group 5: CLI --help mentions transformers; SUPPORTED_PROVIDERS
    contains it; the deprecated functiongemma name does not.

The model-load smoke (real torch + real weights + a GPU) lives outside
the always-on path; we exercise it manually against a real checkpoint
in ``data/posttraining/checkpoints/v1/`` rather than gating CI on a
heavy fixture.
"""

from __future__ import annotations

import sys
from unittest.mock import MagicMock

import pytest

from mili_llm_bench.cli import SUPPORTED_PROVIDERS, build_factories
from mili_llm_bench.driver import EvalConfig


class TestLazyImportGate:
    """Importing the CLI must not drag torch / transformers into sys.modules."""

    def test_cli_import_does_not_import_torch(self) -> None:
        for mod in ("torch", "transformers"):
            sys.modules.pop(mod, None)
        sys.modules.pop("mili_llm_bench.cli", None)
        sys.modules.pop("mili_llm_bench.providers.transformers", None)
        import mili_llm_bench.cli  # noqa: F401
        for mod in ("torch", "transformers"):
            assert mod not in sys.modules, (
                f"{mod} loaded during `import mili_llm_bench.cli` — keep "
                "the transformers provider lazy-imported."
            )


class TestProviderImportSafety:
    """Provider import succeeds on a machine without torch."""

    def test_provider_import_without_torch(self) -> None:
        """Importing TransformersProvider does not require torch."""
        from mili_llm_bench.providers.transformers import TransformersProvider

        provider = TransformersProvider(model_path="/nonexistent")
        assert provider is not None


class TestProviderFactoryBuilder:
    """``build_factories("transformers", …)`` returns a valid bundle."""

    def test_build_factories_requires_model_path(self) -> None:
        config = EvalConfig()
        with pytest.raises(ValueError, match="--model-path"):
            build_factories("transformers", config=config)

    def test_build_factories_with_model_path_returns_bundle(self) -> None:
        config = EvalConfig()
        bundle = build_factories(
            "transformers",
            config=config,
            transformers_model_path="/p/vast1/whitmore/cadsat/mili-rs/data/posttraining/checkpoints/v1/checkpoint-21",
        )
        assert bundle.provider_name == "transformers"
        assert bundle.model_id.endswith("checkpoint-21")
        assert bundle.provider_factory is not None
        assert bundle.dispatcher_factory is not None

    def test_build_factories_creates_transformers_provider(self) -> None:
        from mili_llm_bench.providers.transformers import TransformersProvider

        config = EvalConfig()
        bundle = build_factories(
            "transformers",
            config=config,
            transformers_model_path="/tmp/fake",
        )
        provider = bundle.provider_factory(None)
        assert isinstance(provider, TransformersProvider)
        assert provider.model_path == "/tmp/fake"


class TestGeneratePath:
    """generate() round-trips: apply_chat_template → model.generate →
    decode → parse_fg_envelopes. Stubs torch+transformers so we don't
    actually load weights."""

    def _stub_runtime(
        self,
        monkeypatch,
        *,
        decoded_with_specials: str,
        decoded_without_specials: str | None = None,
    ) -> dict[str, MagicMock]:
        """Patch ``_import_runtime`` to hand back mock torch/transformers
        modules. The mock tokenizer's ``decode`` returns
        ``decoded_with_specials`` when called with ``skip_special_tokens=
        False`` and ``decoded_without_specials`` (or the same string
        with specials stripped) otherwise.

        Returns the captured mocks so tests can assert call args.
        """
        from mili_llm_bench.providers import transformers as mod

        mock_torch = MagicMock(name="torch")
        # Provide a no_grad context manager.
        mock_torch.no_grad.return_value.__enter__ = lambda self: None
        mock_torch.no_grad.return_value.__exit__ = (
            lambda self, exc_type, exc, tb: None
        )
        mock_torch.bfloat16 = "bfloat16-sentinel"

        # Tokenizer: apply_chat_template returns a rendered prompt str;
        # __call__ returns an inputs dict whose .to(device) yields a
        # dict-like that supports __getitem__("input_ids").shape[-1].
        mock_tokenizer = MagicMock(name="AutoTokenizer-instance")
        mock_tokenizer.apply_chat_template.return_value = "PROMPT"
        mock_tokenizer.eos_token_id = 1

        input_ids_t = MagicMock(name="input_ids")
        input_ids_t.shape = [1, 7]  # prompt_len = 7
        inputs_dict_on_device = {"input_ids": input_ids_t}
        inputs_dict = MagicMock(name="tokenized")
        inputs_dict.to.return_value = inputs_dict_on_device
        # Make ** unpacking work on the .to() result.
        inputs_dict_on_device_keys = list(inputs_dict_on_device.keys())

        # Stand-in dict-like that supports ** unpacking AND __getitem__:
        class _InputsOnDevice(dict):
            pass

        device_inputs = _InputsOnDevice(inputs_dict_on_device)
        inputs_dict.to.return_value = device_inputs

        mock_tokenizer.return_value = inputs_dict

        # Decode side effect — branch on skip_special_tokens.
        def _decode(_ids, *, skip_special_tokens=False):
            if skip_special_tokens:
                return (
                    decoded_without_specials
                    if decoded_without_specials is not None
                    else decoded_with_specials
                )
            return decoded_with_specials

        mock_tokenizer.decode.side_effect = _decode

        # Model: generate returns a tensor whose [0, prompt_len:] slice
        # has a ``.shape[-1]`` and is passed to decode (whose return
        # value we control).
        mock_model = MagicMock(name="model")
        mock_model.eval.return_value = None
        mock_model.device = "cuda:0"
        generated = MagicMock(name="output_ids")
        completion = MagicMock(name="completion_ids")
        completion.shape = [9]  # 9 new tokens
        generated.__getitem__.return_value = completion
        mock_model.generate.return_value = generated

        mock_transformers = MagicMock(name="transformers")
        mock_transformers.AutoTokenizer.from_pretrained.return_value = (
            mock_tokenizer
        )
        mock_transformers.AutoModelForCausalLM.from_pretrained.return_value = (
            mock_model
        )

        monkeypatch.setattr(
            mod,
            "_import_runtime",
            lambda: (mock_torch, mock_transformers),
        )
        return {
            "torch": mock_torch,
            "transformers": mock_transformers,
            "tokenizer": mock_tokenizer,
            "model": mock_model,
        }

    def test_apply_chat_template_called_with_openai_shape_tools(
        self, monkeypatch
    ) -> None:
        """Bench harness passes W1-shape tools
        (``{name, description, input_schema}``); the FG chat template
        expects OpenAI shape (``{type, function: {name, parameters}}``).
        The provider must convert before calling apply_chat_template —
        the same projection ``assemble.project_sft_record`` applied at
        training time, so prompt-time and train-time agree."""
        from mili_llm_bench.providers.transformers import TransformersProvider

        mocks = self._stub_runtime(
            monkeypatch,
            decoded_with_specials="No envelope here.",
            decoded_without_specials="No envelope here.",
        )
        provider = TransformersProvider(model_path="/tmp/fake")
        provider.generate(
            messages=[{"role": "user", "content": "hi"}],
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
            max_new_tokens=8,
            seed=0,
        )
        kwargs = mocks["tokenizer"].apply_chat_template.call_args.kwargs
        sent_tools = kwargs["tools"]
        assert len(sent_tools) == 1
        assert sent_tools[0]["type"] == "function"
        assert sent_tools[0]["function"]["name"] == "load"
        assert sent_tools[0]["function"]["parameters"]["type"] == "object"
        assert kwargs["add_generation_prompt"] is True
        assert kwargs["tokenize"] is False

    def test_envelope_in_decoded_text_parses_to_tool_calls(
        self, monkeypatch
    ) -> None:
        """When the (specials-included) decode contains an FG envelope,
        generate() returns it as canonical tool_calls — not final_text."""
        from mili_llm_bench.providers.transformers import TransformersProvider

        envelope = (
            "<start_function_call>call:load{root:<escape>cylinder<escape>}"
            "<end_function_call>"
        )
        self._stub_runtime(
            monkeypatch,
            decoded_with_specials=envelope,
            decoded_without_specials="",
        )
        provider = TransformersProvider(model_path="/tmp/fake")
        out = provider.generate(
            messages=[{"role": "user", "content": "load cylinder"}],
            tools=[],
            temperature=0.0,
            max_new_tokens=64,
            seed=0,
        )
        assert out.tool_calls == [
            {"name": "load", "arguments": {"root": "cylinder"}}
        ]
        assert out.final_text is None

    def test_no_envelope_falls_back_to_final_text(self, monkeypatch) -> None:
        from mili_llm_bench.providers.transformers import TransformersProvider

        self._stub_runtime(
            monkeypatch,
            decoded_with_specials="I cannot do that.<end_of_turn>",
            decoded_without_specials="I cannot do that.",
        )
        provider = TransformersProvider(model_path="/tmp/fake")
        out = provider.generate(
            messages=[{"role": "user", "content": "do nothing"}],
            tools=[],
            temperature=0.0,
            max_new_tokens=64,
            seed=0,
        )
        assert out.tool_calls is None
        assert out.final_text == "I cannot do that."

    def test_greedy_path_pins_do_sample_false(self, monkeypatch) -> None:
        """temperature=0.0 → do_sample=False; the model.generate call
        must not be passed do_sample=True under the SFT eval recipe."""
        from mili_llm_bench.providers.transformers import TransformersProvider

        mocks = self._stub_runtime(
            monkeypatch,
            decoded_with_specials="",
            decoded_without_specials="",
        )
        provider = TransformersProvider(model_path="/tmp/fake")
        provider.generate(
            messages=[{"role": "user", "content": "hi"}],
            tools=[],
            temperature=0.0,
            max_new_tokens=8,
            seed=0,
        )
        gen_kwargs = mocks["model"].generate.call_args.kwargs
        assert gen_kwargs["do_sample"] is False
        assert gen_kwargs["max_new_tokens"] == 8


class TestCliHelp:
    """CLI --help mentions transformers; functiongemma is gone."""

    def test_cli_help_mentions_transformers(self, capsys) -> None:
        from mili_llm_bench.cli import main

        with pytest.raises(SystemExit):
            main(["run", "--help"])
        captured = capsys.readouterr()
        help_text = captured.out
        assert "transformers" in help_text
        assert "--model-path" in help_text

    def test_supported_providers_contains_transformers(self) -> None:
        assert "transformers" in SUPPORTED_PROVIDERS

    def test_functiongemma_provider_is_gone(self) -> None:
        """The deleted provider name is no longer accepted."""
        assert "functiongemma" not in SUPPORTED_PROVIDERS
        with pytest.raises(ValueError, match="unknown provider"):
            build_factories("functiongemma", config=EvalConfig())
