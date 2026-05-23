//! The `egui` shell: toolbar, left dock, viewport overlays, status
//! bar — the **L1** default layout (`phase-5-m3.md` Decision 46).
//!
//! [`build_shell_ui`] is a pure, GPU-free function of an explicit
//! [`ShellState`]; it returns the [`UiAction`]s the windowed app
//! lowers to the frozen proto `Command`s. This is the milestone's
//! always-on test core (the M1-Decision-40 pattern): the
//! transport-affecting logic lives in small [`ShellState`] methods
//! that are unit-tested directly, and the layout is exercised
//! head­lessly by running the function with synthetic `RawInput`.

use std::time::{Duration, Instant};

use egui::Ui;

use crate::camera::Camera;
use crate::catalog::ResultCatalog;

/// Wall-clock interval between drag-time preview `Cmd::Cutplane` emits
/// (`phase-5-m8.md` Decision 85 — 30 Hz). Frame-rate independent: a
/// 60 Hz frame loop must not emit two commands per frame.
pub const CUT_PREVIEW_INTERVAL: Duration = Duration::from_millis(33);

/// The three non-agent session states M3 must render visibly
/// (wireframes §"Session states"; the agent states are M6).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionPhase {
    /// No run loaded — viewport shows the "attach to session" card,
    /// status bar reads `— not attached —`.
    NotAttached,
    /// A run is loaded and the session is quiescent (default L1).
    AttachedIdle,
    /// Playback is running — `▶ animate` shows as `⏸ pause`, the
    /// state counter increments.
    Animating,
}

/// Mirror of the broadcast `LoadedState`.
#[derive(Debug, Clone, Default)]
pub struct LoadedInfo {
    pub db: String,
    pub num_states: u32,
    pub state_times: Vec<f64>,
    pub class_names: Vec<String>,
}

/// Mirror of the broadcast `ResultState` (+ the `GeometryRef`
/// counts), the source for the title/legend overlays.
#[derive(Debug, Clone, Default)]
pub struct ResultInfo {
    pub name: String,
    pub component: String,
    pub min: f64,
    pub max: f64,
    pub num_vertices: u64,
    pub num_indices: u64,
}

/// Which viewport HUD overlay a toolbar chip toggles.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Overlay {
    Title,
    State,
    Legend,
    Axes,
    Bbox,
}

/// The five overlay toggles — **all on by default** (wireframes
/// §"Toolbar").
#[derive(Debug, Clone, Copy)]
pub struct Overlays {
    pub title: bool,
    pub state: bool,
    pub legend: bool,
    pub axes: bool,
    pub bbox: bool,
}

impl Default for Overlays {
    fn default() -> Self {
        Self {
            title: true,
            state: true,
            legend: true,
            axes: true,
            bbox: true,
        }
    }
}

impl Overlays {
    #[must_use]
    pub fn get(&self, o: Overlay) -> bool {
        match o {
            Overlay::Title => self.title,
            Overlay::State => self.state,
            Overlay::Legend => self.legend,
            Overlay::Axes => self.axes,
            Overlay::Bbox => self.bbox,
        }
    }
    fn toggle(&mut self, o: Overlay) {
        let slot = match o {
            Overlay::Title => &mut self.title,
            Overlay::State => &mut self.state,
            Overlay::Legend => &mut self.legend,
            Overlay::Axes => &mut self.axes,
            Overlay::Bbox => &mut self.bbox,
        };
        *slot = !*slot;
    }
}

/// How the renderer draws the mesh (VB-003 + Phase 5 M7). The default
/// [`RenderMode::Shaded`] is the unchanged single filled
/// `TriangleList` pass, so the byte-stable M3 composite path
/// (`render_shell_to_image`, always `Shaded`) is unaffected
/// (`bug-tracker.md` VB-001).
///
/// `Translucent` and `Xray` are Phase 5 M7 (Decisions 81–82): they
/// consume the new `MVG3` server-supplied per-element edge buffer
/// when present, and fall back to the legacy triangle-edge extractor
/// for `MVG1`/`MVG2` servers (byte-stable). `Interior` is **not** a
/// `RenderMode`; the include-interior toggle lives separately on
/// [`ShellState::interior_on`] (Decision 83 — server round-trip via
/// the reserved `MaterialVisibility{ material: u32::MAX }` sentinel).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum RenderMode {
    /// Filled lit hull only — the M2/M3 pass, unchanged.
    #[default]
    Shaded,
    /// Filled hull **plus** a depth-tested unique-edge overlay, so
    /// only the visible front edges draw over the surface
    /// (hidden-line overlay).
    Edges,
    /// Unique mesh edges only over the cleared background — a
    /// see-through wireframe (no fill to occlude back edges).
    Wireframe,
    /// Alpha-blended fill (depth-test on, depth-write off): the
    /// silhouette plus any interior-triangle layer the server is
    /// shipping. Phase 5 M7 Decision 81.
    Translucent,
    /// `Translucent` fill plus the element-edge overlay — the
    /// high-information "see-through but edges visible" mode.
    /// Phase 5 M7 Decision 81.
    Xray,
}

impl RenderMode {
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            RenderMode::Shaded => "shaded",
            RenderMode::Edges => "shaded + edges",
            RenderMode::Wireframe => "wireframe",
            RenderMode::Translucent => "translucent",
            RenderMode::Xray => "x-ray",
        }
    }
}

/// A cut-plane the gizmo edits. Origin + (unit-ish) normal in world
/// units, matching the frozen `Cmd::Cutplane` `ox..oz` / `nx..nz`
/// fields (`phase-5-m8.md` Decision 86; `phase-4-m8.md` Decision 75).
/// The `relative` toggle stays `false` — the gizmo always emits an
/// absolute plane; clearing is a zero-normal `Cmd::Cutplane` per
/// `phase-4-m8.md`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CutPlaneState {
    pub origin: [f32; 3],
    pub normal: [f32; 3],
}

/// Lower a [`CutPlaneState`] to the frozen `Cmd::Cutplane`
/// (`phase-5-m8.md` § "What lands"). Absolute plane (`relative` stays
/// `false`); `slice_only` stays `None` — the proto3 default the
/// server's M8 cut arm reads (the M9 sibling [`slice_cmd`] sets
/// `Some(true)` instead). Keeping `None` (not `Some(false)`) preserves
/// byte-stability against M8-only clients (`m8_cut_gizmo.rs`
/// `lowering_copies_origin_normal_and_keeps_proto3_defaults` pins it).
#[must_use]
pub fn cutplane_cmd(plane: CutPlaneState) -> mili_viz_proto::v1::command::Cmd {
    mili_viz_proto::v1::command::Cmd::Cutplane(mili_viz_proto::v1::CutPlane {
        ox: f64::from(plane.origin[0]),
        oy: f64::from(plane.origin[1]),
        oz: f64::from(plane.origin[2]),
        nx: f64::from(plane.normal[0]),
        ny: f64::from(plane.normal[1]),
        nz: f64::from(plane.normal[2]),
        relative: false,
        slice_only: None,
    })
}

/// Lower a [`CutPlaneState`] as a **slice** verb (`phase-5-m9.md` § "What
/// lands"). Same byte shape as [`cutplane_cmd`] but with `slice_only =
/// Some(true)` — the server's M9 arm
/// (`crates/mili-viz-server/src/clip.rs` `ClipMode::Slice`) reads this
/// flag and emits cap-only triangles tagged
/// [`mili_viz_server::CAP_MATERIAL`]-sibling `u32::MAX - 2`. Composes
/// server-side with any active cut (`phase-4-m9.md` Decision 80).
#[must_use]
pub fn slice_cmd(plane: CutPlaneState) -> mili_viz_proto::v1::command::Cmd {
    mili_viz_proto::v1::command::Cmd::Cutplane(mili_viz_proto::v1::CutPlane {
        ox: f64::from(plane.origin[0]),
        oy: f64::from(plane.origin[1]),
        oz: f64::from(plane.origin[2]),
        nx: f64::from(plane.normal[0]),
        ny: f64::from(plane.normal[1]),
        nz: f64::from(plane.normal[2]),
        relative: false,
        slice_only: Some(true),
    })
}

impl CutPlaneState {
    /// Seed the gizmo at the **mesh AABB centre** with the camera's
    /// view-plane normal (`phase-5-m8.md` Decision 86). The view-plane
    /// normal is the camera basis' forward axis (negated — it points
    /// from the eye toward the scene; the cut keep-side is the far
    /// half-space so this orients "cut the half nearest the viewer").
    #[must_use]
    pub fn from_aabb_and_camera(aabb: ([f32; 3], [f32; 3]), camera: &Camera) -> Self {
        let (lo, hi) = aabb;
        let origin = [
            0.5 * (lo[0] + hi[0]),
            0.5 * (lo[1] + hi[1]),
            0.5 * (lo[2] + hi[2]),
        ];
        let (_, _, fwd) = camera.basis();
        Self {
            origin,
            normal: fwd.to_array(),
        }
    }
}

/// Wall-clock throttle for gizmo-drag preview emits
/// (`phase-5-m8.md` Decision 85). A 30 Hz cap on the drag-time
/// `Cmd::Cutplane` stream; drag-end is the un-throttled canonical
/// commit (call [`CutThrottle::reset`] then emit unconditionally).
#[derive(Debug, Clone, Default)]
pub struct CutThrottle {
    last_emit_at: Option<Instant>,
}

impl CutThrottle {
    #[must_use]
    pub fn new() -> Self {
        Self { last_emit_at: None }
    }

    /// Return `true` (and mark `now` as the last emit) iff at least
    /// [`CUT_PREVIEW_INTERVAL`] has elapsed since the last preview;
    /// otherwise return `false` and leave state unchanged. Pure;
    /// caller-injected clock for testability.
    pub fn try_preview(&mut self, now: Instant) -> bool {
        let pass = match self.last_emit_at {
            None => true,
            Some(t) => now.duration_since(t) >= CUT_PREVIEW_INTERVAL,
        };
        if pass {
            self.last_emit_at = Some(now);
        }
        pass
    }

    /// Forget the last emit so the next preview / commit fires
    /// unconditionally. Called at drag-start and drag-end.
    pub fn reset(&mut self) {
        self.last_emit_at = None;
    }
}

/// The egui visuals theme (wireframes §"Tweaks": *Theme — Dark / Light
/// egui visuals*). The default [`Theme::Dark`] is exactly egui's
/// default `Visuals::dark()`, so applying it is pixel-identical to the
/// untouched M3 path — the byte-stable composite gate (`bug-tracker.md`
/// VB-001) is unaffected by the default.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Theme {
    #[default]
    Dark,
    Light,
}

impl Theme {
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Theme::Dark => "dark",
            Theme::Light => "light",
        }
    }
    fn visuals(self) -> egui::Visuals {
        match self {
            Theme::Dark => egui::Visuals::dark(),
            Theme::Light => egui::Visuals::light(),
        }
    }
}

/// The three peer bottom tabs (wireframes §"Bottom tabs";
/// `phase-5-m3.5.md` Decision 51).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BottomTab {
    /// Layer-0 raw griz / `grizinit` stream (Decision 48).
    CommandLine,
    /// Managed-venv `pygriz` subprocess runner (Decision 49; unblocked
    /// by Phase 6 M2 — `phase-6-m2.md`).
    Scripting,
    /// `egui_plot` host fed by the `Subscribe` stream (Decision 50).
    TimeHistory,
}

