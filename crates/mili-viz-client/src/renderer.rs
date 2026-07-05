//! Minimal `wgpu` renderer: an indexed triangle mesh viewed through
//! the orbit [`Camera`], with a depth buffer and a camera-relative
//! headlight (originally a fixed directional light, `phase-5-m2.md`
//! Decision 42; see the rendering-quality paragraph below). Structured
//! render-to-texture-first (`phase-5-m1.md` Decision 39) — the
//! windowed path in `app.rs` is a thin wrapper that points the same
//! [`Renderer`] at a surface texture, and the gating test points it
//! at an off-screen texture.
//!
//! M2 replaces M1's hard-coded triangle (it was scaffolding,
//! `phase-5-m1.md` Decision 40) with the decoded server [`Mesh`].
//!
//! VB-003 adds an element-edge / wireframe pass. The mesh's unique
//! undirected edges ([`Mesh::edge_indices`]) feed a second
//! `LineList` pipeline that shares the camera bind group and vertex
//! buffer. It is **opt-in** via [`Renderer::set_mode`]:
//! [`RenderMode::Shaded`] (default) is byte-for-byte the original
//! single filled pass — so the headless composite gate
//! (`render_shell_to_image`, always `Shaded`) and VB-001 stay
//! untouched; [`RenderMode::Edges`] overlays the depth-tested edges
//! on the filled hull (front edges only — hidden-line overlay);
//! [`RenderMode::Wireframe`] draws only the edges over the cleared
//! background (see-through wireframe).
//!
//! Rendering-quality pass (2026-07): the fill is depth-biased back so
//! coplanar edge quads win the depth test (no more stippled lines);
//! the blended edge pass no longer writes depth (no more speckle at
//! shared vertices); edge colour is per-mode (light in the fill-less
//! modes, dark charcoal over the fill) with a projected-length density
//! fade so dense meshes dissolve into the fill instead of blacking it
//! out; and shading moved to a camera-relative headlight + specular in
//! linear RGB with explicit sRGB handling on both the windowed and
//! headless targets (see `mesh.wgsl` / `edges.wgsl`).
//!
//! [`Mesh::edge_indices`]: crate::mesh::Mesh::edge_indices

use bytemuck::{Pod, Zeroable};
use glam::Mat4;
use wgpu::util::DeviceExt;

use crate::camera::Camera;
use crate::mesh::Mesh;
use crate::shell::RenderMode;

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
    /// `(viewport_px.x, viewport_px.y, line_width_px, srgb_flag)`.
    /// The first three are consumed by the screen-space line-quad edge
    /// pass (`edges.wgsl`) for pixel-correct line widths; `srgb_flag`
    /// is `1.0` when the color target is an sRGB format (hardware
    /// encodes, shaders output linear) and `0.0` when it is not (the
    /// headless RGBA8 path — shaders encode manually so both targets
    /// display identically).
    viewport_and_width: [f32; 4],
    /// World-space unit vector surface→light. A camera-relative
    /// headlight tilted up/right, recomputed per frame from the orbit
    /// camera so the model is always lit no matter its orientation.
    /// `.w` carries the edge-pass global alpha strength (see
    /// [`edge_params_for`]) — the mesh shader ignores it.
    light_dir: [f32; 4],
    /// World-space unit vector surface→eye (the headlight's specular
    /// partner).
    view_dir: [f32; 4],
    /// Edge-pass parameters: `rgb` = per-mode edge colour (linear),
    /// `w` = density-fade floor (`1.0` disables the projected-length
    /// fade — Wireframe; `0.0` lets dense overlay edges vanish).
    edge_params: [f32; 4],
}

