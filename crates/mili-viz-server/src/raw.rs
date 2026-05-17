//! Layer-0 ↔ Layer-1 bridge: the `Command.raw` griz/grizinit line
//! syntax ↔ the typed `Command` `oneof`.
//!
//! These are exact inverses by construction. [`to_raw`] emits the
//! canonical line for a typed command; [`parse_line`] parses it back;
//! [`parse_raw`] splits a `;`/newline-joined stream. The
//! "Layer-0 ≡ raw" acceptance-gate test relies on `parse_line ∘
//! to_raw == id` *and* on both dispatch paths producing identical
//! `StateDelta`s — that is the M1 form of a parity test (there is no
//! upstream oracle; see `phase-4-m1.md` Decision 5 reasoning).
//!
//! Syntax note: this is the griz command *shape* (verb + args), not a
//! byte-for-byte griz grammar port — griz is read-only reference and
//! not in any CI path, so the contract M1 must hold is internal
//! round-trip + dispatch equivalence, not griz-string identity.

use mili_viz_proto::v1 as pb;

type Cmd = pb::command::Cmd;

fn f(x: f64) -> String {
    // Rust's default f64 Display is shortest round-trippable.
    format!("{x}")
}

fn list_f64(xs: &[f64]) -> String {
    xs.iter().map(|x| f(*x)).collect::<Vec<_>>().join(",")
}

fn list_u32(xs: &[u32]) -> String {
    xs.iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join(",")
}

/// Canonical griz-shaped line for a typed command.
#[must_use]
pub fn to_raw(cmd: &Cmd) -> String {
    match cmd {
        Cmd::Raw(s) => s.clone(),
        Cmd::Load(l) => format!("load {}", l.root),
        Cmd::Close(_) => "close".to_string(),
        Cmd::SetState(s) => format!("state {}", s.state),
        Cmd::Step(s) => match pb::step::Dir::try_from(s.dir).unwrap_or(pb::step::Dir::Next) {
            pb::step::Dir::Next => "next",
            pb::step::Dir::Prev => "prev",
            pb::step::Dir::First => "first",
            pb::step::Dir::Last => "last",
        }
        .to_string(),
        Cmd::Select(s) => format!("select {} {}", s.class_name, s.range),
        Cmd::Clrsel(c) => format!("clrsel {}", c.class_name),
        Cmd::Show(s) => {
            let mut out = format!("show {}", s.result);
            if !s.component.is_empty() {
                out.push_str(&format!(" {}", s.component));
            }
            let mut opts: Vec<_> = s.opts.iter().collect();
            opts.sort_by(|a, b| a.0.cmp(b.0));
            for (k, v) in opts {
                out.push_str(&format!(" {k}={v}"));
            }
            out
        }
        Cmd::View(v) => view_to_raw(v),
        Cmd::Iso(i) => {
            let mut out = format!("iso {} {}", if i.on { "on" } else { "off" }, i.result);
            if i.count > 0 {
                out.push_str(&format!(" count={}", i.count));
            }
            if let Some(m) = i.min {
                out.push_str(&format!(" min={}", f(m)));
            }
            if let Some(m) = i.max {
                out.push_str(&format!(" max={}", f(m)));
            }
            if !i.levels.is_empty() {
                out.push_str(&format!(" levels={}", list_f64(&i.levels)));
            }
            out
        }
        Cmd::Contour(c) => format!("contour {} {}", c.result, c.count),
        Cmd::Material(m) => {
            let verb = if m.enable { "enable" } else { "disable" };
            let class = if m.class_name.is_empty() {
                "*"
            } else {
                &m.class_name
            };
            match m.material {
                Some(mat) => format!("{verb} {class} {mat}"),
                None => format!("{verb} {class}"),
            }
        }
        Cmd::Cutplane(c) => format!(
            "{} {} {} {} {} {} {}",
            if c.relative { "cutrpln" } else { "cutpln" },
            f(c.ox),
            f(c.oy),
            f(c.oz),
            f(c.nx),
            f(c.ny),
            f(c.nz)
        ),
        Cmd::Colormap(c) => format!("cmap {}", c.name),
        Cmd::Legend(l) => format!(
            "legend {} {}",
            l.min.map_or_else(|| "*".to_string(), f),
            l.max.map_or_else(|| "*".to_string(), f)
        ),
        Cmd::NamedView(nv) => {
            let op = match pb::named_view::Op::try_from(nv.op).unwrap_or(pb::named_view::Op::List) {
                pb::named_view::Op::Save => "save",
                pb::named_view::Op::Restore => "restore",
                pb::named_view::Op::List => "list",
            };
            if nv.name.is_empty() {
                format!("view {op}")
            } else {
                format!("view {op} {}", nv.name)
            }
        }
        Cmd::Render(r) => {
            let mut out = String::from("render");
            if !r.path.is_empty() {
                out.push_str(&format!(" path={}", r.path));
            }
            out.push_str(&format!(" width={}", r.width));
            out.push_str(&format!(" height={}", r.height));
            if !r.format.is_empty() {
                out.push_str(&format!(" format={}", r.format));
            }
            if !r.states.is_empty() {
                out.push_str(&format!(" states={}", list_u32(&r.states)));
            }
            out
        }
    }
}