/// How a [`TranscriptLine`] renders in the command-line tab.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TranscriptKind {
    /// An echoed user command (`griz>` prompt, green).
    Command,
    /// A dim server outcome line (`ok`).
    Response,
    /// A command rejection (the `CommandReply.error`, danger colour).
    Error,
}

/// One client-side command-line transcript row (`phase-5-m3.5.md`
/// Decision 48). The transcript is pure client state — griz commands
/// carry no text payload; their effect is the broadcast `StateDelta`.
#[derive(Debug, Clone, PartialEq)]
pub struct TranscriptLine {
    pub kind: TranscriptKind,
    pub text: String,
}

/// One time-history sample: the active result's data-range envelope
/// at a visited state, accumulated from the broadcast `ResultState`
/// (`phase-5-m3.5.md` Decision 50).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TimeSample {
    pub state: u32,
    pub t: f64,
    pub min: f64,
    pub max: f64,
}

/// A client-side intent emitted by the shell. The windowed app lowers
/// the transport-affecting variants to the **frozen** proto `Command`
/// (`phase-5-m3.md` Decision 46, `phase-5-m3.5.md` Decision 48); the
/// pure-client variants (`ToggleOverlay`, `SetStride`,
/// `SelectBottomTab`, `CollapseBottomTabs`) have already been applied
/// to [`ShellState`] and are returned for observability/persistence.
#[derive(Debug, Clone, PartialEq)]
pub enum UiAction {
    First,
    Prev,
    Next,
    Last,
    SetStride(u32),
    ToggleAnimate,
    StopAnimate,
    ViewReset,
    Fit,
    ToggleOverlay(Overlay),
    Show(String),
    /// A verbatim Layer-0 line to send as `Command{ raw }`
    /// (`phase-5-m3.5.md` Decision 48). The echo row is already on
    /// [`ShellState::transcript`]; the app appends the outcome row.
    RunCommand(String),
    SelectBottomTab(BottomTab),
    CollapseBottomTabs,
    /// Pick a named colormap (`phase-5-m4.md` Decision 66). Lowered to
    /// the frozen `Command::Colormap`; the visual effect is applied
    /// client-side (the server treats it as a recolor no-op).
    SetColormap(String),
    /// Set/clear the `LegendLimits` override (`min`, `max`); `None`
    /// autoscales that end. Lowered to `Command::Legend`.
    SetLegendLimits(Option<f64>, Option<f64>),
    /// Pure-client render-mode switch (VB-003). Already applied to
    /// [`ShellState`]; the windowed app retargets the renderer. No
    /// proto change — the frozen `Command` set is untouched.
    SetRenderMode(RenderMode),
    /// Include-interior toggle (Phase 5 M7 Decision 83). Already
    /// applied to [`ShellState::interior_on`]; the windowed app
    /// lowers this to the frozen `Cmd::Material` with the reserved
    /// `material: Some(u32::MAX)` sentinel that the server reads via
    /// `MaterialsState.visible` (Phase 4 M7 Decision 74). The
    /// re-emitted blob then carries the interior triangles.
    SetInteriorMode(bool),
    /// Pure-client picking-mode toggle. Already applied to
    /// [`ShellState`]; client-side ray-cast against the cached hull,
    /// no proto command.
    TogglePicking,
    /// Enable/disable a class's materials. Lowered to the frozen
    /// `Command::Material` (`MaterialVisibility{ enable, class_name }`,
    /// no `material` id — whole class). The server is already done
    /// (status 8); this is the GUI affordance.
    SetMaterialVisible {
        class_name: String,
        visible: bool,
    },
    /// Pure-client: run the scripting-tab buffer in a managed `pygriz`
    /// subprocess (`client.md` decision 3, `phase-6-m2.md`). Already
    /// applied to [`ShellState`] (the running flag is set + the output
    /// pane cleared); the windowed app spawns the child and streams its
    /// stdout/stderr back via [`ShellState::push_script_output`] /
    /// [`ShellState::finish_script`]. No proto command — the script
    /// owns its own connection (`griz.launch()` spawns a headless
    /// server; `attach()`-into-*this* in-process GUI is gated on the
    /// deferred Phase 5 M5 remote mode). The subprocess path is
    /// windowed-only — not headlessly verifiable in CI.
    RunScript(String),
    /// Pure-client theme switch (wireframes §"Tweaks"). Already applied
    /// to [`ShellState`]; [`build_shell_ui`] sets the egui visuals from
    /// it each frame. Returned for observability/persistence (the tweak
    /// state "should persist between sessions"). No proto command.
    SetTheme(Theme),
    /// Pure-client left-dock collapse (wireframes §"Tweaks": *Left dock
    /// collapsed — L1 ↔ left-rail-only*). Already applied to
    /// [`ShellState`]; the shell renders the 28 px rail instead of the
    /// full dock. Returned for observability/persistence. No proto
    /// command.
    SetDockCollapsed(bool),
    /// Pure-client L3 focus-mode toggle (wireframes §"L3 — Focus
    /// mode"; `Ctrl+\`). Already applied to [`ShellState`]
    /// (`set_focus_mode` also collapses the dock); the shell hides the
    /// AI rail + bottom tabs. Returned for observability/persistence.
    /// No proto command.
    SetFocusMode(bool),
    /// Commit a cut plane unconditionally (Phase 5 M8 Decision 85).
    /// Lowered by the windowed app to the frozen `Cmd::Cutplane`;
    /// includes both the canonical drag-end commit and any
    /// out-of-drag menu emit. State already mutated via
    /// [`ShellState::set_cut_plane`].
    SetCutPlane(CutPlaneState),
    /// In-drag preview emit; the windowed app gates this through the
    /// [`CutThrottle`] (30 Hz wall-clock; Decision 85) and also drops
    /// it entirely when interactive clip is off (Decision 86). State
    /// already mutated.
    PreviewCutPlane(CutPlaneState),
    /// Clear the active cut. Lowered to a zero-normal `Cmd::Cutplane`
    /// which the server (`phase-4-m8.md`) treats as a clear.
    ClearCut,
    /// Toggle gizmo visibility (Rendering → Cut menu row;
    /// `phase-5-m8.md` § "What lands"). Pure-client; no proto.
    SetCutGizmoVisible(bool),
    /// Toggle interactive-clip preview emission
    /// (Preferences → Interactive clip; Decision 86). Pure-client;
    /// cross-session-persisted via `tweaks.json`. Already applied to
    /// [`ShellState`]; observability/persistence-only.
    SetInteractiveClip(bool),
    /// Phase 5 M9 Decision 87 — commit a slice plane unconditionally.
    /// Lowered by the windowed app to [`slice_cmd`] (the
    /// `slice_only: Some(true)` variant of `Cmd::Cutplane`); covers the
    /// canonical drag-end commit and any out-of-drag menu emit. State
    /// already mutated via [`ShellState::set_slice_plane`].
    SetSlicePlane(CutPlaneState),
    /// Phase 5 M9 — in-drag slice preview. Gated by the windowed app
    /// through the same [`CutThrottle`] as the cut sibling (one wall-
    /// clock 30 Hz budget for both verbs — a user only drags one
    /// gizmo at a time) and dropped entirely when interactive clip is
    /// off (Decision 86). State already mutated.
    PreviewSlicePlane(CutPlaneState),
    /// Phase 5 M9 — clear the active slice. Lowers to a zero-normal
    /// `Cmd::Cutplane { slice_only: Some(true), .. }` which the server
    /// (`phase-4-m9.md`) treats as a slice clear (the same
    /// `Plane::from_proto` `None`-on-zero-normal lever the cut uses).
    ClearSlice,
    /// Phase 5 M9 — toggle the slice gizmo overlay (Rendering → Slice
    /// row). Pure-client; no proto.
    SetSliceGizmoVisible(bool),
    /// Phase 5 M6 — toggle the AI panel between the 28 px collapsed
    /// rail and the 340 px expanded panel (wireframes §"L1" / §"L2").
    /// Already applied to [`ShellState::ai`]; the action is observability-
    /// only (no proto command).
    SetAiExpanded(bool),
    /// Phase 5 M6 — toggle the 📷 attach-frame pending flag on the
    /// composer (`client.md` §"Vision is deliberate but agent-initiated").
    /// Already applied; pure-client.
    ToggleAttachFrame,
    /// Phase 5 M6 — send a user-turn to the server-hosted agent via
    /// the frozen `AgentChat` RPC. The windowed app pre-encodes the
    /// pinned framebuffer via `CaptureFrame` when `attach_frame == true`.
    AgentChat {
        text: String,
        attach_frame: bool,
    },
    /// Phase 5 M6 — barge-in. Calls the frozen `Interrupt` RPC for
    /// the in-flight turn (empty `turn_id` = "current turn", per the
    /// frozen-proto convention).
    AgentInterrupt {
        turn_id: String,
    },
    /// Phase 5 M6 — `↶ revert to here`. Lowers a captured
    /// [`TurnSnapshot`] to its typed `SetState` / `Show` /
    /// `View(SetCamera)` sequence (Decision 97) and executes them in
    /// order; no `raw`.
    AgentRevert {
        turn_id: String,
    },
}

/// Static fallback derived-result names, shown only until a real
/// catalog is attached. Once the side-channel catalog arrives
/// (`phase-5-m4.md` Decision 71) the left dock lists the run's
/// DB-filtered `ResultCatalog::derived` instead; this set keeps the
/// default (no-catalog) `ShellState` chrome byte-identical (VB-001).
pub const DERIVED_RESULTS: &[&str] = &[
    "disp_mag",
    "disp_x",
    "pressure",
    "eff_stress",
    "prin_stress1",
    "prin_strain1",
    "triaxiality",
];

/// Seed for the scripting editor. `attach()`-into-*this* GUI needs the
/// deferred Phase 5 M5 remote transport (the in-process client writes
/// no `~/.griz` session file), so the template uses the landed Phase 6
/// M2 `griz.launch()` (a headless server the script drives).
pub const DEFAULT_SCRIPT: &str = "\
import griz
# This GUI runs in-process; attach() to it needs Phase 5 M5 (remote mode).
# launch() spawns a headless mili-viz-server you drive from here.
s = griz.launch()
print(s)
";

/// The `Control` menu rows: a label plus the **already-existing,
/// already-lowered** [`UiAction`] each emits (`wireframe-parity.md`
/// "Menu bar"; MVP-cut item 1). The legacy griz `Control` Motif menu
/// (`reference/griz/Src/gui.c`) is session/app control — Copyright,
/// Material Mgr, Session save/load, Quit — all of which need a proto
/// or windowed-lifecycle contract this slice deliberately does not
/// touch. So `Control` instead hosts the session-control verbs that
/// already have a `UiAction` and an `app.rs` lowering (the griz idiom
/// of menus duplicating the toolbar / `Time` menu): transport,
/// animate/stop, view-reset/fit. Pure data so the wiring is
/// unit-testable without driving egui pointer input — the menu just
/// iterates this and the windowed app lowers each variant exactly as
/// the toolbar's clicks already do. No frozen-proto change, no new
/// `UiAction`.
#[must_use]
pub fn control_menu_items() -> Vec<(&'static str, UiAction)> {
    vec![
        ("⏮ first state", UiAction::First),
        ("◀ prev state", UiAction::Prev),
        ("▶ next state", UiAction::Next),
        ("⏭ last state", UiAction::Last),
        ("▶/⏸ animate", UiAction::ToggleAnimate),
        ("⏹ stop animate", UiAction::StopAnimate),
        ("⟲ view reset", UiAction::ViewReset),
        ("⊞ fit", UiAction::Fit),
    ]
}

