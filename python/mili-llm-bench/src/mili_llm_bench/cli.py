"""W6 CLI — ``mili-llm-bench {derive-schemas, run, replay}``.

Three subcommands:

* ``derive-schemas`` — regenerate ``data/posttraining/grammar/
  tools.json`` from the proto (W1's honest-diff seam, surfaced for
  operators).
* ``run`` — drive ``run_eval`` against the chosen ``--provider``;
  writes ``rollouts.jsonl`` + ``summary.json`` + ``config.yaml`` +
  ``report.md`` under ``--out``. The v0 baseline number is published
  from this command's report.
* ``replay`` — re-grade a stored ``rollouts.jsonl`` against the
  current verifier (via the W4a ``ReplayLlmProvider`` seam). Writes a
  fresh rollouts.jsonl + summary.json + report.md to ``--out``;
  round-trip identity on an unchanged verifier, drift detected when
  the verifier is intentionally changed.

Test seam — option (c) from PR-5 review. The factory-building step
lives in **public** functions (``build_factories``, ``build_replay_factories``,
``write_config_yaml``) that tests call directly without going through
``argparse`` — so the always-on test path can plug in a
``FakeDispatcher`` (no pygriz) and a ``MockLlmProvider`` (no LLM)
without hidden ``--dispatcher`` flags or env-var hooks.

Heavy deps (``transformers`` / ``torch`` / ``anthropic`` / ``pygriz``)
are lazy-imported inside the provider/dispatcher factory branches so
``derive-schemas`` runs on a bare-metal Python install and the
always-on tests don't drag in the heavy stack.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import sys
import time
from dataclasses import dataclass, field
from pathlib import Path
from typing import Any, Callable

import yaml

from . import driver, gepa_integration, report
from .driver import EvalConfig, compute_system_prompt_hash, run_eval, run_one_scenario
from .harness import Dispatcher, FakeDispatcher, Registry
from .providers.base import LlmProvider
from .providers.mock import MockLlmProvider
from .providers.replay import ReplayLlmProvider
from .providers.base import ProviderOutput
from .scenarios import Scenario, load_scenarios


# ---------------------------------------------------------------------------
# Utility: Find repo root and normalize relative paths
# ---------------------------------------------------------------------------


def _find_repo_root() -> Path:
    """Find the repository root by looking for .git or CLAUDE.md.

    Returns the first parent directory (walking up from CWD) that contains
    either .git or CLAUDE.md. Falls back to CWD if not found.
    """
    current = Path.cwd()
    for parent in [current, *current.parents]:
        if (parent / ".git").exists() or (parent / "CLAUDE.md").exists():
            return parent
    return current


def _resolve_path(path_str: str) -> Path:
    """Resolve a path, preferring repo root for relative paths.

    If path_str is relative and starts with 'data/', resolve it relative
    to the repo root. Otherwise resolve relative to CWD. Absolute paths
    are returned unchanged.
    """
    path = Path(path_str)
    if path.is_absolute():
        return path

    # If path starts with "data/", resolve from repo root
    if path_str.startswith("data/"):
        return _find_repo_root() / path

    # Otherwise, resolve from CWD
    return path.resolve()

# The five-element closed set the operator sees.
SUPPORTED_PROVIDERS: tuple[str, ...] = ("mock", "replay", "functiongemma", "anthropic", "llamacpp")


# ---------------------------------------------------------------------------
# Factory bundle — explicit data class so tests + production share one
# call signature into ``run_eval``.
# ---------------------------------------------------------------------------


@dataclass
class FactoryBundle:
    """The provider + dispatcher factories ``run_eval`` consumes.

    ``provider_name`` is the human-readable label that lands in the
    rollout record + report headline (e.g. ``"functiongemma"``).
    ``model_id`` is the *pinned* model identifier — for the local
    runtimes this is the HF repo id; for ``mock`` / ``replay`` it's
    the literal provider name. Recorded verbatim in ``config.yaml``.
    """

    provider_factory: Callable[[Scenario], LlmProvider]
    dispatcher_factory: Callable[[Scenario], Dispatcher]
    provider_name: str
    model_id: str


# ---------------------------------------------------------------------------
# Provider/dispatcher factory builders — public so tests skip argparse.
# ---------------------------------------------------------------------------


def _mock_provider_factory(_scenario: Scenario) -> LlmProvider:
    """The ``--provider mock`` factory: one ``final_text="(mock)"`` per
    scenario. The W6 acceptance-gate smoke (baseline.md §"Acceptance
    gate" #3) needs the CLI to run deterministically on a no-GPU
    laptop without a scripted-rollouts file; emitting a single
    final-text turn satisfies that — the rollout grades as parse_error
    / wrong_* per the post-condition, the loop exits cleanly, the four
    output files land. The mock is a *smoke* provider, not an oracle.
    """
    return MockLlmProvider([ProviderOutput(final_text="(mock)")])


def _fake_dispatcher_factory(_scenario: Scenario) -> Dispatcher:
    """The fallback when ``pygriz`` is not installed (always-on tests
    and the ``mock`` smoke). Returns ``ok=True`` for every tool — the
    smoke run never actually reaches dispatch since mock yields
    final_text immediately."""
    return FakeDispatcher(default_response={"ok": True})


def build_factories(
    provider_name: str,
    *,
    config: EvalConfig,
    anthropic_model: str | None = None,
    functiongemma_model: str | None = None,
    functiongemma_revision: str | None = None,
    # Test seams — production callers leave both None.
    provider_factory_override: Callable[[Scenario], LlmProvider] | None = None,
    dispatcher_factory_override: Callable[[Scenario], Dispatcher] | None = None,
) -> FactoryBundle:
    """Build the ``(provider_factory, dispatcher_factory, provider_name,
    model_id)`` bundle the CLI's ``run`` subcommand hands to
    ``run_eval``.

    Public so tests can build the same bundle without going through
    ``argparse`` (option (c) from the PR-5 review). The
    ``*_override`` parameters let tests inject a ``FakeDispatcher`` +
    ``MockLlmProvider`` per scenario without the live pygriz / LLM
    deps.

    The four supported provider names are
    ``{mock, replay, functiongemma, anthropic}`` — anything else
    raises ``ValueError`` so the operator sees the closed-set message
    instead of a downstream traceback. The default dispatcher is the
    pygriz one (the production lowering); when pygriz is unavailable
    or the test seam fires we fall back to the ``FakeDispatcher``.
    """
    if provider_name not in SUPPORTED_PROVIDERS:
        raise ValueError(
            f"unknown provider {provider_name!r}; "
            f"expected one of {SUPPORTED_PROVIDERS}"
        )

    if provider_name == "replay":
        # The ``run`` subcommand cannot drive a ReplayProvider sensibly
        # (no rollouts to replay from); the ``replay`` subcommand has
        # its own factory builder (``build_replay_factories``).
        raise ValueError(
            "build_factories does not support provider='replay'; "
            "use build_replay_factories from the `replay` subcommand instead."
        )

    if provider_factory_override is not None:
        provider_factory = provider_factory_override
        model_id = "test-override"
    elif provider_name == "mock":
        provider_factory = _mock_provider_factory
        model_id = "mock"
    elif provider_name == "functiongemma":
        from .providers.functiongemma import (  # lazy
            DEFAULT_MODEL_ID as FG_DEFAULT_ID,
            FunctionGemmaProvider,
        )
        chosen_model = functiongemma_model or FG_DEFAULT_ID
        chosen_revision = functiongemma_revision
        # One provider per run — the model weights are loaded once and
        # reused across scenarios.
        provider = FunctionGemmaProvider(
            model_id=chosen_model,
            revision=chosen_revision,
        )
        provider_factory = lambda _s: provider  # noqa: E731
        model_id = chosen_model
    elif provider_name == "anthropic":
        from .providers.anthropic import (  # lazy
            DEFAULT_MODEL_ID as ANT_DEFAULT_ID,
            AnthropicProvider,
        )
        chosen_model = anthropic_model or ANT_DEFAULT_ID
        provider = AnthropicProvider(model=chosen_model)
        provider_factory = lambda _s: provider  # noqa: E731
        model_id = chosen_model
    elif provider_name == "llamacpp":
        from .providers.llamacpp import (  # lazy
            DEFAULT_MODEL_ID as LLAMACPP_DEFAULT_ID,
            LlamaCppProvider,
        )
        # One provider per run — the server stays alive across scenarios.
        provider = LlamaCppProvider()
        provider_factory = lambda _s: provider  # noqa: E731
        model_id = LLAMACPP_DEFAULT_ID
    else:
        # Unreachable — guarded by the SUPPORTED_PROVIDERS check above.
        raise ValueError(f"unsupported provider {provider_name!r}")

    if dispatcher_factory_override is not None:
        dispatcher_factory = dispatcher_factory_override
    else:
        dispatcher_factory = _resolve_default_dispatcher_factory()

    return FactoryBundle(
        provider_factory=provider_factory,
        dispatcher_factory=dispatcher_factory,
        provider_name=provider_name,
        model_id=model_id,
    )


def _resolve_default_dispatcher_factory() -> Callable[[Scenario], Dispatcher]:
    """Pick the production dispatcher factory when pygriz is installed;
    otherwise fall back to ``FakeDispatcher`` so the ``--provider mock``
    smoke (baseline.md §"Acceptance gate" #3) runs on a no-GPU laptop
    without the optional dep. Probes the ``griz`` import eagerly so the
    fallback fires at factory-build time, not deep inside ``run_eval``.
    """
    try:
        import griz  # type: ignore[import-not-found]  # noqa: F401
    except ImportError:
        return _fake_dispatcher_factory
    from .dispatchers.pygriz import pygriz_dispatcher_factory  # lazy

    return pygriz_dispatcher_factory()


def build_replay_factories(
    rollouts_path: Path,
    *,
    dispatcher_factory_override: Callable[[Scenario], Dispatcher] | None = None,
) -> FactoryBundle:
    """Build the factory bundle the ``replay`` subcommand hands to its
    per-scenario driver loop.

    The provider is a fresh ``ReplayLlmProvider`` per scenario, keyed
    on the scenario id — exhaustion mid-loop raises (clear signal that
    the live verifier or dispatcher is driving past the recording).
    The default dispatcher is the pygriz one; tests override with a
    ``FakeDispatcher``.
    """

    def provider_factory(scenario: Scenario) -> LlmProvider:
        return ReplayLlmProvider(
            rollouts_path=Path(rollouts_path),
            scenario_id=scenario.id,
        )

    if dispatcher_factory_override is not None:
        dispatcher_factory = dispatcher_factory_override
    else:
        dispatcher_factory = _resolve_default_dispatcher_factory()

    return FactoryBundle(
        provider_factory=provider_factory,
        dispatcher_factory=dispatcher_factory,
        provider_name="replay",
        model_id="replay",
    )


# ---------------------------------------------------------------------------
# config.yaml writer — the falsifiability artifact.
# ---------------------------------------------------------------------------


def _sha256_file(path: Path) -> str:
    h = hashlib.sha256()
    with Path(path).open("rb") as f:
        for chunk in iter(lambda: f.read(65536), b""):
            h.update(chunk)
    return h.hexdigest()


def _sha256_text(text: str) -> str:
    return hashlib.sha256(text.encode("utf-8")).hexdigest()


def write_config_yaml(
    out_path: Path,
    *,
    config: EvalConfig,
    provider_name: str,
    model_id: str,
    scenarios_sha256: str,
    tools_sha256: str,
    bench_version: str,
    run_timestamp: float | None = None,
) -> dict[str, Any]:
    """Write ``config.yaml`` with every pin the report headline references.

    Without ``config.yaml`` the report number is unfalsifiable (per
    baseline.md §"Acceptance gate" #5). The fields here are the
    minimum set that lets a third party reproduce — or refute — the
    run.

    Returns the dict written (for tests).
    """
    payload: dict[str, Any] = {
        "bench_version": bench_version,
        "provider": provider_name,
        "model_id": model_id,
        "system_prompt_sha256": compute_system_prompt_hash(
            config.system_prompt, prefix_len=64
        ),
        "tools_sha256": tools_sha256,
        "scenarios_sha256": scenarios_sha256,
        "step_cap": config.step_cap,
        "max_new_tokens": config.max_new_tokens,
        "temperature": config.temperature,
        "seed": config.seed,
        "per_turn_timeout_s": config.per_turn_timeout_s,
        "run_timestamp": float(run_timestamp if run_timestamp is not None else time.time()),
    }
    Path(out_path).write_text(yaml.safe_dump(payload, sort_keys=True))
    return payload


# ---------------------------------------------------------------------------
# Subcommand handlers.
# ---------------------------------------------------------------------------


def _cmd_derive_schemas(args: argparse.Namespace) -> int:
    from .schemas import default_artifact_path, derive_tools, dump_tools_json

    tools = derive_tools()
    text = dump_tools_json(tools)

    if args.check:
        pinned = default_artifact_path().read_text()
        if pinned != text:
            sys.stderr.write(
                "derive-schemas --check: tools.json drift detected. "
                "Re-run `mili-llm-bench derive-schemas` to regenerate.\n"
            )
            return 1
        print("derive-schemas --check: ok")
        return 0

    out = Path(args.out) if args.out else default_artifact_path()
    out.parent.mkdir(parents=True, exist_ok=True)
    out.write_text(text)
    print(f"wrote {len(tools)} tools to {out}")
    return 0


def _resolve_eval_config(args: argparse.Namespace) -> EvalConfig:
    """Build an ``EvalConfig`` from the parsed CLI args, honoring
    the baseline-pinned defaults when a flag is unset."""
    base = EvalConfig()
    # ``--temperature`` lives on the run subcommand only; replay /
    # synth never expose it. ``getattr`` with the pinned default
    # keeps both code paths shape-compatible.
    temperature = getattr(args, "temperature", None)
    return EvalConfig(
        step_cap=args.step_cap if args.step_cap is not None else base.step_cap,
        max_new_tokens=(
            args.max_new_tokens
            if args.max_new_tokens is not None
            else base.max_new_tokens
        ),
        temperature=temperature if temperature is not None else base.temperature,
        seed=args.seed if args.seed is not None else base.seed,
        per_turn_timeout_s=(
            args.per_turn_timeout_s
            if args.per_turn_timeout_s is not None
            else base.per_turn_timeout_s
        ),
        system_prompt=base.system_prompt,
    )


def _cmd_run(args: argparse.Namespace) -> int:
    from . import __version__ as bench_version

    # Resolve paths relative to repo root for consistency
    scenarios_path = _resolve_path(args.scenarios)
    tools_path = _resolve_path(args.tools) if args.tools else None
    out_dir = _resolve_path(args.out)

    scenarios = load_scenarios(scenarios_path)
    # ``--limit N`` caps the run to the first N scenarios so the Stage-5
    # pilot (50 of 175) stays under the budget gate; applied here so the
    # cost telemetry + retention rate reflect the *capped* sweep, not
    # the full corpus. K=1 bench-as-eval still uses this; absent flag =
    # no cap.
    if getattr(args, "limit", None) is not None and args.limit > 0:
        scenarios = scenarios[: args.limit]
    if tools_path is not None:
        registry = Registry.load_from_artifact(tools_path)
    else:
        registry = Registry.load_from_artifact()
        tools_path = Path(_default_tools_path())
    tool_list = registry.all()

    config = _resolve_eval_config(args)
    bundle = build_factories(
        args.provider,
        config=config,
        anthropic_model=args.anthropic_model,
        functiongemma_model=args.functiongemma_model,
        functiongemma_revision=args.functiongemma_revision,
    )

    k = int(getattr(args, "k", 1) or 1)
    retain = getattr(args, "retain", "all") or "all"
    if k > 1 and config.temperature == 0.0 and args.provider == "anthropic":
        sys.stderr.write(
            f"warning: --k {k} against anthropic with --temperature 0.0 will "
            "produce K identical rollouts (the Anthropic API does not honor "
            "a seed parameter; only temperature creates per-pass diversity). "
            "Consider --temperature 0.7 for the Stage 5 teacher-rollout pilot.\n"
        )

    out_dir.mkdir(parents=True, exist_ok=True)

    # config.yaml — falsifiability artifact. Compute the hashes first
    # so a missing/bad scenarios file fails before we burn a model call.
    scenarios_sha256 = _sha256_file(Path(args.scenarios))
    tools_sha256 = _sha256_file(tools_path)
    write_config_yaml(
        out_dir / "config.yaml",
        config=config,
        provider_name=bundle.provider_name,
        model_id=bundle.model_id,
        scenarios_sha256=scenarios_sha256,
        tools_sha256=tools_sha256,
        bench_version=bench_version,
    )

    try:
        summary = run_eval(
            scenarios,
            provider_factory=bundle.provider_factory,
            dispatcher_factory=bundle.dispatcher_factory,
            config=config,
            out_dir=out_dir,
            provider_name=bundle.provider_name,
            registry=registry,
            tools=tool_list,
            k=k,
            retain=retain,
            model_id=bundle.model_id,
        )
    except Exception as exc:
        sys.stderr.write(f"run failed: {exc!r}\n")
        return 1

    report.write_report(
        out_dir / "report.md",
        summary,
        out_dir / "rollouts.jsonl",
        config_yaml_path=out_dir / "config.yaml",
        summary_path=out_dir / "summary.json",
        provider_name=bundle.provider_name,
        model_id=bundle.model_id,
        scenarios_sha256=scenarios_sha256,
    )

    msg = (
        f"run complete: L3 pass rate {summary['l3_pass_rate']:.3f} "
        f"({summary['by_max_tier'].get('3', 0)}/{summary['total']})"
    )
    if k > 1:
        msg += (
            f"; retention {summary.get('scenarios_retained', 0)}/"
            f"{summary.get('scenarios_total', 0)} "
            f"({summary.get('retention_rate', 0.0):.1%})"
        )
    if "cost_estimate_usd" in summary:
        msg += f"; cost ${summary['cost_estimate_usd']:.2f}"
    msg += f"; see {out_dir / 'report.md'}"
    print(msg)
    return 0


def _cmd_replay(args: argparse.Namespace) -> int:
    """Re-grade a stored ``rollouts.jsonl`` under the current verifier.

    Self-contained: each ``rollouts.jsonl`` record carries the
    scenario id / fixture / intent_id / instruction / postcondition,
    so we reconstruct a ``Scenario`` from the record rather than
    requiring a separate bootstrap file. The round-trip identity
    pin: a replay with an unchanged verifier reproduces the original
    ``max_tier`` per scenario.
    """
    from . import __version__ as bench_version

    # Resolve paths relative to repo root for consistency
    in_path = _resolve_path(args.rollouts)
    out_dir = _resolve_path(args.out)
    tools_path = _resolve_path(args.tools) if args.tools else None

    out_dir.mkdir(parents=True, exist_ok=True)

    # Reconstruct scenarios from the stored records.
    scenarios = _scenarios_from_rollouts(in_path)

    if tools_path is not None:
        registry = Registry.load_from_artifact(tools_path)
    else:
        registry = Registry.load_from_artifact()
        tools_path = Path(_default_tools_path())
    tool_list = registry.all()

    config = _resolve_eval_config(args)
    bundle = build_replay_factories(in_path)

    scenarios_sha256 = _sha256_file(in_path)
    tools_sha256 = _sha256_file(tools_path)
    write_config_yaml(
        out_dir / "config.yaml",
        config=config,
        provider_name=bundle.provider_name,
        model_id=bundle.model_id,
        scenarios_sha256=scenarios_sha256,
        tools_sha256=tools_sha256,
        bench_version=bench_version,
    )

    try:
        summary = run_eval(
            scenarios,
            provider_factory=bundle.provider_factory,
            dispatcher_factory=bundle.dispatcher_factory,
            config=config,
            out_dir=out_dir,
            provider_name=bundle.provider_name,
            registry=registry,
            tools=tool_list,
        )
    except Exception as exc:
        sys.stderr.write(f"replay failed: {exc!r}\n")
        return 1

    report.write_report(
        out_dir / "report.md",
        summary,
        out_dir / "rollouts.jsonl",
        config_yaml_path=out_dir / "config.yaml",
        summary_path=out_dir / "summary.json",
        provider_name=bundle.provider_name,
        model_id=bundle.model_id,
        scenarios_sha256=scenarios_sha256,
    )
    print(
        f"replay complete: L3 pass rate {summary['l3_pass_rate']:.3f} "
        f"({summary['by_max_tier'].get('3', 0)}/{summary['total']})"
    )
    return 0


def _scenarios_from_rollouts(rollouts_path: Path) -> list[Scenario]:
    """Reconstruct ``Scenario`` objects from a ``rollouts.jsonl``.

    Self-contained replay (PR-5 spec) — every record carries every
    field the W4b driver needs to re-run the loop.
    """
    from .scenarios import Postcondition

    out: list[Scenario] = []
    with Path(rollouts_path).open() as f:
        for lineno, line in enumerate(f, start=1):
            line = line.strip()
            if not line:
                continue
            obj = json.loads(line)
            verifier_meta = obj.get("verifier") or {}
            pc = verifier_meta.get("postcondition") or {}
            kind = pc.get("kind")
            if kind is None:
                raise ValueError(
                    f"{rollouts_path}:{lineno}: rollout record missing "
                    "verifier.postcondition.kind"
                )
            out.append(
                Scenario(
                    id=obj["id"],
                    fixture=obj["fixture"],
                    intent_id=obj.get("intent_id", "unknown"),
                    instruction=obj.get("instruction", ""),
                    postcondition=Postcondition(
                        kind=kind, expect=dict(pc.get("expect", {}))
                    ),
                )
            )
    return out


def _default_tools_path() -> Path:
    from .schemas import default_artifact_path

    return default_artifact_path()


def _cmd_synth(args: argparse.Namespace) -> int:
    """Stage 3 of the M5 SFT pipeline.

    Reads ``data/posttraining/intents/catalog.yaml`` and writes a
    JSONL scenario corpus + Markdown report to ``--out``. Login-node
    safe; no GPU / no Anthropic API. See
    ``planning/mili-viz/mili-agent/m5-sft-pipeline.md`` Stage 3 row.
    """
    from .synth import run_synth

    catalog_path = _resolve_path(args.catalog)
    out_path = _resolve_path(args.out)
    report_path = (
        _resolve_path(args.report) if args.report else out_path.with_suffix(".report.md")
    )

    try:
        report = run_synth(
            catalog_path=catalog_path,
            out_path=out_path,
            report_path=report_path,
            seed=args.seed if args.seed is not None else 42,
            target_total=args.target_total,
            compound_ratio=args.compound_ratio,
            confirm_fixtures=not args.no_confirm_fixtures,
        )
    except Exception as exc:
        sys.stderr.write(f"synth failed: {exc!r}\n")
        return 1

    print(
        f"synth complete: {report.total} scenarios "
        f"({report.compound_count} compound, "
        f"ratio {report.compound_ratio:.2%}); "
        f"wrote {out_path} + {report_path}"
    )
    if report.skipped:
        print(f"  skipped {len(report.skipped)} rows; see report for details")
    return 0


def _cmd_run_gepa(args: argparse.Namespace) -> int:
    """Run GEPA optimization loop on artifact (prompt + step_cap + tools).

    Wraps gepa_integration.run_gepa_optimization to propose and evaluate
    artifact variants via GEPA, including system prompt, step_cap, and
    tool descriptions. Automatically finds and seeds from the most recent
    previous run (unless --seed-artifact-dir is explicitly provided).

    Output directories are timestamped (gepa-run-YYYYMMDD-HHMMSS) for
    automatic discovery of previous runs.
    """
    try:
        # Resolve paths relative to repo root for consistency
        scenarios_path = _resolve_path(args.scenarios)
        output_path = _resolve_path(args.out)
        seed_path = _resolve_path(args.seed_artifact_dir) if args.seed_artifact_dir else None

        # Auto-generate timestamped output dir if user didn't provide explicit path
        if not output_path.name.startswith("gepa-run-"):
            # User provided a base directory; generate timestamped subdir
            from datetime import datetime
            timestamp = datetime.now().strftime("%Y%m%d-%H%M%S")
            output_path = output_path / f"gepa-run-{timestamp}"

        config = gepa_integration.GepaRunConfig(
            dataset_path=scenarios_path,
            output_dir=output_path,
            provider_name=args.provider,
            num_scenarios=args.num_scenarios,
            artifact_mode="config",  # Optimizes system_prompt, step_cap, tools
            max_iterations=args.max_iterations,
            gepa_engine=args.gepa_engine,
            gepa_reflection=args.gepa_reflection,
            seed_artifact_dir=seed_path,  # None = auto-discover
        )

        result = gepa_integration.run_gepa_optimization(config)

        print(
            f"GEPA optimization complete: best score {result['best_score']:.3f}"
        )
        print(f"Results saved to: {output_path}")
        if isinstance(result["best_artifact"], dict) and "tools" in result["best_artifact"]:
            print(f"\n✓ Best tools saved to best_tools.json")
            print(f"  Next run will auto-discover these tools and iterate further.")
        return 0

    except ImportError as e:
        sys.stderr.write(
            f"GEPA library not installed: {e}\n"
            "Install with: pip install gepa\n"
        )
        return 1
    except Exception as exc:
        import traceback
        sys.stderr.write(f"run-gepa failed: {exc!r}\n")
        traceback.print_exc(file=sys.stderr)
        return 1


# ---------------------------------------------------------------------------
# argparse plumbing.
# ---------------------------------------------------------------------------


def _add_run_common_flags(parser: argparse.ArgumentParser) -> None:
    parser.add_argument(
        "--tools",
        default=None,
        help=(
            "Override the tools.json path "
            "(defaults to data/posttraining/grammar/tools.json)."
        ),
    )
    parser.add_argument(
        "--step-cap",
        type=int,
        default=None,
        help="Max turns per scenario (default: 8).",
    )
    parser.add_argument(
        "--per-turn-timeout-s",
        type=float,
        default=None,
        help="Per-turn wall-clock timeout (default: 60).",
    )
    parser.add_argument(
        "--max-new-tokens",
        type=int,
        default=None,
        help="Max new tokens per provider call (default: 256).",
    )
    parser.add_argument(
        "--seed", type=int, default=None, help="Provider RNG seed (default: 0)."
    )


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(prog="mili-llm-bench")
    subs = parser.add_subparsers(dest="cmd", required=True)

    derive = subs.add_parser(
        "derive-schemas",
        help="Regenerate data/posttraining/grammar/tools.json from the proto.",
    )
    derive.add_argument(
        "--out",
        default=None,
        help="Override output path (defaults to data/posttraining/grammar/tools.json).",
    )
    derive.add_argument(
        "--check",
        action="store_true",
        help="Diff regenerated content vs pinned file; exit nonzero on drift.",
    )
    derive.set_defaults(func=_cmd_derive_schemas)

    run = subs.add_parser(
        "run",
        help=(
            "Run the bench against a provider; emits "
            "rollouts.jsonl + summary.json + config.yaml + report.md. "
            f"Providers: {', '.join(SUPPORTED_PROVIDERS)}."
        ),
    )
    run.add_argument(
        "--provider",
        choices=list(SUPPORTED_PROVIDERS),
        required=True,
        help=f"One of: {', '.join(SUPPORTED_PROVIDERS)}.",
    )
    run.add_argument("--scenarios", required=True, help="Path to scenarios JSONL.")
    run.add_argument("--out", required=True, help="Output directory for run artifacts.")
    run.add_argument(
        "--anthropic-model",
        default=None,
        help="Pin a specific Claude model id (defaults to the baseline pin).",
    )
    run.add_argument(
        "--functiongemma-model",
        default=None,
        help="Pin a specific FunctionGemma HF repo id (defaults to the baseline pin).",
    )
    run.add_argument(
        "--functiongemma-revision",
        default=None,
        help="Pin a specific HF revision/commit for the FunctionGemma model.",
    )
    run.add_argument(
        "--limit",
        type=int,
        default=None,
        help=(
            "Cap the run to the first N scenarios. Used by the Stage 5 "
            "pilot (--limit 50 against the 175-row synth.jsonl) to stay "
            "under the $50 budget gate."
        ),
    )
    run.add_argument(
        "--k",
        type=int,
        default=1,
        help=(
            "Number of rollouts per scenario (Stage 5 teacher-rollout "
            "fan-out). K>1 writes K rollouts per scenario into "
            "rollouts.jsonl with a k_idx + retained field; per-pass "
            "seed = config.seed + k_idx. Default 1 (bench-as-eval)."
        ),
    )
    run.add_argument(
        "--retain",
        choices=["all", "passing"],
        default="all",
        help=(
            "Stage 5 retention filter. 'passing' marks only L3 rollouts "
            "as retained=true (the Stage 6 SFT-corpus filter key); 'all' "
            "marks every rollout retained=true. Default 'all'."
        ),
    )
    run.add_argument(
        "--temperature",
        type=float,
        default=None,
        help=(
            "Sampling temperature (default 0.0). Stage 5 K=3 pilots "
            "against Anthropic require temperature > 0 to produce "
            "diverse rollouts (the API does not honor a seed parameter)."
        ),
    )
    _add_run_common_flags(run)
    run.set_defaults(func=_cmd_run)

    replay = subs.add_parser(
        "replay",
        help=(
            "Re-grade a stored rollouts.jsonl under the current verifier. "
            "Self-contained — each record carries its own scenario fields."
        ),
    )
    replay.add_argument("--rollouts", required=True, help="Path to stored rollouts.jsonl.")
    replay.add_argument("--out", required=True, help="Output directory for re-graded artifacts.")
    _add_run_common_flags(replay)
    replay.set_defaults(func=_cmd_replay)

    synth = subs.add_parser(
        "synth",
        help=(
            "Stage 3 scenario synthesis. Reads data/posttraining/intents/"
            "catalog.yaml; writes data/posttraining/scenarios/synth.jsonl "
            "+ synth.report.md. Login-node safe."
        ),
    )
    synth.add_argument(
        "--catalog",
        default="data/posttraining/intents/catalog.yaml",
        help="Path to the intent catalog (default: data/posttraining/intents/catalog.yaml).",
    )
    synth.add_argument(
        "--out",
        default="data/posttraining/scenarios/synth.jsonl",
        help="Output JSONL path (default: data/posttraining/scenarios/synth.jsonl).",
    )
    synth.add_argument(
        "--report",
        default=None,
        help="Output report path (default: <out>.report.md).",
    )
    synth.add_argument(
        "--seed",
        type=int,
        default=None,
        help="Sampler RNG seed (default: 42).",
    )
    synth.add_argument(
        "--target-total",
        type=int,
        default=200,
        help="Informational target for total scenario count (default: 200).",
    )
    synth.add_argument(
        "--compound-ratio",
        type=float,
        default=0.20,
        help="Minimum compound ratio gate (default: 0.20).",
    )
    synth.add_argument(
        "--no-confirm-fixtures",
        action="store_true",
        help=(
            "Skip the pygriz fixture-fact confirmation pass. Use this "
            "when pygriz is not installed or mili-viz-server isn't built."
        ),
    )
    synth.set_defaults(func=_cmd_synth)

    gepa = subs.add_parser(
        "run-gepa",
        help=(
            "Run GEPA optimization loop: system prompt + step_cap + tool definitions. "
            "Auto-seeds from most recent previous run for continuous improvement."
        ),
    )
    gepa.add_argument(
        "--scenarios",
        required=True,
        help="Path to scenarios JSONL (e.g., data/posttraining/eval/bootstrap.jsonl).",
    )
    gepa.add_argument(
        "--out",
        required=True,
        help=(
            "Output directory or base path. If not a gepa-run-* path, "
            "a timestamped subdirectory (gepa-run-YYYYMMDD-HHMMSS) is auto-created. "
            "Example: --out data/posttraining/gepa-runs"
        ),
    )
    gepa.add_argument(
        "--provider",
        choices=["llamacpp", "anthropic"],
        default="llamacpp",
        help="LLM provider for evaluation (default: llamacpp).",
    )
    gepa.add_argument(
        "--num-scenarios",
        type=int,
        default=None,
        help="Limit evaluation to N scenarios (for faster iteration; default: all).",
    )
    gepa.add_argument(
        "--max-iterations",
        type=int,
        default=5,
        help="Max GEPA optimization iterations (default: 5).",
    )
    gepa.add_argument(
        "--gepa-engine",
        default="claude-opus-4-7",
        help="GEPA proposer model ID (default: claude-opus-4-7).",
    )
    gepa.add_argument(
        "--gepa-reflection",
        choices=["shallow", "medium", "deep"],
        default="medium",
        help="GEPA reflection depth (default: medium).",
    )
    gepa.add_argument(
        "--seed-artifact-dir",
        default=None,
        help=(
            "Optional: Path to a specific GEPA run to seed from. "
            "If omitted, auto-discovers the most recent run in the same parent directory. "
            "(default: None — auto-discover, fall back to baseline tools)"
        ),
    )
    gepa.set_defaults(func=_cmd_run_gepa)

    return parser


def main(argv: list[str] | None = None) -> int:
    parser = build_parser()
    args = parser.parse_args(argv)
    return args.func(args)


if __name__ == "__main__":
    sys.exit(main())


__all__ = [
    "FactoryBundle",
    "SUPPORTED_PROVIDERS",
    "build_factories",
    "build_parser",
    "build_replay_factories",
    "main",
    "write_config_yaml",
]
