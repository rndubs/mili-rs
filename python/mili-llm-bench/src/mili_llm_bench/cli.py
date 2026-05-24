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

from . import driver, report
from .driver import EvalConfig, compute_system_prompt_hash, run_eval, run_one_scenario
from .harness import Dispatcher, FakeDispatcher, Registry
from .providers.base import LlmProvider
from .providers.mock import MockLlmProvider
from .providers.replay import ReplayLlmProvider
from .providers.base import ProviderOutput
from .scenarios import Scenario, load_scenarios

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
    return EvalConfig(
        step_cap=args.step_cap if args.step_cap is not None else base.step_cap,
        max_new_tokens=(
            args.max_new_tokens
            if args.max_new_tokens is not None
            else base.max_new_tokens
        ),
        temperature=base.temperature,
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

    scenarios = load_scenarios(args.scenarios)
    tools_path = Path(args.tools) if args.tools else None
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

    out_dir = Path(args.out)
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

    print(
        f"run complete: L3 pass rate {summary['l3_pass_rate']:.3f} "
        f"({summary['by_max_tier'].get('3', 0)}/{summary['total']}); "
        f"see {out_dir / 'report.md'}"
    )
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

    in_path = Path(args.rollouts)
    out_dir = Path(args.out)
    out_dir.mkdir(parents=True, exist_ok=True)

    # Reconstruct scenarios from the stored records.
    scenarios = _scenarios_from_rollouts(in_path)

    tools_path = Path(args.tools) if args.tools else None
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