/// The L3 focus-mode icon-rail entries (wireframes §"L3 — Focus mode":
/// *R/M/S/P glyphs for Results / Materials / Surfaces / Picking*). Pure
/// data: a single-char glyph + its hover text. The `Picking` entry's
/// hint reflects the live picking state so the collapsed rail doubles
/// as an at-a-glance status read-out. Every glyph's only action is to
/// expand the dock (`UiAction::SetDockCollapsed(false)` — no proto, no
/// new `UiAction`); the rail is the `dock_collapsed` view of the same
/// left dock.
#[must_use]
pub fn dock_rail_glyphs(picking: bool) -> [(&'static str, &'static str); 4] {
    [
        ("R", "Results — expand dock"),
        ("M", "Materials — expand dock"),
        ("S", "Surfaces — expand dock"),
        (
            "P",
            if picking {
                "Picking: on — expand dock"
            } else {
                "Picking: off — expand dock"
            },
        ),
    ]
}

/// All shell state the layout is a pure function of.
#[derive(Debug, Clone)]
pub struct ShellState {
    pub phase: SessionPhase,
    pub session_id: String,
    pub host: String,
    pub loaded: Option<LoadedInfo>,
    pub result: Option<ResultInfo>,
    /// 1-based current state (mirrors griz / the proto).
    pub state: u32,
    pub stride: u32,
    pub overlays: Overlays,
    pub fps: f32,
    /// Status-bar pick readout (`—` when nothing picked / picking
    /// off).
    pub pick: String,
    /// Whether left-click does a client-side ray-cast pick instead of
    /// starting an orbit. Default off, driven by the `Picking` menu.
    pub picking: bool,
    /// World-space point of the last ray-cast hit, for the viewport
    /// highlight glyph (MVP-cut 4). `None` (the default, a miss, or
    /// picking off) → no glyph, so the headless composite gate stays
    /// byte-stable (`bug-tracker.md` VB-001).
    pub pick_point: Option<[f32; 3]>,
    /// Classes whose materials are toggled **off** in the left-dock
    /// Materials section. Empty = all visible (the default, so the M3
    /// composite gate is unchanged). The proto's `MaterialsState` is
    /// keyed by material id with no client-side class catalog, so
    /// visibility is tracked client-authoritatively by class name and
    /// pushed to the server via the typed command.
    pub hidden_materials: std::collections::BTreeSet<String>,
    /// Currently highlighted Results-tree row.
    pub selected_result: Option<String>,
    /// Open bottom tab, or `None` for the collapsed 22 px strip
    /// (`phase-5-m3.5.md` Decision 51 — default-collapsed keeps the
    /// M3 render seam byte-stable).
    pub bottom_tab: Option<BottomTab>,
    /// Command-line transcript (echo + outcome rows, Decision 48).
    pub transcript: Vec<TranscriptLine>,
    /// Command-line input buffer.
    pub cmdline_input: String,
    /// Scripting-tab editor buffer (`client.md` decision 3). Seeded
    /// with a `griz.launch()` template — the in-process GUI has no
    /// session file to `attach()` to (that needs Phase 5 M5).
    pub script: String,
    /// Streamed stdout/stderr of the last/active script run.
    pub script_output: String,
    /// A script subprocess is in flight (disables Run).
    pub script_running: bool,
    /// The `venv: … · attach: …` status line under the runner.
    pub script_status: String,
    /// Time-history series accumulated from `ResultState`
    /// (Decision 50).
    pub time_history: Vec<TimeSample>,
    /// Active colormap name (`phase-5-m4.md` Decision 66); one of
    /// [`crate::colormap::NAMES`], default `cool`.
    pub colormap: String,
    /// `LegendLimits` override; an unset bound autoscales that end
    /// from the broadcast `ResultState` (Decision 66).
    pub legend_min: Option<f64>,
    pub legend_max: Option<f64>,
    /// Active render mode (VB-003), driven by the `Rendering` menu.
    /// Default [`RenderMode::Shaded`] keeps the M3 composite gate
    /// byte-stable.
    pub render_mode: RenderMode,
    /// Phase 5 M7 (Decision 83) include-interior toggle. `false`
    /// (default) keeps the server emitting an `MVG1`/`MVG2` boundary
    /// hull so the M2/M3/M4/MVP-polish composite paths stay
    /// byte-identical. Flipping it true lowers a `Cmd::Material` with
    /// the reserved `u32::MAX` sentinel, which makes the next `show`
    /// re-emit an `MVG3` blob carrying the interior triangles
    /// (`tri_flags & 1 == 1`).
    pub interior_on: bool,
    /// Active egui theme (wireframes §"Tweaks"), driven by the
    /// Preferences menu. Default [`Theme::Dark`] == egui's default
    /// visuals, so the M3 composite path is pixel-unchanged.
    pub theme: Theme,
    /// Whether the left dock is collapsed to a 28 px rail (wireframes
    /// §"Tweaks": *Left dock collapsed*). Default `false` keeps the L1
    /// full dock, so `scene_frac` / the composite gate are unchanged.
    pub dock_collapsed: bool,
    /// L3 focus mode (wireframes §"L3 — Focus mode"): stripped to the
    /// viewport — the AI rail + bottom tabs are hidden and the dock is
    /// the icon rail. Toggled with `Ctrl+\`. Default `false` keeps the
    /// full L1 chrome, so `scene_frac` / the composite gate are
    /// unchanged (`bug-tracker.md` VB-001).
    pub focus_mode: bool,
    /// The central viewport the panels leave, as `[x, y, w, h]`
    /// fractions of the full egui screen (`0..1`, top-left origin).
    /// `None` until the first [`build_shell_ui`] measures it. The
    /// windowed app maps it onto the physical surface so the `wgpu`
    /// mesh pass frames — and orbits — about the centre of the
    /// *visible* scene, not the full surface the docks occlude.
    /// Resolution-independent (a fraction, not pixels) so no
    /// `pixels_per_point` plumbing is needed.
    pub scene_frac: Option<[f32; 4]>,
    /// The live windowed camera, published each frame so the bbox /
    /// axes overlays project against the real view. `None` (the
    /// default, and the headless composite path) keeps the M3
    /// placeholder bbox/gizmo so that gate stays byte-stable.
    pub camera: Option<Camera>,
    /// World-space AABB `(min, max)` of the current-state hull, for
    /// the projected-bbox overlay. `None` → placeholder.
    pub model_aabb: Option<([f32; 3], [f32; 3])>,
    /// Result catalog fetched over the side-channel after a `load`
    /// (`phase-5-m3.md` Decision 67; MVP-cut 8). `None` (the default,
    /// and the headless composite path) keeps the left-dock `primal`
    /// sub-tree as the static `(catalog: M4+)` placeholder, so that
    /// gate stays byte-stable (`bug-tracker.md` VB-001).
    pub catalog: Option<ResultCatalog>,
    /// Active cut plane (`phase-5-m8.md` Decisions 84–86). `None` means
    /// "no cut" — keeps the M2/M3/M4/MVP-polish composite path
    /// byte-stable (`bug-tracker.md` VB-001).
    pub cut_plane: Option<CutPlaneState>,
    /// Whether the Rendering → Cut gizmo overlay is drawn over the
    /// viewport. Default `false`. Pure-client; no proto.
    pub cut_gizmo_visible: bool,
    /// Whether drag-time `Cmd::Cutplane` previews are emitted
    /// (Preferences → Interactive clip). Default `true` matches griz's
    /// `cutpln` live-feel; flipping off suppresses preview emits for
    /// low-bandwidth links (the drag-end commit still fires). Cross-
    /// session-persisted via `tweaks.json` (Decision 86).
    pub interactive_clip: bool,
    /// Active slice plane (`phase-5-m9.md` Decisions 87–89). `None`
    /// means "no slice" — independent of [`ShellState::cut_plane`] so
    /// the two verbs **compose** (`phase-4-m9.md` Decision 80). Default
    /// `None` keeps the M2/M3/M4/MVP-polish + M8 composite path
    /// byte-stable (`bug-tracker.md` VB-001).
    pub slice_plane: Option<CutPlaneState>,
    /// Whether the Rendering → Slice gizmo overlay is drawn over the
    /// viewport. Default `false`. Pure-client; no proto.
    pub slice_gizmo_visible: bool,
    /// Phase 5 M6 AI Assistant panel state (`phase-5-m6.md` Decisions
    /// 94–99 / `client.md` §"AI Assistant panel"). The
    /// `cap_agent`-false default keeps the right dock as the 28 px
    /// placeholder rail — byte-stable against M3 (`bug-tracker.md`
    /// VB-001 — every prior composite-render gate is run with
    /// `cap_agent = false`).
    pub ai: crate::ai_panel::AiPanelState,
}

impl Default for ShellState {
    fn default() -> Self {
        Self {
            phase: SessionPhase::NotAttached,
            session_id: String::new(),
            host: "in-process".to_string(),
            loaded: None,
            result: None,
            state: 1,
            stride: 1,
            overlays: Overlays::default(),
            fps: 0.0,
            pick: "—".to_string(),
            picking: false,
            pick_point: None,
            hidden_materials: std::collections::BTreeSet::new(),
            selected_result: None,
            bottom_tab: None,
            transcript: Vec::new(),
            cmdline_input: String::new(),
            script: DEFAULT_SCRIPT.to_string(),
            script_output: String::new(),
            script_running: false,
            script_status: "venv: — · attach: —".to_string(),
            time_history: Vec::new(),
            colormap: "cool".to_string(),
            legend_min: None,
            legend_max: None,
            render_mode: RenderMode::default(),
            interior_on: false,
            theme: Theme::default(),
            dock_collapsed: false,
            focus_mode: false,
            scene_frac: None,
            camera: None,
            model_aabb: None,
            catalog: None,
            cut_plane: None,
            cut_gizmo_visible: false,
            interactive_clip: true,
            slice_plane: None,
            slice_gizmo_visible: false,
            ai: crate::ai_panel::AiPanelState::default(),
        }
    }
}

impl ShellState {
    #[must_use]
    pub fn total_states(&self) -> u32 {
        self.loaded.as_ref().map_or(1, |l| l.num_states.max(1))
    }

    /// Current-state simulation time, if a run is loaded.
    #[must_use]
    pub fn state_time(&self) -> Option<f64> {
        let l = self.loaded.as_ref()?;
        l.state_times.get(self.state as usize - 1).copied()
    }

    fn select_result(&mut self, name: &str) -> UiAction {
        self.selected_result = Some(name.to_string());
        UiAction::Show(name.to_string())
    }

    /// Switch the render mode (VB-003). Pure client state; the
    /// returned action is observability-only (no proto command).
    pub fn set_render_mode(&mut self, mode: RenderMode) -> UiAction {
        self.render_mode = mode;
        UiAction::SetRenderMode(mode)
    }

    /// Toggle the include-interior flag (Phase 5 M7 Decision 83).
    /// Updates [`ShellState::interior_on`]; the returned action is
    /// lowered by the windowed app to the frozen `Cmd::Material`
    /// with the reserved `u32::MAX` sentinel.
    pub fn set_interior_mode(&mut self, on: bool) -> UiAction {
        self.interior_on = on;
        UiAction::SetInteriorMode(on)
    }

    /// Switch the egui theme (wireframes §"Tweaks"). Pure client
    /// state; the returned action is observability/persistence-only
    /// (no proto command).
    pub fn set_theme(&mut self, theme: Theme) -> UiAction {
        self.theme = theme;
        UiAction::SetTheme(theme)
    }