fn view_to_raw(v: &pb::View) -> String {
    use pb::view::Op;
    match &v.op {
        None => "view reset".to_string(),
        Some(Op::Rotate(r)) => format!("rot {} {} {}", f(r.x), f(r.y), f(r.z)),
        Some(Op::Translate(t)) => {
            format!("translate {} {} {}", f(t.dx), f(t.dy), f(t.dz))
        }
        Some(Op::Scale(s)) => format!("scale {}", f(s.factor)),
        Some(Op::Zoom(z)) => format!("zoom {}", f(z.factor)),
        Some(Op::Set(c)) => {
            let mut out = format!(
                "view set {} {} {}",
                f(c.azimuth),
                f(c.elevation),
                f(c.distance)
            );
            if let (Some(x), Some(y), Some(z)) = (c.fx, c.fy, c.fz) {
                out.push_str(&format!(" {} {} {}", f(x), f(y), f(z)));
            }
            out
        }
        Some(Op::Reset(_)) => "view reset".to_string(),
    }
}

/// Split a `;`/newline-joined Layer-0 stream and parse each line.
/// Blank lines and `#`/`//` comments are skipped (grizinit style).
///
/// # Errors
/// Returns the first line that fails to parse.
pub fn parse_raw(stream: &str) -> Result<Vec<Cmd>, String> {
    let mut cmds = Vec::new();
    for piece in stream.split([';', '\n']) {
        let line = piece.trim();
        if line.is_empty() || line.starts_with('#') || line.starts_with("//") {
            continue;
        }
        cmds.push(parse_line(line)?);
    }
    Ok(cmds)
}

fn pf(tok: &str, what: &str) -> Result<f64, String> {
    tok.parse::<f64>()
        .map_err(|_| format!("bad {what}: {tok:?}"))
}
fn pu(tok: &str, what: &str) -> Result<u32, String> {
    tok.parse::<u32>()
        .map_err(|_| format!("bad {what}: {tok:?}"))
}
fn opt_f(tok: &str, what: &str) -> Result<Option<f64>, String> {
    if tok == "*" {
        Ok(None)
    } else {
        Ok(Some(pf(tok, what)?))
    }
}

