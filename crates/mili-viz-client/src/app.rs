//! The windowed path: a `winit` [`ApplicationHandler`] that owns a
//! surface, the mesh [`Renderer`], the additive [`EguiPaint`] layer,
//! and a live in-process [`Session`] (`phase-5-m3.md` Decision 46).
//!
//! Each redraw: drain the `Subscribe` broadcast into the
//! [`ShellState`], build the L1 `egui` shell, lower its [`UiAction`]s
//! to the **frozen** proto `Command`s, render the mesh pass
//! (unchanged) then composite the `egui` pass over it. The camera
//! stays server-authoritative — M3 emits the view command and
//! locally re-frames for responsiveness; the full reconcile is M4.
//! Not exercised by CI (no display); the gating test drives the
//! headless composite in `renderer.rs`.

use std::sync::Arc;
use std::time::Instant;

use mili_viz_proto::v1 as pb;
use winit::application::ApplicationHandler;
use winit::event::{ElementState, MouseButton, MouseScrollDelta, WindowEvent};
use winit::event_loop::{ActiveEventLoop, EventLoop};
use winit::window::{Window, WindowId};

use crate::camera::Camera;
use crate::egui_layer::EguiPaint;
use crate::mesh::Mesh;
use crate::renderer::Renderer;
use crate::session::Session;
use crate::shell::{build_shell_ui, ResultInfo, SessionPhase, ShellState, UiAction};

struct WindowState {
    window: Arc<Window>,
    surface: wgpu::Surface<'static>,
    config: wgpu::SurfaceConfiguration,
    renderer: Renderer,
    egui: EguiPaint,
    egui_winit: egui_winit::State,
    /// Negotiated `max_texture_dimension_2d`; the surface config and
    /// the offscreen target are clamped to it (Decision 62).
    max_dim: u32,
}

/// Which mouse-drag gesture is in progress over the viewport
/// (`phase-5-m4.md` Decision 64).
#[derive(Clone, Copy, PartialEq, Eq)]
enum DragKind {
    /// Left-drag → orbit (azimuth/elevation).
    Orbit,
    /// Right/middle-drag → pan (focus translate in the view plane).
    Pan,
}

struct App {
    rt: tokio::runtime::Runtime,
    session: Session,
    shell: ShellState,
    camera: Camera,
    mesh: Option<Mesh>,
    bounds: Option<(glam::Vec3, f32)>,
    state: Option<WindowState>,
    last_anim: Instant,
    /// In-progress viewport drag + the last cursor position, the M4
    /// predict half (`phase-5-m4.md` Decision 64).
    drag: Option<DragKind>,
    last_cursor: Option<glam::Vec2>,
    /// Scripting-runner subprocess channel (`client.md` decision 3):
    /// the worker thread streams stdout/stderr + a final status; the
    /// frame loop drains it into [`ShellState`].
    script_tx: std::sync::mpsc::Sender<ScriptMsg>,
    script_rx: std::sync::mpsc::Receiver<ScriptMsg>,
}

/// A message from the scripting-runner worker thread.
enum ScriptMsg {
    /// A streamed stdout/stderr chunk (newline-terminated).
    Out(String),
    /// The child exited; payload is the `venv: … · attach: …` line.
    Done(String),
}

