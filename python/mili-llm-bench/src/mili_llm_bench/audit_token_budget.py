"""Preflight #5 — token-budget audit on ``sft/train.jsonl``.

Multi-step compound scenarios with the full ~18-tool inventory can
silently exceed FunctionGemma's pinned ``max_length=512`` (Google's
fine-tuning recipe, mirrored in ``cluster-setup.md`` §6). The trainer
truncates from the right, which throws away exactly the assistant
turns we want to learn from — a silent failure mode that only shows
up as bad SFT results after the cluster run.

This module renders each row through the same
``tokenizer.apply_chat_template(messages, tools, ...)`` call the
trainer uses, collects per-row token counts, and writes a pass/fail
report under ``data/posttraining/sft/preflight-5-token-budget.md``.
The threshold defaults to 512 (Google's pin); ``--max-length`` lets a
caller deliberately raise the bar after recording the decision in
``m5-sft-pipeline.md``.

See ``planning/mili-viz/mili-agent/sft-preflight-gpu.md`` §5.
"""

from __future__ import annotations

import json
import statistics
import sys
from collections import defaultdict
from dataclasses import dataclass, field
from pathlib import Path
from typing import Any


DEFAULT_TOKENIZER_ID = "google/functiongemma-270m-it"
DEFAULT_MAX_LENGTH = 512


@dataclass
class TokenBudgetReport:
    train_path: Path
    tokenizer_id: str
    max_length: int
    n_rows: int
    counts: list[int]
    by_intent: dict[str, list[int]] = field(default_factory=dict)
    over_budget_ids: list[str] = field(default_factory=list)

    @property
    def passed(self) -> bool:
        return all(c <= self.max_length for c in self.counts)

    @property
    def n_over_budget(self) -> int:
        return sum(1 for c in self.counts if c > self.max_length)

    @property
    def p50(self) -> int:
        return int(statistics.median(self.counts)) if self.counts else 0

    @property
    def p95(self) -> int:
        if not self.counts:
            return 0
        srt = sorted(self.counts)
        idx = min(len(srt) - 1, max(0, int(round(0.95 * (len(srt) - 1)))))
        return srt[idx]

    @property
    def max_tokens(self) -> int:
        return max(self.counts) if self.counts else 0

    @property
    def min_tokens(self) -> int:
        return min(self.counts) if self.counts else 0


def _load_tokenizer(tokenizer_id: str) -> Any:
    """Load the HF tokenizer; raise a friendly error on a 401 (license
    not accepted) or a missing-cache miss."""
    try:
        from transformers import AutoTokenizer
    except ImportError as exc:
        raise ImportError(
            "audit_token_budget requires the `transformers` package. "
            "Install with `uv sync --directory python --extra train` "
            "(or `--extra functiongemma`)."
        ) from exc
    return AutoTokenizer.from_pretrained(tokenizer_id)


def audit_token_budget(
    train_path: Path,
    *,
    tokenizer_id: str = DEFAULT_TOKENIZER_ID,
    max_length: int = DEFAULT_MAX_LENGTH,
) -> TokenBudgetReport:
    """Render every row of ``train_path`` through
    ``tok.apply_chat_template(messages, tools=tools, tokenize=True)``
    and return the token-count distribution.

    Rows missing ``messages`` or ``tools`` are skipped (they would
    crash ``apply_chat_template`` and don't represent the trained
    shape). The skipped rows are NOT counted in the distribution; the
    operator is expected to repair them upstream.
    """
    tok = _load_tokenizer(tokenizer_id)
    counts: list[int] = []
    by_intent: dict[str, list[int]] = defaultdict(list)
    over_budget_ids: list[str] = []
    n_skipped = 0

    with Path(train_path).open() as f:
        for line in f:
            line = line.strip()
            if not line:
                continue
            row = json.loads(line)
            messages = row.get("messages")
            tools = row.get("tools")
            if messages is None or tools is None:
                n_skipped += 1
                continue
            ids = tok.apply_chat_template(
                messages,
                tools=tools,
                add_generation_prompt=False,
                tokenize=True,
            )
            n = len(ids)
            counts.append(n)
            intent = str(row.get("intent_id", "<unknown>"))
            by_intent[intent].append(n)
            if n > max_length:
                over_budget_ids.append(str(row.get("scenario_id", row.get("id", "<no-id>"))))

    if n_skipped:
        sys.stderr.write(
            f"audit_token_budget: skipped {n_skipped} malformed row(s) "
            f"in {train_path}\n"
        )

    return TokenBudgetReport(
        train_path=Path(train_path),
        tokenizer_id=tokenizer_id,
        max_length=max_length,
        n_rows=len(counts),
        counts=counts,
        by_intent=dict(by_intent),
        over_budget_ids=over_budget_ids,
    )


