//! `+show-face` (ZIG/src/cli/show_face.zig).

use std::io::Write;

use ghostty_config::{
    cli_args::U21,
    config::Config,
    config_enum,
    config_struct,
    diagnostics::{Diagnostic, DiagnosticList, Location},
};
use ghostty_font::{
    self as font,
    SharedGrid,
    SharedGridSet,
    collection::{EntryError, GetFaceError},
    face::DesiredSize,
    shared_grid_set::DerivedConfig,
};
use ghostty_os::osbytes::EncodedOs;

use crate::{
    error::Error,
    zig_writer::{Sink, ZigWriter, stdout_sink},
};

config_enum! {
    /// Font style (Zig: `font.Style`).
    pub enum Style {
        Regular => "regular",
        Bold => "bold",
        Italic => "italic",
        BoldItalic => "bold_italic",
    }
}

config_enum! {
    /// Font presentation (Zig: `font.Presentation`).
    pub enum Presentation {
        Text => "text",
        Emoji => "emoji",
    }
}

config_struct! {
    pub struct Options {
        /// The codepoint to search for.
        "cp" => cp: Option<U21> = None,

        /// Search for all of the codepoints in the string.
        "string" => string: Option<EncodedOs> = None,

        /// Font style to search for.
        "style" => style: Style = Style::Regular,

        /// If specified, force text or emoji presentation.
        "presentation" => presentation: Option<Presentation> = None,
    }
    extra {
        // Enable arg parsing diagnostics so that we don't get an error if
        // there is a "normal" config setting on the cli.
        diagnostics: DiagnosticList = DiagnosticList::default(),
    }
}
cli_options!(Options, help, diagnostics);

/// The `show-face` command shows what font face Ghostty will use to render a
/// specific codepoint. Note that this command does not take into consideration
/// grapheme clustering or any other Unicode features that might modify the
/// presentation of a codepoint, so this may show a different font face than
/// Ghostty uses to render a codepoint in a terminal session.
///
/// Flags:
///
///   * `--cp`: Find the face for a single codepoint. The codepoint may be specified
///     in decimal (`--cp=65`), hexadecimal (`--cp=0x41`), octal (`--cp=0o101`), or
///     binary (`--cp=0b1000001`).
///
///   * `--string`: Find the face for all of the codepoints in a string. The
///     string must be a valid UTF-8 sequence.
///
///   * `--style`: Search for a specific style. Valid options are `regular`, `bold`,
///     `italic`, and `bold_italic`.
///
///   * `--presentation`: If set, force searching for a specific presentation
///     style. Valid options are `text` and `emoji`. If unset, the presentation
///     style of a codepoint will be inferred from the Unicode standard.
pub fn run(args: &[impl AsRef<[u8]>]) -> Result<u8, Error> {
    let mut opts = Options::default();
    crate::parse_options(&mut opts, args)?;

    // Zig creates both writers on stdout (`File.stdout()`), each with its own
    // 1024 byte buffer, and flushes stdout then stderr at the end.
    let sink: Sink = stdout_sink();
    let mut stdout = ZigWriter::new(sink.clone(), 1024);
    let mut stderr = ZigWriter::new(sink, 1024);
    let result = run_args(&opts, &mut stdout, &mut stderr, &Config::load);
    let _ = stdout.flush();
    let _ = stderr.flush();
    result
}

fn run_args(
    opts: &Options,
    stdout: &mut dyn std::io::Write,
    stderr: &mut dyn std::io::Write,
    load_config: &dyn Fn() -> Result<Config, ghostty_config::config::Error>,
) -> Result<u8, Error> {
    // Print out any diagnostics, unless it's likely that the diagnostic was
    // generated trying to parse a "normal" configuration setting. Exit with an
    // error code if any diagnostics were printed.
    const FIELDS: [&str; 4] = ["cp", "string", "style", "presentation"];
    let mut exit = false;
    for d in opts.diagnostics.items() {
        if !matches!(d.location, Location::Cli(_)) {
            continue;
        }
        if FIELDS.contains(&d.key.as_str()) {
            write_diagnostic(stderr, d)?;
            exit = true;
        }
    }
    if exit {
        return Ok(1);
    }

    if opts.cp.is_none() && opts.string.is_none() {
        writeln!(
            stderr,
            "You must specify a codepoint with --cp or a string with --string"
        )?;
        return Ok(1);
    }

    let config = match load_config() {
        Ok(c) => c,
        Err(err) => {
            write!(stderr, "Unable to load config: {err}")?;
            return Ok(1);
        }
    };

    // Print out any diagnostics generated from parsing the config, unless
    // the diagnostic might have been generated because it's actually an
    // action-specific argument.
    for d in config.diagnostics.items() {
        if FIELDS.contains(&d.key.as_str())
            && matches!(d.location, Location::None | Location::Cli(_))
        {
            continue;
        }
        write_diagnostic(stderr, d)?;
    }

    let font_grid_set = match SharedGridSet::new() {
        Ok(v) => v,
        Err(err) => {
            write!(stderr, "Unable to initialize font grid set: {err}")?;
            return Ok(1);
        }
    };

    let font_size = DesiredSize {
        points: config.font_size,
        xdpi: 96,
        ydpi: 96,
    };
    let font_config = DerivedConfig::new(&config);
    let (key, grid) = match font_grid_set.ref_(&font_config, font_size) {
        Ok(v) => v,
        Err(err) => {
            write!(stderr, "Unable to get font grid: {err}")?;
            return Ok(1);
        }
    };

    let style = match opts.style {
        Style::Regular => font::Style::Regular,
        Style::Bold => font::Style::Bold,
        Style::Italic => font::Style::Italic,
        Style::BoldItalic => font::Style::BoldItalic,
    };
    let presentation = opts.presentation.map(|p| match p {
        Presentation::Text => font::Presentation::Text,
        Presentation::Emoji => font::Presentation::Emoji,
    });

    let mut rc = None;
    if let Some(cp) = opts.cp {
        rc = lookup(stdout, stderr, &grid, style, presentation, cp.0)?;
    }
    if rc.is_none()
        && let Some(string) = &opts.string
    {
        // `Utf8View.init` validates the whole string before any lookup.
        if let Ok(text) = std::str::from_utf8(&string.to_bytes()) {
            for ch in text.chars() {
                rc = lookup(stdout, stderr, &grid, style, presentation, ch as u32)?;
                if rc.is_some() {
                    break;
                }
            }
        } else {
            write!(
                stderr,
                "Unable to parse string as unicode: error.InvalidUtf8"
            )?;
            rc = Some(1);
        }
    }

    drop(grid);
    font_grid_set.deref(&key);
    Ok(rc.unwrap_or(0))
}

