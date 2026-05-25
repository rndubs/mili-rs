//! Composited-GUI screenshot capture.
//!
//! The windowed app composites the mesh pass and the egui pass into
//! the surface every frame. This module adds a parallel path that, on
//! request, also renders the same content into an offscreen RGBA8
//! texture, reads it back, encodes a PNG, and writes it to disk. It
//! exists because the agent-on-the-other-end (Claude Code, or a
//! teammate's headless reviewer) needs to see *what the user sees* —
//! `CaptureFrame` in the proto only carries the mesh viewport with no
//! egui chrome (`crates/mili-viz-proto/proto/mili_viz.proto:76`), and
//! `render_shell_to_image` is headless-only with no live `ShellState`.
//!
//! Two trigger paths feed the same write:
//! - F12 in the running window → app sets `pending_capture`.
//! - An external process drops a request file at
//!   `~/.griz/snapshots/.capture-request` (or `$GRIZ_SNAPSHOTS_DIR`-
//!   relative). The redraw loop polls for it once per frame and
//!   services it the same way. The `mili-viz-client snapshot` CLI
//!   uses this path so Claude Code can take a screenshot without
//!   focusing the window.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

/// `$GRIZ_SNAPSHOTS_DIR` (tests / redirection) else
/// `~/.griz/snapshots`. Mirrors the `sessions_dir()` convention in
/// `session.rs` so the two halves of `~/.griz/*` agree.
#[must_use]
pub fn snapshots_dir() -> PathBuf {
    if let Some(d) = std::env::var_os("GRIZ_SNAPSHOTS_DIR") {
        return PathBuf::from(d);
    }
    let home = std::env::var_os("HOME").map_or_else(|| PathBuf::from("."), PathBuf::from);
    home.join(".griz").join("snapshots")
}

/// The well-known request-file path. A leading dot keeps it out of
/// the directory listing of completed snapshots.
#[must_use]
pub fn request_file() -> PathBuf {
    snapshots_dir().join(".capture-request")
}

/// The well-known "latest snapshot" path. Overwritten on every
/// capture so a stable URL/path always points at the most recent
/// frame.
#[must_use]
pub fn latest_path() -> PathBuf {
    snapshots_dir().join("latest.png")
}

/// One pending capture, parsed from the request file (or built
/// directly by the F12 path).
#[derive(Debug, Clone)]
pub struct CaptureRequest {
    /// Where to write the PNG. Always absolute by the time it reaches
    /// the redraw loop.
    pub out_path: PathBuf,
}

/// Pick a default timestamped output path under
/// [`snapshots_dir`]. Used by the F12 hotkey, which has no caller to
/// supply an out path.
#[must_use]
pub fn timestamped_path(tag: &str) -> PathBuf {
    let ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    snapshots_dir().join(format!("{tag}-{ms}.png"))
}

/// Read the request file (if any) and remove it. The body is a
/// single line: an absolute output path. An empty body means "use a
/// timestamped default under `snapshots_dir()`". Malformed bodies
/// fall through the same default rather than failing — capture is a
/// best-effort observability tool.
///
/// Returns `Some(req)` when a request file existed; `None` when none
/// was present.
#[must_use]
pub fn try_consume_request_file() -> Option<CaptureRequest> {
    let path = request_file();
    let text = std::fs::read_to_string(&path).ok()?;
    let _ = std::fs::remove_file(&path);
    let trimmed = text.trim();
    let out_path = if trimmed.is_empty() {
        timestamped_path("snap")
    } else {
        PathBuf::from(trimmed)
    };
    Some(CaptureRequest { out_path })
}

/// Atomically drop a request file at [`request_file`] whose body is
/// `out_path`. The redraw loop in the running app will pick it up on
/// the next frame, capture, write the PNG, and remove the request.
///
/// # Errors
/// Returns the underlying I/O error if the snapshot dir cannot be
/// created or the request file cannot be written.
pub fn write_request_file(out_path: &Path) -> std::io::Result<()> {
    let dir = snapshots_dir();
    std::fs::create_dir_all(&dir)?;
    // Write to a sibling temp file first, then rename, so a polling
    // reader never sees a partially-written request body.
    let tmp = dir.join(".capture-request.tmp");
    {
        let mut f = std::fs::File::create(&tmp)?;
        f.write_all(out_path.as_os_str().as_encoded_bytes())?;
        f.sync_all().ok();
    }
    std::fs::rename(&tmp, request_file())
}

