//! Minimal `wgpu` renderer: an indexed triangle mesh viewed through
//! the orbit [`Camera`], with a depth buffer and a single fixed
//! directional light (`phase-5-m2.md` Decision 42). Structured
//! render-to-texture-first (`phase-5-m1.md` Decision 39) — the
//! windowed path in `app.rs` is a thin wrapper that points the same
//! [`Renderer`] at a surface texture, and the gating test points it
//! at an off-screen texture.
//!
//! M2 replaces M1's hard-coded triangle (it was scaffolding,
//! `phase-5-m1.md` Decision 40) with the decoded server [`Mesh`].

use bytemuck::{Pod, Zeroable};
use glam::Mat4;
use wgpu::util::DeviceExt;

use crate::camera::Camera;
use crate::mesh::Mesh;

/// Off-screen render target format. Non-sRGB so pixel-readback in the
/// gating test is exact (no gamma surprises).
pub const OFFSCREEN_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

/// Depth target format (depth-test `Less` so an overlapping closed
/// hull renders correctly without relying on consistent winding).
pub const DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;

/// Background clear color (linear RGBA). A corner pixel of a rendered
/// frame is this; a framed-mesh center pixel is not.
pub const CLEAR_COLOR: wgpu::Color = wgpu::Color {
    r: 0.02,
    g: 0.02,
    b: 0.08,
    a: 1.0,
};

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Vertex {
    position: [f32; 3],
    normal: [f32; 3],
    color: [f32; 3],
}

/// The M2 uniform lit base colour, used when a vertex carries no
/// `MVG2` scalar (bare hull / no result). Kept identical to the M2
/// shader constant so `m2_render_server_output.rs` is unaffected
/// (`phase-5-m3.md` Decision 47).
const BASE_COLOR: [f32; 3] = [0.62, 0.68, 0.80];

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Uniforms {
    view_proj: [[f32; 4]; 4],
}

struct MeshBuffers {
    vertex_buffer: wgpu::Buffer,
    index_buffer: wgpu::Buffer,
    index_count: u32,
}

/// A device + pipeline that can draw an uploaded indexed [`Mesh`] into
/// any `TextureView` of `target_format`.
pub struct Renderer {
    device: wgpu::Device,
    queue: wgpu::Queue,
    pipeline: wgpu::RenderPipeline,
    uniform_buffer: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
    mesh: Option<MeshBuffers>,
}

