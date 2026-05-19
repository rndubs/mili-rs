//! Cross-session tweak persistence (`wireframe-parity.md`
//! "Tweaks / Preferences"; MVP-cut item 7 remainder).
//!
//! The wireframe (`griz_wgpu_wireframes/README.md` §"Tweaks") names
//! **Theme** and **Left dock collapsed** as preferences, and says each
//! overlay chip's on/off state "should persist between sessions". This
//! module is the smallest contract-preserving carrier for exactly that
//! set: a small `serde` struct written to a standard per-user config
//! path on change and loaded into [`ShellState`] at *windowed*
//! startup. No frozen-proto change, no Phase 4 crate touched.
//!
//! `stride` / `focus_mode` are deliberately **not** persisted: the
//! wireframe scopes persistence to the Tweaks table + the overlay
//! chips, and stride/focus are runtime modes, not preferences.
//!
//! The headless composite path ([`crate::render_shell_to_image`])
//! never calls this module — disk is touched only by the windowed
//! [`crate::run`]. With **no config file present**, [`load`] returns
//! [`PersistedTweaks::default`], whose [`PersistedTweaks::apply_to`]
//! leaves a default [`ShellState`] byte-identical to
//! `ShellState::default()`, so the M3 composite gate
//! (`bug-tracker.md` VB-001) is unperturbed.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::shell::{ShellState, Theme, UiAction};

/// Serialized form of [`Theme`] (a small, readable JSON enum that does
/// not couple the on-disk format to egui's visuals types).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ThemePref {
    Dark,
    Light,
}

/// The wireframe-justified persistent set: the five overlay-chip
/// states + the two Tweaks-table preferences (Theme, Left dock
/// collapsed). `#[serde(default)]` makes a missing/partial file fall
/// back field-by-field to the byte-stable defaults, so a hand-edited
/// or older config never desynchronizes the gate.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct PersistedTweaks {
    pub overlay_title: bool,
    pub overlay_state: bool,
    pub overlay_legend: bool,
    pub overlay_axes: bool,
    pub overlay_bbox: bool,
    pub theme: ThemePref,
    pub dock_collapsed: bool,
}

impl Default for PersistedTweaks {
    /// By construction equal to `from_state(&ShellState::default())`,
    /// so an absent config is exactly the byte-stable default state.
    fn default() -> Self {
        Self::from_state(&ShellState::default())
    }
}

impl PersistedTweaks {
    /// Snapshot the persisted fields out of the live shell state.
    #[must_use]
    pub fn from_state(s: &ShellState) -> Self {
        Self {
            overlay_title: s.overlays.title,
            overlay_state: s.overlays.state,
            overlay_legend: s.overlays.legend,
            overlay_axes: s.overlays.axes,
            overlay_bbox: s.overlays.bbox,
            theme: match s.theme {
                Theme::Dark => ThemePref::Dark,
                Theme::Light => ThemePref::Light,
            },
            dock_collapsed: s.dock_collapsed,
        }
    }

    /// Apply the persisted fields onto a shell state, in place. Pure
    /// (no I/O); touches only the persisted fields — every other
    /// `ShellState` field (stride, picking, focus, transcript, …) is
    /// left exactly as the caller set it.
    pub fn apply_to(&self, s: &mut ShellState) {
        s.overlays.title = self.overlay_title;
        s.overlays.state = self.overlay_state;
        s.overlays.legend = self.overlay_legend;
        s.overlays.axes = self.overlay_axes;
        s.overlays.bbox = self.overlay_bbox;
        s.theme = match self.theme {
            ThemePref::Dark => Theme::Dark,
            ThemePref::Light => Theme::Light,
        };
        s.dock_collapsed = self.dock_collapsed;
    }

    /// Pretty JSON for the on-disk file.
    #[must_use]
    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(self).unwrap_or_default()
    }

    /// Parse from JSON; `None` on malformed input (the caller then
    /// falls back to the byte-stable default).
    #[must_use]
    pub fn from_json(s: &str) -> Option<Self> {
        serde_json::from_str(s).ok()
    }

    /// Load from an explicit path. An absent or unparseable file
    /// yields [`PersistedTweaks::default`] — never an error, so a
    /// missing config is always exactly the byte-stable default state.
    #[must_use]
    pub fn load_from(path: &Path) -> Self {
        match std::fs::read_to_string(path) {
            Ok(txt) => Self::from_json(&txt).unwrap_or_default(),
            Err(_) => Self::default(),
        }
    }

    /// Write to an explicit path, creating the parent directory.
    ///
    /// # Errors
    /// Returns the underlying `std::io::Error` if the directory or
    /// file cannot be written.
    pub fn save_to(&self, path: &Path) -> std::io::Result<()> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        std::fs::write(path, self.to_json())
    }
}

/// Whether a [`UiAction`] mutates a persisted field — i.e. the
/// windowed app should re-write the config after applying it. Exactly
/// the three pure-client tweak actions the wireframe scopes to
/// persistence (overlay chips + Theme + Left-dock-collapse).
#[must_use]
pub fn is_persisted_action(a: &UiAction) -> bool {
    matches!(
        a,
        UiAction::ToggleOverlay(_) | UiAction::SetTheme(_) | UiAction::SetDockCollapsed(_)
    )
}

/// The standard per-user config file, or `None` if no home/config
/// base can be resolved (then persistence is silently a no-op — a
/// headless/sandboxed run still works, just without remembering
/// tweaks). `MILI_VIZ_CONFIG` overrides the path outright. Otherwise
/// the XDG base dir spec: `$XDG_CONFIG_HOME` (must be absolute per the
/// spec) else `$HOME/.config`, then `mili-viz/tweaks.json`.
fn config_path() -> Option<PathBuf> {
    if let Some(p) = std::env::var_os("MILI_VIZ_CONFIG") {
        return Some(PathBuf::from(p));
    }
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))?;
    Some(base.join("mili-viz").join("tweaks.json"))
}

/// Load the persisted tweaks for windowed startup. No config base / no
/// file ⇒ [`PersistedTweaks::default`] (the byte-stable default state).
#[must_use]
pub fn load() -> PersistedTweaks {
    config_path().map_or_else(PersistedTweaks::default, |p| PersistedTweaks::load_from(&p))
}

/// Persist the current tweaks (best-effort; an unwritable config dir
/// is silently ignored — losing persistence must never break the GUI).
pub fn save(t: &PersistedTweaks) {
    if let Some(p) = config_path() {
        let _ = t.save_to(&p);
    }
}
