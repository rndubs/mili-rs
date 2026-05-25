//! The portable griz-subset argv parser (`phase-4-m1.md` Decision 4,
//! coded here as `phase-5-m4.md` Decision 63). griz muscle memory is
//! `mili-viz-client -i <plotfile>`; before this, `argv[1]` was passed
//! verbatim as the load root, so `-i` itself became the (unopenable)
//! root and the viewport came up blank.
//!
//! Exactly the portable subset is accepted: `-i <base>` → initial
//! load, a bare positional path → same, `-b`/`-batch <file>` →
//! startup script (parsed; a logged no-op until the Phase 6 runner is
//! wired), `-w <w> <h>` → initial window size (parsed; a logged
//! no-op — the window is created at the OS default until M4+ honours
//! it), `-V` → print version and exit. Every other flag is a clear
//! error rather than silently becoming a filename. Pure + GPU-free so
//! it is the always-on test core.

/// The parsed portable-subset arguments.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CliArgs {
    /// `-i <base>` or a bare positional — the database root to `load`.
    pub load_root: Option<String>,
    /// `-b`/`-batch <file>` — a startup script (no-op until the
    /// Phase 6 runner lands; recorded so the flag is not an error).
    pub batch_script: Option<String>,
    /// `-w <w> <h>` — requested initial window size (no-op for now).
    pub window_size: Option<(u32, u32)>,
    /// `-r`/`--remote <host:port>` or `--attach [<id>]` — the
    /// transport choice (`phase-5-m5.md` Decision 91). `None` keeps
    /// the M4 in-process default.
    pub transport: Option<TransportChoice>,
}

/// How the windowed shell should reach a `mili-viz-server`
/// (`phase-5-m5.md` Decision 91).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TransportChoice {
    /// `-r <host:port>` (also `--remote`) — direct TCP endpoint.
    /// Stored as the bare `host:port`; the constructor prepends
    /// `http://` for tonic's `Endpoint`.
    Remote(String),
    /// `--attach [<id>]` — resolve through `~/.griz/sessions/<id>.json`
    /// (bare `--attach` ⇒ newest-live), the same JSON file the server
    /// binary's `main` writes (phase-6-m2.md Decision 56).
    Attach(Option<String>),
}

/// `mili-viz-client snapshot [--out PATH] [--timeout SECS]` — out-of-
/// band arguments for the snapshot subcommand. The subcommand asks the
/// running windowed client to write a PNG of the composited GUI to
/// `out` (default: a timestamped path under `~/.griz/snapshots/`) and
/// prints the resolved path to stdout. Used by Claude Code and other
/// scripted agents to "see" the current app status.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct SnapshotArgs {
    /// `--out PATH` — destination PNG. Relative paths are resolved
    /// against the current working directory.
    pub out: Option<String>,
    /// `--timeout SECS` — how long to wait for a running app to pick
    /// up the request before giving up. Default `5.0`.
    pub timeout_secs: Option<f64>,
}

/// The result of parsing argv.
#[derive(Debug, Clone, PartialEq)]
pub enum CliOutcome {
    /// Continue startup with these arguments.
    Run(CliArgs),
    /// `-V` was given — the caller prints the version and exits 0.
    Version,
    /// `snapshot` subcommand — the caller dispatches to
    /// [`crate::run_snapshot_cli`].
    Snapshot(SnapshotArgs),
}

