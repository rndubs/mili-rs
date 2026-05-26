"""Stage 6 assembler pins (rev 13 — m5-sft-pipeline.md).

Covers:

* W1 ↔ FG/OpenAI tool-format conversion via the shared
  ``mili_llm_bench.tool_format`` helper — both the assembler and the
  llamacpp inference provider call into the same function so train-
  and inference-time can't drift.
* Dedup under the §6 ``(normalized_instruction, fixture,
  tool_calls_flat)`` key.
* Per-intent held-out partition: smaller cell wins, alphabetical
  fixture tiebreak, per-intent reasons surfaced.
* Compound ratio gate fires in both directions when violated.
* Contamination: scenario ids in heldout never reappear in train/val;
  (intent, fixture) cells in heldout never reappear in train.
* End-to-end against the Stage 5 full-sweep rollouts file
  (``data/posttraining/runs/stage5-fullsweep-anthropic-...``) — the
  realistic shape pin, gated on the artifact being present.

The artifact gate uses ``pytest.skip`` rather than fail-loud so the
test suite stays runnable on a fresh clone (Stage 5 rollouts are
generated artifacts, not committed)."""

from __future__ import annotations

import json
from pathlib import Path

import pytest

from mili_llm_bench import assemble as A
from mili_llm_bench.harness import Registry
from mili_llm_bench.tool_format import w1_to_openai_tool, w1_tools_to_openai


# ---------------------------------------------------------------------------
# tool_format helpers.
# ---------------------------------------------------------------------------


class TestToolFormatHelper:
    """The shared W1 → FG/OpenAI converter; pin the shape end-to-end."""

    def test_w1_to_openai_tool_shape(self) -> None:
        w1 = {
            "name": "load",
            "description": "Load a database.",
            "input_schema": {"type": "object", "properties": {"root": {"type": "string"}}},
            "output_schema": {"type": "object"},  # dropped on the way out
        }
        out = w1_to_openai_tool(w1)
        assert out == {
            "type": "function",
            "function": {
                "name": "load",
                "description": "Load a database.",
                "parameters": {
                    "type": "object",
                    "properties": {"root": {"type": "string"}},
                },
            },
        }
        # output_schema is intentionally absent from the OpenAI shape.
        assert "output_schema" not in out["function"]

    def test_llamacpp_provider_uses_shared_helper(self) -> None:
        """Round-trip pin: the llamacpp inference path produces the
        same FG/OpenAI tool entry as the shared helper. If a future
        refactor introduces a private copy of the conversion, this
        test flags the drift."""
        from mili_llm_bench.providers.llamacpp import LlamaCppProvider

        w1 = {
            "name": "show",
            "description": "Color the mesh by a result.",
            "input_schema": {"type": "object", "properties": {"result": {"type": "string"}}},
            "output_schema": {"type": "object"},
        }
        provider = LlamaCppProvider()
        assert provider._convert_to_openai_tool(w1) == w1_to_openai_tool(w1)

    def test_w1_tools_to_openai_vectorized(self) -> None:
        w1 = [{"name": "a", "description": "", "input_schema": {}}, {"name": "b"}]
        out = w1_tools_to_openai(w1)
        assert [t["function"]["name"] for t in out] == ["a", "b"]
        assert all(t["type"] == "function" for t in out)


# ---------------------------------------------------------------------------
# Synthetic rollouts (no dependency on the Stage 5 artifact).
# ---------------------------------------------------------------------------


def _build_minimal_registry() -> Registry:
    return Registry(
        tools={
            "load": {
                "name": "load",
                "description": "Load a database",
                "input_schema": {
                    "type": "object",
                    "properties": {"root": {"type": "string"}},
                },
                "output_schema": {"type": "object"},
            },
            "show": {
                "name": "show",
                "description": "Show a result",
                "input_schema": {
                    "type": "object",
                    "properties": {"result": {"type": "string"}},
                },
                "output_schema": {"type": "object"},
            },
            "material": {
                "name": "material",
                "description": "Toggle a material",
                "input_schema": {
                    "type": "object",
                    "properties": {
                        "enable": {"type": "boolean"},
                        "material": {"type": "integer"},
                    },
                },
                "output_schema": {"type": "object"},
            },
        }
    )