fn lookup(
    stdout: &mut dyn std::io::Write,
    stderr: &mut dyn std::io::Write,
    grid: &SharedGrid,
    style: font::Style,
    presentation: Option<font::Presentation>,
    cp: u32,
) -> Result<Option<u8>, Error> {
    let ch = char::from_u32(cp).unwrap_or('\u{FFFD}');
    let Ok(Some(idx)) = grid.get_index(cp, style, presentation) else {
        write_lookup(stdout, cp, ch, " \u{bb} not found.\n", None)?;
        return Ok(None);
    };

    let mut st = grid.write();
    let face = match st.resolver.collection.get_face(idx) {
        Ok(f) => f,
        Err(GetFaceError::Entry(EntryError::SpecialHasNoFace)) => {
            write_lookup(
                stdout,
                cp,
                ch,
                " \u{bb} is handled by Ghostty's internal sprites.\n",
                None,
            )?;
            return Ok(None);
        }
        Err(err) => {
            write!(stderr, "Unable to get face: {err}")?;
            return Ok(Some(1));
        }
    };

    let name = face.name();
    write_lookup(
        stdout,
        cp,
        ch,
        " \u{bb} found in face \u{201c}",
        Some(name.as_bytes()),
    )?;
    Ok(None)
}

/// Zig's `Writer.print` issues one `writeAll` per literal segment and per
/// formatted argument, and an aligned integer writes its padding separately
/// from its digits. Where the fixed-size buffer drains depends on those
/// fragments, so every message below is written in Zig's exact fragments.
fn write_parts(w: &mut dyn std::io::Write, parts: &[&[u8]]) -> std::io::Result<()> {
    for part in parts {
        w.write_all(part)?;
    }
    Ok(())
}

/// `stderr.print("config error: {f}\n", .{diagnostic})` in Zig
/// (`cli/diagnostics.zig:19-37` formats through nested `print` calls).
fn write_diagnostic(w: &mut dyn std::io::Write, d: &Diagnostic) -> std::io::Result<()> {
    use ghostty_os::osbytes::config_str_to_bytes as enc;
    let mut parts: Vec<Vec<u8>> = vec![b"config error: ".to_vec()];
    match &d.location {
        Location::None => {}
        Location::Cli(index) => {
            parts.push(b"cli:".to_vec());
            parts.push(index.to_string().into_bytes());
            parts.push(b":".to_vec());
        }
        Location::File { path, line } => {
            parts.push(enc(path));
            parts.push(b":".to_vec());
            parts.push(line.to_string().into_bytes());
            parts.push(b":".to_vec());
        }
    }
    if !d.key.is_empty() {
        parts.push(enc(&d.key));
        parts.push(b": ".to_vec());
    } else if d.location != Location::None {
        parts.push(b" ".to_vec());
    }
    parts.push(enc(&d.message));
    parts.push(b"\n".to_vec());
    let refs: Vec<&[u8]> = parts.iter().map(Vec::as_slice).collect();
    write_parts(w, &refs)
}