/// Parse the portable griz subset from an argv iterator **excluding**
/// the program name.
///
/// # Errors
/// Returns a one-line, user-facing message for a missing flag value,
/// a malformed `-w`, more than one load root, or any unknown flag —
/// so an unrecognised token is a clear error, never a silent filename.
pub fn parse_args<I: IntoIterator<Item = String>>(args: I) -> Result<CliOutcome, String> {
    let mut out = CliArgs::default();
    let mut it = args.into_iter().peekable();
    // The `snapshot` subcommand is recognised only as the *first*
    // positional token so a plotfile literally named "snapshot" can
    // still be loaded as `-i snapshot` or `./snapshot`.
    if it.peek().map(String::as_str) == Some("snapshot") {
        let _ = it.next();
        return parse_snapshot(it);
    }
    let mut pending: Option<String> = None;
    loop {
        let arg = if let Some(p) = pending.take() {
            p
        } else if let Some(a) = it.next() {
            a
        } else {
            break;
        };
        match arg.as_str() {
            "-V" | "-version" | "--version" => return Ok(CliOutcome::Version),
            "-i" => {
                let path = it
                    .next()
                    .ok_or_else(|| "flag `-i` requires a <plotfile> argument".to_string())?;
                set_root(&mut out, path)?;
            }
            "-b" | "-batch" => {
                let path = it
                    .next()
                    .ok_or_else(|| format!("flag `{arg}` requires a <file> argument"))?;
                out.batch_script = Some(path);
            }
            "-r" | "--remote" => {
                let ep = it
                    .next()
                    .ok_or_else(|| "flag `-r`/`--remote` requires <host:port>".to_string())?;
                set_transport(&mut out, TransportChoice::Remote(ep))?;
            }
            "--attach" => {
                // The next token is an optional id; treat any
                // dash-prefixed next token as a fresh flag (bare
                // `--attach` ⇒ newest-live).
                let id = it.next();
                let id = match id {
                    Some(t) if t.starts_with('-') => {
                        // Push back: re-handle the flag in the next
                        // iteration. The simplest cheat is a small
                        // pending buffer.
                        pending = Some(t);
                        None
                    }
                    other => other,
                };
                set_transport(&mut out, TransportChoice::Attach(id))?;
            }
            "-w" => {
                let w = it
                    .next()
                    .ok_or_else(|| "flag `-w` requires <width> <height>".to_string())?;
                let h = it
                    .next()
                    .ok_or_else(|| "flag `-w` requires <width> <height>".to_string())?;
                let w: u32 = w
                    .parse()
                    .map_err(|_| format!("`-w` width `{w}` is not a positive integer"))?;
                let h: u32 = h
                    .parse()
                    .map_err(|_| format!("`-w` height `{h}` is not a positive integer"))?;
                out.window_size = Some((w, h));
            }
            other if other.starts_with('-') => {
                return Err(format!(
                    "unknown flag `{other}` (portable subset: -i <base>, \
                     -b/-batch <file>, -w <w> <h>, -V, \
                     -r/--remote <host:port>, --attach [<id>])"
                ));
            }
            _ => set_root(&mut out, arg)?,
        }
    }
    Ok(CliOutcome::Run(out))
}

fn parse_snapshot<I: Iterator<Item = String>>(mut it: I) -> Result<CliOutcome, String> {
    let mut a = SnapshotArgs::default();
    while let Some(tok) = it.next() {
        match tok.as_str() {
            "--out" | "-o" => {
                let p = it
                    .next()
                    .ok_or_else(|| format!("flag `{tok}` requires a <path> argument"))?;
                a.out = Some(p);
            }
            "--timeout" => {
                let s = it
                    .next()
                    .ok_or_else(|| "flag `--timeout` requires <seconds>".to_string())?;
                let v: f64 = s
                    .parse()
                    .map_err(|_| format!("`--timeout` value `{s}` is not a number"))?;
                if v <= 0.0 {
                    return Err(format!("`--timeout` must be positive, got `{s}`"));
                }
                a.timeout_secs = Some(v);
            }
            "-h" | "--help" => {
                return Err(
                    "usage: mili-viz-client snapshot [--out <path>] [--timeout <seconds>]\n\
                     \n\
                     Asks the running `mili-viz-client` window to write a PNG of the\n\
                     composited GUI (mesh + egui chrome) to <path>. Default <path> is\n\
                     a timestamped file under ~/.griz/snapshots/. Also overwrites\n\
                     ~/.griz/snapshots/latest.png. Press F12 in the window to take a\n\
                     snapshot without using the CLI."
                        .to_string(),
                );
            }
            other => {
                return Err(format!(
                    "snapshot: unexpected argument `{other}` \
                     (accepted: --out <path>, --timeout <seconds>)"
                ));
            }
        }
    }
    Ok(CliOutcome::Snapshot(a))
}

