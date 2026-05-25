"""Pins for ``MaskAssistantOnlyCollator`` (preflight #4 — m5-sft-pipeline.md rev 17).

Always-on: synthetic ``input_ids`` only, no model / tokenizer load. Uses a
recorded-base-collator shim instead of ``DataCollatorForLanguageModeling``
so the test runs in the default ``pytest`` invocation.
"""

from __future__ import annotations

import pytest

torch = pytest.importorskip("torch")

from mili_llm_bench.assistant_only_collator import (
    MaskAssistantOnlyCollator,
    _COLON_ID,
    _EFR_ID,
    _EOT_ID,
    _MODEL_HEADER_IDS,
    _RESPONSE_ID,
    _SFR_ID,
)


class _StubBase:
    """Stand-in for the base causal-LM collator.

    Treats each ``feature`` as a dict ``{"input_ids": [int, ...]}``,
    pads to a uniform length with ``pad_id``, and emits ``labels`` =
    ``input_ids`` with ``pad_id`` positions masked to ``-100`` (the
    HF default).
    """

    def __init__(self, pad_id: int = 0):
        self.pad_id = pad_id

    def __call__(self, features):
        max_n = max(len(f["input_ids"]) for f in features)
        b = len(features)
        ids = torch.full((b, max_n), self.pad_id, dtype=torch.long)
        for i, f in enumerate(features):
            row = f["input_ids"]
            ids[i, : len(row)] = torch.tensor(row, dtype=torch.long)
        labels = ids.clone()
        labels[ids == self.pad_id] = -100
        return {
            "input_ids": ids,
            "attention_mask": (ids != self.pad_id).long(),
            "labels": labels,
        }


def _header():
    return list(_MODEL_HEADER_IDS)


class TestSingleAssistantTurn:
    """Header + content + EOT — the simplest case."""

    def test_single_turn_unmasks_content_and_eot(self):
        # [user prefix=999,998] [<sot> model \n] [content=300,301] [<eot>]
        seq = [999, 998] + _header() + [300, 301] + [_EOT_ID]
        coll = MaskAssistantOnlyCollator(_StubBase())
        out = coll([{"input_ids": seq}])
        labels = out["labels"][0].tolist()
        # Positions 0,1 (user prefix) masked
        assert labels[0] == -100
        assert labels[1] == -100
        # Positions 2,3,4 (header tokens) masked
        assert labels[2] == -100
        assert labels[3] == -100
        assert labels[4] == -100
        # Positions 5,6 (content) unmasked
        assert labels[5] == 300
        assert labels[6] == 301
        # Position 7 (<end_of_turn>) unmasked — model learns to stop
        assert labels[7] == _EOT_ID


class TestToolResponseSubtraction:
    """Tool-response payload INSIDE a model turn must be remasked."""

    def test_tool_response_payload_remasked(self):
        # model turn:
        #   <sot> model \n  <sfc> call payload <efc>
        #   <sfr> response : tool-payload <efr>
        #   <eot>
        seq = (
            _header()
            + [48, 700, 49]              # <start_function_call> 700 <end_function_call>
            + [_SFR_ID, _RESPONSE_ID, _COLON_ID, 800, 801, _EFR_ID]  # tool response
            + [_EOT_ID]
        )
        coll = MaskAssistantOnlyCollator(_StubBase())
        out = coll([{"input_ids": seq}])
        labels = out["labels"][0].tolist()
        # header masked
        assert labels[0] == -100
        # tool call envelope unmasked
        assert labels[3] == 48
        assert labels[4] == 700
        assert labels[5] == 49
        # <start_function_response> (model's cue) unmasked
        assert labels[6] == _SFR_ID
        # tool-response payload remasked
        assert labels[7] == -100  # "response"
        assert labels[8] == -100  # ":"
        assert labels[9] == -100  # 800
        assert labels[10] == -100  # 801
        assert labels[11] == -100  # <end_function_response>
        # <end_of_turn> unmasked
        assert labels[12] == _EOT_ID

    def test_bare_sfr_without_response_stays_unmasked(self):
        # The assistant emits a bare <start_function_response> at the end
        # of its last tool_call. If NOT followed by "response:", it must
        # stay unmasked.
        seq = (
            _header()
            + [48, 700, 49]              # tool call
            + [_SFR_ID]                  # bare SFR (assistant's cue, no tool payload)
            + [_EOT_ID]
        )
        coll = MaskAssistantOnlyCollator(_StubBase())
        labels = coll([{"input_ids": seq}])["labels"][0].tolist()
        # bare SFR remains unmasked
        assert labels[6] == _SFR_ID
        # EOT unmasked
        assert labels[7] == _EOT_ID