impl Renderer {
    /// Build a renderer for an existing device/queue targeting
    /// `target_format` (the window surface format, or
    /// [`OFFSCREEN_FORMAT`] for headless). No mesh until
    /// [`upload_mesh`](Self::upload_mesh).
    #[must_use]
    pub fn new(
        device: wgpu::Device,
        queue: wgpu::Queue,
        target_format: wgpu::TextureFormat,
    ) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("mili-viz mesh shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("mesh.wgsl").into()),
        });

        let uniform_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("camera uniforms"),
            size: std::mem::size_of::<Uniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("camera bind group layout"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });

        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("camera bind group"),
            layout: &bind_group_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: uniform_buffer.as_entire_binding(),
            }],
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("mesh pipeline layout"),
            bind_group_layouts: &[Some(&bind_group_layout)],
            immediate_size: 0,
        });

        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("mesh pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                buffers: &[wgpu::VertexBufferLayout {
                    array_stride: std::mem::size_of::<Vertex>() as u64,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x3, 2 => Float32x3],
                }],
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: target_format,
                    blend: Some(wgpu::BlendState::REPLACE),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                cull_mode: None,
                ..Default::default()
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: DEPTH_FORMAT,
                depth_write_enabled: Some(true),
                depth_compare: Some(wgpu::CompareFunction::Less),
                stencil: wgpu::StencilState::default(),
                bias: wgpu::DepthBiasState::default(),
            }),
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });

        Self {
            device,
            queue,
            pipeline,
            uniform_buffer,
            bind_group,
            mesh: None,
        }
    }

    /// The underlying device — `app.rs` configures its surface with
    /// it.
    #[must_use]
    pub fn device(&self) -> &wgpu::Device {
        &self.device
    }

    /// The underlying queue — the `egui` pass writes its buffers
    /// through it (`phase-5-m3.md` Decision 45).
    #[must_use]
    pub fn queue(&self) -> &wgpu::Queue {
        &self.queue
    }

    /// Upload (or replace) the mesh this renderer draws. When the
    /// mesh carries an `MVG2` scalar and `range` is `Some((min,max))`
    /// the per-vertex scalar is mapped through the cool→warm colormap
    /// (`phase-5-m3.md` Decision 47); otherwise every vertex gets the
    /// M2 uniform [`BASE_COLOR`] (so a bare hull renders exactly as in
    /// M2).
    pub fn upload_mesh(&mut self, mesh: &Mesh, range: Option<(f32, f32)>, colormap: &str) {
        let color_of = |i: usize| -> [f32; 3] {
            match (&mesh.scalars, range) {
                (Some(s), Some((lo, hi))) => {
                    let t = crate::colormap::normalize(s[i], lo, hi);
                    crate::colormap::sample_named(colormap, t)
                }
                _ => BASE_COLOR,
            }
        };
        let verts: Vec<Vertex> = mesh
            .positions
            .iter()
            .zip(&mesh.normals)
            .enumerate()
            .map(|(i, (&position, &normal))| Vertex {
                position,
                normal,
                color: color_of(i),
            })
            .collect();
        let vertex_buffer = self
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("mesh vertices"),
                contents: bytemuck::cast_slice(&verts),
                usage: wgpu::BufferUsages::VERTEX,
            });
        let index_buffer = self
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("mesh indices"),
                contents: bytemuck::cast_slice(&mesh.indices),
                usage: wgpu::BufferUsages::INDEX,
            });
        self.mesh = Some(MeshBuffers {
            vertex_buffer,
            index_buffer,
            index_count: mesh.indices.len() as u32,
        });
    }

    /// Record + submit one frame: clear, then draw the uploaded mesh
    /// (if any) into `view`.
    pub fn render(&self, view: &wgpu::TextureView, width: u32, height: u32, camera: &Camera) {
        let vp: Mat4 = camera.view_projection(width, height);
        let uniforms = Uniforms {
            view_proj: vp.to_cols_array_2d(),
        };
        self.queue
            .write_buffer(&self.uniform_buffer, 0, bytemuck::bytes_of(&uniforms));

        // Defensively clamp the offscreen depth target to the
        // negotiated `max_texture_dimension_2d` so an over-large
        // surface size never trips texture-size validation
        // (`phase-5-m4.md` Decision 62).
        let max_dim = self.device.limits().max_texture_dimension_2d;
        let depth = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("depth"),
            size: wgpu::Extent3d {
                width: width.clamp(1, max_dim),
                height: height.clamp(1, max_dim),
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: DEPTH_FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        });
        let depth_view = depth.create_view(&wgpu::TextureViewDescriptor::default());

        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("frame encoder"),
            });
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("mesh pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view,
                    resolve_target: None,
                    depth_slice: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(CLEAR_COLOR),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &depth_view,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(1.0),
                        store: wgpu::StoreOp::Discard,
                    }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            if let Some(m) = &self.mesh {
                pass.set_pipeline(&self.pipeline);
                pass.set_bind_group(0, &self.bind_group, &[]);
                pass.set_vertex_buffer(0, m.vertex_buffer.slice(..));
                pass.set_index_buffer(m.index_buffer.slice(..), wgpu::IndexFormat::Uint32);
                pass.draw_indexed(0..m.index_count, 0, 0..1);
            }
        }
        self.queue.submit(std::iter::once(encoder.finish()));
    }
}

/// Acquire a `wgpu` device with no surface (headless). Returns `None`
/// when no adapter is available — there is no GPU and no software
/// rasterizer (e.g. a bare CI runner). Callers treat `None` as
/// skip-on-absent (`phase-5-m1.md` Decision 39), not an error.
#[must_use]
pub fn headless_device() -> Option<(wgpu::Device, wgpu::Queue)> {
    pollster::block_on(async {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::default(),
                compatible_surface: None,
                force_fallback_adapter: false,
            })
            .await
            .ok()?;
        // Mirror the windowed path: `downlevel_defaults()` floor with
        // the adapter's real `max_texture_dimension_2d` so a HiDPI
        // offscreen size never trips validation (Decision 62).
        let mut limits = wgpu::Limits::downlevel_defaults();
        limits.max_texture_dimension_2d = adapter.limits().max_texture_dimension_2d;
        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor {
                label: Some("mili-viz headless device"),
                required_features: wgpu::Features::empty(),
                required_limits: limits,
                experimental_features: wgpu::ExperimentalFeatures::default(),
                memory_hints: wgpu::MemoryHints::default(),
                trace: wgpu::Trace::Off,
            })
            .await
            .ok()?;
        Some((device, queue))
    })
}