/// Parse one canonical line into a typed command.
///
/// # Errors
/// Returns a message if the verb is unknown or args are malformed.
#[allow(clippy::too_many_lines)]
pub fn parse_line(line: &str) -> Result<Cmd, String> {
    let toks: Vec<&str> = line.split_whitespace().collect();
    let verb = *toks.first().ok_or("empty line")?;
    let rest = &toks[1..];

    let kv = |prefix: &str| -> Option<String> {
        rest.iter()
            .find_map(|t| t.strip_prefix(prefix).map(ToString::to_string))
    };

    match verb {
        "load" => Ok(Cmd::Load(pb::Load {
            root: rest.first().ok_or("load: missing root")?.to_string(),
        })),
        "close" => Ok(Cmd::Close(pb::Close {})),
        "state" => Ok(Cmd::SetState(pb::SetState {
            state: pu(rest.first().ok_or("state: missing N")?, "state")?,
        })),
        "next" | "prev" | "first" | "last" => {
            let dir = match verb {
                "next" => pb::step::Dir::Next,
                "prev" => pb::step::Dir::Prev,
                "first" => pb::step::Dir::First,
                _ => pb::step::Dir::Last,
            };
            Ok(Cmd::Step(pb::Step { dir: dir as i32 }))
        }
        "select" => Ok(Cmd::Select(pb::Select {
            class_name: rest.first().ok_or("select: missing class")?.to_string(),
            range: rest.get(1).ok_or("select: missing range")?.to_string(),
        })),
        "clrsel" => Ok(Cmd::Clrsel(pb::ClearSelection {
            class_name: rest.first().ok_or("clrsel: missing class")?.to_string(),
        })),
        "show" => {
            let result = rest.first().ok_or("show: missing result")?.to_string();
            let mut component = String::new();
            let mut opts = std::collections::HashMap::new();
            for t in &rest[1..] {
                if let Some((k, v)) = t.split_once('=') {
                    opts.insert(k.to_string(), v.to_string());
                } else if component.is_empty() {
                    component = (*t).to_string();
                }
            }
            Ok(Cmd::Show(pb::Show {
                result,
                component,
                opts,
            }))
        }
        "rot" => Ok(Cmd::View(pb::View {
            op: Some(pb::view::Op::Rotate(pb::Rotate {
                x: pf(rest.first().ok_or("rot x")?, "rot x")?,
                y: pf(rest.get(1).ok_or("rot y")?, "rot y")?,
                z: pf(rest.get(2).ok_or("rot z")?, "rot z")?,
            })),
        })),
        "translate" => Ok(Cmd::View(pb::View {
            op: Some(pb::view::Op::Translate(pb::Translate {
                dx: pf(rest.first().ok_or("tx")?, "tx")?,
                dy: pf(rest.get(1).ok_or("ty")?, "ty")?,
                dz: pf(rest.get(2).ok_or("tz")?, "tz")?,
            })),
        })),
        "scale" => Ok(Cmd::View(pb::View {
            op: Some(pb::view::Op::Scale(pb::Scale {
                factor: pf(rest.first().ok_or("scale")?, "scale")?,
            })),
        })),
        "zoom" => Ok(Cmd::View(pb::View {
            op: Some(pb::view::Op::Zoom(pb::Zoom {
                factor: pf(rest.first().ok_or("zoom")?, "zoom")?,
            })),
        })),
        "view" => {
            let sub = *rest.first().ok_or("view: missing subcommand")?;
            match sub {
                "reset" => Ok(Cmd::View(pb::View {
                    op: Some(pb::view::Op::Reset(true)),
                })),
                "set" => {
                    let a = &rest[1..];
                    let mut cam = pb::SetCamera {
                        azimuth: pf(a.first().ok_or("view set azimuth")?, "azimuth")?,
                        elevation: pf(a.get(1).ok_or("view set elevation")?, "elevation")?,
                        distance: pf(a.get(2).ok_or("view set distance")?, "distance")?,
                        fx: None,
                        fy: None,
                        fz: None,
                    };
                    if a.len() >= 6 {
                        cam.fx = Some(pf(a[3], "fx")?);
                        cam.fy = Some(pf(a[4], "fy")?);
                        cam.fz = Some(pf(a[5], "fz")?);
                    }
                    Ok(Cmd::View(pb::View {
                        op: Some(pb::view::Op::Set(cam)),
                    }))
                }
                "save" | "restore" | "list" => {
                    let op = match sub {
                        "save" => pb::named_view::Op::Save,
                        "restore" => pb::named_view::Op::Restore,
                        _ => pb::named_view::Op::List,
                    };
                    Ok(Cmd::NamedView(pb::NamedView {
                        op: op as i32,
                        name: rest.get(1).map(ToString::to_string).unwrap_or_default(),
                    }))
                }
                other => Err(format!("view: unknown subcommand {other:?}")),
            }
        }
        "iso" => {
            let on = match *rest.first().ok_or("iso: missing on/off")? {
                "on" => true,
                "off" => false,
                o => return Err(format!("iso: expected on/off, got {o:?}")),
            };
            let result = rest.get(1).ok_or("iso: missing result")?.to_string();
            let count = kv("count=")
                .map(|s| pu(&s, "count"))
                .transpose()?
                .unwrap_or(0);
            let min = kv("min=").map(|s| pf(&s, "min")).transpose()?;
            let max = kv("max=").map(|s| pf(&s, "max")).transpose()?;
            let levels = match kv("levels=") {
                Some(s) if !s.is_empty() => {
                    s.split(',')
                        .map(|t| pf(t, "level"))
                        .collect::<Result<Vec<_>, _>>()?
                }
                _ => vec![],
            };
            Ok(Cmd::Iso(pb::Isosurface {
                result,
                on,
                levels,
                count,
                min,
                max,
            }))
        }
        "contour" => Ok(Cmd::Contour(pb::Contour {
            result: rest.first().ok_or("contour: missing result")?.to_string(),
            count: pu(rest.get(1).ok_or("contour: missing count")?, "count")?,
        })),
        "enable" | "disable" => {
            let class = match rest.first() {
                Some(&"*") | None => String::new(),
                Some(c) => (*c).to_string(),
            };
            let material = rest.get(1).map(|m| pu(m, "material")).transpose()?;
            Ok(Cmd::Material(pb::MaterialVisibility {
                enable: verb == "enable",
                class_name: class,
                material,
            }))
        }
        "cutpln" | "cutrpln" => Ok(Cmd::Cutplane(pb::CutPlane {
            ox: pf(rest.first().ok_or("ox")?, "ox")?,
            oy: pf(rest.get(1).ok_or("oy")?, "oy")?,
            oz: pf(rest.get(2).ok_or("oz")?, "oz")?,
            nx: pf(rest.get(3).ok_or("nx")?, "nx")?,
            ny: pf(rest.get(4).ok_or("ny")?, "ny")?,
            nz: pf(rest.get(5).ok_or("nz")?, "nz")?,
            relative: verb == "cutrpln",
        })),
        "cmap" => Ok(Cmd::Colormap(pb::Colormap {
            name: rest.first().ok_or("cmap: missing name")?.to_string(),
        })),
        "legend" => Ok(Cmd::Legend(pb::LegendLimits {
            min: opt_f(rest.first().ok_or("legend: missing min")?, "min")?,
            max: opt_f(rest.get(1).ok_or("legend: missing max")?, "max")?,
        })),
        "render" => Ok(Cmd::Render(pb::Render {
            path: kv("path=").unwrap_or_default(),
            width: kv("width=")
                .map(|s| pu(&s, "width"))
                .transpose()?
                .unwrap_or(0),
            height: kv("height=")
                .map(|s| pu(&s, "height"))
                .transpose()?
                .unwrap_or(0),
            states: match kv("states=") {
                Some(s) if !s.is_empty() => {
                    s.split(',')
                        .map(|t| pu(t, "state"))
                        .collect::<Result<Vec<_>, _>>()?
                }
                _ => vec![],
            },
            format: kv("format=").unwrap_or_default(),
        })),
        other => Err(format!("unknown command verb {other:?}")),
    }
}