def _make_rollout(
    *,
    scenario_id: str,
    intent_id: str,
    fixture: str,
    instruction: str,
    tool_calls_flat: list[dict],
    retained: bool = True,
    max_tier: int = 3,
    tools: list[str] | None = None,
) -> dict:
    """Tiny rollout fixture with the minimum keys the assembler reads."""
    assistant_calls = [
        {
            "id": f"call_{i}_0",
            "type": "function",
            "function": {
                "name": tc["name"],
                "arguments": json.dumps(tc["arguments"]),
            },
        }
        for i, tc in enumerate(tool_calls_flat)
    ]
    messages = [
        {"role": "developer", "content": "sys"},
        {"role": "user", "content": instruction},
    ]
    if assistant_calls:
        messages.append({"role": "assistant", "tool_calls": assistant_calls})
        for i, tc in enumerate(tool_calls_flat):
            messages.append(
                {
                    "role": "tool",
                    "tool_call_id": f"call_{i}_0",
                    "name": tc["name"],
                    "content": "{\"ok\": true}",
                }
            )
    return {
        "id": scenario_id,
        "fixture": fixture,
        "intent_id": intent_id,
        "instruction": instruction,
        "instruction_source": "template",
        "tools": tools or ["load", "show", "material"],
        "messages": messages,
        "tool_calls_flat": tool_calls_flat,
        "verifier": {
            "max_tier": max_tier,
            "reward": 1.0 if max_tier == 3 else 0.3,
            "failure_mode": None if max_tier == 3 else "wrong_final_state",
            "postcondition": {"kind": "state_index", "expect": {"state": 1}},
        },
        "provider": {"name": "anthropic", "config_hash": "deadbeef00000000"},
        "split": "eval",
        "k_idx": 0,
        "retained": retained,
        "usage": {"input_tokens": 1, "output_tokens": 1},
    }


def _write_rollouts(path: Path, records: list[dict]) -> None:
    with path.open("w") as f:
        for rec in records:
            f.write(json.dumps(rec))
            f.write("\n")


class TestDedup:
    """Dedup pins under the §6 ``(normalized_instruction, fixture,
    tool_calls_flat)`` key."""

    def test_k_pass_zero_diversity_collapses_to_one(self, tmp_path: Path) -> None:
        """Three K-pass rollouts of the same scenario produce one
        deduped trajectory — the rev-12 zero-diversity finding pinned
        in code."""
        path = tmp_path / "rollouts.jsonl"
        triplet = [
            _make_rollout(
                scenario_id="synth-001",
                intent_id="load",
                fixture="d3samp6",
                instruction="load the d3samp6 database",
                tool_calls_flat=[{"name": "load", "arguments": {"root": "d3samp6"}}],
            )
            for _ in range(3)
        ]
        _write_rollouts(path, triplet)
        rollouts = A.load_rollouts([path])
        assert len(rollouts) == 3
        unique = A.dedup_retained(rollouts)
        assert len(unique) == 1

    def test_paraphrase_diversity_preserved(self, tmp_path: Path) -> None:
        """Different normalized instructions producing the same tool
        calls land as separate rows — the §6 key is instruction-aware
        on purpose so SFT sees paraphrase diversity."""
        path = tmp_path / "rollouts.jsonl"
        records = [
            _make_rollout(
                scenario_id="synth-001",
                intent_id="load",
                fixture="d3samp6",
                instruction="load the d3samp6 database",
                tool_calls_flat=[{"name": "load", "arguments": {"root": "d3samp6"}}],
            ),
            _make_rollout(
                scenario_id="synth-002",
                intent_id="load",
                fixture="d3samp6",
                instruction="open d3samp6",
                tool_calls_flat=[{"name": "load", "arguments": {"root": "d3samp6"}}],
            ),
        ]
        _write_rollouts(path, records)
        unique = A.dedup_retained(A.load_rollouts([path]))
        assert len(unique) == 2

    def test_non_retained_dropped(self, tmp_path: Path) -> None:
        path = tmp_path / "rollouts.jsonl"
        records = [
            _make_rollout(
                scenario_id="synth-001",
                intent_id="load",
                fixture="d3samp6",
                instruction="x",
                tool_calls_flat=[{"name": "load", "arguments": {"root": "d3samp6"}}],
                retained=False,
                max_tier=2,
            ),
            _make_rollout(
                scenario_id="synth-002",
                intent_id="load",
                fixture="d3samp6",
                instruction="y",
                tool_calls_flat=[{"name": "load", "arguments": {"root": "d3samp6"}}],
                retained=True,
            ),
        ]
        _write_rollouts(path, records)
        unique = A.dedup_retained(A.load_rollouts([path]))
        assert len(unique) == 1
        assert unique[0].scenario_id == "synth-002"