fn set_root(out: &mut CliArgs, path: String) -> Result<(), String> {
    if out.load_root.is_some() {
        return Err("more than one load root given (use a single -i <base> \
                    or one positional path)"
            .to_string());
    }
    out.load_root = Some(path);
    Ok(())
}

fn set_transport(out: &mut CliArgs, t: TransportChoice) -> Result<(), String> {
    if out.transport.is_some() {
        return Err(
            "at most one of -r/--remote <host:port> or --attach [<id>] may be given \
             (they are mutually exclusive transport choices)"
                .to_string(),
        );
    }
    out.transport = Some(t);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(a: &[&str]) -> Result<CliOutcome, String> {
        parse_args(a.iter().map(|s| (*s).to_string()))
    }

    #[test]
    fn bare_positional_is_the_load_root() {
        assert_eq!(
            parse(&["basic1.pltA"]).unwrap(),
            CliOutcome::Run(CliArgs {
                load_root: Some("basic1.pltA".into()),
                ..Default::default()
            })
        );
    }

    #[test]
    fn dash_i_sets_the_load_root() {
        assert_eq!(
            parse(&["-i", "basic1.pltA"]).unwrap(),
            CliOutcome::Run(CliArgs {
                load_root: Some("basic1.pltA".into()),
                ..Default::default()
            })
        );
    }

    #[test]
    fn dash_v_requests_version() {
        assert_eq!(parse(&["-V"]).unwrap(), CliOutcome::Version);
    }

    #[test]
    fn batch_and_window_are_parsed_not_errors() {
        let out = parse(&["-b", "init.grizinit", "-w", "1920", "1080", "-i", "x.pltA"]).unwrap();
        assert_eq!(
            out,
            CliOutcome::Run(CliArgs {
                load_root: Some("x.pltA".into()),
                batch_script: Some("init.grizinit".into()),
                window_size: Some((1920, 1080)),
                transport: None,
            })
        );
    }

    #[test]
    fn no_args_is_the_not_attached_run() {
        assert_eq!(parse(&[]).unwrap(), CliOutcome::Run(CliArgs::default()));
    }

    #[test]
    fn unknown_flag_is_a_clear_error_not_a_filename() {
        let e = parse(&["-s"]).unwrap_err();
        assert!(e.contains("unknown flag `-s`"), "{e}");
    }

    #[test]
    fn missing_flag_value_errors() {
        assert!(parse(&["-i"]).unwrap_err().contains("`-i` requires"));
        assert!(parse(&["-w", "800"]).unwrap_err().contains("`-w` requires"));
    }

    #[test]
    fn bad_window_size_errors() {
        assert!(parse(&["-w", "wide", "tall"])
            .unwrap_err()
            .contains("not a positive integer"));
    }

    #[test]
    fn two_roots_error() {
        assert!(parse(&["a.pltA", "-i", "b.pltA"])
            .unwrap_err()
            .contains("more than one load root"));
    }

    // ── Phase 5 M5 — `-r`/`--remote` and `--attach` ─────────────────

    #[test]
    fn dash_r_picks_remote_transport() {
        let CliOutcome::Run(a) = parse(&["-r", "host.example:50051"]).unwrap() else {
            panic!("Run expected");
        };
        assert_eq!(
            a.transport,
            Some(TransportChoice::Remote("host.example:50051".into()))
        );
    }

    #[test]
    fn long_remote_picks_remote_transport() {
        let CliOutcome::Run(a) = parse(&["--remote", "1.2.3.4:7000", "-i", "x.pltA"]).unwrap()
        else {
            panic!("Run expected");
        };
        assert_eq!(
            a.transport,
            Some(TransportChoice::Remote("1.2.3.4:7000".into()))
        );
        assert_eq!(a.load_root.as_deref(), Some("x.pltA"));
    }

    #[test]
    fn bare_attach_picks_newest_live() {
        let CliOutcome::Run(a) = parse(&["--attach"]).unwrap() else {
            panic!("Run expected");
        };
        assert_eq!(a.transport, Some(TransportChoice::Attach(None)));
    }

    #[test]
    fn attach_with_id_carries_the_id() {
        let CliOutcome::Run(a) = parse(&["--attach", "abc123"]).unwrap() else {
            panic!("Run expected");
        };
        assert_eq!(
            a.transport,
            Some(TransportChoice::Attach(Some("abc123".into())))
        );
    }

    #[test]
    fn attach_then_flag_does_not_swallow_the_flag() {
        // `--attach -i x.pltA` ⇒ bare `--attach` + `-i x.pltA`.
        let CliOutcome::Run(a) = parse(&["--attach", "-i", "x.pltA"]).unwrap() else {
            panic!("Run expected");
        };
        assert_eq!(a.transport, Some(TransportChoice::Attach(None)));
        assert_eq!(a.load_root.as_deref(), Some("x.pltA"));
    }

    #[test]
    fn remote_and_attach_are_mutually_exclusive() {
        let e = parse(&["-r", "h:1", "--attach"]).unwrap_err();
        assert!(e.contains("mutually exclusive"), "{e}");
        let e = parse(&["--attach", "id1", "-r", "h:1"]).unwrap_err();
        assert!(e.contains("mutually exclusive"), "{e}");
    }

    #[test]
    fn two_remotes_are_mutually_exclusive() {
        let e = parse(&["-r", "a:1", "--remote", "b:2"]).unwrap_err();
        assert!(e.contains("mutually exclusive"), "{e}");
    }

    #[test]
    fn remote_missing_value_errors() {
        let e = parse(&["-r"]).unwrap_err();
        assert!(e.contains("`-r`/`--remote` requires"), "{e}");
    }

    // ── `snapshot` subcommand ───────────────────────────────────────

    #[test]
    fn bare_snapshot_subcommand_parses() {
        assert_eq!(
            parse(&["snapshot"]).unwrap(),
            CliOutcome::Snapshot(SnapshotArgs::default())
        );
    }

    #[test]
    fn snapshot_out_flag() {
        let CliOutcome::Snapshot(a) = parse(&["snapshot", "--out", "/tmp/frame.png"]).unwrap()
        else {
            panic!("Snapshot expected");
        };
        assert_eq!(a.out.as_deref(), Some("/tmp/frame.png"));
    }

    #[test]
    fn snapshot_short_out_flag() {
        let CliOutcome::Snapshot(a) = parse(&["snapshot", "-o", "f.png"]).unwrap() else {
            panic!("Snapshot expected");
        };
        assert_eq!(a.out.as_deref(), Some("f.png"));
    }

    #[test]
    fn snapshot_timeout_flag() {
        let CliOutcome::Snapshot(a) =
            parse(&["snapshot", "--timeout", "2.5", "-o", "f.png"]).unwrap()
        else {
            panic!("Snapshot expected");
        };
        assert_eq!(a.timeout_secs, Some(2.5));
        assert_eq!(a.out.as_deref(), Some("f.png"));
    }

    #[test]
    fn snapshot_bad_timeout_errors() {
        assert!(parse(&["snapshot", "--timeout", "abc"])
            .unwrap_err()
            .contains("not a number"));
        assert!(parse(&["snapshot", "--timeout", "0"])
            .unwrap_err()
            .contains("must be positive"));
    }

    #[test]
    fn snapshot_unknown_arg_errors() {
        let e = parse(&["snapshot", "--bogus"]).unwrap_err();
        assert!(e.contains("unexpected argument"), "{e}");
    }

    #[test]
    fn snapshot_only_matches_as_first_token() {
        // A plotfile literally named "snapshot" is loaded via `-i`, not
        // the subcommand — guards against an accidental capture.
        let CliOutcome::Run(a) = parse(&["-i", "snapshot"]).unwrap() else {
            panic!("Run expected");
        };
        assert_eq!(a.load_root.as_deref(), Some("snapshot"));
    }

    #[test]
    fn default_transport_is_in_process() {
        let CliOutcome::Run(a) = parse(&["-i", "x.pltA"]).unwrap() else {
            panic!("Run expected");
        };
        assert_eq!(
            a.transport, None,
            "M4 default unchanged when no -r/--attach"
        );
    }
}
