//! The windowed path: a thin [`winit::application::ApplicationHandler`]
//! that owns a surface and points the shared [`Renderer`] at it
//! (`phase-5-m1.md` Decision 39). Not exercised by CI (no display);
//! the gating test drives the headless path in `renderer.rs`.

use std::sync::Arc;

use winit::application::ApplicationHandler;
use winit::event::WindowEvent;
use winit::event_loop::{ActiveEventLoop, EventLoop};
use winit::window::{Window, WindowId};

use crate::camera::Camera;
use crate::mesh::Mesh;
use crate::renderer::Renderer;

struct WindowState {
    window: Arc<Window>,
    surface: wgpu::Surface<'static>,
    config: wgpu::SurfaceConfiguration,
    renderer: Renderer,
}

#[derive(Default)]
struct App {
    camera: Camera,
    mesh: Option<Mesh>,
    state: Option<WindowState>,
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
            renderer.upload_mesh(mesh);
            let (center, radius) = mesh.bounds();
            self.camera = Camera::looking_at(center, radius);
        }
        self.state = Some(WindowState {
            window,
            surface,
            config,
            renderer,
        });
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        let Some(state) = self.state.as_mut() else {
            return;
        };
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::Resized(size) => {
                state.config.width = size.width.max(1);
                state.config.height = size.height.max(1);
                state
                    .surface
                    .configure(state.renderer.device(), &state.config);
                state.window.request_redraw();
            }
            WindowEvent::RedrawRequested => {
                let frame = match state.surface.get_current_texture() {
                    wgpu::CurrentSurfaceTexture::Success(f)
                    | wgpu::CurrentSurfaceTexture::Suboptimal(f) => f,
                    _ => {
                        state
                            .surface
                            .configure(state.renderer.device(), &state.config);
                        return;
                    }
                };
                let view = frame
                    .texture
                    .create_view(&wgpu::TextureViewDescriptor::default());
                state
                    .renderer
                    .render(&view, state.config.width, state.config.height, &self.camera);
                frame.present();
            }
            _ => {}
        }
    }
}

/// Open a window and render `mesh` (auto-framed) until the window is
/// closed. `None` renders the empty (clear) scene — interactive
/// `load` is Phase 5 M3. The Phase 5 entrypoint.
///
/// # Errors
/// Returns the `winit` event-loop error if the loop fails to start.
pub fn run(mesh: Option<Mesh>) -> Result<(), winit::error::EventLoopError> {
    let event_loop = EventLoop::new()?;
    let mut app = App {
        mesh,
        ..App::default()
    };
    event_loop.run_app(&mut app)
}
