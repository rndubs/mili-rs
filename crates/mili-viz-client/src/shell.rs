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

/// A client-side intent emitted by the shell. The windowed app lowers
/// the transport-affecting variants to the **frozen** proto `Command`
/// (`phase-5-m3.md` Decision 46); the pure-client variants
/// (`ToggleOverlay`, `SetStride`) have already been applied to
/// [`ShellState`] and are returned for observability/persistence.
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
    /// Status-bar pick readout (`—` until picking lands, M4+).
    pub pick: String,
    /// Currently highlighted Results-tree row.
    pub selected_result: Option<String>,
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
            selected_result: None,
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

    egui::Panel::top("menu")
        .exact_size(26.0)
        .show_inside(ui, |ui| {
            egui::MenuBar::new().ui(ui, |ui| {
                for m in [
                    "Control",
                    "Rendering",
                    "Picking",
                    "Results",
                    "Time",
                    "Plot",
                    "Help",
                ] {
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

    // Collapsed bottom-tabs stub — command line / scripting /
    // time-history are M3.5 (`phase-5-m3.md` Goal).
    egui::Panel::bottom("tabs")
        .exact_size(22.0)
        .show_inside(ui, |ui| {
            ui.horizontal(|ui| {
                ui.add_enabled(false, egui::Button::new("command line"));
                ui.add_enabled(false, egui::Button::new("scripting"));
                ui.add_enabled(false, egui::Button::new("time-history"));
                ui.weak("(M3.5)");
            });
        });

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

        let n_classes = state.loaded.as_ref().map_or(0, |l| l.class_names.len());
        egui::CollapsingHeader::new(format!("Materials · {n_classes}"))
            .default_open(false)
            .show(ui, |ui| {
                if let Some(l) = &state.loaded {
                    for c in &l.class_names {
                        ui.horizontal(|ui| {
                            ui.label("●");
                            ui.label(c);
                        });
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
        axes_gizmo(&painter, rect);
    }

    if state.overlays.bbox {
        // Dashed inset rectangle standing in for the model bbox; the
        // true projected box is M4 (needs the live camera).
        let inset = rect.shrink2(egui::vec2(rect.width() * 0.18, rect.height() * 0.18));
        let stroke = egui::Stroke::new(1.0, egui::Color32::from_white_alpha(90));
        dashed_rect(&painter, inset, stroke);
    }
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
        let c = crate::colormap::sample(t);
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
    let (lo, hi) = state.result.as_ref().map_or((0.0, 1.0), |r| (r.min, r.max));
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

fn axes_gizmo(painter: &egui::Painter, rect: egui::Rect) {
    let o = rect.right_bottom() + egui::vec2(-44.0, -44.0);
    let len = 26.0;
    let stroke = |c| egui::Stroke::new(2.0, c);
    painter.line_segment(
        [o, o + egui::vec2(len, 0.0)],
        stroke(egui::Color32::from_rgb(220, 70, 70)),
    );
    painter.line_segment(
        [o, o + egui::vec2(0.0, -len)],
        stroke(egui::Color32::from_rgb(70, 200, 90)),
    );
    painter.line_segment(
        [o, o + egui::vec2(-len * 0.7, len * 0.7)],
        stroke(egui::Color32::from_rgb(90, 130, 230)),
    );
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
