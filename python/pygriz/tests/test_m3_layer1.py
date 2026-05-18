"""Phase 6 M3 acceptance gate (planning/mili-viz/phase-6-m3.md
§ "M3 acceptance gate", Decisions 59–61).

Two halves, the CLAUDE.md / M1 / M2 skip-on-absent convention:

* **always-on pure logic** — a fake stub captures the emitted
  ``Command``; every Layer-1 call is asserted to lower to the exact
  *typed* oneof variant + fields (Decision 59: never the ``raw`` arm),
  and the server-authoritative reads (``Result.range``, ``s.state``)
  are exercised against a fabricated ``Subscribe`` snapshot (no server);
* **skip-on-absent** — the **Layer-0 ≡ Layer-1** equivalence: each
  representative Layer-1 call and its hand-written equivalent Layer-0
  ``s.command("<griz line>")`` are applied to two freshly-launched real
  ``mili-viz-server`` processes; the authoritative session state read
  from the opening ``DELTA_SNAPSHOT`` must be identical (Decision 60).
  Skipped (never failed) when ``cargo``/the binary is absent.
"""

from __future__ import annotations

import pathlib

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
# Always-on pure logic: Layer-1 -> exact typed Command (no server)
# --------------------------------------------------------------------


class _FakeStream:
    def __init__(self, deltas):
        self._it = iter(deltas)
        self.cancelled = False

    def __iter__(self):
        return self._it

    def cancel(self):
        self.cancelled = True


class _FakeStub:
    """Captures every ``Execute(Command)`` and serves a fabricated
    opening snapshot for the one-shot ``Subscribe`` reads (Decision
    61) — so the whole Layer-1 surface is pinned with no server."""

    def __init__(self, snapshot=None):
        self.sent = []
        self._snapshot = snapshot

    def Execute(self, command):  # noqa: N802 - gRPC stub casing
        self.sent.append(command)
        return object()

    def Subscribe(self, request):  # noqa: N802 - gRPC stub casing
        from griz._proto import mili_viz_pb2 as pb

        delta = pb.StateDelta(kind=pb.DELTA_SNAPSHOT)
        if self._snapshot is not None:
            delta.snapshot.CopyFrom(self._snapshot)
        return _FakeStream([delta])


def _session(snapshot=None):
    import griz

    stub = _FakeStub(snapshot)
    return griz.Session(channel=None, stub=stub, hello=None), stub


def _last(stub):
    assert stub.sent, "no Command emitted"
    return stub.sent[-1]