class TestMultipleToolCalls:
    """Two tool calls + responses, one model turn."""

    def test_two_tool_responses_each_remasked(self):
        seq = (
            _header()
            + [48, 700, 49]                                        # call #1
            + [_SFR_ID, _RESPONSE_ID, _COLON_ID, 800, _EFR_ID]    # resp #1
            + [48, 701, 49]                                        # call #2
            + [_SFR_ID, _RESPONSE_ID, _COLON_ID, 802, _EFR_ID]    # resp #2
            + [_EOT_ID]
        )
        labels = MaskAssistantOnlyCollator(_StubBase())(
            [{"input_ids": seq}]
        )["labels"][0].tolist()
        # First tool call kept
        assert labels[3] == 48 and labels[4] == 700 and labels[5] == 49
        # First response payload masked
        assert labels[7] == -100 and labels[8] == -100 and labels[9] == -100
        assert labels[10] == -100  # EFR
        # Second tool call kept
        assert labels[11] == 48 and labels[12] == 701 and labels[13] == 49
        # Second response payload masked
        assert labels[15] == -100 and labels[16] == -100 and labels[17] == -100
        assert labels[18] == -100  # EFR
        # EOT kept
        assert labels[19] == _EOT_ID


class TestEdgeCases:
    def test_no_model_turn_all_masked(self):
        # only developer + user turns, no model header
        seq = [999, 998, 997, 996]
        labels = MaskAssistantOnlyCollator(_StubBase())(
            [{"input_ids": seq}]
        )["labels"][0].tolist()
        assert all(x == -100 for x in labels)

    def test_pad_tokens_stay_masked(self):
        # Row with model content followed by pad — base collator's -100
        # for pad must survive the mask path.
        seq = _header() + [300, _EOT_ID]
        # _StubBase pads to max length; here only one row, no padding.
        # Add a 2-row batch with different lengths to force padding.
        long = _header() + [300, 301, 302, _EOT_ID]
        out = MaskAssistantOnlyCollator(_StubBase())(
            [{"input_ids": seq}, {"input_ids": long}]
        )
        labels = out["labels"]
        # Row 0 is padded — pad positions must stay -100
        n0 = len(seq)
        assert (labels[0, n0:] == -100).all()
        # Row 0 content unmasked
        assert labels[0, 3].item() == 300
        assert labels[0, 4].item() == _EOT_ID
        # Row 1 content unmasked through EOT
        assert labels[1, 3].item() == 300
        assert labels[1, 4].item() == 301
        assert labels[1, 5].item() == 302
        assert labels[1, 6].item() == _EOT_ID

    def test_multiple_model_turns_in_one_row(self):
        # Two assistant turns separated by a user turn.
        seq = (
            _header() + [300, _EOT_ID]               # model turn 1
            + [999, 998]                              # user turn (masked)
            + _header() + [400, 401, _EOT_ID]         # model turn 2
        )
        labels = MaskAssistantOnlyCollator(_StubBase())(
            [{"input_ids": seq}]
        )["labels"][0].tolist()
        # Turn 1 content + EOT
        assert labels[3] == 300
        assert labels[4] == _EOT_ID
        # User-turn tokens masked
        assert labels[5] == -100 and labels[6] == -100
        # Turn 2 content + EOT
        assert labels[10] == 400
        assert labels[11] == 401
        assert labels[12] == _EOT_ID


class TestTokenIdConstants:
    """Pin the token IDs against the FG tokenizer when available.

    Skip-on-absent: ``transformers`` and the FG model are gated. CI's
    ``test-heavy`` job has them; local default suite skips.
    """

    def test_token_ids_match_fg_tokenizer(self):
        transformers = pytest.importorskip("transformers")
        try:
            tok = transformers.AutoTokenizer.from_pretrained(
                "google/functiongemma-270m-it"
            )
        except Exception as e:
            pytest.skip(f"FG tokenizer not available: {e}")

        assert tok.encode("<start_of_turn>", add_special_tokens=False) == [
            _MODEL_HEADER_IDS[0]
        ]
        assert tok.encode("model", add_special_tokens=False) == [
            _MODEL_HEADER_IDS[1]
        ]
        assert tok.encode("\n", add_special_tokens=False) == [
            _MODEL_HEADER_IDS[2]
        ]
        assert tok.encode("<end_of_turn>", add_special_tokens=False) == [_EOT_ID]
        assert tok.encode("<start_function_response>", add_special_tokens=False) == [
            _SFR_ID
        ]
        assert tok.encode("<end_function_response>", add_special_tokens=False) == [
            _EFR_ID
        ]
        assert tok.encode("response", add_special_tokens=False) == [_RESPONSE_ID]
        assert tok.encode(":", add_special_tokens=False) == [_COLON_ID]
