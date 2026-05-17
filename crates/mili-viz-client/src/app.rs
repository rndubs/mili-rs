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
use winit::event::WindowEvent;
use winit::event_loop::{ActiveEventLoop, EventLoop};
use winit::window::{Window, WindowId};

use crate::camera::Camera;
use crate::egui_layer::EguiPaint;
use crate::mesh::Mesh;
use crate::renderer::Renderer;
use crate::session::Session;
use crate::shell::{build_shell_ui, Overlay, ResultInfo, SessionPhase, ShellState, UiAction};

struct WindowState {
    window: Arc<Window>,
    surface: wgpu::Surface<'static>,
    config: wgpu::SurfaceConfiguration,
    renderer: Renderer,
    egui: EguiPaint,
    egui_winit: egui_winit::State,
}

struct App {
    rt: tokio::runtime::Runtime,
    session: Session,
    shell: ShellState,
    camera: Camera,
    mesh: Option<Mesh>,
    range: Option<(f32, f32)>,
    bounds: Option<(glam::Vec3, f32)>,
    state: Option<WindowState>,
    last_anim: Instant,
}

impl App {
    fn ingest_deltas(&mut self) {
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
                }
                Some(pb::state_delta::Payload::Loaded(l)) => self.apply_loaded(&l),
                Some(pb::state_delta::Payload::State(n)) => self.shell.state = n,
                Some(pb::state_delta::Payload::Result(r)) => self.apply_result(&r),
                Some(pb::state_delta::Payload::Closed(_)) => {
                    self.shell.phase = SessionPhase::NotAttached;
                    self.shell.loaded = None;
                    self.shell.result = None;
                    self.mesh = None;
                }
                _ => {}
            }
        }
    }

    fn apply_loaded(&mut self, l: &pb::LoadedState) {
        self.shell.loaded = Some(crate::shell::LoadedInfo {
            db: l.db.clone(),
            num_states: l.num_states,
            state_times: l.state_times.clone(),
            class_names: l.class_names.clone(),
        });
        if !l.db.is_empty() && self.shell.phase == SessionPhase::NotAttached {
            self.shell.phase = SessionPhase::AttachedIdle;
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
                self.range = if r.result.is_empty() {
                    None
                } else {
                    Some((r.min as f32, r.max as f32))
                };
                let b = mesh.bounds();
                self.bounds = Some(b);
                self.camera = Camera::looking_at(b.0, b.1);
                if let Some(ws) = &mut self.state {
                    ws.renderer.upload_mesh(&mesh, self.range);
                }
                self.mesh = Some(mesh);
            }
        }
    }

    fn apply_action(&mut self, a: &UiAction) {
        let cmd = match a {
            UiAction::First => Some(step(pb::step::Dir::First)),
            UiAction::Prev => Some(step(pb::step::Dir::Prev)),
            UiAction::Next => Some(step(pb::step::Dir::Next)),
            UiAction::Last => Some(step(pb::step::Dir::Last)),
            UiAction::ViewReset | UiAction::Fit => {
                if let Some((c, r)) = self.bounds {
                    self.camera = Camera::looking_at(c, r);
                }
                Some(pb::command::Cmd::View(pb::View {
                    op: Some(pb::view::Op::Reset(true)),
                }))
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
            // Client-only: already applied to ShellState by the UI.
            UiAction::SetStride(_)
            | UiAction::ToggleOverlay(_)
            | UiAction::SelectBottomTab(_)
            | UiAction::CollapseBottomTabs => None,
        };
        if let Some(cmd) = cmd {
            let _ = self.rt.block_on(self.session.execute(cmd));
        }
    }
}

fn step(dir: pb::step::Dir) -> pb::command::Cmd {
    pb::command::Cmd::Step(pb::Step { dir: dir as i32 })
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
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("mili-viz device"),
            required_features: wgpu::Features::empty(),
            required_limits: wgpu::Limits::downlevel_defaults(),
            experimental_features: wgpu::ExperimentalFeatures::default(),
            memory_hints: wgpu::MemoryHints::default(),
            trace: wgpu::Trace::Off,
        }))
        .expect("request device");

        let size = window.inner_size();
        let caps = surface.get_capabilities(&adapter);
        let format = caps.formats[0];
        let config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format,
            width: size.width.max(1),
            height: size.height.max(1),
            present_mode: caps.present_modes[0],
            alpha_mode: caps.alpha_modes[0],
            view_formats: vec![],
            desired_maximum_frame_latency: 2,
        };
        surface.configure(&device, &config);

        let mut renderer = Renderer::new(device, queue, format);
        if let Some(mesh) = &self.mesh {
            renderer.upload_mesh(mesh, self.range);
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
        });
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        if self.state.is_none() {
            return;
        }
        if let Some(ws) = &mut self.state {
            let _ = ws.egui_winit.on_window_event(&ws.window, &event);
        }
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::Resized(size) => {
                if let Some(ws) = &mut self.state {
                    ws.config.width = size.width.max(1);
                    ws.config.height = size.height.max(1);
                    ws.surface.configure(ws.renderer.device(), &ws.config);
                    ws.window.request_redraw();
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

        // Mesh pass — unchanged Renderer::render.
        ws.renderer.render(&view, w, h, &self.camera);

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
        // Persisted-toggle hook (overlay on/off between sessions) is a
        // tweak surface — out of M3 (wireframes §"Tweaks").
        let _ = Overlay::Title;
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

    let mut shell = ShellState::default();
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
        range: None,
        bounds: None,
        state: None,
        last_anim: Instant::now(),
    };
    event_loop.run_app(&mut app)?;
    Ok(())
}