impl App {
    fn ingest_deltas(&mut self) {
        let prev_state = self.shell.state;
        for delta in self.session.poll_deltas() {
            match delta.payload {
                Some(pb::state_delta::Payload::Snapshot(s)) => {
                    if let Some(l) = s.loaded {
                        self.apply_loaded(&l);
                    }
                    if s.state > 0 {
                        self.shell.state = s.state;
                    }
                    if let Some(r) = s.result {
                        self.apply_result(&r);
                    }
                    if let Some(c) = s.camera {
                        self.apply_camera(&c);
                    }
                }
                Some(pb::state_delta::Payload::Loaded(l)) => self.apply_loaded(&l),
                Some(pb::state_delta::Payload::State(n)) => self.shell.state = n,
                Some(pb::state_delta::Payload::Result(r)) => self.apply_result(&r),
                Some(pb::state_delta::Payload::Camera(c)) => self.apply_camera(&c),
                Some(pb::state_delta::Payload::Closed(_)) => {
                    self.shell.phase = SessionPhase::NotAttached;
                    self.shell.loaded = None;
                    self.shell.result = None;
                    self.mesh = None;
                }
                _ => {}
            }
        }
        // The frozen contract makes `state`/`next`/`prev`/`first`/
        // `last` a bare `DELTA_STATE` — it moves the cursor but carries
        // no geometry, so the mesh would stay frozen while the counter
        // (and the time-history series, fed off `DELTA_RESULT`)
        // advanced. Round-trip the active result once per drain so the
        // deformed hull + field colours track the cursor during manual
        // stepping *and* animation (the loop coalesces a strided burst
        // to the final state, so this is one re-show, not `stride`).
        if self.shell.state != prev_state {
            self.refresh_result_geometry();
        }
    }

    /// Re-issue the active `show` so the server re-encodes geometry at
    /// the just-changed state (the contract-preserving counterpart to
    /// `DELTA_STATE` carrying no geometry). No-op before the first
    /// `show` (the in-process session always issues one on connect, so
    /// even the bare hull deforms per state).
    fn refresh_result_geometry(&mut self) {
        let Some(r) = self.shell.result.as_ref() else {
            return;
        };
        let cmd = pb::command::Cmd::Show(pb::Show {
            result: r.name.clone(),
            component: r.component.clone(),
            opts: std::collections::HashMap::new(),
        });
        let _ = self.rt.block_on(self.session.execute(cmd));
    }

    fn apply_loaded(&mut self, l: &pb::LoadedState) {
        let new_run = self.shell.loaded.as_ref().is_none_or(|p| p.db != l.db);
        self.shell.loaded = Some(crate::shell::LoadedInfo {
            db: l.db.clone(),
            num_states: l.num_states,
            state_times: l.state_times.clone(),
            class_names: l.class_names.clone(),
        });
        if !l.db.is_empty() && self.shell.phase == SessionPhase::NotAttached {
            self.shell.phase = SessionPhase::AttachedIdle;
        }
        // A new run must re-frame on its first geometry; clearing the
        // cached bounds is the trigger (`phase-5-m4.md` Decision 64).
        if new_run {
            self.bounds = None;
        }
    }

    /// Reconcile the predicted camera against the server-authoritative
    /// broadcast — last-broadcast-wins, unconditional, including
    /// self-caused echoes (`phase-5-m4.md` Decision 64). `azimuth`/
    /// `elevation`/`distance`/focus map field-for-field; the
    /// client-only projection planes are re-bracketed around the
    /// reconciled distance and the cached model radius.
    fn apply_camera(&mut self, c: &pb::CameraState) {
        let radius = self.bounds.map_or(1.0, |(_, r)| r);
        self.camera = camera_from_state(c, radius);
    }

    /// Re-colour the uploaded mesh from the current colormap +
    /// effective range without a geometry round-trip (the server
    /// treats `colormap`/`legend` as a recolor no-op — Decision 66).
    fn reupload(&mut self) {
        if let (Some(mesh), Some(ws)) = (&self.mesh, &mut self.state) {
            ws.renderer
                .upload_mesh(mesh, self.shell.effective_range(), &self.shell.colormap);
        }
    }

