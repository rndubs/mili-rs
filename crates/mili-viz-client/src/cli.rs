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
}

/// The result of parsing argv.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CliOutcome {
    /// Continue startup with these arguments.
    Run(CliArgs),
    /// `-V` was given — the caller prints the version and exits 0.
    Version,
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
    let mut it = args.into_iter();
    while let Some(arg) = it.next() {
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
                     -b/-batch <file>, -w <w> <h>, -V)"
                ));
            }
            _ => set_root(&mut out, arg)?,
        }
    }
    Ok(CliOutcome::Run(out))
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
}