/// Encode raw RGBA8 pixels as PNG and write to `path`. The byte
/// layout is the same one [`Renderer::copy_back`] produces — tightly
/// packed, row-major, top-left origin. Also overwrites
/// [`latest_path`] with a copy so a "fetch the most recent screenshot"
/// caller has a stable URL.
///
/// # Errors
/// Bubbles PNG encode or filesystem errors.
///
/// [`Renderer::copy_back`]: crate::renderer::Renderer
pub fn write_png(
    path: &Path,
    width: u32,
    height: u32,
    rgba8: &[u8],
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let expected = (width as usize) * (height as usize) * 4;
    if rgba8.len() != expected {
        return Err(format!(
            "snapshot: expected {expected} pixel bytes for {width}x{height}, got {}",
            rgba8.len()
        )
        .into());
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let img: image::RgbaImage = image::ImageBuffer::from_raw(width, height, rgba8.to_vec())
        .ok_or("snapshot: from_raw rejected RGBA8 buffer")?;
    img.save_with_format(path, image::ImageFormat::Png)?;

    // Best-effort latest.png pointer. A failure here doesn't fail the
    // primary write — the timestamped file is the source of truth.
    let latest = latest_path();
    if latest != path {
        let _ = std::fs::copy(path, &latest);
    }
    Ok(())
}

/// `mili-viz-client snapshot [--out PATH]` — drop a request file,
/// wait for the running app to service it, print the resulting path.
/// Returns a non-zero exit by way of `Err` if no running app
/// services the request inside the timeout.
///
/// # Errors
/// - No running client picked up the trigger within the timeout.
/// - Filesystem error writing the request or polling for the result.
pub fn run_snapshot_cli(
    out_path: Option<PathBuf>,
    timeout: std::time::Duration,
) -> Result<PathBuf, Box<dyn std::error::Error + Send + Sync>> {
    let out_path = out_path.unwrap_or_else(|| timestamped_path("cli"));
    let out_path = if out_path.is_absolute() {
        out_path
    } else {
        std::env::current_dir()?.join(out_path)
    };
    // Clear any stale PNG at `out_path` so "file appeared" is a
    // reliable completion signal.
    let _ = std::fs::remove_file(&out_path);
    write_request_file(&out_path)?;

    let deadline = std::time::Instant::now() + timeout;
    let trigger = request_file();
    loop {
        if out_path.exists() && !trigger.exists() {
            return Ok(out_path);
        }
        if std::time::Instant::now() >= deadline {
            // Tidy: remove our trigger so the next run starts clean.
            let _ = std::fs::remove_file(&trigger);
            return Err(format!(
                "snapshot: no running `mili-viz-client` serviced the request \
                 within {:?}. Start the windowed client (`mili-viz-client` \
                 or `mili-viz-client -i <plotfile>`) and retry.",
                timeout
            )
            .into());
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    /// The env-var override is process-global. Tests run in parallel
    /// by default, so any test that mutates `GRIZ_SNAPSHOTS_DIR` has
    /// to take this lock for the duration of its env reads/writes.
    static ENV_LOCK: Mutex<()> = Mutex::new(());

    /// Set a per-test snapshot dir so the suite doesn't touch
    /// `~/.griz/snapshots`. Caller must hold [`ENV_LOCK`].
    fn isolate(dir: &Path) {
        // SAFETY: every call site holds `ENV_LOCK` for the lifetime
        // of the env access, so no other thread can read or mutate
        // GRIZ_SNAPSHOTS_DIR concurrently.
        unsafe {
            std::env::set_var("GRIZ_SNAPSHOTS_DIR", dir);
        }
    }

    #[test]
    fn snapshots_dir_honours_env_override() {
        let _g = ENV_LOCK.lock().unwrap();
        let tmp = std::env::temp_dir().join("snap-test-env");
        isolate(&tmp);
        assert_eq!(snapshots_dir(), tmp);
    }

    #[test]
    fn write_then_consume_request_roundtrip() {
        let _g = ENV_LOCK.lock().unwrap();
        let tmp = std::env::temp_dir().join("snap-test-roundtrip");
        let _ = std::fs::remove_dir_all(&tmp);
        isolate(&tmp);
        let out = tmp.join("frame.png");
        write_request_file(&out).unwrap();
        assert!(request_file().exists());
        let req = try_consume_request_file().expect("request should parse");
        assert_eq!(req.out_path, out);
        assert!(!request_file().exists(), "consume should remove the file");
    }

    #[test]
    fn empty_request_body_falls_back_to_timestamped() {
        let _g = ENV_LOCK.lock().unwrap();
        let tmp = std::env::temp_dir().join("snap-test-empty");
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();
        isolate(&tmp);
        std::fs::write(request_file(), "").unwrap();
        let req = try_consume_request_file().expect("request should parse");
        assert!(req.out_path.starts_with(&tmp));
        assert!(req.out_path.extension().and_then(|s| s.to_str()) == Some("png"));
    }

    #[test]
    fn write_png_rejects_wrong_buffer_size() {
        // No env access — no lock needed.
        let tmp = std::env::temp_dir().join("snap-test-bad-buf");
        let _ = std::fs::remove_dir_all(&tmp);
        let path = tmp.join("frame.png");
        let err = write_png(&path, 2, 2, &[0u8; 8]).unwrap_err();
        assert!(err.to_string().contains("expected"));
    }

    #[test]
    fn write_png_writes_a_decodable_image_and_updates_latest() {
        let _g = ENV_LOCK.lock().unwrap();
        let tmp = std::env::temp_dir().join("snap-test-png");
        let _ = std::fs::remove_dir_all(&tmp);
        isolate(&tmp);
        let path = tmp.join("frame.png");
        // 2×2 solid red.
        let rgba: Vec<u8> = (0..4).flat_map(|_| [255, 0, 0, 255]).collect();
        write_png(&path, 2, 2, &rgba).unwrap();
        assert!(path.exists());
        assert!(
            latest_path().exists(),
            "latest.png should be copied alongside"
        );
        let img = image::open(&path).unwrap().to_rgba8();
        assert_eq!(img.width(), 2);
        assert_eq!(img.height(), 2);
        assert_eq!(img.get_pixel(0, 0).0, [255, 0, 0, 255]);
    }
}
