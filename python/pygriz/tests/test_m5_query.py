"""Phase 6 M5 acceptance gate (planning/mili-viz/phase-6-m5.md
Decisions 67–69).

Two halves, the CLAUDE.md / M1 / M2 / M3 skip-on-absent convention:

* **always-on pure logic** — a fake stub captures the emitted
  ``QueryRequest`` and replays a fabricated ``QueryReply``; every
  field on the typed message is asserted (no griz string formatted,
  M3's "no second emitter" invariant), and the ``QueryResult`` →
  ``to_dataframe()`` shape is pinned for both scalar
  (``components==1``) and multi-component returns;
* **skip-on-absent** — the real ``Query`` RPC against a freshly
  launched server with ``serial/basic1``: an unfiltered primal query
  for ``sand[brick]`` returns finite values whose ``to_dataframe()``
  is indexed by states and columned by labels, and a derived-result
  request (``pressure``) surfaces the server's typed ``not yet
  supported`` error as a :class:`griz.QueryError`. Skipped (never
  failed) when ``cargo``/the binary is absent.
"""

from __future__ import annotations

import pytest

from conftest import REPO_ROOT, server_binary

FIXTURE = (
    REPO_ROOT
    / "reference"
    / "mili-python"
    / "tests"
    / "data"
    / "serial"
    / "basic1"
    / "basic1.pltA"
)


# --------------------------------------------------------------------
# Always-on pure logic: Query → typed QueryRequest, no server
# --------------------------------------------------------------------


class _QueryStub:
    """Captures every ``Query(QueryRequest)`` and serves a fabricated
    ``QueryReply``. Mirrors the M3 ``_FakeStub`` shape so the M5 gate
    doesn't depend on a server transport (Decision 67)."""

    def __init__(self, reply=None):
        self.sent = []
        self._reply = reply

    def Query(self, request):  # noqa: N802 - gRPC stub casing
        self.sent.append(request)
        return self._reply


def _ok_inline(labels, states, values, components):
    from griz._proto import mili_viz_pb2 as pb

    reply = pb.QueryReply(ok=True, error="")
    reply.inline.labels.extend(labels)
    reply.inline.states.extend(states)
    reply.inline.values.extend(values)
    reply.inline.components = components
    return reply


def _session(reply=None):
    import griz

    stub = _QueryStub(reply)
    return griz.Session(channel=None, stub=stub, hello=None), stub


def test_query_lowers_to_typed_QueryRequest_with_filled_fields():
    """Decision 67: ``Session.query`` builds the exact typed
    ``QueryRequest`` — never via a griz string (M3's "no second
    emitter" invariant generalizes from ``Command`` to ``Query``)."""
    reply = _ok_inline(
        labels=[1, 2, 3], states=[10, 20], values=[0.0] * 6, components=1
    )
    s, stub = _session(reply)

    qr = s.query(
        "sx", "brick", labels=[1, 2, 3], states=[10, 20], component="eff"
    )
    assert len(stub.sent) == 1
    req = stub.sent[0]
    assert req.result == "sx"
    assert req.class_name == "brick"
    assert list(req.labels) == [1, 2, 3]
    assert list(req.states) == [10, 20]
    assert req.component == "eff"

    # The reply must come back as a QueryResult carrying the request
    # context (so multi-class scripts don't lose which class produced
    # which series) plus the proto's inline payload verbatim.
    import griz

    assert isinstance(qr, griz.QueryResult)
    assert qr.result == "sx" and qr.class_name == "brick"
    assert qr.component == "eff"
    assert qr.labels == [1, 2, 3] and list(qr.states) == [10, 20]
    assert qr.components == 1


def test_query_passes_empty_labels_and_states_verbatim_for_server_fill():
    """The proto contract: empty ``labels`` ⇒ all labels, empty
    ``states`` ⇒ current cursor (``proto/mili_viz.proto:379``). The
    client must forward both empties as-is — never invent a default,
    never reach into the snapshot to fill the cursor (the server is
    the authority on state, M3 Decision 61)."""
    reply = _ok_inline(labels=[42], states=[1], values=[3.14], components=1)
    s, stub = _session(reply)

    s.query("sx", "brick")
    req = stub.sent[0]
    assert list(req.labels) == []
    assert list(req.states) == []
    assert req.component == ""


def test_query_raises_QueryError_on_server_ok_false_with_verbatim_message():
    """Decision 67: the server signals failure with ``ok=false`` + a
    typed ``error`` string (not a transport ``Status``); the client
    surfaces that string verbatim on :class:`QueryError` so callers
    can branch on the message (unknown svar / no run loaded /
    derived-result deferred / out-of-range state)."""
    from griz._proto import mili_viz_pb2 as pb
    import griz

    reply = pb.QueryReply(ok=False, error="query: no run loaded")
    s, _ = _session(reply)
    with pytest.raises(griz.QueryError) as exc:
        s.query("sx", "brick")
    assert str(exc.value) == "query: no run loaded"


def test_query_raises_for_flight_ticket_arm_until_arrow_lands():
    """Decision 68: the proto's ``oneof data { inline; flight_ticket }``
    reserves Flight for the large-result path. pygriz M5 ships only
    the inline arm, so a flight-ticket reply must raise a clear
    :class:`QueryError` — never silently drop the payload."""
    from griz._proto import mili_viz_pb2 as pb
    import griz

    reply = pb.QueryReply(ok=True)
    reply.flight_ticket = b"\x01\x02\x03"
    s, _ = _session(reply)
    with pytest.raises(griz.QueryError) as exc:
        s.query("sx", "brick")
    assert "Flight" in str(exc.value)