    fn apply_result(&mut self, r: &pb::ResultState) {
        let (verts, idx) = r
            .geometry
            .as_ref()
            .map_or((0, 0), |g| (g.num_vertices, g.num_indices));
        self.shell.result = Some(ResultInfo {
            name: r.result.clone(),
            component: r.component.clone(),
            min: r.min,
            max: r.max,
            num_vertices: verts,
            num_indices: idx,
        });
        // Accumulate the time-history series from the broadcast
        // result range (`phase-5-m3.5.md` Decision 50).
        self.shell.record_time_sample();
        if let Some(g) = &r.geometry {
            if let Ok(mesh) = self.session.resolve_geometry(g) {
                let b = mesh.bounds();
                // Frame once per run (Decision 64): the first geometry
                // after a `load` proposes the auto-frame to the
                // server via an absolute `SetCamera` so the
                // server-authoritative camera *is* the framed one;
                // subsequent results (state steps, recolours) keep
                // the user's orbit. Predict locally for this frame;
                // the `DELTA_CAMERA` echo reconciles it.
                let first_geometry = self.bounds.is_none();
                self.bounds = Some(b);
                if first_geometry {
                    let framed = Camera::looking_at(b.0, b.1);
                    self.camera = framed;
                    self.send_set_camera(&framed);
                }
                let range = self.shell.effective_range();
                if let Some(ws) = &mut self.state {
                    ws.renderer.upload_mesh(&mesh, range, &self.shell.colormap);
                }
                self.mesh = Some(mesh);
            }
        }
    }

    /// Lower the framed orbit camera to the frozen absolute
    /// `View::SetCamera` so the server's authoritative state becomes
    /// the auto-frame (`phase-5-m4.md` Decision 64).
    fn send_set_camera(&mut self, cam: &Camera) {
        let cmd = pb::command::Cmd::View(pb::View {
            op: Some(pb::view::Op::Set(pb::SetCamera {
                azimuth: f64::from(cam.azimuth),
                elevation: f64::from(cam.elevation),
                distance: f64::from(cam.distance),
                fx: Some(f64::from(cam.focus.x)),
                fy: Some(f64::from(cam.focus.y)),
                fz: Some(f64::from(cam.focus.z)),
            })),
        });
        let _ = self.rt.block_on(self.session.execute(cmd));
    }

    /// Size of the visible scene viewport in physical pixels (≥ 1), or
    /// a 1×1 fallback before the surface exists. This is the central
    /// rect the egui panels leave (not the full surface), so a
    /// full-drag orbit / pan is calibrated to what the user actually
    /// sees and matches the projection aspect the renderer uses.
    fn viewport(&self) -> (f32, f32) {
        self.state.as_ref().map_or((1.0, 1.0), |ws| {
            let w = ws.config.width.max(1) as f32;
            let h = ws.config.height.max(1) as f32;
            match self.shell.scene_frac {
                Some([_, _, fw, fh]) => ((fw * w).max(1.0), (fh * h).max(1.0)),
                None => (w, h),
            }
        })
    }

    /// Left-drag orbit: a full viewport width = π rad azimuth, a full
    /// height = π rad elevation. Predict locally, emit `View::Rotate`
    /// in radians (`phase-5-m4.md` Decisions 64–65; the pole guard is
    /// `Camera::eye`).
    fn orbit(&mut self, dx: f32, dy: f32) {
        let (w, h) = self.viewport();
        let daz = -(dx / w) * std::f32::consts::PI;
        let del = -(dy / h) * std::f32::consts::PI;
        self.camera.azimuth += daz;
        self.camera.elevation += del;
        let cmd = pb::command::Cmd::View(pb::View {
            op: Some(pb::view::Op::Rotate(pb::Rotate {
                x: f64::from(daz),
                y: f64::from(del),
                z: 0.0,
            })),
        });
        let _ = self.rt.block_on(self.session.execute(cmd));
    }

    /// Right/middle-drag pan: translate the focus in the view plane so
    /// the grabbed point tracks the cursor. Predict locally, emit
    /// `View::Translate` in world units (Decision 64).
    fn pan(&mut self, dx: f32, dy: f32) {
        let (_, h) = self.viewport();
        let half = (self.camera.fov_y * 0.5).tan();
        let per_px = 2.0 * self.camera.distance * half / h;
        let (right, up, _) = self.camera.basis();
        let delta = right * (-dx * per_px) + up * (dy * per_px);
        self.camera.focus += delta;
        let cmd = pb::command::Cmd::View(pb::View {
            op: Some(pb::view::Op::Translate(pb::Translate {
                dx: f64::from(delta.x),
                dy: f64::from(delta.y),
                dz: f64::from(delta.z),
            })),
        });
        let _ = self.rt.block_on(self.session.execute(cmd));
    }

