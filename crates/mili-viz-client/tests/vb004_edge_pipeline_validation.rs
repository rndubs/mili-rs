//! VB-004 gating test — the VB-003 `LineList` edge pipeline must build
//! without a wgpu validation error.
//!
//! wgpu 29 rejects a non-zero `DepthBiasState` on a non-triangle
//! topology, so the old edge pipeline aborted `Renderer::new` at
//! startup on a real device (the windowed app on macOS/Metal). The
//! headless composite gate only ever exercises the `Shaded` triangle
//! pipeline, so it never caught this — hence a dedicated leg that
//! actually builds the edge pipeline.
//!
//!  * `edge_pipeline_builds_without_validation_error` — skip-on-absent
//!    when no `wgpu` adapter (CLAUDE.md convention); otherwise always
//!    on. Wraps `Renderer::new` in a `Validation` error scope so the
//!    pipeline-creation error is caught deterministically instead of
//!    reaching the panicking uncaptured-error handler.

use mili_viz_client::{headless_device, Renderer, OFFSCREEN_FORMAT};

#[test]
fn edge_pipeline_builds_without_validation_error() {
    let Some((device, queue)) = headless_device() else {
        eprintln!("skip: no wgpu adapter (skip-on-absent per CLAUDE.md)");
        return;
    };

    let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
    let _renderer = Renderer::new(device, queue, OFFSCREEN_FORMAT);
    let err = pollster::block_on(scope.pop());
    assert!(
        err.is_none(),
        "Renderer::new produced a wgpu validation error \
         (VB-004 — the LineList edge pipeline must carry zero depth \
         bias): {err:?}"
    );
}