def test_scene_calls_lower_to_typed_variants():
    """Decision 59: every scene Layer-1 call builds the exact typed
    ``Command`` oneof — and *never* the ``raw`` arm (that stays the
    Layer-0 escape hatch)."""
    import griz

    s, stub = _session()

    db = s.open("d3samp6.plt")
    c = _last(stub)
    assert c.WhichOneof("cmd") == "load" and c.load.root == "d3samp6.plt"
    assert isinstance(db, griz.Database) and db.root == "d3samp6.plt"

    s.state = 10
    c = _last(stub)
    assert c.WhichOneof("cmd") == "set_state" and c.set_state.state == 10

    from griz._proto import mili_viz_pb2 as pb

    for meth, want in [
        (s.next, pb.Step.NEXT),
        (s.prev, pb.Step.PREV),
        (s.first, pb.Step.FIRST),
        (s.last, pb.Step.LAST),
    ]:
        meth()
        c = _last(stub)
        assert c.WhichOneof("cmd") == "step" and c.step.dir == want

    s.select("brick", "1-100,150")
    c = _last(stub)
    assert c.WhichOneof("cmd") == "select"
    assert c.select.class_name == "brick" and c.select.range == "1-100,150"

    s.selection.clear("brick")
    c = _last(stub)
    assert c.WhichOneof("cmd") == "clrsel" and c.clrsel.class_name == "brick"

    r = s.show("stress", "eff", scale="log")
    c = _last(stub)
    assert c.WhichOneof("cmd") == "show"
    assert c.show.result == "stress" and c.show.component == "eff"
    assert c.show.opts["scale"] == "log"
    assert isinstance(r, griz.Result) and r.result == "stress"

    iso = s.isosurface("sx", levels=[1e4, 2e4])
    c = _last(stub)
    assert c.WhichOneof("cmd") == "iso"
    assert c.iso.result == "sx" and c.iso.on is True
    assert list(c.iso.levels) == [1e4, 2e4]
    assert isinstance(iso, griz.Isosurface)
    iso.remove()
    c = _last(stub)
    assert c.WhichOneof("cmd") == "iso" and c.iso.on is False
    assert c.iso.result == "sx"

    s.isosurface("sx", count=5, vmin=0.0, vmax=1.0)
    c = _last(stub)
    assert c.iso.count == 5 and c.iso.min == 0.0 and c.iso.max == 1.0

    con = s.contour("sx", count=3)
    c = _last(stub)
    assert c.WhichOneof("cmd") == "contour"
    assert c.contour.result == "sx" and c.contour.count == 3
    assert isinstance(con, griz.Contour) and con.count == 3

    s.materials.disable(3)
    c = _last(stub)
    assert c.WhichOneof("cmd") == "material"
    assert c.material.enable is False and c.material.class_name == ""
    assert c.material.material == 3

    s.materials.enable("brick", mat=2)
    c = _last(stub)
    assert c.material.enable is True and c.material.class_name == "brick"
    assert c.material.material == 2

    s.cutplane(origin=(1, 2, 3), normal=(0, 1, 0), relative=True)
    c = _last(stub)
    assert c.WhichOneof("cmd") == "cutplane"
    assert (c.cutplane.ox, c.cutplane.oy, c.cutplane.oz) == (1, 2, 3)
    assert (c.cutplane.nx, c.cutplane.ny, c.cutplane.nz) == (0, 1, 0)
    assert c.cutplane.relative is True

    s.colormap("cool")
    c = _last(stub)
    assert c.WhichOneof("cmd") == "colormap" and c.colormap.name == "cool"

    s.legend.limits = (0.0, 5e4)
    c = _last(stub)
    assert c.WhichOneof("cmd") == "legend"
    assert c.legend.min == 0.0 and c.legend.max == 5e4

    s.legend.limits = (None, 7.0)
    c = _last(stub)
    assert not c.legend.HasField("min"), "None bound must autoscale"
    assert c.legend.max == 7.0

    # No Layer-1 call ever used the raw arm (Decision 59).
    assert all(
        cmd.WhichOneof("cmd") != "raw" for cmd in stub.sent
    ), "Layer-1 must never lower to Command.raw"


def test_view_calls_lower_to_typed_variants():
    """Server-authoritative view (Decision 61): ``s.view.*`` *emits*
    the typed ``View``/``NamedView`` — it does not predict."""
    s, stub = _session()
    from griz._proto import mili_viz_pb2 as pb

    s.view.rotate(x=30, y=15)
    c = _last(stub)
    assert c.WhichOneof("cmd") == "view" and c.view.WhichOneof("op") == "rotate"
    assert c.view.rotate.x == 30 and c.view.rotate.y == 15
    assert c.view.rotate.z == 0

    s.view.translate(0.1, 0, 0)
    c = _last(stub)
    assert c.view.WhichOneof("op") == "translate"
    assert c.view.translate.dx == pytest.approx(0.1)

    s.view.scale(2.0)
    c = _last(stub)
    assert c.view.WhichOneof("op") == "scale" and c.view.scale.factor == 2.0

    s.view.zoom(1.5)
    c = _last(stub)
    assert c.view.WhichOneof("op") == "zoom" and c.view.zoom.factor == 1.5

    s.view.set(azimuth=45, elevation=20, distance=3.0)
    c = _last(stub)
    assert c.view.WhichOneof("op") == "set"
    assert c.view.set.azimuth == 45 and c.view.set.elevation == 20
    assert c.view.set.distance == 3.0
    assert not c.view.set.HasField("fx"), "focal point unset by default"

    s.view.set(azimuth=1, elevation=2, distance=3, fx=4, fy=5, fz=6)
    c = _last(stub)
    assert c.view.set.HasField("fx")
    assert (c.view.set.fx, c.view.set.fy, c.view.set.fz) == (4, 5, 6)

    s.view.reset()
    c = _last(stub)
    assert c.view.WhichOneof("op") == "reset" and c.view.reset is True

    s.view.save("v1")
    c = _last(stub)
    assert c.WhichOneof("cmd") == "named_view"
    assert c.named_view.op == pb.NamedView.SAVE and c.named_view.name == "v1"

    s.view.restore("v1")
    c = _last(stub)
    assert c.named_view.op == pb.NamedView.RESTORE

    s.view.list()
    c = _last(stub)
    assert c.named_view.op == pb.NamedView.LIST and c.named_view.name == ""

    assert all(cmd.WhichOneof("cmd") != "raw" for cmd in stub.sent)