    /// Collapse/expand the left dock (wireframes §"Tweaks"). Pure
    /// client state; observability/persistence-only (no proto command).
    pub fn set_dock_collapsed(&mut self, collapsed: bool) -> UiAction {
        self.dock_collapsed = collapsed;
        UiAction::SetDockCollapsed(collapsed)
    }

    /// Toggle L3 focus mode (wireframes §"L3 — Focus mode"; `Ctrl+\`).
    /// Entering also collapses the dock so the rail shows; exiting
    /// restores it — so a single key round-trips the full L1 ↔ L3
    /// chrome. Pure client state; observability/persistence-only (no
    /// proto command).
    pub fn set_focus_mode(&mut self, on: bool) -> UiAction {
        self.focus_mode = on;
        self.dock_collapsed = on;
        UiAction::SetFocusMode(on)
    }

    /// Toggle client-side picking. Turning it off clears the readout
    /// back to `—`. Pure client state; the action is observability-only
    /// (no proto command).
    pub fn toggle_picking(&mut self) -> UiAction {
        self.picking = !self.picking;
        if !self.picking {
            self.pick = "—".to_string();
            self.pick_point = None;
        }
        UiAction::TogglePicking
    }

    /// Fold a ray-cast result into the status-bar readout. The frozen
    /// proto has no label catalog, so a hit shows the node/triangle
    /// indices the cached hull actually carries (plus the `MVG2`
    /// scalar when present); a miss reads `(no hit)`.
    pub fn apply_pick(&mut self, hit: Option<&crate::mesh::Pick>) {
        self.pick = match hit {
            None => "(no hit)".to_string(),
            Some(p) => match p.scalar {
                Some(v) => format!("node {} · tri {} · v={v:.3e}", p.node, p.tri),
                None => format!("node {} · tri {}", p.node, p.tri),
            },
        };
        // Remember the hit point for the viewport highlight glyph; a
        // miss clears it so a stale marker never lingers (MVP-cut 4).
        self.pick_point = hit.map(|p| p.point);
    }

    /// Whether a class's materials are currently shown.
    #[must_use]
    pub fn material_visible(&self, class_name: &str) -> bool {
        !self.hidden_materials.contains(class_name)
    }

    /// Flip a class's material visibility and emit the typed command
    /// for the app to lower to the frozen `Command::Material`.
    pub fn toggle_material(&mut self, class_name: &str) -> UiAction {
        let visible = if self.hidden_materials.remove(class_name) {
            true
        } else {
            self.hidden_materials.insert(class_name.to_string());
            false
        };
        UiAction::SetMaterialVisible {
            class_name: class_name.to_string(),
            visible,
        }
    }

    /// Commit a cut plane (Phase 5 M8 Decision 85; the canonical
    /// drag-end commit or any out-of-drag menu emit). Mutates the
    /// state; the returned action is lowered to `Cmd::Cutplane` by
    /// the windowed app, unconditionally (no throttle).
    pub fn set_cut_plane(&mut self, plane: CutPlaneState) -> UiAction {
        self.cut_plane = Some(plane);
        UiAction::SetCutPlane(plane)
    }

    /// Emit a drag-time preview update (Phase 5 M8 Decision 85). The
    /// windowed app gates these through the [`CutThrottle`] and drops
    /// them entirely when [`ShellState::interactive_clip`] is `false`.
    /// State mutated for visual immediacy regardless of whether the
    /// emit lands on the wire.
    pub fn preview_cut_plane(&mut self, plane: CutPlaneState) -> UiAction {
        self.cut_plane = Some(plane);
        UiAction::PreviewCutPlane(plane)
    }

    /// Clear the active cut. Lowers to a zero-normal `Cmd::Cutplane`
    /// which the server treats as a clear (`phase-4-m8.md`).
    pub fn clear_cut(&mut self) -> UiAction {
        self.cut_plane = None;
        UiAction::ClearCut
    }

    /// Show/hide the gizmo overlay (Rendering → Cut row).
    pub fn set_cut_gizmo_visible(&mut self, on: bool) -> UiAction {
        self.cut_gizmo_visible = on;
        UiAction::SetCutGizmoVisible(on)
    }

    /// Toggle interactive-clip preview emission
    /// (Preferences → Interactive clip; Decision 86 — persisted).
    pub fn set_interactive_clip(&mut self, on: bool) -> UiAction {
        self.interactive_clip = on;
        UiAction::SetInteractiveClip(on)
    }

    /// Commit a slice plane (`phase-5-m9.md` Decision 87; the canonical
    /// drag-end commit or any out-of-drag menu emit). Mutates the
    /// state; the returned action is lowered to [`slice_cmd`] by the
    /// windowed app, unconditionally (no throttle).
    pub fn set_slice_plane(&mut self, plane: CutPlaneState) -> UiAction {
        self.slice_plane = Some(plane);
        UiAction::SetSlicePlane(plane)
    }

    /// Emit a drag-time slice preview update (`phase-5-m9.md`). The
    /// windowed app gates these through the same [`CutThrottle`] the
    /// cut sibling uses and drops them entirely when
    /// [`ShellState::interactive_clip`] is `false`. State mutated for
    /// visual immediacy regardless of whether the emit lands on the
    /// wire.
    pub fn preview_slice_plane(&mut self, plane: CutPlaneState) -> UiAction {
        self.slice_plane = Some(plane);
        UiAction::PreviewSlicePlane(plane)
    }

    /// Clear the active slice. Lowers to a zero-normal
    /// `Cmd::Cutplane { slice_only: Some(true), .. }` which the server
    /// treats as a slice clear (`phase-4-m9.md`).
    pub fn clear_slice(&mut self) -> UiAction {
        self.slice_plane = None;
        UiAction::ClearSlice
    }

    /// Show/hide the slice gizmo overlay (Rendering → Slice row).
    pub fn set_slice_gizmo_visible(&mut self, on: bool) -> UiAction {
        self.slice_gizmo_visible = on;
        UiAction::SetSliceGizmoVisible(on)
    }

    /// Toggle a bottom tab: open it, or collapse the body if it is
    /// already the open tab (`phase-5-m3.5.md` Decision 51).
    pub fn toggle_tab(&mut self, tab: BottomTab) -> UiAction {
        if self.bottom_tab == Some(tab) {
            self.bottom_tab = None;
            UiAction::CollapseBottomTabs
        } else {
            self.bottom_tab = Some(tab);
            UiAction::SelectBottomTab(tab)
        }
    }

    /// Echo a submitted Layer-0 line and emit it for the app to lower
    /// to `Command{ raw }` (`phase-5-m3.5.md` Decision 48). Returns
    /// `None` for a blank line.
    pub fn submit_command(&mut self) -> Option<UiAction> {
        let line = self.cmdline_input.trim().to_string();
        self.cmdline_input.clear();
        if line.is_empty() {
            return None;
        }
        self.transcript.push(TranscriptLine {
            kind: TranscriptKind::Command,
            text: line.clone(),
        });
        Some(UiAction::RunCommand(line))
    }

    /// Append the dim outcome row after an `Execute` returns
    /// (`phase-5-m3.5.md` Decision 48). Called by the windowed app.
    pub fn push_command_outcome(&mut self, ok: bool, error: &str) {
        let (kind, text) = if ok {
            (TranscriptKind::Response, "ok".to_string())
        } else {
            (TranscriptKind::Error, error.to_string())
        };
        self.transcript.push(TranscriptLine { kind, text });
    }

    /// Start a script run (`client.md` decision 3). Sets the running
    /// flag + clears the output pane and emits the buffer for the app
    /// to spawn the `pygriz` subprocess. `None` while a run is already
    /// in flight or the buffer is blank.
    pub fn run_script(&mut self) -> Option<UiAction> {
        if self.script_running || self.script.trim().is_empty() {
            return None;
        }
        self.script_running = true;
        self.script_output.clear();
        self.script_status = "venv: starting · attach: launch".to_string();
        Some(UiAction::RunScript(self.script.clone()))
    }

    /// Append a streamed stdout/stderr chunk from the running script
    /// subprocess (called by the windowed app each frame).
    pub fn push_script_output(&mut self, chunk: &str) {
        self.script_output.push_str(chunk);
    }

    /// Mark the script subprocess finished and update the status line
    /// (called by the windowed app when the child exits).
    pub fn finish_script(&mut self, status: &str) {
        self.script_running = false;
        self.script_status = status.to_string();
    }

    /// Record a time-history sample for the active result at the
    /// current state (`phase-5-m3.5.md` Decision 50). No-op without a
    /// result or a known state time; replaces an existing sample for
    /// the same state so scrubbing back and forth does not duplicate.
    pub fn record_time_sample(&mut self) {
        let Some(r) = self.result.as_ref() else {
            return;
        };
        if r.name.is_empty() {
            return;
        }
        let Some(t) = self.state_time() else {
            return;
        };
        let sample = TimeSample {
            state: self.state,
            t,
            min: r.min,
            max: r.max,
        };
        if let Some(s) = self
            .time_history
            .iter_mut()
            .find(|s| s.state == sample.state)
        {
            *s = sample;
        } else {
            self.time_history.push(sample);
            self.time_history.sort_by_key(|s| s.state);
        }
    }

    /// The colour-mapping range the renderer and legend use
    /// (`phase-5-m4.md` Decision 66): a `LegendLimits` bound overrides
    /// that end, an unset bound autoscales from the broadcast
    /// `ResultState`. `None` when there is no scalar result (the bare
    /// hull renders the M2 base colour exactly as before).
    #[must_use]
    pub fn effective_range(&self) -> Option<(f32, f32)> {
        let r = self.result.as_ref()?;
        if r.name.is_empty() {
            return None;
        }
        let lo = self.legend_min.unwrap_or(r.min);
        let hi = self.legend_max.unwrap_or(r.max);
        Some((lo as f32, hi as f32))
    }
}