class TestSplitPlan:
    """Per-intent partition pins."""

    def test_smaller_cell_held_out(self) -> None:
        from mili_llm_bench.assemble import LoadedRollout

        def lr(intent: str, fixture: str, scenario_id: str) -> LoadedRollout:
            return LoadedRollout(
                record={},
                scenario_id=scenario_id,
                intent_id=intent,
                fixture=fixture,
                normalized_instruction=scenario_id,
                tool_calls_flat_key="[]",
                retained=True,
                max_tier=3,
            )

        rollouts = (
            [lr("load", "d3samp6", f"a-{i}") for i in range(3)]
            + [lr("load", "cylinder", f"b-{i}") for i in range(5)]
        )
        plan = A.plan_per_intent_heldout(rollouts, A.HELDOUT_POLICY_PER_INTENT)
        assert ("load", "d3samp6") in plan.heldout_cells
        assert ("load", "cylinder") in plan.train_cells

    def test_alphabetical_tiebreak_on_equal_counts(self) -> None:
        from mili_llm_bench.assemble import LoadedRollout

        def lr(intent: str, fixture: str, scenario_id: str) -> LoadedRollout:
            return LoadedRollout(
                record={},
                scenario_id=scenario_id,
                intent_id=intent,
                fixture=fixture,
                normalized_instruction=scenario_id,
                tool_calls_flat_key="[]",
                retained=True,
                max_tier=3,
            )

        rollouts = (
            [lr("step", "d3samp6", f"a-{i}") for i in range(4)]
            + [lr("step", "cylinder", f"b-{i}") for i in range(4)]
        )
        plan = A.plan_per_intent_heldout(rollouts, A.HELDOUT_POLICY_PER_INTENT)
        # Equal counts → alphabetical fixture name wins. cylinder < d3samp6.
        assert ("step", "cylinder") in plan.heldout_cells
        assert ("step", "d3samp6") in plan.train_cells

    def test_whole_fixture_not_implemented(self) -> None:
        with pytest.raises(NotImplementedError):
            A.plan_per_intent_heldout([], A.HELDOUT_POLICY_WHOLE_FIXTURE)