/// Piecewise sRGB → linear decode. The base colour, colormap stops and
/// edge colours are authored as display (sRGB) values — the legend UI
/// shows them verbatim — so they are linearized before lighting math
/// and re-encoded at display time (`mesh.wgsl`/`edges.wgsl`).
fn srgb_to_linear(c: [f32; 3]) -> [f32; 3] {
    c.map(|v| {
        if v <= 0.04045 {
            v / 12.92
        } else {
            ((v + 0.055) / 1.055).powf(2.4)
        }
    })
}

/// Pixel width of the screen-space line-quad edge pass. 1.5 px reads as
/// a crisp ~2-px line after the analytical 1-px AA feather and 4×
/// MSAA — tweak here if you want chunkier or thinner edges.
const LINE_WIDTH_PX: f32 = 1.5;

/// Edge colour (sRGB) for the overlay modes (`Edges`/`FeatureEdges`) —
/// dark charcoal over the lit fill, replacing the hard-coded opaque
/// black that swamped dense meshes.
const EDGE_COLOR_DARK: [f32; 3] = [0.08, 0.09, 0.11];

/// Edge colour (sRGB) for the fill-less / translucent modes
/// (`Wireframe`/`Xray`) — light steel that reads over the dark clear
/// colour (black-on-near-black was invisible).
const EDGE_COLOR_LIGHT: [f32; 3] = [0.75, 0.78, 0.88];

/// Per-mode `(edge_color_srgb, fade_floor, strength)`.
///
/// The fade floor is the minimum of the projected-edge-length alpha
/// fade (`edges.wgsl`): `1.0` disables the fade entirely (Wireframe —
/// with no fill beneath, fading dense edges would leave nothing),
/// `0.0` lets a dense overlay dissolve into the fill, and the
/// in-between floors keep sparse structural edges from ever fully
/// vanishing.
///
/// `strength` is a global alpha multiplier. Overlay edges stay < 1 so
/// even where projected edges pack tighter than a pixel (a strongly
/// foreshortened face — the length-based fade can't catch that) the
/// lit fill still shows through instead of blacking out.
fn edge_params_for(mode: RenderMode) -> ([f32; 3], f32, f32) {
    match mode {
        RenderMode::Wireframe => (EDGE_COLOR_LIGHT, 1.0, 1.0),
        RenderMode::Xray => (EDGE_COLOR_LIGHT, 0.3, 0.8),
        RenderMode::FeatureEdges => (EDGE_COLOR_DARK, 0.6, 0.9),
        _ => (EDGE_COLOR_DARK, 0.0, 0.55),
    }
}

/// Dihedral-angle threshold for [`RenderMode::FeatureEdges`]
/// (`planning/mili-viz/feature-edges.md` Decision 101). Triangle pairs
/// folding by more than this — plus boundary and non-manifold edges —
/// are kept; the rest of the mesh subdivision is hidden. 30° is the de
/// facto default across ParaView / Blender Auto-Smooth / OpenSCAD; a
/// future Preferences slider can wire the value through without
/// re-opening the milestone.
const FEATURE_EDGE_ANGLE_DEG: f32 = 30.0;

/// The 6 corners of the per-instance line quad (two triangles). `.x` ∈
/// `{0,1}` picks the start/end endpoint; `.y` ∈ `{-1,+1}` picks which
/// side of the line the corner extrudes toward. Built once per
/// `Renderer` and reused for every frame and every edge.
const EDGE_CORNERS: [[f32; 2]; 6] = [
    [0.0, -1.0],
    [1.0, -1.0],
    [1.0, 1.0],
    [0.0, -1.0],
    [1.0, 1.0],
    [0.0, 1.0],
];