/// Build the L1 shell into the root `ui` and return the frame's
/// actions (`phase-5-m3.md` Decision 46). Pure + GPU-free: no `wgpu`,
/// no transport — the windowed app owns lowering the result to the
/// frozen `Command` and pumping `Subscribe` deltas back into `state`.
/// Panels are added outermost-first (egui 0.34 `Panel::show_inside`);
/// the leftover rect is the transparent central viewport the `wgpu`
/// mesh pass shows through (`phase-5-m3.md` Decision 45).
pub fn build_shell_ui(ui: &mut Ui, state: &mut ShellState) -> Vec<UiAction> {
    let mut actions = Vec::new();
    // Apply the tweak theme (wireframes §"Tweaks"). The default
    // `Theme::Dark` is egui's own `Visuals::dark()`, so on the
    // default-`ShellState` composite path this is pixel-identical to
    // the untouched M3 chrome (`bug-tracker.md` VB-001); a runtime
    // switch back to Dark also reverts cleanly.
    ui.ctx().set_visuals(state.theme.visuals());
    // L3 focus-mode toggle (wireframes §"L3 — Focus mode"): `Ctrl+\`
    // round-trips the full L1 ↔ stripped-viewport chrome. A key event
    // is real input, so this only fires when the user presses it (the
    // "no input ⇒ no actions" invariant holds without the key).
    if ui.input(|i| i.modifiers.ctrl && i.key_pressed(egui::Key::Backslash)) {
        actions.push(state.set_focus_mode(!state.focus_mode));
    }
    // Full extent before any panel carves into it; the leftover
    // central rect is normalized against this below.
    let full = ui.max_rect();

    egui::Panel::top("menu")
        .exact_size(26.0)
        .show_inside(ui, |ui| {
            egui::MenuBar::new().ui(ui, |ui| {
                ui.menu_button("Control", |ui| {
                    // Transport/animate/view need an attached run, like
                    // the toolbar's equivalents; grey the whole menu
                    // body when not attached rather than emit no-ops.
                    let attached = state.phase != SessionPhase::NotAttached;
                    ui.add_enabled_ui(attached, |ui| {
                        for (i, (label, action)) in control_menu_items().into_iter().enumerate() {
                            // griz groups: transport | animate | view.
                            if i == 4 || i == 6 {
                                ui.separator();
                            }
                            // A `Button` click auto-closes the menu.
                            if ui.button(label).clicked() {
                                actions.push(action);
                            }
                        }
                    });
                });
                ui.menu_button("Rendering", |ui| {
                    for mode in [
                        RenderMode::Shaded,
                        RenderMode::Edges,
                        RenderMode::Wireframe,
                        RenderMode::Translucent,
                        RenderMode::Xray,
                    ] {
                        let mark = if state.render_mode == mode {
                            "● "
                        } else {
                            "○ "
                        };
                        // A `Button` click auto-closes the egui menu.
                        if ui.button(format!("{mark}{}", mode.label())).clicked() {
                            actions.push(state.set_render_mode(mode));
                        }
                    }
                    ui.separator();
                    // Include-interior is composable with any RenderMode
                    // (Phase 5 M7 Decision 83). Visible cell-cell faces
                    // need the server round-trip; the toggle re-emits.
                    let mark = if state.interior_on { "● " } else { "○ " };
                    if ui.button(format!("{mark}include interior")).clicked() {
                        actions.push(state.set_interior_mode(!state.interior_on));
                    }
                    ui.separator();
                    // Cut sub-section (Phase 5 M8 § "What lands"):
                    // gizmo-visible toggle + clear-cut row. The plane
                    // itself is edited via the gizmo overlay (drag) and
                    // seeded at the mesh AABB centre on first show
                    // (Decision 86).
                    let mark = if state.cut_gizmo_visible {
                        "● "
                    } else {
                        "○ "
                    };
                    if ui.button(format!("{mark}show cut gizmo")).clicked() {
                        actions.push(state.set_cut_gizmo_visible(!state.cut_gizmo_visible));
                    }
                    let cut_active = state.cut_plane.is_some();
                    if ui
                        .add_enabled(cut_active, egui::Button::new("clear cut"))
                        .clicked()
                    {
                        actions.push(state.clear_cut());
                    }
                    ui.separator();
                    // Slice sub-section (Phase 5 M9 § "What lands"):
                    // gizmo-visible toggle + clear-slice row, parallel
                    // to the Cut block above. The slice and cut planes
                    // compose server-side (`phase-4-m9.md` Decision 80)
                    // so this menu does not force a "pick one" between
                    // them — both can be active at once.
                    let mark = if state.slice_gizmo_visible {
                        "● "
                    } else {
                        "○ "
                    };
                    if ui.button(format!("{mark}show slice gizmo")).clicked() {
                        actions.push(state.set_slice_gizmo_visible(!state.slice_gizmo_visible));
                    }
                    let slice_active = state.slice_plane.is_some();
                    if ui
                        .add_enabled(slice_active, egui::Button::new("clear slice"))
                        .clicked()
                    {
                        actions.push(state.clear_slice());
                    }
                });
                ui.menu_button("Picking", |ui| {
                    let mark = if state.picking { "● " } else { "○ " };
                    if ui.button(format!("{mark}enable picking")).clicked() {
                        actions.push(state.toggle_picking());
                    }
                });
                // View / Preferences host (wireframes §"Tweaks"; the
                // legacy griz menu bar has no settings menu — the
                // wireframe maps the Tweaks set to a "View /
                // Preferences" menu). MVP scope is the two tweaks that
                // are pure-client and need no proto/contract change:
                // Theme and Left-dock-collapse. "Show bottom tabs" is
                // already reachable via the tab strip's ▾ hide;
                // "AI panel position" is M6 (panel is a placeholder).
                ui.menu_button("Preferences", |ui| {
                    ui.label("Theme");
                    for t in [Theme::Dark, Theme::Light] {
                        let mark = if state.theme == t { "● " } else { "○ " };
                        if ui.button(format!("{mark}{}", t.label())).clicked() {
                            actions.push(state.set_theme(t));
                        }
                    }
                    ui.separator();
                    let mut collapsed = state.dock_collapsed;
                    if ui.checkbox(&mut collapsed, "Left dock collapsed").clicked() {
                        actions.push(state.set_dock_collapsed(collapsed));
                    }
                    ui.separator();
                    // Preferences → Interactive clip (Phase 5 M8
                    // Decision 86; persisted via tweaks.json). When off,
                    // the drag-time preview path is suppressed — only
                    // the canonical drag-end commit lands on the wire.
                    let mut live = state.interactive_clip;
                    if ui.checkbox(&mut live, "Interactive clip").clicked() {
                        actions.push(state.set_interactive_clip(live));
                    }
                });
                for m in ["Results", "Time", "Plot", "Help"] {
                    let _ = ui.menu_button(m, |_| {});
                }
            });
        });

    egui::Panel::top("toolbar")
        .exact_size(30.0)
        .show_inside(ui, |ui| {
            toolbar(ui, state, &mut actions);
        });

    egui::Panel::bottom("status")
        .exact_size(20.0)
        .show_inside(ui, |ui| {
            status_bar(ui, state);
        });

    // Bottom tabs are hidden in L3 focus mode (wireframes §"L3").
    if !state.focus_mode {
        bottom_tabs(ui, state, &mut actions);
    }

    if state.dock_collapsed {
        // L3 focus-mode icon rail (wireframes §"L3 — Focus mode" /
        // §"Tweaks": *Left dock collapsed*): a 28 px strip showing the
        // R/M/S/P section glyphs; any glyph expands the dock. Off by
        // default so `scene_frac` / the composite gate are unchanged.
        egui::Panel::left("dock")
            .resizable(false)
            .exact_size(28.0)
            .show_inside(ui, |ui| {
                ui.vertical_centered(|ui| {
                    ui.add_space(6.0);
                    for (glyph, tip) in dock_rail_glyphs(state.picking) {
                        if ui.button(glyph).on_hover_text(tip).clicked() {
                            // In focus mode a glyph restores the full
                            // L1 chrome (exit focus); otherwise it just
                            // expands the dock.
                            actions.push(if state.focus_mode {
                                state.set_focus_mode(false)
                            } else {
                                state.set_dock_collapsed(false)
                            });
                        }
                    }
                });
            });
    } else {
        egui::Panel::left("dock")
            .resizable(true)
            .default_size(230.0)
            .show_inside(ui, |ui| {
                left_dock(ui, state, &mut actions);
            });
    }

    // Phase 5 M6 (`phase-5-m6.md` Decisions 94–99): the right dock is
    // now the AI Assistant — the 28 px collapsed rail (default L1) or
    // the 340 px expanded panel (L2). When the server did not
    // advertise `CAP_AGENT` the panel is hidden entirely
    // (`scripting.md` capability-negotiation pattern); the byte-stable
    // default (`cap_agent = false`) keeps the M3/M3.5/M4/M5 composite
    // gate from painting any new panel (VB-001).
    if !state.focus_mode && state.ai.cap_agent {
        ai_dock(ui, state, &mut actions);
    } else if !state.focus_mode {
        // Capability absent: keep the 28 px placeholder rail so the
        // central viewport sub-rect is unchanged from the
        // pre-M6 layout (zero scene_frac drift for the
        // composite-render gates that build a default ShellState).
        egui::Panel::right("ai")
            .resizable(false)
            .exact_size(28.0)
            .show_inside(ui, |ui| {
                ui.vertical_centered(|ui| {
                    ui.add_space(8.0);
                    ui.label("AI");
                });
            });
    }

    // The leftover space is the central viewport: a transparent
    // region the full-surface mesh pass shows through; the five
    // overlays (or the not-attached card) paint over it.
    let rect = ui.available_rect_before_wrap();
    // Publish the leftover central rect as screen fractions so the
    // windowed mesh pass can target exactly the visible scene.
    if full.width() > 0.0 && full.height() > 0.0 {
        state.scene_frac = Some([
            (rect.min.x - full.min.x) / full.width(),
            (rect.min.y - full.min.y) / full.height(),
            rect.width() / full.width(),
            rect.height() / full.height(),
        ]);
    }
    if state.phase == SessionPhase::NotAttached {
        attach_card(ui, rect);
    } else {
        overlays(ui, rect, state);
    }

    actions
}

fn toolbar(ui: &mut egui::Ui, state: &mut ShellState, actions: &mut Vec<UiAction>) {
    let enabled = state.phase != SessionPhase::NotAttached;
    ui.horizontal_centered(|ui| {
        ui.add_enabled_ui(enabled, |ui| {
            if ui.button("⏮").on_hover_text("first state").clicked() {
                actions.push(UiAction::First);
            }
            if ui.button("◀").on_hover_text("prev state").clicked() {
                actions.push(UiAction::Prev);
            }
            if ui.button("▶").on_hover_text("next state").clicked() {
                actions.push(UiAction::Next);
            }
            if ui.button("⏭").on_hover_text("last state").clicked() {
                actions.push(UiAction::Last);
            }
        });
        ui.separator();

        ui.label("stride");
        let mut stride = state.stride;
        if ui
            .add_sized(
                [28.0, 22.0],
                egui::DragValue::new(&mut stride).range(1..=999),
            )
            .changed()
        {
            state.stride = stride.max(1);
            actions.push(UiAction::SetStride(state.stride));
        }
        ui.separator();

        ui.add_enabled_ui(enabled, |ui| {
            let animating = state.phase == SessionPhase::Animating;
            let label = if animating {
                "⏸ pause"
            } else {
                "▶ animate"
            };
            if ui.selectable_label(animating, label).clicked() {
                actions.push(UiAction::ToggleAnimate);
            }
            if ui.button("⏹").on_hover_text("stop").clicked() {
                actions.push(UiAction::StopAnimate);
            }
        });
        ui.separator();

        ui.add_enabled_ui(enabled, |ui| {
            if ui.button("⟲").on_hover_text("view reset").clicked() {
                actions.push(UiAction::ViewReset);
            }
            if ui.button("⊞").on_hover_text("fit").clicked() {
                actions.push(UiAction::Fit);
            }
        });
        ui.separator();

        ui.label("overlays");
        for (o, name) in [
            (Overlay::Title, "title"),
            (Overlay::State, "state"),
            (Overlay::Legend, "legend"),
            (Overlay::Axes, "axes"),
            (Overlay::Bbox, "bbox"),
        ] {
            if ui.selectable_label(state.overlays.get(o), name).clicked() {
                state.overlays.toggle(o);
                actions.push(UiAction::ToggleOverlay(o));
            }
        }

        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.monospace(format!("state {} / {}", state.state, state.total_states()));
        });
    });
}

