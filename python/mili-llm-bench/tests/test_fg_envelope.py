"""Pin the shared FunctionGemma response-envelope parser.

The helper lives at ``mili_llm_bench.providers._fg_envelope`` and is
consumed by ``LlamaCppProvider`` (the rev-10 client-side fallback) and
``TransformersProvider`` (direct HF generate). These tests exercise the
parser in isolation so a regression in the regex set surfaces here
before it leaks into either provider's behavior.

The provider-level tests still pin the *gates* around when the parser
fires (caps probe for llamacpp, envelope-or-final_text for
transformers). This module pins the parser itself.
"""

from __future__ import annotations

from mili_llm_bench.providers._fg_envelope import (
    coerce_fg_scalar,
    parse_fg_envelopes,
)


class TestParseEnvelopes:
    """Single, multiple, and absent envelopes round-trip to the canonical
    ``[{"name", "arguments": dict}, ...]`` shape."""

    def test_single_call_with_string_arg(self) -> None:
        content = (
            "<start_function_call>call:load{root:<escape>d3samp6<escape>}"
            "<end_function_call>"
        )
        assert parse_fg_envelopes(content) == [
            {"name": "load", "arguments": {"root": "d3samp6"}}
        ]

    def test_multiple_calls_preserve_source_order(self) -> None:
        """Compound intents emit two envelopes in one generation; the
        verifier postcondition can depend on the order, so pin it."""
        content = (
            "<start_function_call>call:select{class_:<escape>brick<escape>,"
            "range:<escape>1-10<escape>}<end_function_call>"
            "<start_function_call>call:show{result:<escape>sx<escape>}"
            "<end_function_call>"
        )
        calls = parse_fg_envelopes(content)
        assert len(calls) == 2
        assert calls[0]["name"] == "select"
        assert calls[0]["arguments"] == {"class_": "brick", "range": "1-10"}
        assert calls[1]["name"] == "show"
        assert calls[1]["arguments"] == {"result": "sx"}

    def test_no_envelope_returns_empty(self) -> None:
        assert parse_fg_envelopes("I cannot do that.") == []
        assert parse_fg_envelopes("") == []

    def test_mixed_string_and_bare_args(self) -> None:
        """One call carrying a string (escape-wrapped), a bool, an int,
        and a float. Coerce true/false to bool; try int then float for
        the rest; leave unrecognized as string."""
        content = (
            "<start_function_call>call:setopts{"
            "name:<escape>shell_mat2<escape>,"
            "wireframe:true,"
            "count:42,"
            "scale:1.5"
            "}<end_function_call>"
        )
        calls = parse_fg_envelopes(content)
        assert len(calls) == 1
        assert calls[0] == {
            "name": "setopts",
            "arguments": {
                "name": "shell_mat2",
                "wireframe": True,
                "count": 42,
                "scale": 1.5,
            },
        }

    def test_envelope_with_whitespace_variant(self) -> None:
        """``call:NAME { … }`` whitespace variant parses the same as
        the more common no-whitespace form."""
        content = (
            "<start_function_call> call:load { root:<escape>cylinder<escape> }"
            " <end_function_call>"
        )
        assert parse_fg_envelopes(content) == [
            {"name": "load", "arguments": {"root": "cylinder"}}
        ]

    def test_json_literal_body_shape(self) -> None:
        """The v1 SFT corpus accidentally trained on JSON-literal
        bodies (``call:NAME{ {"k": "v"} }``) because the Stage 5
        driver wrote ``function.arguments`` as a JSON string. The
        chat template's string-arguments branch then inserted that
        literal between the call braces. The v1 checkpoints emit this
        shape at inference; the parser must accept it.

        ``TODO(v2)``: re-render training data with arguments as a
        dict; this branch can retire once new checkpoints emit
        canonical FG-DSL.
        """
        content = (
            "<start_function_call>call:load{"
            '                    {"root": "cylinder"}}'
            "<end_function_call>"
        )
        assert parse_fg_envelopes(content) == [
            {"name": "load", "arguments": {"root": "cylinder"}}
        ]

    def test_json_literal_with_compound_calls(self) -> None:
        """Multi-call generations under the JSON-literal shape — each
        envelope's body is independently JSON-parsed."""
        content = (
            "<start_function_call>call:select{"
            '{"class_": "brick", "range": "1-10"}}'
            "<end_function_call>"
            "<start_function_call>call:show{"
            '{"result": "sx"}}'
            "<end_function_call>"
        )
        calls = parse_fg_envelopes(content)
        assert calls == [
            {"name": "select", "arguments": {"class_": "brick", "range": "1-10"}},
            {"name": "show", "arguments": {"result": "sx"}},
        ]

    def test_empty_body_returns_empty_args(self) -> None:
        """``call:NAME{}`` (no args) is a recognized envelope with
        ``arguments={}``; common for zero-arg tools like
        ``clrsel()``."""
        content = "<start_function_call>call:clrsel{}<end_function_call>"
        assert parse_fg_envelopes(content) == [
            {"name": "clrsel", "arguments": {}}
        ]


class TestCoerceFgScalar:
    """The bare-scalar coercion table — bool > int > float > string."""

    def test_true_false(self) -> None:
        assert coerce_fg_scalar("true") is True
        assert coerce_fg_scalar("false") is False

    def test_int_then_float(self) -> None:
        assert coerce_fg_scalar("42") == 42
        assert isinstance(coerce_fg_scalar("42"), int)
        assert coerce_fg_scalar("3.14") == 3.14
        assert isinstance(coerce_fg_scalar("3.14"), float)

    def test_string_fallthrough(self) -> None:
        # Whitespace stripped but the value is otherwise preserved.
        assert coerce_fg_scalar(" shell_mat2 ") == "shell_mat2"
        # ``true`` is the only recognized bool; ``TRUE`` is a string.
        assert coerce_fg_scalar("TRUE") == "TRUE"


class TestSharedHelperDrift:
    """Both LlamaCppProvider and TransformersProvider must reference the
    shared helper, not a local copy. The failure mode this guards
    against: someone copy-pastes the regexes back into one provider to
    "make a quick fix" and the two paths silently diverge.

    Asserted at module load: importing both providers must point
    ``parse_fg_envelopes`` to the SAME function object as the canonical
    ``_fg_envelope`` module.
    """

    def test_llamacpp_uses_shared_helper(self) -> None:
        from mili_llm_bench.providers import _fg_envelope
        from mili_llm_bench.providers import llamacpp

        assert llamacpp.parse_fg_envelopes is _fg_envelope.parse_fg_envelopes

    def test_transformers_uses_shared_helper(self) -> None:
        from mili_llm_bench.providers import _fg_envelope
        from mili_llm_bench.providers import transformers as transformers_provider

        assert (
            transformers_provider.parse_fg_envelopes
            is _fg_envelope.parse_fg_envelopes
        )