class TestProjectRecord:
    """Per-record projection pins."""

    def test_tools_expanded_to_full_schema(self, tmp_path: Path) -> None:
        path = tmp_path / "rollouts.jsonl"
        _write_rollouts(
            path,
            [
                _make_rollout(
                    scenario_id="synth-001",
                    intent_id="load",
                    fixture="d3samp6",
                    instruction="load",
                    tool_calls_flat=[
                        {"name": "load", "arguments": {"root": "d3samp6"}}
                    ],
                    tools=["load", "show"],
                )
            ],
        )
        rollouts = A.dedup_retained(A.load_rollouts([path]))
        rec = A.project_sft_record(rollouts[0], _build_minimal_registry())
        assert [t["function"]["name"] for t in rec["tools"]] == ["load", "show"]
        # FG/OpenAI shape, parameters not input_schema.
        assert rec["tools"][0]["function"]["parameters"]["type"] == "object"

    def test_driver_stop_markers_stripped(self, tmp_path: Path) -> None:
        path = tmp_path / "rollouts.jsonl"
        rec_dict = _make_rollout(
            scenario_id="synth-001",
            intent_id="load",
            fixture="d3samp6",
            instruction="load",
            tool_calls_flat=[{"name": "load", "arguments": {"root": "d3samp6"}}],
        )
        rec_dict["messages"].append({"role": "system", "content": "stop:step_cap_hit"})
        _write_rollouts(path, [rec_dict])
        rollouts = A.dedup_retained(A.load_rollouts([path]))
        out = A.project_sft_record(rollouts[0], _build_minimal_registry())
        for m in out["messages"]:
            content = m.get("content")
            assert not (
                m.get("role") == "system"
                and isinstance(content, str)
                and content.startswith("stop:")
            )

    def test_postcondition_preserved(self, tmp_path: Path) -> None:
        """rev 14 / option (a): the heldout/SFT record carries
        ``postcondition`` as a top-level field so Stage 7's loader can
        reconstruct ``Scenario`` objects without joining against
        synth.jsonl. Drift here invalidates the eval set silently."""
        path = tmp_path / "rollouts.jsonl"
        rec = _make_rollout(
            scenario_id="synth-001",
            intent_id="load",
            fixture="d3samp6",
            instruction="load",
            tool_calls_flat=[{"name": "load", "arguments": {"root": "d3samp6"}}],
        )
        # rec already includes verifier.postcondition from _make_rollout.
        _write_rollouts(path, [rec])
        rollouts = A.dedup_retained(A.load_rollouts([path]))
        out = A.project_sft_record(rollouts[0], _build_minimal_registry())
        assert "postcondition" in out
        assert out["postcondition"] == {
            "kind": "state_index",
            "expect": {"state": 1},
        }

    def test_unknown_tool_dropped(self, tmp_path: Path) -> None:
        path = tmp_path / "rollouts.jsonl"
        _write_rollouts(
            path,
            [
                _make_rollout(
                    scenario_id="synth-001",
                    intent_id="load",
                    fixture="d3samp6",
                    instruction="load",
                    tool_calls_flat=[
                        {"name": "load", "arguments": {"root": "d3samp6"}}
                    ],
                    tools=["load", "ghost_tool"],
                )
            ],
        )
        rollouts = A.dedup_retained(A.load_rollouts([path]))
        rec = A.project_sft_record(rollouts[0], _build_minimal_registry())
        names = [t["function"]["name"] for t in rec["tools"]]
        assert names == ["load"]

    def test_tool_call_arguments_normalized_string_to_dict(
        self, tmp_path: Path
    ) -> None:
        """m5-sft-pipeline.md Risks §6 / rev 21 (4) — path (b) fix.
        Stage 5 wrote ``function.arguments`` as a JSON string; Stage 6
        normalizes back to dict so the next training run renders
        canonical FG-DSL (``call:NAME{key:<escape>value<escape>}``)
        instead of the double-braced ``call:NAME{<JSON>}`` shape the
        v1 corpus accidentally trained on."""
        path = tmp_path / "rollouts.jsonl"
        # ``_make_rollout`` serializes ``arguments`` via ``json.dumps``,
        # matching the rev-12 corpus shape exactly.
        _write_rollouts(
            path,
            [
                _make_rollout(
                    scenario_id="synth-001",
                    intent_id="load",
                    fixture="d3samp6",
                    instruction="load",
                    tool_calls_flat=[
                        {"name": "load", "arguments": {"root": "d3samp6"}}
                    ],
                )
            ],
        )
        rollouts = A.dedup_retained(A.load_rollouts([path]))
        rec = A.project_sft_record(rollouts[0], _build_minimal_registry())
        assistant = next(m for m in rec["messages"] if m["role"] == "assistant")
        args = assistant["tool_calls"][0]["function"]["arguments"]
        assert isinstance(args, dict)
        assert args == {"root": "d3samp6"}

    def test_tool_call_arguments_normalization_idempotent_on_dict(
        self, tmp_path: Path
    ) -> None:
        """Once Stage 5 emits dicts directly (rev 21 path (a)), Stage 6
        must remain a no-op on the already-fixed input. Idempotent on
        dicts; the helper checks ``isinstance(args, str)`` before
        parsing."""
        path = tmp_path / "rollouts.jsonl"
        rec_dict = _make_rollout(
            scenario_id="synth-001",
            intent_id="load",
            fixture="d3samp6",
            instruction="load",
            tool_calls_flat=[{"name": "load", "arguments": {"root": "d3samp6"}}],
        )
        # Patch the rollout in place to emit dict-shaped arguments —
        # the future Stage-5-side fix's shape.
        for m in rec_dict["messages"]:
            for tc in m.get("tool_calls") or []:
                tc["function"]["arguments"] = {"root": "d3samp6"}
        _write_rollouts(path, [rec_dict])
        rollouts = A.dedup_retained(A.load_rollouts([path]))
        out = A.project_sft_record(rollouts[0], _build_minimal_registry())
        assistant = next(m for m in out["messages"] if m["role"] == "assistant")
        args = assistant["tool_calls"][0]["function"]["arguments"]
        assert args == {"root": "d3samp6"}

    def test_terminating_assistant_text_appended(self, tmp_path: Path) -> None:
        """m7 Delta 1 — every projected SFT record must end on a
        content-only assistant message so the trainer's loss-mask
        includes a positive "stop after success" signal. See
        m7-bench-live-parity.md §"Delta 1" and the loss-mask probe
        in preflight-4-loss-mask.md §"Single-row probe"."""
        path = tmp_path / "rollouts.jsonl"
        _write_rollouts(
            path,
            [
                _make_rollout(
                    scenario_id="synth-001",
                    intent_id="load",
                    fixture="d3samp6",
                    instruction="load d3samp6",
                    tool_calls_flat=[
                        {"name": "load", "arguments": {"root": "d3samp6"}}
                    ],
                )
            ],
        )
        rollouts = A.dedup_retained(A.load_rollouts([path]))
        rec = A.project_sft_record(rollouts[0], _build_minimal_registry())
        last = rec["messages"][-1]
        assert last == {"role": "assistant", "content": A.DEFAULT_TERMINATING_TEXT}
        # The penultimate message is still the tool response — the
        # helper appends, it doesn't replace.
        assert rec["messages"][-2]["role"] == "tool"

    def test_terminating_assistant_text_idempotent(self, tmp_path: Path) -> None:
        """If the source rollout already terminates on a content-only
        assistant message (a future training corpus that emits final
        text natively), the projector leaves it alone — no duplicate
        terminator, no overwrite."""
        path = tmp_path / "rollouts.jsonl"
        rec_dict = _make_rollout(
            scenario_id="synth-001",
            intent_id="load",
            fixture="d3samp6",
            instruction="load d3samp6",
            tool_calls_flat=[{"name": "load", "arguments": {"root": "d3samp6"}}],
        )
        rec_dict["messages"].append({"role": "assistant", "content": "Loaded."})
        _write_rollouts(path, [rec_dict])
        rollouts = A.dedup_retained(A.load_rollouts([path]))
        rec = A.project_sft_record(rollouts[0], _build_minimal_registry())
        assert rec["messages"][-1] == {"role": "assistant", "content": "Loaded."}
        # No extra "Done." appended.
        ack_count = sum(
            1
            for m in rec["messages"]
            if m.get("role") == "assistant"
            and isinstance(m.get("content"), str)
            and m["content"].strip()
            and not m.get("tool_calls")
        )
        assert ack_count == 1

    def test_tool_call_arguments_normalized_multi_call(
        self, tmp_path: Path
    ) -> None:
        """A compound assistant turn with two tool calls normalizes
        both. Catches the failure mode where the helper short-circuits
        after the first call in a list."""
        path = tmp_path / "rollouts.jsonl"
        _write_rollouts(
            path,
            [
                _make_rollout(
                    scenario_id="synth-001",
                    intent_id="compound-material-show",
                    fixture="d3samp6",
                    instruction="disable mat 2 then show sx",
                    tool_calls_flat=[
                        {
                            "name": "material",
                            "arguments": {"enable": False, "material": 2},
                        },
                        {"name": "show", "arguments": {"result": "sx"}},
                    ],
                )
            ],
        )
        rollouts = A.dedup_retained(A.load_rollouts([path]))
        rec = A.project_sft_record(rollouts[0], _build_minimal_registry())
        assistant = next(m for m in rec["messages"] if m["role"] == "assistant")
        assert len(assistant["tool_calls"]) == 2
        for tc in assistant["tool_calls"]:
            assert isinstance(tc["function"]["arguments"], dict)
        assert assistant["tool_calls"][0]["function"]["arguments"] == {
            "enable": False,
            "material": 2,
        }
        assert assistant["tool_calls"][1]["function"]["arguments"] == {
            "result": "sx",
        }