fn left_dock(ui: &mut egui::Ui, state: &mut ShellState, actions: &mut Vec<UiAction>) {
    egui::ScrollArea::vertical().show(ui, |ui| {
        let runs = i32::from(state.loaded.is_some());
        egui::CollapsingHeader::new(format!("Runs/sessions · {runs}"))
            .default_open(true)
            .show(ui, |ui| {
                if let Some(l) = &state.loaded {
                    ui.horizontal(|ui| {
                        ui.label("●");
                        ui.label(&l.db);
                    });
                } else {
                    ui.weak("— no run loaded —");
                }
            });

        // Primal + derived names from the side-channel catalog
        // (`phase-5-m4.md` Decision 67 / 71). Cloned up front so the
        // selectable rows can call `state.select_result` without
        // aliasing `state.catalog`. `None`/absent ⇒ `primal` empty and
        // `derived` falls back to the static `DERIVED_RESULTS`, so the
        // default (no-catalog) chrome — badge `DERIVED_RESULTS.len()`,
        // bare `derived`/`primal` headers, the `(catalog: M4+)`
        // placeholder — stays byte-identical (VB-001).
        let primal: Vec<String> = state
            .catalog
            .as_ref()
            .map(|c| c.primal.clone())
            .unwrap_or_default();
        let derived: Vec<String> = state
            .catalog
            .as_ref()
            .map(|c| c.derived.clone())
            .unwrap_or_else(|| DERIVED_RESULTS.iter().map(|s| (*s).to_string()).collect());
        let has_catalog = state.catalog.is_some();
        let results_count = derived.len() + primal.len();
        egui::CollapsingHeader::new(format!("Results · {results_count}"))
            .default_open(true)
            .show(ui, |ui| {
                // Bare `derived` (byte-stable default) until a real
                // catalog is attached, then `derived · N`.
                let derived_label = if has_catalog {
                    format!("derived · {}", derived.len())
                } else {
                    "derived".to_string()
                };
                egui::CollapsingHeader::new(derived_label)
                    .default_open(true)
                    .show(ui, |ui| {
                        for r in &derived {
                            let sel = state.selected_result.as_deref() == Some(r.as_str());
                            if ui.selectable_label(sel, r).clicked() {
                                actions.push(state.select_result(r));
                            }
                        }
                    });
                // Header label is kept exactly `primal` when empty so
                // the default (no-catalog) chrome is byte-identical to
                // pre-Decision-67 (VB-001); the count badge appears
                // only once a real catalog is attached.
                let primal_label = if primal.is_empty() {
                    "primal".to_string()
                } else {
                    format!("primal · {}", primal.len())
                };
                egui::CollapsingHeader::new(primal_label)
                    .default_open(false)
                    .show(ui, |ui| {
                        if primal.is_empty() {
                            // No real run loaded (or none queriable) —
                            // the static placeholder, byte-stable.
                            ui.weak("(catalog: M4+)");
                        } else {
                            for r in &primal {
                                let sel = state.selected_result.as_deref() == Some(r.as_str());
                                if ui.selectable_label(sel, r).clicked() {
                                    actions.push(state.select_result(r));
                                }
                            }
                        }
                    });
                egui::CollapsingHeader::new("time-indep")
                    .default_open(false)
                    .show(ui, |ui| {
                        // No mili-rs time-independent accessor yet
                        // (`phase-5-m4.md` Decision 69 — a re-port, not
                        // a reshape) — honest placeholder, not a stub
                        // that looks live. Rendered text unchanged.
                        ui.weak("(time-indep: no catalog path yet)");
                    });
            });

        egui::CollapsingHeader::new("Colormap")
            .default_open(false)
            .show(ui, |ui| {
                let mut cmap = state.colormap.clone();
                egui::ComboBox::from_id_salt("colormap")
                    .selected_text(&cmap)
                    .show_ui(ui, |ui| {
                        for &n in crate::colormap::NAMES {
                            ui.selectable_value(&mut cmap, n.to_string(), n);
                        }
                    });
                if cmap != state.colormap {
                    state.colormap = cmap.clone();
                    actions.push(UiAction::SetColormap(cmap));
                }

                let (auto_lo, auto_hi) =
                    state.result.as_ref().map_or((0.0, 1.0), |r| (r.min, r.max));
                let mut manual = state.legend_min.is_some() || state.legend_max.is_some();
                if ui.checkbox(&mut manual, "manual limits").clicked() {
                    let (lo, hi) = if manual {
                        (Some(auto_lo), Some(auto_hi))
                    } else {
                        (None, None)
                    };
                    state.legend_min = lo;
                    state.legend_max = hi;
                    actions.push(UiAction::SetLegendLimits(lo, hi));
                }
                if manual {
                    let mut lo = state.legend_min.unwrap_or(auto_lo);
                    let mut hi = state.legend_max.unwrap_or(auto_hi);
                    let mut changed = false;
                    ui.horizontal(|ui| {
                        ui.label("min");
                        changed |= ui.add(egui::DragValue::new(&mut lo)).changed();
                    });
                    ui.horizontal(|ui| {
                        ui.label("max");
                        changed |= ui.add(egui::DragValue::new(&mut hi)).changed();
                    });
                    if changed {
                        state.legend_min = Some(lo);
                        state.legend_max = Some(hi);
                        actions.push(UiAction::SetLegendLimits(Some(lo), Some(hi)));
                    }
                }
            });

        let classes: Vec<String> = state
            .loaded
            .as_ref()
            .map(|l| l.class_names.clone())
            .unwrap_or_default();
        egui::CollapsingHeader::new(format!("Materials · {}", classes.len()))
            .default_open(false)
            .show(ui, |ui| {
                for c in &classes {
                    // A row toggles the class's materials: filled dot +
                    // normal label when visible, hollow dot + weak
                    // label when hidden. The whole row is the button.
                    let visible = state.material_visible(c);
                    let dot = if visible { "●" } else { "○" };
                    let text = if visible {
                        egui::RichText::new(format!("{dot} {c}"))
                    } else {
                        egui::RichText::new(format!("{dot} {c}")).weak()
                    };
                    if ui
                        .selectable_label(false, text)
                        .on_hover_text("toggle material visibility")
                        .clicked()
                    {
                        actions.push(state.toggle_material(c));
                    }
                }
            });

        egui::CollapsingHeader::new("Surfaces · 0")
            .default_open(false)
            .show(ui, |ui| {
                ui.weak("(surfaces: M4+)");
            });
    });
}

/// The bottom-tabs panel (`phase-5-m3.5.md` Decision 51): an
/// always-present 22 px tab strip plus a default-collapsed body. The
/// collapsed footprint matches the M3 stub, so `m3_egui_shell.rs`
/// stays green and the Decision-45 composition seam is unchanged.
fn bottom_tabs(ui: &mut egui::Ui, state: &mut ShellState, actions: &mut Vec<UiAction>) {
    let panel = egui::Panel::bottom("tabs");
    let panel = if state.bottom_tab.is_some() {
        panel.resizable(true).default_size(200.0)
    } else {
        panel.resizable(false).exact_size(22.0)
    };
    panel.show_inside(ui, |ui| {
        ui.horizontal(|ui| {
            tab_button(ui, state, actions, BottomTab::CommandLine, "command line");
            tab_button(ui, state, actions, BottomTab::Scripting, "scripting");
            tab_button(ui, state, actions, BottomTab::TimeHistory, "time-history");
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if state.bottom_tab.is_some() && ui.small_button("▾ hide").clicked() {
                    state.bottom_tab = None;
                    actions.push(UiAction::CollapseBottomTabs);
                }
            });
        });
        if let Some(tab) = state.bottom_tab {
            ui.separator();
            match tab {
                BottomTab::CommandLine => cmdline_tab(ui, state, actions),
                BottomTab::Scripting => scripting_tab(ui, state, actions),
                BottomTab::TimeHistory => time_history_tab(ui, state),
            }
        }
    });
}

fn tab_button(
    ui: &mut egui::Ui,
    state: &mut ShellState,
    actions: &mut Vec<UiAction>,
    tab: BottomTab,
    label: &str,
) {
    let active = state.bottom_tab == Some(tab);
    if ui.selectable_label(active, label).clicked() {
        actions.push(state.toggle_tab(tab));
    }
}

/// Layer-0 command line (`phase-5-m3.5.md` Decision 48): green
/// `griz>` prompt, echoed commands, dim responses. The input lowers
/// verbatim to `Command{ raw }`; nothing is re-parsed client-side.
fn cmdline_tab(ui: &mut egui::Ui, state: &mut ShellState, actions: &mut Vec<UiAction>) {
    let green = egui::Color32::from_rgb(120, 200, 120);
    let dim = egui::Color32::from_gray(150);
    let danger = egui::Color32::from_rgb(220, 110, 100);
    let mono = egui::TextStyle::Monospace.resolve(ui.style());

    let row_h = 22.0;
    let scroll_h = (ui.available_height() - row_h).max(0.0);
    egui::ScrollArea::vertical()
        .stick_to_bottom(true)
        .auto_shrink([false, false])
        .max_height(scroll_h)
        .show(ui, |ui| {
            for line in &state.transcript {
                let (txt, col) = match line.kind {
                    TranscriptKind::Command => (format!("griz> {}", line.text), green),
                    TranscriptKind::Response => (line.text.clone(), dim),
                    TranscriptKind::Error => (line.text.clone(), danger),
                };
                ui.label(egui::RichText::new(txt).color(col).font(mono.clone()));
            }
        });

    ui.horizontal(|ui| {
        ui.label(egui::RichText::new("griz>").color(green).font(mono.clone()));
        let resp = ui.add(
            egui::TextEdit::singleline(&mut state.cmdline_input)
                .font(mono.clone())
                .desired_width(f32::INFINITY)
                .hint_text("raw griz / grizinit line"),
        );
        let submit = resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
        if submit {
            if let Some(a) = state.submit_command() {
                actions.push(a);
            }
            resp.request_focus();
        }
    });
}

/// Scripting runner (`client.md` decision 3, `phase-6-m2.md`): a
/// monospace editor, a Run button, a streamed output pane, and the
/// `venv: … · attach: …` indicator. Run emits [`UiAction::RunScript`];
/// the windowed app spawns a managed `pygriz` subprocess and streams
/// its output back. The subprocess path is windowed-only — the
/// gating test exercises the pure [`ShellState`] logic, not the child
/// (not headlessly verifiable in CI).
fn scripting_tab(ui: &mut egui::Ui, state: &mut ShellState, actions: &mut Vec<UiAction>) {
    let mono = egui::TextStyle::Monospace.resolve(ui.style());

    ui.add(
        egui::TextEdit::multiline(&mut state.script)
            .code_editor()
            .font(mono.clone())
            .desired_rows(4)
            .desired_width(f32::INFINITY),
    );
    ui.horizontal(|ui| {
        if ui
            .add_enabled(!state.script_running, egui::Button::new("▶ Run"))
            .clicked()
        {
            if let Some(a) = state.run_script() {
                actions.push(a);
            }
        }
        if state.script_running {
            ui.spinner();
            ui.weak("running…");
        }
    });

    let foot = 20.0;
    let scroll_h = (ui.available_height() - foot).max(40.0);
    egui::ScrollArea::vertical()
        .stick_to_bottom(true)
        .auto_shrink([false, false])
        .max_height(scroll_h)
        .show(ui, |ui| {
            if state.script_output.is_empty() {
                ui.weak("(no output)");
            } else {
                ui.label(egui::RichText::new(&state.script_output).font(mono.clone()));
            }
        });
    ui.weak(&state.script_status);
}

