"""W6 CLI entry point (v0 stub — only the ``derive-schemas`` subcommand
ships in PR-1; ``run`` / ``replay`` land with W6).

Usage::

    python -m mili_llm_bench derive-schemas [--out PATH]

Idempotent: regenerates the canonical ``data/posttraining/grammar/
tools.json`` artifact. The honest-diff test in
``tests/test_schemas.py`` fails CI if the checked-in file drifts from
what this command produces.
"""

from __future__ import annotations

import argparse
import sys
from pathlib import Path

from .schemas import default_artifact_path, derive_tools, dump_tools_json


def _cmd_derive_schemas(args: argparse.Namespace) -> int:
    tools = derive_tools()
    out = Path(args.out) if args.out else default_artifact_path()
    out.parent.mkdir(parents=True, exist_ok=True)
    out.write_text(dump_tools_json(tools))
    print(f"wrote {len(tools)} tools to {out}")
    return 0


def main(argv: list[str] | None = None) -> int:
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
    derive.set_defaults(func=_cmd_derive_schemas)

    args = parser.parse_args(argv)
    return args.func(args)


if __name__ == "__main__":
    sys.exit(main())