    /// Scroll-wheel zoom: a geometric distance scale. Predict locally,
    /// emit `View::Zoom` (the server divides distance by `factor`,
    /// matching the prediction — Decision 64).
    fn zoom(&mut self, scroll: f32) {
        let factor = 1.1_f32.powf(scroll);
        if factor <= 0.0 {
            return;
        }
        self.camera.distance = (self.camera.distance / factor).max(f32::MIN_POSITIVE);
        let cmd = pb::command::Cmd::View(pb::View {
            op: Some(pb::view::Op::Zoom(pb::Zoom {
                factor: f64::from(factor),
            })),
        });
        let _ = self.rt.block_on(self.session.execute(cmd));
    }

    /// Client-side pick: ray-cast the window cursor (mapped into the
    /// scene sub-rect the egui panels leave, matching the renderer's
    /// projection) against the cached hull and fold the hit into the
    /// status-bar readout. No transport — the frozen proto carries no
    /// label catalog, so the readout is whatever the cached
    /// `GeometryRef` geometry actually has.
    fn pick_at(&mut self, cursor: glam::Vec2) {
        let Some(ws) = &self.state else { return };
        let (sw, sh) = (ws.config.width as f32, ws.config.height as f32);
        let (sx, sy, scw, sch) = match self.shell.scene_frac {
            Some([fx, fy, fw, fh]) => (fx * sw, fy * sh, (fw * sw).max(1.0), (fh * sh).max(1.0)),
            None => (0.0, 0.0, sw, sh),
        };
        let (rx, ry) = (cursor.x - sx, cursor.y - sy);
        if rx < 0.0 || ry < 0.0 || rx > scw || ry > sch {
            return;
        }
        let (o, d) = self.camera.ray_from_screen(rx, ry, scw as u32, sch as u32);
        let hit = self.mesh.as_ref().and_then(|m| m.pick(o, d));
        self.shell.apply_pick(hit.as_ref());
    }

    fn apply_action(&mut self, a: &UiAction) {
        let cmd = match a {
            UiAction::First => Some(step(pb::step::Dir::First)),
            UiAction::Prev => Some(step(pb::step::Dir::Prev)),
            UiAction::Next => Some(step(pb::step::Dir::Next)),
            UiAction::Last => Some(step(pb::step::Dir::Last)),
            UiAction::ViewReset | UiAction::Fit => {
                // `reset`/`fit` mean *re-frame to the model*. The
                // proto `View::reset` lowers server-side to a
                // distance-1 default with no knowledge of the model
                // bounds, so both lower to an absolute `SetCamera` of
                // the client's auto-frame instead (`phase-5-m4.md`
                // Decision 64); predict now, the echo reconciles.
                if let Some((c, r)) = self.bounds {
                    let framed = Camera::looking_at(c, r);
                    self.camera = framed;
                    self.send_set_camera(&framed);
                }
                None
            }
            UiAction::Show(name) => Some(pb::command::Cmd::Show(pb::Show {
                result: name.clone(),
                component: String::new(),
                opts: std::collections::HashMap::new(),
            })),
            UiAction::ToggleAnimate => {
                self.shell.phase = if self.shell.phase == SessionPhase::Animating {
                    SessionPhase::AttachedIdle
                } else {
                    SessionPhase::Animating
                };
                None
            }
            UiAction::StopAnimate => {
                self.shell.phase = SessionPhase::AttachedIdle;
                None
            }
            UiAction::RunCommand(raw) => {
                // Verbatim Layer-0 (`phase-5-m3.5.md` Decision 48):
                // the line goes out as `Command{ raw }`; the dim
                // outcome row is appended after the Execute returns.
                let res = self
                    .rt
                    .block_on(self.session.execute(pb::command::Cmd::Raw(raw.clone())));
                match res {
                    Ok(()) => self.shell.push_command_outcome(true, ""),
                    Err(e) => self.shell.push_command_outcome(false, &e.to_string()),
                }
                None
            }
            UiAction::SetColormap(name) => {
                // State already set by the UI; recolour the cached
                // mesh client-side and notify the server (a recolor
                // no-op there) for observability (Decision 66).
                self.reupload();
                Some(pb::command::Cmd::Colormap(pb::Colormap {
                    name: name.clone(),
                }))
            }
            UiAction::SetLegendLimits(min, max) => {
                self.reupload();
                Some(pb::command::Cmd::Legend(pb::LegendLimits {
                    min: *min,
                    max: *max,
                }))
            }
            UiAction::SetMaterialVisible {
                class_name,
                visible,
            } => Some(pb::command::Cmd::Material(pb::MaterialVisibility {
                enable: *visible,
                class_name: class_name.clone(),
                material: None,
            })),
            UiAction::SetRenderMode(m) => {
                // Pure-client (VB-003): retarget the renderer; no
                // proto command (the frozen set is untouched).
                if let Some(ws) = &mut self.state {
                    ws.renderer.set_mode(*m);
                }
                None
            }
            UiAction::RunScript(src) => {
                // Pure-client (`client.md` decision 3): spawn the
                // managed `pygriz` subprocess; output streams back
                // through the channel into ShellState each frame. No
                // proto command — the script owns its connection.
                self.spawn_script(src.clone());
                None
            }
            // Client-only: already applied to ShellState by the UI.
            UiAction::SetStride(_)
            | UiAction::ToggleOverlay(_)
            | UiAction::SelectBottomTab(_)
            | UiAction::CollapseBottomTabs
            | UiAction::TogglePicking
            | UiAction::SetTheme(_)
            | UiAction::SetDockCollapsed(_)
            | UiAction::SetFocusMode(_) => None,
        };
        if let Some(cmd) = cmd {
            let _ = self.rt.block_on(self.session.execute(cmd));
        }
    }