struct MeshBuffers {
    vertex_buffer: wgpu::Buffer,
    index_buffer: wgpu::Buffer,
    index_count: u32,
    /// Per-instance endpoint pairs for the screen-space line-quad edge
    /// pass — packed `[ax, ay, az, bx, by, bz]` per edge, stride 24.
    /// `edge_count` is the **instance count** (number of edges), so the
    /// pass draws `6` corners × `edge_count` instances. Empty edge
    /// buffers (a degenerate, no-triangle mesh) are never drawn.
    edge_endpoint_buffer: wgpu::Buffer,
    edge_count: u32,
    /// Same packing as `edge_endpoint_buffer`, but holding only the
    /// dihedral-feature edges (silhouette + creases) used by
    /// [`RenderMode::FeatureEdges`]. Computed at upload via
    /// [`Mesh::compute_feature_edges`]; reuses the same edge pipeline
    /// — only the per-instance buffer differs.
    feature_edge_endpoint_buffer: wgpu::Buffer,
    feature_edge_count: u32,
}

/// A device + pipeline that can draw an uploaded indexed [`Mesh`] into
/// any `TextureView` of `target_format`.
pub struct Renderer {
    device: wgpu::Device,
    queue: wgpu::Queue,
    pipeline: wgpu::RenderPipeline,
    /// `LineList` pipeline for the element-edge / wireframe pass
    /// (VB-003). Built unconditionally but only recorded when
    /// `mode != Shaded`, so the default path's command stream is
    /// byte-for-byte unchanged.
    edge_pipeline: wgpu::RenderPipeline,
    /// Alpha-blended fill pipeline (Phase 5 M7 Decision 81):
    /// depth-test on, depth-write off, source factor =
    /// `Constant`/`OneMinusConstant`. The per-pass blend constant
    /// applied via [`wgpu::RenderPass::set_blend_constant`] picks the
    /// alpha (default 0.35). Only recorded for `Translucent`/`Xray`.
    translucent_pipeline: wgpu::RenderPipeline,
    uniform_buffer: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
    /// Static 6-corner quad shared by every edge instance. Built once
    /// in `new_with_samples` so `upload_mesh` only has to rebuild the
    /// per-instance endpoint buffer.
    edge_corner_buffer: wgpu::Buffer,
    mesh: Option<MeshBuffers>,
    mode: RenderMode,
    /// MSAA sample count for the mesh + edge + translucent pipelines.
    /// `1` is the headless byte-stable default (`Renderer::new`); the
    /// windowed app picks `4` via [`Renderer::new_with_samples`] so the
    /// 1-px `LineList` edge pass and the hull silhouette don't alias.
    /// When > 1, `render_in` allocates an MSAA color texture and
    /// resolves into the caller-supplied `view`.
    sample_count: u32,
    /// Color format the pipelines were built against — needed to
    /// allocate a matching MSAA color attachment when `sample_count > 1`.
    target_format: wgpu::TextureFormat,
}

/// Phase 5 M7 Decision 81: default alpha for `Translucent`/`Xray`. A
/// future `Preferences → Transparency` tweak can lower this through a
/// setter without re-opening the milestone.
const TRANSLUCENT_ALPHA: f32 = 0.35;

impl Renderer {
    /// Build a renderer for an existing device/queue targeting
    /// `target_format` (the window surface format, or
    /// [`OFFSCREEN_FORMAT`] for headless). No mesh until
    /// [`upload_mesh`](Self::upload_mesh).
    ///
    /// Pipelines are built with `sample_count = 1` — the headless
    /// byte-stable path (`render_mesh_to_image`, `render_shell_to_image`)
    /// depends on this so the VB-001 / status 23 composite gate stays
    /// pixel-exact. The windowed app calls [`new_with_samples`] with
    /// `4` for MSAA.
    ///
    /// [`new_with_samples`]: Self::new_with_samples
    #[must_use]
    pub fn new(
        device: wgpu::Device,
        queue: wgpu::Queue,
        target_format: wgpu::TextureFormat,
    ) -> Self {
        Self::new_with_samples(device, queue, target_format, 1)
    }

