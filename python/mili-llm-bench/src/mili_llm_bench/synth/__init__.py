"""Stage 3 scenario synthesis for the M5 SFT pipeline.

Consumes ``data/posttraining/intents/catalog.yaml`` and emits a JSONL
scenario corpus (``data/posttraining/scenarios/synth.jsonl``) in the
same per-line shape as ``data/posttraining/eval/bootstrap.jsonl`` so
the existing harness consumes it unchanged.

Public surface:

* ``run_synth(catalog_path, out_path, seed, target_total, compound_ratio)``
  — orchestrator; called by the CLI ``synth`` subcommand and the
  round-trip test.
* ``SynthReport`` — what ``run_synth`` returns: per-cell counts,
  compound ratio, paraphrase-source breakdown, skipped rows, and the
  fixture-fact confirmation log.

See ``planning/mili-viz/mili-agent/m5-sft-pipeline.md`` Stage 3 row
and ``planning/mili-viz/mili-agent/posttraining-dataset.md`` §3 for
the design context.
"""

from __future__ import annotations

from .run import SynthReport, run_synth

__all__ = ["SynthReport", "run_synth"]