    /// Spawn the managed `pygriz` runner for `src` on a worker thread
    /// (`client.md` decision 3). Windowed-only; not CI-exercised.
    fn spawn_script(&self, src: String) {
        let tx = self.script_tx.clone();
        std::thread::spawn(move || run_script_subprocess(&src, &tx));
    }

    /// Drain the script worker's streamed output into [`ShellState`]
    /// (called once per frame, mirroring `ingest_deltas`).
    fn poll_script(&mut self) {
        let msgs: Vec<ScriptMsg> = self.script_rx.try_iter().collect();
        for msg in msgs {
            match msg {
                ScriptMsg::Out(s) => self.shell.push_script_output(&s),
                ScriptMsg::Done(status) => self.shell.finish_script(&status),
            }
        }
    }
}

/// Run a scripting-tab buffer as a `pygriz` subprocess, streaming
/// stdout/stderr back through `tx` (`client.md` decision 3,
/// `phase-6-m2.md`). The interpreter is `$GRIZ_PYTHON` (else
/// `python3`); the pure-Python `griz` package is made importable via
/// `$GRIZ_PYGRIZ_SRC` (else the repo's `python/pygriz/src`) prepended
/// to `PYTHONPATH`. A full managed/`pip install`ed venv (decision 3's
/// production shape) is the documented forward path; this is the
/// smallest wiring that genuinely runs the landed Phase 6 path.
fn run_script_subprocess(src: &str, tx: &std::sync::mpsc::Sender<ScriptMsg>) {
    use std::io::{BufRead, BufReader};
    use std::path::Path;
    use std::process::{Command, Stdio};

    let py = std::env::var("GRIZ_PYTHON").unwrap_or_else(|_| "python3".to_string());
    let pygriz_src = std::env::var("GRIZ_PYGRIZ_SRC").unwrap_or_else(|_| {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("..")
            .join("python")
            .join("pygriz")
            .join("src")
            .to_string_lossy()
            .into_owned()
    });
    let pythonpath = match std::env::var("PYTHONPATH") {
        Ok(p) if !p.is_empty() => {
            let sep = if cfg!(windows) { ";" } else { ":" };
            format!("{pygriz_src}{sep}{p}")
        }
        _ => pygriz_src,
    };

    let tmp = std::env::temp_dir().join(format!("griz-script-{}.py", std::process::id()));
    if let Err(e) = std::fs::write(&tmp, src) {
        let _ = tx.send(ScriptMsg::Done(format!(
            "venv: {py} · attach: launch · could not stage script: {e}"
        )));
        return;
    }

    let mut child = match Command::new(&py)
        .arg(&tmp)
        .env("PYTHONPATH", &pythonpath)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
    {
        Ok(c) => c,
        Err(e) => {
            let _ = std::fs::remove_file(&tmp);
            let _ = tx.send(ScriptMsg::Done(format!(
                "venv: {py} · attach: launch · spawn failed: {e}"
            )));
            return;
        }
    };

    let mut readers = Vec::new();
    for pipe in [
        child
            .stdout
            .take()
            .map(|s| Box::new(s) as Box<dyn std::io::Read + Send>),
        child
            .stderr
            .take()
            .map(|s| Box::new(s) as Box<dyn std::io::Read + Send>),
    ]
    .into_iter()
    .flatten()
    {
        let tx = tx.clone();
        readers.push(std::thread::spawn(move || {
            for line in BufReader::new(pipe).lines().map_while(Result::ok) {
                if tx.send(ScriptMsg::Out(format!("{line}\n"))).is_err() {
                    break;
                }
            }
        }));
    }

    let status = child.wait();
    for r in readers {
        let _ = r.join();
    }
    let _ = std::fs::remove_file(&tmp);
    let summary = match status {
        Ok(s) if s.success() => format!("venv: {py} (PYTHONPATH) · attach: launch · ok"),
        Ok(s) => format!(
            "venv: {py} (PYTHONPATH) · attach: launch · exited {}",
            s.code()
                .map_or_else(|| "(signal)".to_string(), |c| c.to_string())
        ),
        Err(e) => format!("venv: {py} · attach: launch · wait failed: {e}"),
    };
    let _ = tx.send(ScriptMsg::Done(summary));
}