class TestEndToEnd:
    """The full ``assemble()`` pipeline against a synthetic corpus."""

    def _seed_two_intent_two_fixture(self, path: Path) -> None:
        """A minimal corpus with the compound-family ratio constraint
        already satisfied. Three intents: load (2 cells × 2-3 rows),
        material (2 cells × 3-4 rows), compound-material-then-show
        (2 cells × 2-3 rows). Compound rows = ~5/16 ≈ 31% — above
        the 20% gate."""
        records: list[dict] = []
        idx = 0

        def add(intent: str, fixture: str, instruction: str, calls: list[dict]) -> None:
            nonlocal idx
            records.append(
                _make_rollout(
                    scenario_id=f"synth-{idx:04d}",
                    intent_id=intent,
                    fixture=fixture,
                    instruction=instruction,
                    tool_calls_flat=calls,
                )
            )
            idx += 1

        # load: cylinder=2, d3samp6=3 → cylinder heldout
        for i in range(2):
            add("load", "cylinder", f"open cyl {i}", [{"name": "load", "arguments": {"root": "cylinder"}}])
        for i in range(3):
            add("load", "d3samp6", f"open d3 {i}", [{"name": "load", "arguments": {"root": "d3samp6"}}])
        # material: cylinder=3, d3samp6=4 → cylinder heldout
        for i in range(3):
            add("material", "cylinder", f"disable mat {i} cyl", [{"name": "material", "arguments": {"enable": False, "material": i + 1}}])
        for i in range(4):
            add("material", "d3samp6", f"disable mat {i} d3", [{"name": "material", "arguments": {"enable": False, "material": i + 1}}])
        # compound: cylinder=2, d3samp6=3 → cylinder heldout
        for i in range(2):
            add(
                "compound-material-then-show",
                "cylinder",
                f"hide mat {i} then show stress cyl",
                [
                    {"name": "material", "arguments": {"enable": False, "material": i + 1}},
                    {"name": "show", "arguments": {"result": "stress"}},
                ],
            )
        for i in range(3):
            add(
                "compound-material-then-show",
                "d3samp6",
                f"hide mat {i} then show stress d3",
                [
                    {"name": "material", "arguments": {"enable": False, "material": i + 1}},
                    {"name": "show", "arguments": {"result": "stress"}},
                ],
            )
        _write_rollouts(path, records)

    def test_full_pipeline_writes_files_and_card(self, tmp_path: Path) -> None:
        rollouts_path = tmp_path / "rollouts.jsonl"
        self._seed_two_intent_two_fixture(rollouts_path)
        out_dir = tmp_path / "out"
        report = A.assemble(
            [rollouts_path],
            out_dir,
            registry=_build_minimal_registry(),
            heldout_policy=A.HELDOUT_POLICY_PER_INTENT,
            query_policy=A.QUERY_POLICY_ACCEPT,
            seed=42,
            floor_per_intent=1,
            compound_ratio_min=0.20,
            val_fraction=0.10,
        )
        # Files exist.
        assert (out_dir / "sft" / "train.jsonl").exists()
        assert (out_dir / "sft" / "val.jsonl").exists()
        assert (out_dir / "eval" / "heldout.jsonl").exists()
        assert (out_dir / "pref" / "train.jsonl").exists()
        assert (out_dir / "pref" / "val.jsonl").exists()

        A.write_dataset_card(out_dir, report, rollouts_paths=[rollouts_path])
        assert (out_dir / "dataset_card.md").exists()

        # Heldout cells are exactly the smaller-cell ones.
        assert report.split_plan.heldout_cells == {
            ("load", "cylinder"),
            ("material", "cylinder"),
            ("compound-material-then-show", "cylinder"),
        }

    def test_every_assembled_record_terminates_on_assistant_content(
        self, tmp_path: Path
    ) -> None:
        """m7 Delta 1 validation §A — the audit script the M7 plan
        prescribes, codified as a unit test. Every record across
        train / val / heldout must end on a content-only assistant
        message so the loss-mask covers the "stop after success"
        signal. If this fails, ``assemble.project_sft_record`` is no
        longer appending the terminator — investigate before retraining."""
        rollouts_path = tmp_path / "rollouts.jsonl"
        self._seed_two_intent_two_fixture(rollouts_path)
        out_dir = tmp_path / "out"
        A.assemble(
            [rollouts_path],
            out_dir,
            registry=_build_minimal_registry(),
            floor_per_intent=1,
            compound_ratio_min=0.20,
            val_fraction=0.10,
        )
        for split in ("sft/train.jsonl", "sft/val.jsonl", "eval/heldout.jsonl"):
            path = out_dir / split
            if not path.exists():
                continue
            for line in path.read_text().splitlines():
                rec = json.loads(line)
                last = rec["messages"][-1]
                assert last.get("role") == "assistant", (
                    f"{split}/{rec['scenario_id']} must end on assistant; "
                    f"got role={last.get('role')!r}"
                )
                content = last.get("content")
                assert isinstance(content, str) and content.strip(), (
                    f"{split}/{rec['scenario_id']} terminating assistant "
                    f"message must carry non-empty content"
                )
                assert not last.get("tool_calls"), (
                    f"{split}/{rec['scenario_id']} terminator must not "
                    f"carry tool_calls"
                )

    def test_contamination_clean(self, tmp_path: Path) -> None:
        rollouts_path = tmp_path / "rollouts.jsonl"
        self._seed_two_intent_two_fixture(rollouts_path)
        out_dir = tmp_path / "out"
        A.assemble(
            [rollouts_path],
            out_dir,
            registry=_build_minimal_registry(),
            floor_per_intent=1,
            compound_ratio_min=0.20,
            val_fraction=0.0,  # keep val empty so every train row is observable
        )
        train_ids = {
            json.loads(line)["scenario_id"]
            for line in (out_dir / "sft" / "train.jsonl").open()
        }
        val_ids = {
            json.loads(line)["scenario_id"]
            for line in (out_dir / "sft" / "val.jsonl").open()
        }
        held_ids = {
            json.loads(line)["scenario_id"]
            for line in (out_dir / "eval" / "heldout.jsonl").open()
        }
        assert train_ids.isdisjoint(held_ids)
        assert val_ids.isdisjoint(held_ids)
        # No scenario duplicated within train + val either.
        assert train_ids.isdisjoint(val_ids)
        # And cells don't bleed across train/heldout.
        train_cells = {
            (json.loads(line)["intent_id"], json.loads(line)["fixture"])
            for line in (out_dir / "sft" / "train.jsonl").open()
        }
        heldout_cells = {
            (json.loads(line)["intent_id"], json.loads(line)["fixture"])
            for line in (out_dir / "eval" / "heldout.jsonl").open()
        }
        assert train_cells.isdisjoint(heldout_cells)

    def test_under_floor_flagged(self, tmp_path: Path) -> None:
        rollouts_path = tmp_path / "rollouts.jsonl"
        self._seed_two_intent_two_fixture(rollouts_path)
        report = A.assemble(
            [rollouts_path],
            tmp_path / "out",
            registry=_build_minimal_registry(),
            floor_per_intent=10,  # nothing clears
            compound_ratio_min=0.20,
            val_fraction=0.0,
        )
        assert report.under_floor_intents == [
            "compound-material-then-show",
            "load",
            "material",
        ]

    def test_compound_ratio_gate_fires(self, tmp_path: Path) -> None:
        """An all-atomic corpus must fail the compound-ratio gate."""
        rollouts_path = tmp_path / "rollouts.jsonl"
        records = [
            _make_rollout(
                scenario_id=f"synth-{i:04d}",
                intent_id="load",
                fixture="d3samp6" if i % 2 else "cylinder",
                instruction=f"open #{i}",
                tool_calls_flat=[
                    {"name": "load", "arguments": {"root": f"r{i}"}}
                ],
            )
            for i in range(8)
        ]
        _write_rollouts(rollouts_path, records)
        with pytest.raises(RuntimeError, match="compound ratio"):
            A.assemble(
                [rollouts_path],
                tmp_path / "out",
                registry=_build_minimal_registry(),
                floor_per_intent=1,
                compound_ratio_min=0.20,
            )

    def test_query_policy_drop_removes_intent(self, tmp_path: Path) -> None:
        rollouts_path = tmp_path / "rollouts.jsonl"
        records = [
            _make_rollout(
                scenario_id=f"synth-{i:04d}",
                intent_id="query",
                fixture="d3samp6" if i % 2 else "cylinder",
                instruction=f"query #{i}",
                tool_calls_flat=[{"name": "load", "arguments": {"root": "x"}}],
            )
            for i in range(6)
        ]
        # Plus enough load + compound to clear the compound-ratio gate.
        next_id = 6
        for f in ("cylinder", "d3samp6"):
            records.append(
                _make_rollout(
                    scenario_id=f"synth-{next_id:04d}",
                    intent_id="load",
                    fixture=f,
                    instruction=f"open {f}",
                    tool_calls_flat=[{"name": "load", "arguments": {"root": f}}],
                )
            )
            next_id += 1
            for k in range(2):
                records.append(
                    _make_rollout(
                        scenario_id=f"synth-{next_id:04d}",
                        intent_id="compound-material-then-show",
                        fixture=f,
                        instruction=f"hide & show {f} {k}",
                        tool_calls_flat=[
                            {"name": "material", "arguments": {"enable": False, "material": k + 1}},
                            {"name": "show", "arguments": {"result": "stress"}},
                        ],
                    )
                )
                next_id += 1
        _write_rollouts(rollouts_path, records)
        report = A.assemble(
            [rollouts_path],
            tmp_path / "out",
            registry=_build_minimal_registry(),
            query_policy=A.QUERY_POLICY_DROP,
            floor_per_intent=1,
            compound_ratio_min=0.20,
            val_fraction=0.0,
        )
        assert "query" not in report.pre_split_intent_counts
        assert "query" not in report.train_intent_counts