def test_database_query_delegates_to_session_query():
    """``scripting.md`` sketch: ``db.query("sx", "brick", labels=[...]
    , states=[...])`` reads more obviously than ``s.query`` once
    ``s.open(...)`` has returned a handle. Both must lower to the
    identical ``QueryRequest`` — the alias is sugar, not a parallel
    path (Decision 69 — single ``Query`` dispatcher, like the single
    ``Command`` dispatcher M3 pinned)."""
    reply = _ok_inline(labels=[7], states=[1], values=[0.5], components=1)
    s, stub = _session(reply)

    # Bypass `s.open` (it would have to spin an Execute round-trip
    # through the same stub; the alias contract is what we're pinning
    # here, not the open flow) — construct the Database handle directly.
    import griz

    db = griz.Database(s, "d3samp6.plt")
    qr = db.query("sx", "brick", labels=[7], states=[1])

    # Exactly one Query went out (no extra dispatcher in Database) and
    # it carries the same typed fields as the equivalent s.query().
    assert len(stub.sent) == 1
    req = stub.sent[0]
    assert req.result == "sx" and req.class_name == "brick"
    assert list(req.labels) == [7] and list(req.states) == [1]
    assert qr.labels == [7] and list(qr.states) == [1]


def test_to_dataframe_scalar_shape_is_states_by_labels():
    """Decision 69: the milox/``mili.utils.query_data_to_dataframe``
    shape — index = states, columns = labels, scalar values flat —
    so viz and analysis ``df`` work the same way the Python oracle's
    do. Pin both axes and a per-cell value to lock the row-major
    ``[state][label]`` interpretation of the proto's ``values``."""
    pd = pytest.importorskip("pandas")

    reply = _ok_inline(
        labels=[10, 20, 30],
        states=[1, 2],
        # row-major [state][label]: s1=[1,2,3], s2=[4,5,6]
        values=[1.0, 2.0, 3.0, 4.0, 5.0, 6.0],
        components=1,
    )
    s, _ = _session(reply)
    df = s.query("sx", "brick").to_dataframe()

    assert list(df.index) == [1, 2]
    assert list(df.columns) == [10, 20, 30]
    # Spot-check the row-major interpretation: (state=2, label=20) is
    # the 5th value, i.e. 5.0.
    assert df.loc[2, 20] == pytest.approx(5.0)
    assert df.loc[1, 10] == pytest.approx(1.0)


def test_to_dataframe_multi_component_uses_per_cell_arrays():
    """Multi-component results (``components > 1``) follow
    mili-python's ``DataFrame.from_records`` arm: each cell holds a
    1-D ``ndarray`` of length ``components`` — preserves the per-
    component vector without collapsing it to a single number."""
    np = pytest.importorskip("numpy")
    pytest.importorskip("pandas")

    reply = _ok_inline(
        labels=[100, 200],
        states=[1],
        # row-major [state][label][component]: l100=[1,2,3], l200=[4,5,6]
        values=[1.0, 2.0, 3.0, 4.0, 5.0, 6.0],
        components=3,
    )
    s, _ = _session(reply)
    qr = s.query("sxx", "brick")
    assert qr.values_3d.shape == (1, 2, 3)
    df = qr.to_dataframe()
    assert list(df.index) == [1]
    assert list(df.columns) == [100, 200]
    np.testing.assert_allclose(df.loc[1, 100], [1.0, 2.0, 3.0])
    np.testing.assert_allclose(df.loc[1, 200], [4.0, 5.0, 6.0])


# --------------------------------------------------------------------
# Skip-on-absent: real Query RPC vs. a freshly launched server
# --------------------------------------------------------------------


@pytest.fixture(scope="module")
def _have_server():
    if server_binary() is None:
        pytest.skip("mili-viz-server binary/cargo unavailable")


@pytest.mark.skipif(not FIXTURE.exists(), reason="serial/basic1 absent")
def test_real_query_returns_finite_values_and_dataframe_shape(
    _have_server, sessions_dir
):
    """Decision 67 end-to-end: ``db.query`` over ``serial/basic1``'s
    ``sand[brick]`` returns finite values for three states; the
    DataFrame is indexed by exactly those three states, columned by
    every brick label, and at least one cell is finite (the M1
    ``Query`` stub returned an empty vec — this leg pins that we are
    now talking to the real `mili-rs` arm landed in
    ``crates/mili-viz-server/tests/query_rpc.rs``)."""
    pytest.importorskip("pandas")
    import math
    import griz

    with griz.launch() as s:
        db = s.open(str(FIXTURE))
        qr = db.query("sand", "brick", states=[1, 2, 3])

    assert qr.components == 1
    assert list(qr.states) == [1, 2, 3]
    assert qr.labels, "brick has at least one element"
    assert len(qr.values) == 3 * len(qr.labels)
    assert any(math.isfinite(v) for v in qr.values)

    df = qr.to_dataframe()
    assert list(df.index) == [1, 2, 3]
    assert list(df.columns) == qr.labels
    assert df.shape == (3, len(qr.labels))


@pytest.mark.skipif(not FIXTURE.exists(), reason="serial/basic1 absent")
def test_real_query_surfaces_derived_result_error_as_QueryError(
    _have_server, sessions_dir
):
    """Decision 67: the server rejects derived results
    (``pressure``/``eff_stress``/...) with ``ok=false`` + a typed
    "not yet supported" message — the geometry-path derived routing
    is the documented forward path (``wireframe-parity.md`` #4
    follow-up). The client must surface that as a
    :class:`QueryError` carrying the verbatim hint."""
    import griz

    with griz.launch() as s:
        s.open(str(FIXTURE))
        with pytest.raises(griz.QueryError) as exc:
            s.query("pressure", "brick", states=[1])

    assert "not yet supported" in str(exc.value)