fn step(dir: pb::step::Dir) -> pb::command::Cmd {
    pb::command::Cmd::Step(pb::Step { dir: dir as i32 })
}

/// Map a server `CameraState` onto the client orbit [`Camera`]
/// (`phase-5-m4.md` Decision 64). `azimuth`/`elevation`/`distance`
/// and focus copy field-for-field (Decision 40 shaped them 1:1, in
/// radians per Decision 65); `fov_y`/`z_near`/`z_far` are client-only
/// projection params the proto does not carry — bracketed around the
/// reconciled distance and the cached model `radius`, mirroring
/// [`Camera::looking_at`].
fn camera_from_state(c: &pb::CameraState, radius: f32) -> Camera {
    Camera::from_orbit(
        c.azimuth as f32,
        c.elevation as f32,
        c.distance as f32,
        glam::Vec3::new(c.fx as f32, c.fy as f32, c.fz as f32),
        radius,
    )
}

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.state.is_some() {
            return;
        }
        let attrs = Window::default_attributes().with_title("mili-viz");
        let window = Arc::new(event_loop.create_window(attrs).expect("create window"));

        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
        let surface = instance
            .create_surface(window.clone())
            .expect("create surface");
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::default(),
            compatible_surface: Some(&surface),
            force_fallback_adapter: false,
        }))
        .expect("no compatible GPU adapter");
        // Keep `downlevel_defaults()` as the CI floor but raise the
        // 2048 `max_texture_dimension_2d` cap to what the adapter
        // actually supports: on a HiDPI display the window's physical
        // pixel size exceeds 2048, and `Surface::configure` validating
        // against a 2048 cap aborts inside winit's non-unwinding
        // `frame_did_change` (`phase-5-m4.md` Decision 62).
        let mut limits = wgpu::Limits::downlevel_defaults();
        limits.max_texture_dimension_2d = adapter.limits().max_texture_dimension_2d;
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("mili-viz device"),
            required_features: wgpu::Features::empty(),
            required_limits: limits.clone(),
            experimental_features: wgpu::ExperimentalFeatures::default(),
            memory_hints: wgpu::MemoryHints::default(),
            trace: wgpu::Trace::Off,
        }))
        .expect("request device");
        let max_dim = limits.max_texture_dimension_2d;

        let size = window.inner_size();
        let caps = surface.get_capabilities(&adapter);
        let format = caps.formats[0];
        let config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format,
            width: size.width.clamp(1, max_dim),
            height: size.height.clamp(1, max_dim),
            present_mode: caps.present_modes[0],
            alpha_mode: caps.alpha_modes[0],
            view_formats: vec![],
            desired_maximum_frame_latency: 2,
        };
        surface.configure(&device, &config);

        let mut renderer = Renderer::new(device, queue, format);
        if let Some(mesh) = &self.mesh {
            renderer.upload_mesh(mesh, self.shell.effective_range(), &self.shell.colormap);
        }
        let egui = EguiPaint::new(renderer.device(), format);
        let egui_winit = egui_winit::State::new(
            egui.context(),
            egui::ViewportId::ROOT,
            window.as_ref(),
            None,
            None,
            None,
        );
        self.state = Some(WindowState {
            window,
            surface,
            config,
            renderer,
            egui,
            egui_winit,
            max_dim,
        });
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        if self.state.is_none() {
            return;
        }
        let egui_consumed = self
            .state
            .as_mut()
            .map(|ws| ws.egui_winit.on_window_event(&ws.window, &event).consumed)
            .unwrap_or(false);
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::Resized(size) => {
                if let Some(ws) = &mut self.state {
                    ws.config.width = size.width.clamp(1, ws.max_dim);
                    ws.config.height = size.height.clamp(1, ws.max_dim);
                    ws.surface.configure(ws.renderer.device(), &ws.config);
                    ws.window.request_redraw();
                }
            }
            // M4 predict half (`phase-5-m4.md` Decision 64): a drag
            // mutates the local camera *now* and emits the matching
            // `View` op; the `DELTA_CAMERA` echo reconciles it. A
            // gesture only *starts* off an egui-unconsumed press (not
            // over a panel/widget) but, once begun, tracks the cursor
            // until release so dragging over a panel still orbits.
            WindowEvent::MouseInput { state, button, .. } => {
                if state == ElementState::Pressed {
                    if !egui_consumed {
                        // In picking mode a left press ray-casts the
                        // cached hull instead of starting an orbit;
                        // pan (right/middle) is unchanged.
                        if self.shell.picking && button == MouseButton::Left {
                            if let Some(c) = self.last_cursor {
                                self.pick_at(c);
                            }
                        } else {
                            self.drag = match button {
                                MouseButton::Left => Some(DragKind::Orbit),
                                MouseButton::Right | MouseButton::Middle => Some(DragKind::Pan),
                                _ => None,
                            };
                        }
                    }
                } else {
                    self.drag = None;
                }
            }
            WindowEvent::CursorMoved { position, .. } => {
                let p = glam::vec2(position.x as f32, position.y as f32);
                if let (Some(kind), Some(prev)) = (self.drag, self.last_cursor) {
                    let d = p - prev;
                    match kind {
                        DragKind::Orbit => self.orbit(d.x, d.y),
                        DragKind::Pan => self.pan(d.x, d.y),
                    }
                }
                self.last_cursor = Some(p);
            }
            WindowEvent::MouseWheel { delta, .. } => {
                if !egui_consumed {
                    let s = match delta {
                        MouseScrollDelta::LineDelta(_, y) => y,
                        MouseScrollDelta::PixelDelta(p) => (p.y as f32) / 50.0,
                    };
                    if s != 0.0 {
                        self.zoom(s);
                    }
                }
            }
            WindowEvent::RedrawRequested => self.redraw(),
            _ => {}
        }
        if let Some(ws) = &self.state {
            ws.window.request_redraw();
        }
    }
}

