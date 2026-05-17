"""Phase 3.3 placeholder — ``mili.append_states.AppendStatesTool``.

The input-dictionary-driven multi-state batch tool (upstream
``src/mili/append_states.py``) is **scoped but not implemented this
session** (decision 22 / ``planning/mili-py/phase-3.md`` § Phase 3.3).
This stub exists only so the redirect harness can import + collect
``reference/.../tests/test_append_states_tool.py`` and mark every case
as an honest strict-xfail with a concrete reason — never a silent
pass, never a deleted case. Phase 3.1 landed ``append_state`` /
``copy_non_state_data`` (the primitive ``AppendStatesTool`` will build
on); Phase 3.3 ports the tool itself.
"""

from __future__ import annotations

from typing import Any, Dict

_PHASE_3_3 = (
    "AppendStatesTool: Phase 3.3 — not yet ported "
    "(see planning/mili-py/phase-3.md § Phase 3.3 + m4.md decision 22)"
)


class AppendStatesTool:
    VALID_OUTPUT_TYPES = ["mili"]
    VALID_OUTPUT_MODES = ["write", "append"]

    def __init__(self, input_dictionary: Dict[str, Any]) -> None:
        # Deliberately *not* a MiliPythonError: every
        # test_append_states_tool case (incl. the invalid-input ones
        # that assertRaises(MiliPythonError)) must fail honestly so the
        # strict harness keeps them xfailed with a concrete Phase-3.3
        # reason — never an accidental silent pass of an unported tool.
        raise NotImplementedError(_PHASE_3_3)