/// Time-history plot (`phase-5-m3.5.md` Decision 50): an `egui_plot`
/// host of the active result's data-range envelope vs. simulation
/// time, accumulated from the broadcast `Subscribe`/`ResultState`
/// stream. The `Query`-fed per-element series is the forward path.
fn time_history_tab(ui: &mut egui::Ui, state: &ShellState) {
    if state.time_history.is_empty() {
        ui.weak("no series yet — select a result and step through states");
        return;
    }
    let mins: Vec<[f64; 2]> = state.time_history.iter().map(|s| [s.t, s.min]).collect();
    let maxs: Vec<[f64; 2]> = state.time_history.iter().map(|s| [s.t, s.max]).collect();
    let label = state
        .result
        .as_ref()
        .map_or_else(|| "result".to_string(), |r| r.name.clone());
    egui_plot::Plot::new("time_history")
        .legend(egui_plot::Legend::default())
        .show(ui, |p| {
            p.line(
                egui_plot::Line::new(format!("{label} max"), maxs)
                    .color(egui::Color32::from_rgb(220, 110, 100)),
            );
            p.line(
                egui_plot::Line::new(format!("{label} min"), mins)
                    .color(egui::Color32::from_rgb(110, 160, 220)),
            );
        });
}

/// Spec status-bar protocol cell. The frozen contract's identity is
/// its **major** version (`Hello` negotiates "major must match" —
/// `mili-viz-proto` `PROTOCOL_VERSION` doc), so the wireframe's
/// `proto v1` is the major of the single-source-of-truth constant, not
/// a hard-coded literal — it follows the constant if the contract's
/// major ever bumps, and stays byte-identical to the M3 composite
/// baseline today (`1.0.0` → `proto v1`). Compile-time, not negotiated:
/// the in-process `Session` is the only transport and never runs
/// `Hello`, so the constant *is* the truth here with no runtime state.
fn proto_cell() -> String {
    let major = mili_viz_proto::v1::PROTOCOL_VERSION
        .split('.')
        .next()
        .unwrap_or("1");
    format!("proto v{major}")
}

fn status_bar(ui: &mut egui::Ui, state: &ShellState) {
    ui.horizontal_centered(|ui| {
        let attached = state.phase != SessionPhase::NotAttached;
        let txt = if attached {
            format!("● attached {}@{}", state.session_id, state.host)
        } else {
            "— not attached —".to_string()
        };
        ui.monospace(txt);
        ui.separator();
        ui.monospace(proto_cell());
        ui.separator();
        ui.monospace(format!("pick: {}", state.pick));
        // Status-bar cut readout (`phase-5-m8.md` § "What lands"): when
        // a cut plane is active, show its origin + normal in compact
        // form. Hidden by default so the byte-stable M3/MVP-polish
        // status-bar composite (VB-001) is unperturbed.
        if let Some(c) = state.cut_plane {
            ui.separator();
            ui.monospace(format!(
                "cut: o=({:.2},{:.2},{:.2}) n=({:.2},{:.2},{:.2})",
                c.origin[0], c.origin[1], c.origin[2], c.normal[0], c.normal[1], c.normal[2],
            ));
        }
        // Status-bar slice readout (`phase-5-m9.md` § "What lands"):
        // distinct from the cut line so when both compose (Decision 80)
        // the user sees two readouts. Hidden by default so the
        // byte-stable M3/MVP-polish status-bar composite (VB-001) is
        // unperturbed.
        if let Some(s) = state.slice_plane {
            ui.separator();
            ui.monospace(format!(
                "slice: o=({:.2},{:.2},{:.2}) n=({:.2},{:.2},{:.2})",
                s.origin[0], s.origin[1], s.origin[2], s.normal[0], s.normal[1], s.normal[2],
            ));
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.monospace(format!("fps {:.0}", state.fps));
            // Phase 5 M6 Decision 99 — peer count is the live
            // `DELTA_AGENT` `Status.detail = "peers=N"` value when
            // `CAP_AGENT` is advertised; otherwise the honest
            // single-peer default the M5 status bar shipped. The
            // default-`ShellState` composite-render gate (cap_agent
            // false) keeps the byte-stable `(1 peer)` text (VB-001).
            if attached {
                ui.separator();
                let n = if state.ai.cap_agent {
                    state.ai.peer_count()
                } else {
                    1
                };
                if n == 1 {
                    ui.monospace("(1 peer)");
                } else {
                    ui.monospace(format!("({n} peers)"));
                }
            }
        });
    });
}

/// Phase 5 M6 — the right-dock AI Assistant panel
/// (`griz_wgpu_wireframes/README.md` §"AI Assistant panel"). Two
/// chrome arms: a 28 px collapsed rail with the vertical
/// `AI ASSISTANT` label + status word (wireframes §"Collapsed state"),
/// or the 340 px expanded panel with header + transcript + composer.
/// Capability-gated by the caller (`state.ai.cap_agent`).
fn ai_dock(ui: &mut egui::Ui, state: &mut ShellState, actions: &mut Vec<UiAction>) {
    use crate::ai_panel::TranscriptRow;
    if state.ai.expanded {
        egui::Panel::right("ai")
            .resizable(true)
            .default_size(340.0)
            .min_size(260.0)
            .show_inside(ui, |ui| {
                // Header: AI ASSISTANT · status pill · › collapse glyph.
                ui.horizontal(|ui| {
                    ui.strong("AI ASSISTANT");
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.small_button("›").on_hover_text("collapse").clicked() {
                            state.ai.set_expanded(false);
                            actions.push(UiAction::SetAiExpanded(false));
                        }
                        ui.label(state.ai.status.label());
                    });
                });
                ui.separator();
                // Transcript: scrollable; tool-call lines render
                // dense per `client.md` §"AI Assistant panel".
                let transcript_h = (ui.available_height() - 96.0).max(80.0);
                egui::ScrollArea::vertical()
                    .max_height(transcript_h)
                    .stick_to_bottom(true)
                    .show(ui, |ui| {
                        for row in &state.ai.rows.clone() {
                            match row {
                                TranscriptRow::User { text, .. } => {
                                    ui.colored_label(ui.visuals().weak_text_color(), "you");
                                    ui.label(text);
                                }
                                TranscriptRow::Assistant { text, .. } => {
                                    ui.colored_label(ui.visuals().weak_text_color(), "claude");
                                    ui.label(text);
                                }
                                TranscriptRow::Tool {
                                    summary,
                                    result,
                                    complete,
                                    ok,
                                    ..
                                } => {
                                    let arrow = if *complete {
                                        if *ok {
                                            "▸"
                                        } else {
                                            "✕"
                                        }
                                    } else {
                                        "…"
                                    };
                                    let body = if result.is_empty() {
                                        format!("{arrow} {summary}")
                                    } else {
                                        format!("{arrow} {summary}     → {result}")
                                    };
                                    ui.monospace(body);
                                }
                                TranscriptRow::TurnBoundary { turn_id, summary } => {
                                    ui.horizontal(|ui| {
                                        ui.weak(format!("· {summary} ·"));
                                        if ui
                                            .small_button("↶ revert to here")
                                            .on_hover_text(
                                                "revert session to this turn's pre-turn snapshot",
                                            )
                                            .clicked()
                                        {
                                            actions.push(UiAction::AgentRevert {
                                                turn_id: turn_id.clone(),
                                            });
                                        }
                                    });
                                }
                                TranscriptRow::Interrupted { .. } => {
                                    ui.colored_label(
                                        ui.visuals().error_fg_color,
                                        "✕ interrupted by user — turn cancelled",
                                    );
                                }
                            }
                        }
                    });
                ui.separator();
                // Composer: attached-frame chip (when pending) +
                // multi-line input + Send/Stop primary.
                if state.ai.attach_frame_pending {
                    ui.horizontal(|ui| {
                        ui.monospace("📷 frame · pending");
                        if ui.small_button("×").clicked() {
                            state.ai.toggle_attach_frame();
                            actions.push(UiAction::ToggleAttachFrame);
                        }
                    });
                }
                let placeholder =
                    if matches!(state.ai.status, crate::ai_panel::AgentStatus::Interrupted,) {
                        "follow up… (turn was interrupted)"
                    } else {
                        "ask…"
                    };
                ui.add(
                    egui::TextEdit::multiline(&mut state.ai.composer)
                        .desired_rows(2)
                        .hint_text(placeholder)
                        .desired_width(f32::INFINITY),
                );
                ui.horizontal(|ui| {
                    if ui.button("📷").on_hover_text("attach frame").clicked() {
                        state.ai.toggle_attach_frame();
                        actions.push(UiAction::ToggleAttachFrame);
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if state.ai.status.in_flight() {
                            if ui
                                .button("⏹ Stop")
                                .on_hover_text("interrupt the agent")
                                .clicked()
                            {
                                actions.push(UiAction::AgentInterrupt {
                                    turn_id: state.ai.active_turn_id.clone().unwrap_or_default(),
                                });
                            }
                        } else if ui.button("Send ↵").clicked() {
                            if let Some(i) = state.ai.submit() {
                                actions.push(UiAction::AgentChat {
                                    text: i.text,
                                    attach_frame: i.attach_frame,
                                });
                            }
                        }
                    });
                });
            });
    } else {
        egui::Panel::right("ai")
            .resizable(false)
            .exact_size(28.0)
            .show_inside(ui, |ui| {
                ui.vertical_centered(|ui| {
                    ui.add_space(8.0);
                    // Click anywhere on the rail to expand
                    // (wireframes §"Collapsed state").
                    if ui
                        .small_button("AI")
                        .on_hover_text("expand AI Assistant")
                        .clicked()
                    {
                        state.ai.set_expanded(true);
                        actions.push(UiAction::SetAiExpanded(true));
                    }
                    ui.add_space(6.0);
                    ui.weak(state.ai.status.label());
                });
            });
    }
}

fn attach_card(ui: &mut egui::Ui, rect: egui::Rect) {
    ui.scope_builder(egui::UiBuilder::new().max_rect(rect), |ui| {
        ui.centered_and_justified(|ui| {
            ui.group(|ui| {
                ui.vertical_centered(|ui| {
                    ui.heading("attach to session");
                    ui.label("No run loaded.");
                    ui.weak("Pass a database root on the command line, or");
                    ui.weak("use the Layer-0 command line (M3.5).");
                });
            });
        });
    });
}