/// Render the built-in unit triangle off-screen — the M1 pipeline
/// smoke (`phase-5-m2.md` Decision 43). `None` when no adapter is
/// available (skip-on-absent).
#[must_use]
pub fn render_to_image(width: u32, height: u32, camera: &Camera) -> Option<Vec<u8>> {
    render_mesh_to_image(width, height, camera, &Mesh::unit_triangle())
}

/// Render `mesh` off-screen at `width`x`height` and return the frame
/// as tightly-packed RGBA8 (`width*height*4` bytes, row-major,
/// top-left origin). `None` when no adapter is available
/// (skip-on-absent).
#[must_use]
pub fn render_mesh_to_image(
    width: u32,
    height: u32,
    camera: &Camera,
    mesh: &Mesh,
) -> Option<Vec<u8>> {
    let (device, queue) = headless_device()?;
    let mut renderer = Renderer::new(device, queue, OFFSCREEN_FORMAT);
    renderer.upload_mesh(mesh, None, "cool");
    Some(renderer.read_back(width, height, camera))
}

/// Render `mesh` **and** the `egui` shell (`state`) into one
/// off-screen texture and read it back as tightly-packed RGBA8 — the
/// M3 composite seam (`phase-5-m3.md` Decision 45). The mesh pass is
/// the unchanged [`Renderer::render`]; the `egui` pass is the
/// additive non-clearing [`EguiPaint`] pass over the same view.
/// `None` when no adapter is available (skip-on-absent).
#[must_use]
pub fn render_shell_to_image(
    width: u32,
    height: u32,
    camera: &Camera,
    mesh: &Mesh,
    range: Option<(f32, f32)>,
    state: &mut crate::shell::ShellState,
) -> Option<Vec<u8>> {
    let (device, queue) = headless_device()?;
    let mut renderer = Renderer::new(device, queue, OFFSCREEN_FORMAT);
    renderer.upload_mesh(mesh, range, "cool");
    let mut egui = crate::egui_layer::EguiPaint::new(&renderer.device, OFFSCREEN_FORMAT);

    let texture = renderer.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("offscreen shell target"),
        size: wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: OFFSCREEN_FORMAT,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());

    // Mesh pass — unchanged Renderer::render.
    renderer.render(&view, width, height, camera);

    // Additive egui pass on the same view (load, no depth).
    let raw_input = egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(
            egui::Pos2::ZERO,
            egui::vec2(width as f32, height as f32),
        )),
        ..Default::default()
    };
    egui.paint(
        &renderer.device,
        &renderer.queue,
        &view,
        &egui_wgpu::ScreenDescriptor {
            size_in_pixels: [width, height],
            pixels_per_point: 1.0,
        },
        raw_input,
        |ui| {
            let _ = crate::shell::build_shell_ui(ui, state);
        },
    );

    Some(renderer.copy_back(&texture, width, height))
}

impl Renderer {
    /// Render into a fresh off-screen texture and copy the pixels
    /// back to host memory as tightly-packed RGBA8.
    #[must_use]
    fn read_back(&self, width: u32, height: u32, camera: &Camera) -> Vec<u8> {
        let texture = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("offscreen target"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: OFFSCREEN_FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        self.render(&view, width, height, camera);
        self.copy_back(&texture, width, height)
    }

    /// Copy an already-rendered off-screen texture back to host
    /// memory as tightly-packed RGBA8 (`width*height*4` bytes,
    /// row-major, top-left origin). Shared by the mesh-only M1/M2
    /// readback and the M3 composite path.
    #[must_use]
    fn copy_back(&self, texture: &wgpu::Texture, width: u32, height: u32) -> Vec<u8> {
        // copy_texture_to_buffer requires bytes_per_row aligned to
        // COPY_BYTES_PER_ROW_ALIGNMENT (256); pad and unpad on read.
        let unpadded = width * 4;
        let align = wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
        let padded = unpadded.div_ceil(align) * align;
        let buffer = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("readback buffer"),
            size: u64::from(padded) * u64::from(height),
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });

        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("readback encoder"),
            });
        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(padded),
                    rows_per_image: Some(height),
                },
            },
            wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
        );
        self.queue.submit(std::iter::once(encoder.finish()));

        let slice = buffer.slice(..);
        slice.map_async(wgpu::MapMode::Read, |_| {});
        self.device
            .poll(wgpu::PollType::wait_indefinitely())
            .expect("device poll");

        let data = slice.get_mapped_range();
        let mut out = Vec::with_capacity((unpadded * height) as usize);
        for row in 0..height {
            let start = (row * padded) as usize;
            out.extend_from_slice(&data[start..start + unpadded as usize]);
        }
        drop(data);
        buffer.unmap();
        out
    }
}