impl App {
    fn redraw(&mut self) {
        self.ingest_deltas();
        self.poll_script();

        // Animation: server-authoritative — step forward by `stride`
        // roughly every 80 ms while the phase is Animating.
        if self.shell.phase == SessionPhase::Animating && self.last_anim.elapsed().as_millis() >= 80
        {
            self.last_anim = Instant::now();
            let stride = self.shell.stride.max(1);
            for _ in 0..stride {
                let _ = self
                    .rt
                    .block_on(self.session.execute(step(pb::step::Dir::Next)));
            }
        }

        let Some(ws) = &mut self.state else {
            return;
        };
        let frame = match ws.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(f)
            | wgpu::CurrentSurfaceTexture::Suboptimal(f) => f,
            _ => {
                ws.surface.configure(ws.renderer.device(), &ws.config);
                return;
            }
        };
        let view = frame
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        let (w, h) = (ws.config.width, ws.config.height);

        // Mesh pass into just the central viewport the egui panels
        // leave (one-frame-stale on a resize — invisible, the rect
        // only moves while a panel is dragged). Framing + orbit are
        // then about the centre of the *visible* scene.
        let scene = self.shell.scene_frac.map(|[fx, fy, fw, fh]| {
            let (sw, sh) = (w as f32, h as f32);
            (fx * sw, fy * sh, (fw * sw).max(1.0), (fh * sh).max(1.0))
        });
        ws.renderer.render_in(&view, w, h, &self.camera, scene);