def write_audit_report(out_path: Path, report: TokenBudgetReport) -> Path:
    """Render the ``TokenBudgetReport`` as a permanent companion to
    ``dataset_card.md``."""
    out_path = Path(out_path)
    lines: list[str] = []
    verdict = "PASS" if report.passed else "FAIL"
    lines.append("# Preflight #5 — token-budget audit")
    lines.append("")
    lines.append(
        f"**Verdict: {verdict}.** max tokens = {report.max_tokens} "
        f"(gate: ≤ {report.max_length})."
    )
    lines.append("")
    lines.append(
        "Generated by `mili-llm-bench audit-token-budget`. Renders "
        "every row of `sft/train.jsonl` through "
        "`tokenizer.apply_chat_template(messages, tools=tools, "
        "tokenize=True)` — the same call SFTTrainer uses — and "
        "collects token counts. See "
        "`planning/mili-viz/mili-agent/sft-preflight-gpu.md` §5."
    )
    lines.append("")

    lines.append("## Inputs")
    lines.append("")
    lines.append(f"- train: `{report.train_path}`")
    lines.append(f"- tokenizer: `{report.tokenizer_id}`")
    lines.append(f"- max_length gate: `{report.max_length}`")
    lines.append(f"- rows audited: `{report.n_rows}`")
    lines.append("")

    lines.append("## Token-count distribution (corpus-wide)")
    lines.append("")
    lines.append("| Statistic | Value |")
    lines.append("| --- | ---: |")
    lines.append(f"| min | {report.min_tokens} |")
    lines.append(f"| p50 | {report.p50} |")
    lines.append(f"| p95 | {report.p95} |")
    lines.append(f"| max | **{report.max_tokens}** |")
    lines.append(
        f"| over budget (> {report.max_length}) | "
        f"{report.n_over_budget} / {report.n_rows} |"
    )
    lines.append("")

    lines.append("## Per-intent breakdown")
    lines.append("")
    lines.append("| Intent | Rows | p50 | p95 | max |")
    lines.append("| --- | ---: | ---: | ---: | ---: |")
    for intent in sorted(report.by_intent):
        vals = report.by_intent[intent]
        srt = sorted(vals)
        p50 = int(statistics.median(srt))
        idx95 = min(len(srt) - 1, max(0, int(round(0.95 * (len(srt) - 1)))))
        p95 = srt[idx95]
        lines.append(
            f"| {intent} | {len(vals)} | {p50} | {p95} | {max(vals)} |"
        )
    lines.append("")

    lines.append("## On-miss action")
    lines.append("")
    if report.passed:
        if report.max_length == DEFAULT_MAX_LENGTH:
            lines.append(
                f"PASS — every row fits inside the "
                f"{report.max_length}-token budget Google's "
                "FunctionGemma fine-tuning recipe pins. The trainer "
                "can use the recipe's default `max_length=512` "
                "without truncating any assistant turn. No action "
                "required."
            )
        else:
            lines.append(
                f"PASS — every row fits inside the "
                f"`max_length={report.max_length}` gate. This is a "
                f"DELIBERATE deviation from Google's FunctionGemma "
                f"recipe default (`max_length={DEFAULT_MAX_LENGTH}`); "
                f"the bump must be recorded in `m5-sft-pipeline.md` "
                f"so the trained checkpoint's context window is "
                f"traceable (per `sft-preflight-gpu.md` §5). The cost "
                f"driver at this corpus shape is the ~18-tool "
                f"inventory (~2700 tokens/row); messages contribute "
                f"a few hundred more."
            )
    else:
        lines.append(
            f"FAIL — {report.n_over_budget} row(s) exceed "
            f"{report.max_length} tokens. The TRL trainer truncates "
            "from the right, dropping assistant turns. Two options "
            "(per `sft-preflight-gpu.md` §5):"
        )
        lines.append("")
        lines.append(
            f"1. **Bump `max_length` to the next power-of-2 above "
            f"{report.max_tokens}** (e.g. 1024 or 2048). H100 VRAM has "
            "headroom; record the decision in `m5-sft-pipeline.md` so "
            "the trained checkpoint's context window is traceable."
        )
        lines.append(
            "2. **Prune the assembled tools array per scenario** so "
            "each record carries only the tools its canonical sequence "
            "calls. Narrows training distribution vs inference — risky; "
            "prefer option 1 unless VRAM forces option 2."
        )
        if report.over_budget_ids:
            lines.append("")
            lines.append(
                "Over-budget scenario IDs: "
                + ", ".join(f"`{i}`" for i in report.over_budget_ids[:20])
                + ("…" if len(report.over_budget_ids) > 20 else "")
            )
    lines.append("")

    out_path.write_text("\n".join(lines))
    return out_path


# ---------------------------------------------------------------------------
# CLI entry point.
# ---------------------------------------------------------------------------


def run_audit_cli(args: Any) -> int:
    train_path = Path(args.train)
    tokenizer_id = args.tokenizer or DEFAULT_TOKENIZER_ID
    max_length = int(args.max_length or DEFAULT_MAX_LENGTH)

    report = audit_token_budget(
        train_path,
        tokenizer_id=tokenizer_id,
        max_length=max_length,
    )

    if args.out:
        out_path = Path(args.out)
    else:
        out_path = train_path.parent.parent / "preflight-5-token-budget.md"
    write_audit_report(out_path, report)

    verdict = "PASS" if report.passed else "FAIL"
    print(
        f"preflight #5 token-budget: {verdict} "
        f"(max={report.max_tokens}, gate=≤{report.max_length}, "
        f"rows={report.n_rows}); report={out_path}"
    )
    return 0 if report.passed else 1


__all__ = [
    "DEFAULT_MAX_LENGTH",
    "DEFAULT_TOKENIZER_ID",
    "TokenBudgetReport",
    "audit_token_budget",
    "run_audit_cli",
    "write_audit_report",
]
