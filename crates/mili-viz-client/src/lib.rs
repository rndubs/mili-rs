//! Phase 5 M3 `mili-viz` client — the `egui` shell.
//!
//! M1 was the `wgpu` renderer skeleton; M2 wired the in-process
//! transport and drew the decoded server hull
//! (`phase-5-m1.md`/`phase-5-m2.md`). M3 grows an `egui` shell onto
//! that renderer: the toolbar, the left dock, and the five viewport
//! overlays in the L1 layout, and the `MVG2` per-vertex scalar now
//! becomes vertex colour through a colormap driven by a left-dock
//! result pick. Scope + Decisions 44–47:
//! `planning/mili-viz/phase-5-m3.md`.
//!
//! [`build_shell_ui`] is the pure, GPU-free layout core (the
//! always-on test weight); [`render_shell_to_image`] composites the
//! unchanged mesh pass with the additive `egui` pass into one
//! off-screen texture. The windowed [`run`] path drives a live
//! [`Session`] over the in-process transport. M3.5 added the bottom
//! tabs (Layer-0 command line, the Phase-6-gated scripting
//! placeholder, the `egui_plot` time-history — `phase-5-m3.5.md`
//! Decisions 48–52). Remote mode (gRPC + Flight TCP) is Phase 5 M5;
//! the AI panel is M6.

#![allow(clippy::pedantic)]

mod app;
mod camera;
mod catalog;
mod cli;
mod colormap;
mod egui_layer;
mod mesh;
mod renderer;
mod session;
mod shell;
mod tweaks;

pub use app::run;
pub use camera::Camera;
pub use catalog::{decode_catalog, ResultCatalog};
pub use cli::{parse_args, CliArgs, CliOutcome};
pub use colormap::{
    normalize as colormap_normalize, sample as colormap_sample,
    sample_named as colormap_sample_named, NAMES as COLORMAP_NAMES,
};
pub use mesh::{decode_mvg, DecodeError, Mesh, Pick};
pub use renderer::{
    headless_device, render_mesh_to_image, render_mesh_to_image_with_mode, render_shell_to_image,
    render_to_image, Renderer, CLEAR_COLOR, OFFSCREEN_FORMAT,
};
pub use session::{fetch_server_mesh, Session};
pub use shell::{
    build_shell_ui, control_menu_items, cutplane_cmd, dock_rail_glyphs, BottomTab, CutPlaneState,
    CutThrottle, LoadedInfo, Overlay, Overlays, RenderMode, ResultInfo, SessionPhase, ShellState,
    Theme, TimeSample, TranscriptKind, TranscriptLine, UiAction, CUT_PREVIEW_INTERVAL,
    DERIVED_RESULTS,
};
pub use tweaks::{is_persisted_action, PersistedTweaks, ThemePref};