# ---------------------------------------------------------------------------
# Stage-5 artifact integration check.
# ---------------------------------------------------------------------------


_STAGE5_FULLSWEEP = (
    Path(__file__).parents[3]
    / "data"
    / "posttraining"
    / "runs"
    / "stage5-fullsweep-anthropic-20260524-223426"
    / "rollouts.jsonl"
)


@pytest.mark.skipif(
    not _STAGE5_FULLSWEEP.exists(),
    reason="Stage 5 full-sweep artifact not present on this checkout",
)
class TestStage5FullSweep:
    """End-to-end pin against the actual rev-12 rollouts artifact."""

    def test_dedup_yields_171_unique_trajectories(self) -> None:
        rollouts = A.load_rollouts([_STAGE5_FULLSWEEP])
        unique = A.dedup_retained(rollouts)
        # The rev-12 ledger: 171 retained scenarios, instruction-aware
        # dedup is a no-op given one canonical instruction per scenario.
        assert len(unique) == 171

    def test_assemble_clears_compound_gate(self, tmp_path: Path) -> None:
        report = A.assemble(
            [_STAGE5_FULLSWEEP],
            tmp_path / "out",
            heldout_policy=A.HELDOUT_POLICY_PER_INTENT,
            query_policy=A.QUERY_POLICY_ACCEPT,
            seed=42,
            floor_per_intent=10,
            compound_ratio_min=0.20,
        )
        assert report.train_compound_ratio >= 0.20
        assert report.heldout_compound_ratio >= 0.20
        # The Stage 5 corpus has zero mixed-tier scenarios under K=3@T=0.7
        # (rev 12 finding) → pref/*.jsonl is empty by construction.
        assert report.pref_count == 0