    /// As [`new`](Self::new), but with an explicit MSAA `sample_count`
    /// for the mesh + edge + translucent pipelines. The windowed app
    /// picks `4` so 1-px `LineList` edges (VB-003) and the hull
    /// silhouette read crisply; the headless paths keep `1` so the
    /// byte-stable composite gate (VB-001) is untouched. When > 1,
    /// [`render_in`](Self::render_in) allocates a matching MSAA color
    /// texture and resolves into the caller-supplied view.
    #[must_use]
    pub fn new_with_samples(
        device: wgpu::Device,
        queue: wgpu::Queue,
        target_format: wgpu::TextureFormat,
        sample_count: u32,
    ) -> Self {
        let multisample = wgpu::MultisampleState {
            count: sample_count,
            mask: !0,
            alpha_to_coverage_enabled: false,
        };
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
                // FRAGMENT is needed for the edge pass — `edges.wgsl`
                // reads `viewport_and_width.z` (line width) in its
                // fragment stage for the analytical-AA alpha taper.
                // VERTEX-only would pass wgpu validation on the mesh
                // pipeline but trip "shader stage not in visibility
                // flags" when the edge pipeline is created on a real
                // device (the headless tests in this container don't
                // catch it — they skip-on-absent).
                visibility: wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT,
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
                // Push the fill slightly *back* (polygon-offset style)
                // so the coplanar edge quads reliably win the depth
                // test. The edge quads keep their endpoints' exact
                // depth but are expanded sideways in screen space, so
                // without this bias an obliquely-viewed face occludes
                // parts of its own edges — the broken/stippled line
                // look. Slope scaling handles steep faces; the small
                // constant covers face-on ones.
                bias: wgpu::DepthBiasState {
                    constant: 2,
                    slope_scale: 1.0,
                    clamp: 0.0,
                },
            }),
            multisample,
            multiview_mask: None,
            cache: None,
        });

        // Edge / wireframe pass — screen-space line quads. Each input
        // edge is one **instance** of a 6-vertex triangle quad that the
        // vertex shader expands along the screen-space normal to a
        // `LINE_WIDTH_PX`-wide ribbon with a 1-px AA feather. Two
        // vertex buffers:
        //   slot 0 — per-vertex `corner: vec2<f32>` from the static
        //            `edge_corner_buffer` built below;
        //   slot 1 — per-instance `(endpoint_a, endpoint_b)` packed
        //            into `MeshBuffers.edge_endpoint_buffer` by
        //            `upload_mesh`.
        // Alpha-blended over the filled hull so the AA feather
        // composites cleanly; depth-test `LessEqual`, depth-write
        // **off** — the opaque fill owns the depth buffer (and is
        // depth-biased back so coplanar edges win). Writing depth from
        // this blended pass made the ~zero-alpha AA feather of one
        // edge quad occlude later overlapping quads, speckling every
        // shared vertex; and in Wireframe mode it only ever
        // implemented a see-through wireframe anyway (all edges of a
        // bare hull are drawn — that is the documented semantics).
        let edge_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("mili-viz edge shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("edges.wgsl").into()),
        });
        let edge_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("edge pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &edge_shader,
                entry_point: Some("vs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                buffers: &[
                    // Per-vertex quad corner.
                    wgpu::VertexBufferLayout {
                        array_stride: (std::mem::size_of::<f32>() * 2) as u64,
                        step_mode: wgpu::VertexStepMode::Vertex,
                        attributes: &wgpu::vertex_attr_array![0 => Float32x2],
                    },
                    // Per-instance endpoint pair (two vec3 packed).
                    wgpu::VertexBufferLayout {
                        array_stride: (std::mem::size_of::<f32>() * 6) as u64,
                        step_mode: wgpu::VertexStepMode::Instance,
                        attributes: &wgpu::vertex_attr_array![1 => Float32x3, 2 => Float32x3],
                    },
                ],
            },
            fragment: Some(wgpu::FragmentState {
                module: &edge_shader,
                entry_point: Some("fs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: target_format,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
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
                depth_write_enabled: Some(false),
                depth_compare: Some(wgpu::CompareFunction::LessEqual),
                stencil: wgpu::StencilState::default(),
                bias: wgpu::DepthBiasState::default(),
            }),
            multisample,
            multiview_mask: None,
            cache: None,
        });

        let edge_corner_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("edge corner quad"),
            contents: bytemuck::cast_slice(&EDGE_CORNERS),
            usage: wgpu::BufferUsages::VERTEX,
        });

        // Translucent fill pipeline (Phase 5 M7 Decision 81). Reuses
        // the mesh shader and vertex layout; the only differences are
        // blend = constant-alpha over destination, depth-write off so
        // overlapping translucent triangles don't occlude each other.
        // The shader still outputs alpha = 1.0; the visible alpha is
        // the per-pass blend constant set in [`render_in`].
        let translucent_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("translucent pipeline"),
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
                    blend: Some(wgpu::BlendState {
                        color: wgpu::BlendComponent {
                            src_factor: wgpu::BlendFactor::Constant,
                            dst_factor: wgpu::BlendFactor::OneMinusConstant,
                            operation: wgpu::BlendOperation::Add,
                        },
                        alpha: wgpu::BlendComponent {
                            src_factor: wgpu::BlendFactor::One,
                            dst_factor: wgpu::BlendFactor::Zero,
                            operation: wgpu::BlendOperation::Add,
                        },
                    }),
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
                depth_write_enabled: Some(false),
                depth_compare: Some(wgpu::CompareFunction::Less),
                stencil: wgpu::StencilState::default(),
                bias: wgpu::DepthBiasState::default(),
            }),
            multisample,
            multiview_mask: None,
            cache: None,
        });

        Self {
            device,
            queue,
            pipeline,
            edge_pipeline,
            translucent_pipeline,
            uniform_buffer,
            bind_group,
            edge_corner_buffer,
            mesh: None,
            mode: RenderMode::default(),
            sample_count,
            target_format,
        }
    }

    /// Switch the render mode (VB-003). The windowed app calls this
    /// when the `Rendering` menu emits [`UiAction::SetRenderMode`].
    /// [`RenderMode::Shaded`] (the default) leaves the pass identical
    /// to M2/M3.
    ///
    /// [`UiAction::SetRenderMode`]: crate::shell::UiAction::SetRenderMode
    pub fn set_mode(&mut self, mode: RenderMode) {
        self.mode = mode;
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
        // Colormap stops and BASE_COLOR are authored as display (sRGB)
        // values — the legend swatches show them verbatim — so they are
        // linearized here; lighting runs in linear and the shader
        // re-encodes at display time.
        let color_of = |i: usize| -> [f32; 3] {
            srgb_to_linear(match (&mesh.scalars, range) {
                (Some(s), Some((lo, hi))) => {
                    let t = crate::colormap::normalize(s[i], lo, hi);
                    crate::colormap::sample_named(colormap, t)
                }
                _ => BASE_COLOR,
            })
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
        // Phase 5 M7 Decision 82: prefer the server-supplied per-element
        // edge buffer (`MVG3.element_edges`) over the on-the-fly
        // triangle-edge extractor. The extractor over-emits face
        // diagonals for any superclass whose face is not already a
        // triangle (VB-005, the hex case); the server table enumerates
        // the element's true edges. The legacy fallback keeps `MVG1`/
        // `MVG2` byte-stable (VB-001).
        let edges: Vec<u32> = mesh
            .element_edges
            .clone()
            .unwrap_or_else(|| mesh.edge_indices());
        // Pack the per-instance endpoint pairs the screen-space line
        // quad pipeline consumes: one `[ax, ay, az, bx, by, bz]` per
        // edge. `edge_count` is the instance count.
        let endpoints: Vec<[f32; 6]> = edges
            .chunks_exact(2)
            .map(|p| {
                let a = mesh.positions[p[0] as usize];
                let b = mesh.positions[p[1] as usize];
                [a[0], a[1], a[2], b[0], b[1], b[2]]
            })
            .collect();
        let edge_count = endpoints.len() as u32;
        // wgpu rejects a zero-sized buffer; a degenerate (no-triangle)
        // mesh keeps a 1-instance placeholder that `edge_count == 0`
        // never draws.
        let endpoints_payload: &[[f32; 6]] = if endpoints.is_empty() {
            &[[0.0; 6]]
        } else {
            &endpoints
        };
        let edge_endpoint_buffer =
            self.device
                .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some("mesh edge endpoints"),
                    contents: bytemuck::cast_slice(endpoints_payload),
                    usage: wgpu::BufferUsages::VERTEX,
                });

        // Feature / "geometry-only" edges (RenderMode::FeatureEdges) —
        // computed once per upload; reuses the edge pipeline verbatim.
        let feature_edges = mesh.compute_feature_edges(FEATURE_EDGE_ANGLE_DEG.to_radians());
        let feature_endpoints: Vec<[f32; 6]> = feature_edges
            .chunks_exact(2)
            .map(|p| {
                let a = mesh.positions[p[0] as usize];
                let b = mesh.positions[p[1] as usize];
                [a[0], a[1], a[2], b[0], b[1], b[2]]
            })
            .collect();
        let feature_edge_count = feature_endpoints.len() as u32;
        let feature_payload: &[[f32; 6]] = if feature_endpoints.is_empty() {
            &[[0.0; 6]]
        } else {
            &feature_endpoints
        };
        let feature_edge_endpoint_buffer =
            self.device
                .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some("mesh feature edge endpoints"),
                    contents: bytemuck::cast_slice(feature_payload),
                    usage: wgpu::BufferUsages::VERTEX,
                });

        self.mesh = Some(MeshBuffers {
            vertex_buffer,
            index_buffer,
            index_count: mesh.indices.len() as u32,
            edge_endpoint_buffer,
            edge_count,
            feature_edge_endpoint_buffer,
            feature_edge_count,
        });
    }

    /// Record + submit one frame: clear, then draw the uploaded mesh
    /// (if any) into `view`, filling the whole `width`x`height` target.
    pub fn render(&self, view: &wgpu::TextureView, width: u32, height: u32, camera: &Camera) {
        self.render_in(view, width, height, camera, None);
    }

    /// As [`render`](Self::render), but when `scene` is
    /// `Some((x, y, w, h))` the mesh is drawn into just that sub-rect
    /// of the `width`x`height` attachment (physical pixels, top-left
    /// origin) and the projection aspect is taken from `w`/`h`. The
    /// windowed app passes the central viewport the `egui` panels
    /// leave, so the model is framed — and orbits — about the centre
    /// of the *visible* scene, not the centre of the full surface that
    /// the left dock / bottom tabs occlude.
    pub fn render_in(
        &self,
        view: &wgpu::TextureView,
        width: u32,
        height: u32,
        camera: &Camera,
        scene: Option<(f32, f32, f32, f32)>,
    ) {
        let (proj_w, proj_h) = match scene {
            Some((_, _, w, h)) => (w.max(1.0) as u32, h.max(1.0) as u32),
            None => (width, height),
        };
        let vp: Mat4 = camera.view_projection(proj_w, proj_h);
        // Camera-relative headlight: from over the viewer's upper-right
        // shoulder, recomputed per frame so the model is lit in every
        // orbit orientation (`basis()` forward points eye→focus).
        let (right, up, forward) = camera.basis();
        let light = (-forward + up * 0.5 + right * 0.3).normalize();
        let view_dir = -forward;
        let (edge_srgb, fade_floor, edge_strength) = edge_params_for(self.mode);
        let edge_lin = srgb_to_linear(edge_srgb);
        let uniforms = Uniforms {
            view_proj: vp.to_cols_array_2d(),
            // The edge pass needs the **scene** viewport size in pixels
            // (not the framebuffer) — that's what `set_viewport` shrinks
            // the rasterisation to, and the screen-space line-quad math
            // is in pixel units of that rect. `.w` tells the shaders
            // whether the target encodes sRGB in hardware or they must
            // encode manually (the headless RGBA8 path).
            viewport_and_width: [
                proj_w as f32,
                proj_h as f32,
                LINE_WIDTH_PX,
                if self.target_format.is_srgb() { 1.0 } else { 0.0 },
            ],
            light_dir: [light.x, light.y, light.z, edge_strength],
            view_dir: [view_dir.x, view_dir.y, view_dir.z, 0.0],
            edge_params: [edge_lin[0], edge_lin[1], edge_lin[2], fade_floor],
        };
        self.queue
            .write_buffer(&self.uniform_buffer, 0, bytemuck::bytes_of(&uniforms));

        // Defensively clamp the offscreen depth target to the
        // negotiated `max_texture_dimension_2d` so an over-large
        // surface size never trips texture-size validation
        // (`phase-5-m4.md` Decision 62).
        let max_dim = self.device.limits().max_texture_dimension_2d;
        let target_w = width.clamp(1, max_dim);
        let target_h = height.clamp(1, max_dim);
        let depth = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("depth"),
            size: wgpu::Extent3d {
                width: target_w,
                height: target_h,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: self.sample_count,
            dimension: wgpu::TextureDimension::D2,
            format: DEPTH_FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        });
        let depth_view = depth.create_view(&wgpu::TextureViewDescriptor::default());

        // MSAA color attachment when `sample_count > 1` — the pipelines
        // require it to match their declared sample count. The single-
        // sample headless path (`sample_count == 1`) keeps writing the
        // caller's `view` directly so the byte-stable composite gate
        // (VB-001) sees the same command stream.
        let msaa_color = (self.sample_count > 1).then(|| {
            let tex = self.device.create_texture(&wgpu::TextureDescriptor {
                label: Some("msaa color"),
                size: wgpu::Extent3d {
                    width: target_w,
                    height: target_h,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: self.sample_count,
                dimension: wgpu::TextureDimension::D2,
                format: self.target_format,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
                view_formats: &[],
            });
            tex.create_view(&wgpu::TextureViewDescriptor::default())
        });

        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("frame encoder"),
            });
        {
            // MSAA path: render into the multisampled color and resolve
            // into the caller's single-sample `view`; the surface only
            // needs the resolved pixels stored (`StoreOp::Discard` on
            // the MSAA target is fine but explicit `Store` keeps the
            // resolve well-defined across backends). Single-sample path
            // writes the view directly — byte-identical command stream
            // to the original code (VB-001).
            let (color_view, resolve_target) = match msaa_color.as_ref() {
                Some(msaa) => (msaa, Some(view)),
                None => (view, None),
            };
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("mesh pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: color_view,
                    resolve_target,
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
            if let Some((x, y, w, h)) = scene {
                // Clamp into the attachment so a stale (pre-resize)
                // rect can never trip viewport validation.
                let fw = width as f32;
                let fh = height as f32;
                // `.max(1.0)` on the upper bound keeps it ≥ the lower
                // bound so `f32::clamp` can never be called min > max
                // (it panics) even for a degenerate stale rect.
                let x = x.clamp(0.0, (fw - 1.0).max(0.0));
                let y = y.clamp(0.0, (fh - 1.0).max(0.0));
                let w = w.clamp(1.0, (fw - x).max(1.0));
                let h = h.clamp(1.0, (fh - y).max(1.0));
                pass.set_viewport(x, y, w, h, 0.0, 1.0);
            }
            if let Some(m) = &self.mesh {
                // Shaded (default) keeps the exact M2/M3 command
                // sequence so the byte-stable composite gate (VB-001 /
                // status 23) is untouched. Edges adds a depth-tested
                // overlay; Wireframe draws only the edges (VB-003).
                // Phase 5 M7 (Decision 81): Translucent swaps the fill
                // pipeline for the alpha-blended one with depth-write
                // off; Xray is Translucent fill + the element-edge
                // overlay so cell-cell structure stays legible.
                let translucent_fill =
                    matches!(self.mode, RenderMode::Translucent | RenderMode::Xray);
                let draw_fill = self.mode != RenderMode::Wireframe;
                // FeatureEdges binds the dihedral-feature buffer; every
                // other edge-drawing mode binds the full element-edge
                // buffer.
                let feature_buf = matches!(self.mode, RenderMode::FeatureEdges);
                let (edge_buffer, edge_count) = if feature_buf {
                    (&m.feature_edge_endpoint_buffer, m.feature_edge_count)
                } else {
                    (&m.edge_endpoint_buffer, m.edge_count)
                };
                let draw_edges = matches!(
                    self.mode,
                    RenderMode::Edges
                        | RenderMode::Wireframe
                        | RenderMode::Xray
                        | RenderMode::FeatureEdges
                ) && edge_count > 0;
                if draw_fill {
                    if translucent_fill {
                        pass.set_blend_constant(wgpu::Color {
                            r: f64::from(TRANSLUCENT_ALPHA),
                            g: f64::from(TRANSLUCENT_ALPHA),
                            b: f64::from(TRANSLUCENT_ALPHA),
                            a: f64::from(TRANSLUCENT_ALPHA),
                        });
                        pass.set_pipeline(&self.translucent_pipeline);
                    } else {
                        pass.set_pipeline(&self.pipeline);
                    }
                    pass.set_bind_group(0, &self.bind_group, &[]);
                    pass.set_vertex_buffer(0, m.vertex_buffer.slice(..));
                    pass.set_index_buffer(m.index_buffer.slice(..), wgpu::IndexFormat::Uint32);
                    pass.draw_indexed(0..m.index_count, 0, 0..1);
                }
                if draw_edges {
                    // Screen-space line quads: 6 corners × `edge_count`
                    // instances. Slot 0 is the shared corner quad, slot
                    // 1 is the per-instance endpoint pair built by
                    // `upload_mesh` — element edges for the standard
                    // wireframe modes, dihedral-feature edges for
                    // `FeatureEdges`.
                    pass.set_pipeline(&self.edge_pipeline);
                    pass.set_bind_group(0, &self.bind_group, &[]);
                    pass.set_vertex_buffer(0, self.edge_corner_buffer.slice(..));
                    pass.set_vertex_buffer(1, edge_buffer.slice(..));
                    pass.draw(0..6, 0..edge_count);
                }
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
    render_mesh_to_image_with_mode(width, height, camera, mesh, RenderMode::Shaded)
}

/// As [`render_mesh_to_image`] but with an explicit [`RenderMode`] —
/// the headless leg of the VB-003 gating test (skip-on-absent). The
/// `Shaded` default keeps `render_mesh_to_image`'s output unchanged.
#[must_use]
pub fn render_mesh_to_image_with_mode(
    width: u32,
    height: u32,
    camera: &Camera,
    mesh: &Mesh,
    mode: RenderMode,
) -> Option<Vec<u8>> {
    let (device, queue) = headless_device()?;
    let mut renderer = Renderer::new(device, queue, OFFSCREEN_FORMAT);
    renderer.set_mode(mode);
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
    // VB-006: pre-apply the theme's visuals **before** `EguiPaint::paint`
    // runs `ctx.run_ui`. `egui::Context::set_visuals` only takes effect
    // on the next `begin_pass`, but the headless path runs exactly one;
    // the windowed app's in-`run_ui` `set_visuals` line is fine because
    // it has a next frame, but a one-shot composite never gets there.
    egui.set_visuals(state.theme.visuals());

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