/// `U+{0X:0>2} \u{ab} {0u} <tail>` (optionally followed by `name` and the
/// closing `\u{201d}.\n` for the "found in face" line).
fn write_lookup(
    w: &mut dyn std::io::Write,
    cp: u32,
    ch: char,
    tail: &str,
    name: Option<&[u8]>,
) -> std::io::Result<()> {
    let hex = format!("{cp:X}");
    let mut buf = [0u8; 4];
    let mut parts: Vec<&[u8]> = vec![b"U+"];
    if hex.len() < 2 {
        parts.push(b"0");
    }
    parts.push(hex.as_bytes());
    parts.push(" \u{ab} ".as_bytes());
    parts.push(ch.encode_utf8(&mut buf).as_bytes());
    parts.push(tail.as_bytes());
    if let Some(name) = name {
        parts.push(name);
        parts.push("\u{201d}.\n".as_bytes());
    }
    write_parts(w, &parts)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;

    /// Load the config from explicit args instead of the test runner's argv.
    fn test_config() -> Result<Config, ghostty_config::config::Error> {
        let mut config = Config::new_default();
        let args: [&[u8]; 1] = [b"--config-default-files=false"];
        config
            .load_iter(&mut ghostty_config::cli_args::slice_iterator(&args))
            .map_err(|e| ghostty_config::config::Error::Other(e.to_string()))?;
        config.finalize()?;
        Ok(config)
    }

    fn parse(args: &[&[u8]]) -> Options {
        let mut full: Vec<&[u8]> = vec![b"ghostty"];
        full.extend_from_slice(args);
        let mut opts = Options::default();
        crate::parse_options(&mut opts, &full).unwrap();
        opts
    }

    #[test]
    fn cp_is_u21() {
        let o = parse(&[b"--cp=0x1FFFFF"]);
        assert_eq!(o.cp, Some(U21(0x1f_ffff)));
        assert!(o.diagnostics.is_empty());
        for bad in ["--cp=0x200000", "--cp=4294967295", "--cp=-1"] {
            let o = parse(&[bad.as_bytes()]);
            assert_eq!(o.cp, None, "{bad}");
            let d = &o.diagnostics.items()[0];
            assert_eq!(d.key, "cp");
            assert!(d.to_string().contains("invalid value"), "{d}");
        }
    }

    #[test]
    fn invalid_cp_exits_1_without_panic() {
        let o = parse(&[b"--cp=4294967295"]);
        let mut out = Vec::new();
        let mut err = Vec::new();
        assert_eq!(run_args(&o, &mut out, &mut err, &test_config).unwrap(), 1);
        let text = String::from_utf8(err).unwrap();
        assert!(
            text.starts_with("config error: cli:") && text.contains("cp: invalid value"),
            "{text}"
        );
    }

    #[test]
    fn string_accepts_raw_bytes_at_parse() {
        let o = parse(&[b"--cp=65", b"--string=\xff"]);
        assert!(o.diagnostics.is_empty());
        assert_eq!(o.string.unwrap().to_bytes(), b"\xff");
    }

    #[test]
    fn invalid_utf8_string_fails_after_cp_lookup() {
        let o = parse(&[b"--config-default-files=false", b"--string=\xff"]);
        let mut out = Vec::new();
        let mut err = Vec::new();
        // Without a usable font setup this can fail earlier; only assert the
        // Zig message when the grid was available.
        let rc = run_args(&o, &mut out, &mut err, &test_config).unwrap();
        let err = String::from_utf8(err).unwrap();
        if err.contains("Unable to") && !err.contains("as unicode") {
            return;
        }
        assert_eq!(rc, 1);
        assert_eq!(err, "Unable to parse string as unicode: error.InvalidUtf8");
    }

    #[derive(Clone, Default)]
    struct Mem(std::rc::Rc<std::cell::RefCell<Vec<u8>>>);
    impl std::io::Write for Mem {
        fn write(&mut self, data: &[u8]) -> std::io::Result<usize> {
            self.0.borrow_mut().extend_from_slice(data);
            Ok(data.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn diagnostics_drain_at_zig_fragment_boundaries() {
        use crate::zig_writer::stream_sink;
        let mem = Mem::default();
        let mut w = ZigWriter::new(stream_sink(mem.clone()), 20);
        let d = Diagnostic {
            location: Location::Cli(0),
            key: "cp".into(),
            message: "invalid value".into(),
        };
        write_diagnostic(&mut w, &d).unwrap();
        // Zig writes "config error: ", "cli:", "0", ":", "cp" (overflows: the
        // buffer drains together with "cp"), then ": ", message and "\n".
        assert_eq!(&*mem.0.borrow(), b"config error: cli:0:cp");
        w.flush().unwrap();
        assert_eq!(&*mem.0.borrow(), b"config error: cli:0:cp: invalid value\n");
    }

    #[test]
    fn lookup_line_pads_hex_and_drains_per_fragment() {
        use crate::zig_writer::stream_sink;
        let mem = Mem::default();
        let mut w = ZigWriter::new(stream_sink(mem.clone()), 1024);
        write_lookup(&mut w, 0xA, '\n', " \u{bb} not found.\n", None).unwrap();
        w.flush().unwrap();
        assert_eq!(
            String::from_utf8(mem.0.borrow().clone()).unwrap(),
            "U+0A \u{ab} \n \u{bb} not found.\n"
        );
    }
}