        // Publish the live camera + current-state AABB so the bbox /
        // axes overlays project against the real view (pure read of
        // distinct fields; `ws` only borrows `self.state`).
        self.shell.camera = Some(self.camera);
        self.shell.model_aabb = self.mesh.as_ref().map(crate::mesh::Mesh::aabb);

        // Additive egui pass; collect the frame's actions.
        let raw_input = ws.egui_winit.take_egui_input(&ws.window);
        let ppp = raw_input.viewport().native_pixels_per_point.unwrap_or(1.0);
        let mut actions = Vec::new();
        let shell = &mut self.shell;
        ws.egui.paint(
            ws.renderer.device(),
            ws.renderer.queue(),
            &view,
            &egui_wgpu::ScreenDescriptor {
                size_in_pixels: [w, h],
                pixels_per_point: ppp,
            },
            raw_input,
            |ui| actions = build_shell_ui(ui, shell),
        );
        frame.present();

        for a in &actions {
            self.apply_action(a);
        }
        // Cross-session tweak persistence (wireframes §"Tweaks";
        // MVP-cut 7): when a frame mutated a persisted field (an
        // overlay chip, Theme or Left-dock-collapse) re-write the
        // per-user config. Windowed-only — the headless
        // `render_shell_to_image` path never reaches here, so the M3
        // composite gate stays disk-free and byte-stable (VB-001).
        if actions.iter().any(crate::tweaks::is_persisted_action) {
            crate::tweaks::save(&crate::tweaks::PersistedTweaks::from_state(&self.shell));
        }
    }
}

/// Open the windowed shell over a live in-process [`Session`]. With
/// `root`, the session `load`s it and is *attached idle*; without, it
/// is *not attached* (the viewport shows the attach card). The
/// Phase 5 entrypoint.
///
/// # Errors
/// Returns a boxed error if the in-process session fails to connect or
/// the `winit` event loop fails to start.
pub fn run(root: Option<String>) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let rt = tokio::runtime::Runtime::new()?;
    let session = rt.block_on(Session::connect_in_process(root.as_deref()))?;
    let (script_tx, script_rx) = std::sync::mpsc::channel();

    let mut shell = ShellState::default();
    // Restore cross-session tweaks (wireframes §"Tweaks"; MVP-cut 7).
    // No config file ⇒ `PersistedTweaks::default`, whose `apply_to`
    // leaves `shell` byte-identical to `ShellState::default()`, so a
    // fresh machine is exactly the byte-stable default (VB-001).
    crate::tweaks::load().apply_to(&mut shell);
    if root.is_some() {
        shell.phase = SessionPhase::AttachedIdle;
    }

    let event_loop = EventLoop::new()?;
    let mut app = App {
        rt,
        session,
        shell,
        camera: Camera::default(),
        mesh: None,
        bounds: None,
        state: None,
        last_anim: Instant::now(),
        drag: None,
        last_cursor: None,
        script_tx,
        script_rx,
    };
    event_loop.run_app(&mut app)?;
    Ok(())
}