def test_authoritative_reads_use_the_snapshot_not_prediction():
    """Decision 61: ``Result.range`` / ``s.state`` / ``legend.limits``
    read the server's one-shot ``Subscribe`` snapshot — no client
    model. Verified against a fabricated snapshot (no server)."""
    from griz._proto import mili_viz_pb2 as pb

    snap = pb.Snapshot(state=7)
    snap.result.result = "sx"
    snap.result.min = -2.5
    snap.result.max = 4.0

    s, _ = _session(snap)
    assert s.state == 7
    r = s.show("sx")
    assert r.range == pytest.approx((-2.5, 4.0))
    assert s.legend.limits == pytest.approx((-2.5, 4.0))


# --------------------------------------------------------------------
# Skip-on-absent: Layer-0 ≡ Layer-1 identical session effect
# --------------------------------------------------------------------


def _root() -> str:
    return str(FIXTURE) if FIXTURE.exists() else "nonexistent_run"


def _key(snap) -> tuple:
    """The authoritative session state that must be identical whether
    reached via a typed Layer-1 call or the equivalent Layer-0 raw
    line — the server's single dispatcher is the only state owner."""
    return (
        snap.loaded.db,
        snap.loaded.num_states,
        snap.state,
        tuple(sorted(snap.selection.by_class.items())),
        snap.result.result,
        snap.result.component,
        round(snap.result.min, 4),
        round(snap.result.max, 4),
        round(snap.camera.azimuth, 4),
        round(snap.camera.elevation, 4),
        round(snap.camera.distance, 4),
        tuple(sorted(snap.materials.visible.items())),
    )


def _l1_state(s):
    s.open(_root())
    s.state = 2


def _l1_step(s):
    s.open(_root())
    s.state = 5
    s.prev()


def _l1_show(s):
    s.open(_root())
    s.show("sx")


def _l1_select(s):
    s.open(_root())
    s.select("node", "1-3")


def _l1_view(s):
    s.open(_root())
    s.view.set(azimuth=45, elevation=20, distance=3.0)


_CASES = [
    ("state", _l1_state, f"load {{}}; state 2"),
    ("step", _l1_step, f"load {{}}; state 5; prev"),
    ("show", _l1_show, f"load {{}}; show sx"),
    ("select", _l1_select, f"load {{}}; select node 1-3"),
    ("view", _l1_view, f"load {{}}; view set 45 20 3.0"),
]


@pytest.fixture(scope="module")
def _have_server():
    if server_binary() is None:
        pytest.skip("mili-viz-server binary/cargo unavailable")


@pytest.mark.parametrize("name,l1,raw_tmpl", _CASES, ids=[c[0] for c in _CASES])
def test_layer0_equals_layer1_identical_session_effect(
    name, l1, raw_tmpl, _have_server, sessions_dir
):
    """Decision 60: a typed Layer-1 call and its hand-written Layer-0
    equivalent, applied to two freshly-launched real servers, converge
    to byte-identical authoritative state — proving the server's one
    dispatcher (`parse_raw`→typed | typed) is the only state owner and
    the migration aid cannot drift. The griz line lives only here in
    the test; the library has no emitter/parser."""
    import griz

    with griz.launch() as s_typed:
        l1(s_typed)
        k_typed = _key(s_typed._snapshot())

    with griz.launch() as s_raw:
        reply = s_raw.command(raw_tmpl.format(_root()))
        assert reply.ok, f"Layer-0 {name!r} failed: {reply.error}"
        k_raw = _key(s_raw._snapshot())

    assert k_typed == k_raw, (
        f"Layer-0 ≡ Layer-1 drift on {name!r}:\n"
        f"  Layer-1 typed -> {k_typed}\n"
        f"  Layer-0 raw   -> {k_raw}"
    )
