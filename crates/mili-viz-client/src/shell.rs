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

use egui::Ui;

use crate::camera::Camera;

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

/// How the renderer draws the mesh (VB-003). The default
/// [`RenderMode::Shaded`] is the unchanged single filled
/// `TriangleList` pass, so the byte-stable M3 composite path
/// (`render_shell_to_image`, always `Shaded`) is unaffected
/// (`bug-tracker.md` VB-001).
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
}

impl RenderMode {
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            RenderMode::Shaded => "shaded",
            RenderMode::Edges => "shaded + edges",
            RenderMode::Wireframe => "wireframe",
        }
    }
}

/// The three peer bottom tabs (wireframes §"Bottom tabs";
/// `phase-5-m3.5.md` Decision 51).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BottomTab {
    /// Layer-0 raw griz / `grizinit` stream (Decision 48).
    CommandLine,
    /// Subprocess+`attach()` runner — disabled placeholder until
    /// Phase 6 `pygriz` lands (Decision 49).
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
}

/// Built-in derived result names the Phase 4 server supports
/// (`status.md` M5–M5d). The frozen proto carries no svar catalog, so
/// the M3 Results tree offers this representative derived set; the
/// primal / time-indep sub-trees are collapsed placeholders until a
/// catalog path exists (out of frozen-proto scope — recorded in
/// `phase-5-m3.md` Decision 47's neighbourhood).
pub const DERIVED_RESULTS: &[&str] = &[
    "disp_mag",
    "disp_x",
    "pressure",
    "eff_stress",
    "prin_stress1",
    "prin_strain1",
    "triaxiality",
];

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
            hidden_materials: std::collections::BTreeSet::new(),
            selected_result: None,
            bottom_tab: None,
            transcript: Vec::new(),
            cmdline_input: String::new(),
            time_history: Vec::new(),
            colormap: "cool".to_string(),
            legend_min: None,
            legend_max: None,
            render_mode: RenderMode::default(),
            scene_frac: None,
            camera: None,
            model_aabb: None,
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

    /// Toggle client-side picking. Turning it off clears the readout
    /// back to `—`. Pure client state; the action is observability-only
    /// (no proto command).
    pub fn toggle_picking(&mut self) -> UiAction {
        self.picking = !self.picking;
        if !self.picking {
            self.pick = "—".to_string();
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
    // Full extent before any panel carves into it; the leftover
    // central rect is normalized against this below.
    let full = ui.max_rect();

    egui::Panel::top("menu")
        .exact_size(26.0)
        .show_inside(ui, |ui| {
            egui::MenuBar::new().ui(ui, |ui| {
                let _ = ui.menu_button("Control", |_| {});
                ui.menu_button("Rendering", |ui| {
                    for mode in [RenderMode::Shaded, RenderMode::Edges, RenderMode::Wireframe] {
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
                });
                ui.menu_button("Picking", |ui| {
                    let mark = if state.picking { "● " } else { "○ " };
                    if ui.button(format!("{mark}enable picking")).clicked() {
                        actions.push(state.toggle_picking());
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

    bottom_tabs(ui, state, &mut actions);

    egui::Panel::left("dock")
        .resizable(true)
        .default_size(230.0)
        .show_inside(ui, |ui| {
            left_dock(ui, state, &mut actions);
        });

    // Collapsed AI rail (28 px) — placeholder only; the panel + agent
    // loop are M6 (`phase-5-m3.md` Goal).
    egui::Panel::right("ai")
        .resizable(false)
        .exact_size(28.0)
        .show_inside(ui, |ui| {
            ui.vertical_centered(|ui| {
                ui.add_space(8.0);
                ui.label("AI");
            });
        });

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

        egui::CollapsingHeader::new(format!("Results · {}", DERIVED_RESULTS.len()))
            .default_open(true)
            .show(ui, |ui| {
                egui::CollapsingHeader::new("derived")
                    .default_open(true)
                    .show(ui, |ui| {
                        for &r in DERIVED_RESULTS {
                            let sel = state.selected_result.as_deref() == Some(r);
                            if ui.selectable_label(sel, r).clicked() {
                                actions.push(state.select_result(r));
                            }
                        }
                    });
                egui::CollapsingHeader::new("primal")
                    .default_open(false)
                    .show(ui, |ui| {
                        ui.weak("(catalog: M4+)");
                    });
                egui::CollapsingHeader::new("time-indep")
                    .default_open(false)
                    .show(ui, |ui| {
                        ui.weak("(catalog: M4+)");
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
                BottomTab::Scripting => scripting_tab(ui),
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

/// Scripting runner — a structured **disabled** placeholder
/// (`phase-5-m3.5.md` Decision 49): the functional subprocess+
/// `attach()` runner is blocked on the uncoded Phase 6 `pygriz`.
fn scripting_tab(ui: &mut egui::Ui) {
    let mut script = String::from("# import griz; s = griz.attach()\n");
    ui.add_enabled(
        false,
        egui::TextEdit::multiline(&mut script)
            .code_editor()
            .desired_rows(4)
            .desired_width(f32::INFINITY),
    );
    ui.horizontal(|ui| {
        ui.add_enabled(false, egui::Button::new("▶ Run"));
        ui.weak("scripting runner requires pygriz (Phase 6) — not yet available");
    });
    ui.add_enabled(
        false,
        egui::Label::new(egui::RichText::new("(no output)").weak()),
    );
    ui.weak("venv: — · attach: —");
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

fn status_bar(ui: &mut egui::Ui, state: &ShellState) {
    ui.horizontal_centered(|ui| {
        let txt = match state.phase {
            SessionPhase::NotAttached => "— not attached —".to_string(),
            _ => format!("● attached {}@{}", state.session_id, state.host),
        };
        ui.monospace(txt);
        ui.separator();
        ui.monospace("proto v1");
        ui.separator();
        ui.monospace(format!("pick: {}", state.pick));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.monospace(format!("fps {:.0}", state.fps));
        });
    });
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
