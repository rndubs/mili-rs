//! The additive `egui` paint pass (`phase-5-m3.md` Decision 45).
//!
//! [`EguiPaint`] owns an [`egui::Context`] and an
//! [`egui_wgpu::Renderer`]. [`EguiPaint::paint`] runs the UI closure
//! against a `RawInput`, then composites the tessellated output onto
//! an existing `TextureView` in a **second, non-clearing** render
//! pass with **no depth attachment** — the mesh pass
//! (`Renderer::render`) is left byte-for-byte unchanged so the
//! M1/M2 render-to-texture seam never moves.

/// An `egui` context + `wgpu` paint backend for one target format.
pub struct EguiPaint {
    ctx: egui::Context,
    renderer: egui_wgpu::Renderer,
}

impl EguiPaint {
    /// Build the paint backend for a target of `color_format`. No
    /// depth/stencil (UI is 2-D screen-space chrome).
    #[must_use]
    pub fn new(device: &wgpu::Device, color_format: wgpu::TextureFormat) -> Self {
        let renderer = egui_wgpu::Renderer::new(
            device,
            color_format,
            egui_wgpu::RendererOptions {
                msaa_samples: 1,
                depth_stencil_format: None,
                dithering: false,
                predictable_texture_filtering: false,
            },
        );
        Self {
            ctx: egui::Context::default(),
            renderer,
        }
    }

    /// The shared context — the windowed path feeds it through
    /// `egui-winit`.
    #[must_use]
    pub fn context(&self) -> egui::Context {
        self.ctx.clone()
    }

    /// Apply `visuals` to the context **before** the next [`paint`]'s
    /// `run_ui`, so a single-frame headless render uses them
    /// (`bug-tracker.md` VB-006). `egui::Context::set_visuals` only
    /// takes effect on the next `begin_pass`; calling it from *inside*
    /// `run_ui` (as `build_shell_ui` does for the windowed app's
    /// multi-frame loop) is silently a no-op for the one-shot
    /// [`crate::render_shell_to_image`] path. Pre-setting here is the
    /// cheap fix (option (a) in the bug entry).
    ///
    /// [`paint`]: Self::paint
    pub fn set_visuals(&self, visuals: egui::Visuals) {
        self.ctx.set_visuals(visuals);
    }

    /// Run `run_ui` against `raw_input` and composite the result onto
    /// `view` (load, no clear, no depth). `screen` carries the
    /// physical target size + DPI scale.
    pub fn paint(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        view: &wgpu::TextureView,
        screen: &egui_wgpu::ScreenDescriptor,
        raw_input: egui::RawInput,
        run_ui: impl FnMut(&mut egui::Ui),
    ) {
        let full_output = self.ctx.run_ui(raw_input, run_ui);
        let paint_jobs = self
            .ctx
            .tessellate(full_output.shapes, screen.pixels_per_point);

        for (id, delta) in &full_output.textures_delta.set {
            self.renderer.update_texture(device, queue, *id, delta);
        }

        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("egui encoder"),
        });
        let cmd_bufs =
            self.renderer
                .update_buffers(device, queue, &mut encoder, &paint_jobs, screen);

        {
            let pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("egui pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view,
                    resolve_target: None,
                    depth_slice: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Load,
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            let mut pass = pass.forget_lifetime();
            self.renderer.render(&mut pass, &paint_jobs, screen);
        }

        queue.submit(
            cmd_bufs
                .into_iter()
                .chain(std::iter::once(encoder.finish())),
        );

        for id in &full_output.textures_delta.free {
            self.renderer.free_texture(id);
        }
    }
}