/// The five viewport overlays drawn over the `wgpu` surface, each
/// gated by its toolbar chip (`phase-5-m3.md` § Goal). Monospace,
/// ~10.5 px, low-contrast white.
fn overlays(ui: &mut egui::Ui, rect: egui::Rect, state: &ShellState) {
    let painter = ui.painter_at(rect);
    let fg = egui::Color32::from_white_alpha(217); // ≈85% on dark
    let font = egui::FontId::monospace(10.5);
    let pad = 8.0;

    if state.overlays.title {
        let run = state.loaded.as_ref().map_or("—", |l| l.db.as_str());
        let res = state.result.as_ref().map_or("(hull)", |r| r.name.as_str());
        let counts = state.result.as_ref().map_or_else(
            || "nodes: — · tris: —".to_string(),
            |r| format!("nodes: {} · tris: {}", r.num_vertices, r.num_indices / 3),
        );
        painter.text(
            rect.left_top() + egui::vec2(pad, pad),
            egui::Align2::LEFT_TOP,
            format!("{run} · {res}\n{counts}"),
            font.clone(),
            fg,
        );
    }

    if state.overlays.state {
        let t = state
            .state_time()
            .map_or_else(String::new, |t| format!("\nt = {t:.4e} s"));
        painter.text(
            rect.right_top() + egui::vec2(-pad, pad),
            egui::Align2::RIGHT_TOP,
            format!("state {} / {}{}", state.state, state.total_states(), t),
            font.clone(),
            fg,
        );
    }

    if state.overlays.legend {
        legend(&painter, rect, state, &font, fg);
    }

    if state.overlays.axes {
        axes_gizmo(&painter, rect, state.camera.as_ref());
    }

    if state.overlays.bbox {
        match state
            .camera
            .as_ref()
            .zip(state.model_aabb)
            .and_then(|(c, aabb)| project_bbox(c, aabb, rect))
        {
            // Real world-space AABB projected through the live camera:
            // its 12 edges track orbit/pan/zoom and per-state deform.
            Some(corners) => {
                let stroke = egui::Stroke::new(1.0, egui::Color32::from_white_alpha(110));
                for &(a, b) in BBOX_EDGES {
                    painter.line_segment([corners[a], corners[b]], stroke);
                }
            }
            // No live camera (headless composite / not attached) — the
            // M3 placeholder inset, byte-stable for that gate.
            None => {
                let inset = rect.shrink2(egui::vec2(rect.width() * 0.18, rect.height() * 0.18));
                let stroke = egui::Stroke::new(1.0, egui::Color32::from_white_alpha(90));
                dashed_rect(&painter, inset, stroke);
            }
        }
    }

    // Cut-plane gizmo overlay (`phase-5-m8.md` Decision 84): a flat
    // disk + normal arrow drawn through the live camera as **egui
    // shapes only** — no new wgpu pipeline (VB-001 / the M3 additive
    // seam stays untouched). Drawn only when the user opted in via
    // Rendering → Cut, a plane is set, and a live camera is attached;
    // the headless composite path (camera `None`) is byte-stable.
    if state.cut_gizmo_visible {
        if let Some((plane, cam)) = state.cut_plane.zip(state.camera.as_ref()) {
            draw_plane_gizmo(&painter, rect, plane, cam, CUT_GIZMO_COLOR);
        }
    }
    // Slice-plane gizmo overlay (`phase-5-m9.md` Decision 87): shares
    // the M8 `draw_plane_gizmo` machinery, contrasting colour so the
    // two are distinguishable when both verbs compose (Decision 80).
    if state.slice_gizmo_visible {
        if let Some((plane, cam)) = state.slice_plane.zip(state.camera.as_ref()) {
            draw_plane_gizmo(&painter, rect, plane, cam, SLICE_GIZMO_COLOR);
        }
    }

    // Picking highlight glyph (MVP-cut 4): a ring + crosshair over the
    // last ray-cast hit, projected through the live camera so it
    // tracks orbit/pan/zoom and per-state deform. Not chip-gated (it
    // is a picking-mode artifact, not one of the five HUD overlays);
    // only drawn when picking is on, a hit is cached, and a live
    // camera is attached — so the headless composite path (camera
    // `None`, picking off) is byte-stable (`bug-tracker.md` VB-001).
    if state.picking {
        if let Some(c) = state
            .pick_point
            .zip(state.camera.as_ref())
            .and_then(|(p, cam)| {
                let w = rect.width().max(1.0) as u32;
                let h = rect.height().max(1.0) as u32;
                cam.project(glam::Vec3::from(p), w, h)
            })
        {
            let at = egui::pos2(
                rect.min.x + c.x * rect.width(),
                rect.min.y + c.y * rect.height(),
            );
            let accent = egui::Color32::from_rgb(255, 190, 60);
            let stroke = egui::Stroke::new(1.5, accent);
            painter.circle_stroke(at, 7.0, stroke);
            for d in [egui::vec2(11.0, 0.0), egui::vec2(0.0, 11.0)] {
                painter.line_segment([at - d, at + d], stroke);
            }
        }
    }
}

/// The 12 edges of a box as index pairs into the 8-corner array laid
/// out as `bit0=x bit1=y bit2=z` (`min`=0, `max`=1 per axis).
const BBOX_EDGES: &[(usize, usize)] = &[
    (0, 1),
    (2, 3),
    (4, 5),
    (6, 7), // x-dir
    (0, 2),
    (1, 3),
    (4, 6),
    (5, 7), // y-dir
    (0, 4),
    (1, 5),
    (2, 6),
    (3, 7), // z-dir
];

/// Project the 8 AABB corners to viewport pixels via the live camera.
/// `None` if any corner is at/behind the eye (a partially-clipped box
/// would draw garbage edges — fall back to the placeholder instead).
fn project_bbox(
    camera: &Camera,
    aabb: ([f32; 3], [f32; 3]),
    rect: egui::Rect,
) -> Option<[egui::Pos2; 8]> {
    let (lo, hi) = aabb;
    let w = rect.width().max(1.0) as u32;
    let h = rect.height().max(1.0) as u32;
    let mut out = [egui::Pos2::ZERO; 8];
    for (i, slot) in out.iter_mut().enumerate() {
        let p = glam::Vec3::new(
            if i & 1 == 0 { lo[0] } else { hi[0] },
            if i & 2 == 0 { lo[1] } else { hi[1] },
            if i & 4 == 0 { lo[2] } else { hi[2] },
        );
        let f = camera.project(p, w, h)?;
        *slot = egui::pos2(
            rect.min.x + f.x * rect.width(),
            rect.min.y + f.y * rect.height(),
        );
    }
    Some(out)
}

fn legend(
    painter: &egui::Painter,
    rect: egui::Rect,
    state: &ShellState,
    font: &egui::FontId,
    fg: egui::Color32,
) {
    let bar = egui::Rect::from_min_size(
        rect.left_bottom() + egui::vec2(10.0, -150.0),
        egui::vec2(14.0, 130.0),
    );
    // 32-band vertical colour ramp (top = max).
    let bands = 32;
    for b in 0..bands {
        let t = 1.0 - (b as f32 + 0.5) / bands as f32;
        let c = crate::colormap::sample_named(&state.colormap, t);
        let y0 = bar.top() + bar.height() * (b as f32 / bands as f32);
        let y1 = bar.top() + bar.height() * ((b + 1) as f32 / bands as f32);
        painter.rect_filled(
            egui::Rect::from_min_max(egui::pos2(bar.left(), y0), egui::pos2(bar.right(), y1)),
            0.0,
            egui::Color32::from_rgb(
                (c[0] * 255.0) as u8,
                (c[1] * 255.0) as u8,
                (c[2] * 255.0) as u8,
            ),
        );
    }
    let (lo, hi) = state
        .effective_range()
        .map_or((0.0, 1.0), |(l, h)| (f64::from(l), f64::from(h)));
    for i in 0..5 {
        let f = i as f32 / 4.0;
        let v = hi + (lo - hi) * f64::from(f);
        let y = bar.top() + bar.height() * f;
        painter.text(
            egui::pos2(bar.right() + 4.0, y),
            egui::Align2::LEFT_CENTER,
            format!("{v:.3e}"),
            font.clone(),
            fg,
        );
    }
}

/// Cut-gizmo accent colour (`phase-5-m8.md` Decision 84) — the
/// existing M8 orange.
pub const CUT_GIZMO_COLOR: egui::Color32 = egui::Color32::from_rgb(255, 160, 70);

/// Slice-gizmo accent colour (`phase-5-m9.md` Decision 87) — a
/// contrasting cyan so cut + slice handles are distinguishable when
/// both verbs compose (`phase-4-m9.md` Decision 80).
pub const SLICE_GIZMO_COLOR: egui::Color32 = egui::Color32::from_rgb(80, 200, 240);

/// Paint a plane gizmo's handles (`phase-5-m8.md` Decision 84;
/// `phase-5-m9.md` Decision 87 reuses verbatim with a different
/// accent): a small ring at the plane origin + a normal-direction
/// arrow, projected through the live camera. Pure egui shapes — no
/// new wgpu pipeline (VB-001 / the M3 additive seam is untouched).
fn draw_plane_gizmo(
    painter: &egui::Painter,
    rect: egui::Rect,
    plane: CutPlaneState,
    camera: &Camera,
    accent: egui::Color32,
) {
    let w = rect.width().max(1.0) as u32;
    let h = rect.height().max(1.0) as u32;
    let origin = glam::Vec3::from(plane.origin);
    let n_world = glam::Vec3::from(plane.normal).normalize_or_zero();
    if n_world.length_squared() < 1e-12 {
        return;
    }
    // Arrow shaft length scales with the camera's view radius so the
    // gizmo stays at a sensible screen size regardless of model scale.
    let len = (camera.distance * 0.15).max(1e-3);
    let tip_world = origin + n_world * len;
    let Some(o_f) = camera.project(origin, w, h) else {
        return;
    };
    let Some(t_f) = camera.project(tip_world, w, h) else {
        return;
    };
    let o = egui::pos2(
        rect.min.x + o_f.x * rect.width(),
        rect.min.y + o_f.y * rect.height(),
    );
    let t = egui::pos2(
        rect.min.x + t_f.x * rect.width(),
        rect.min.y + t_f.y * rect.height(),
    );
    let stroke = egui::Stroke::new(1.5, accent);
    // Origin handle: filled disc + stroked ring.
    painter.circle_filled(o, 3.0, accent);
    painter.circle_stroke(o, 8.0, stroke);
    // Normal handle: shaft + tip cap.
    painter.line_segment([o, t], stroke);
    painter.circle_filled(t, 4.0, accent);
}

fn axes_gizmo(painter: &egui::Painter, rect: egui::Rect, camera: Option<&Camera>) {
    let o = rect.right_bottom() + egui::vec2(-44.0, -44.0);
    let len = 26.0;
    let stroke = |c| egui::Stroke::new(2.0, c);
    let red = egui::Color32::from_rgb(220, 70, 70);
    let green = egui::Color32::from_rgb(70, 200, 90);
    let blue = egui::Color32::from_rgb(90, 130, 230);
    match camera {
        // Track the live view: project each world axis into screen
        // space via the camera basis (screen y is down, so up flips).
        Some(c) => {
            let (right, up, _) = c.basis();
            let screen = |axis: glam::Vec3| egui::vec2(axis.dot(right) * len, -axis.dot(up) * len);
            for (dir, col) in [
                (glam::Vec3::X, red),
                (glam::Vec3::Y, green),
                (glam::Vec3::Z, blue),
            ] {
                painter.line_segment([o, o + screen(dir)], stroke(col));
            }
        }
        // Static triad (headless composite / not attached) — the M3
        // placeholder, byte-stable for that gate.
        None => {
            painter.line_segment([o, o + egui::vec2(len, 0.0)], stroke(red));
            painter.line_segment([o, o + egui::vec2(0.0, -len)], stroke(green));
            painter.line_segment([o, o + egui::vec2(-len * 0.7, len * 0.7)], stroke(blue));
        }
    }
}

fn dashed_rect(painter: &egui::Painter, r: egui::Rect, stroke: egui::Stroke) {
    let dash = 6.0;
    let gap = 4.0;
    let seg = |a: egui::Pos2, b: egui::Pos2| {
        let total = (b - a).length();
        let dir = (b - a) / total.max(1e-3);
        let mut t = 0.0;
        while t < total {
            let s = a + dir * t;
            let e = a + dir * (t + dash).min(total);
            painter.line_segment([s, e], stroke);
            t += dash + gap;
        }
    };
    seg(r.left_top(), r.right_top());
    seg(r.right_top(), r.right_bottom());
    seg(r.right_bottom(), r.left_bottom());
    seg(r.left_bottom(), r.left_top());
}
