//! Crash capture (port of `crash/sentry.zig`).
//!
//! PRIVACY NOTE: Ghostty by default does NOT send any data over the network.
//! Crash reports are only stored locally, as Sentry envelopes in
//! `$XDG_STATE_HOME/ghostty/crash/<uuid>.ghosttycrash`. It is up to the user
//! to grab them and send them to someone if they want to.
//!
//! Upstream uses the Sentry native SDK with a custom transport that writes
//! the envelope to disk. We have no such SDK, so the equivalent event and
//! envelope are produced by hand from a panic hook and a fatal-signal
//! handler (see `signal.rs`). The envelope layout is the same: a header line
//! followed by an `event` item (with `release`, the build tags, the
//! `thread-type` tag and the `Dimensions` context) and, for panics, a
//! backtrace attachment.

use std::{
    fmt::{self, Write as _},
    io,
    path::PathBuf,
    sync::{
        OnceLock,
        atomic::{AtomicBool, Ordering},
    },
    time::{SystemTime, UNIX_EPOCH},
};

use crate::{
    context,
    dir::{self, REPORT_EXTENSION},
    envelope::{Attachment, Envelope, Item, ObjectMap},
    signal,
};

/// Whether this build captures crashes (Zig: `build_options.sentry`).
pub const ENABLED: bool = cfg!(feature = "sentry");

/// Build metadata recorded as tags on every event (Zig: `sentry.setTag`).
#[derive(Debug, Clone)]
pub struct Options {
    /// `build_config.version_string`.
    pub release: String,
    pub build_mode: String,
    pub app_runtime: String,
    pub font_backend: String,
    pub renderer: String,
    /// Override the crash directory; `None` uses [`dir::default_dir`].
    pub dir: Option<PathBuf>,
}

impl Default for Options {
    fn default() -> Options {
        Options {
            release: "1.3.1".into(),
            build_mode: if cfg!(debug_assertions) {
                "Debug"
            } else {
                "ReleaseFast"
            }
            .into(),
            app_runtime: "gtk".into(),
            font_backend: "fontconfig_freetype".into(),
            renderer: "opengl".into(),
            dir: None,
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum InitError {
    #[error("crash dir: {0}")]
    Dir(#[from] io::Error),
}

/// Immutable process-wide state, prepared at init so that the signal handler
/// does not need to allocate or consult the environment.
pub(crate) struct State {
    /// The crash directory as bytes (no trailing slash).
    pub dir: Vec<u8>,
    /// Pre-rendered `"release":"..","environment":"production","tags":{...`
    /// (the tags object is left open so `thread-type` can be appended).
    pub prefix: String,
}

static STATE: OnceLock<State> = OnceLock::new();
static ACTIVE: AtomicBool = AtomicBool::new(false);

pub(crate) fn state() -> Option<&'static State> {
    if ACTIVE.load(Ordering::Acquire) {
        STATE.get()
    } else {
        None
    }
}

/// Process-wide initialization of crash capture; a no-op unless the `sentry`
/// feature is enabled (like upstream's `-Dsentry=false`).
///
/// Call from one thread; [`deinit`] should be called from the same thread.
pub fn init(opts: &Options) -> Result<(), InitError> {
    // Not supported on Windows currently, doesn't build (same as Zig).
    if !ENABLED || cfg!(windows) {
        return Ok(());
    }
    install(opts)
}

/// Install the panic hook and fatal-signal handlers regardless of the
/// `sentry` feature. [`init`] is the feature-gated entry point.
pub fn install(opts: &Options) -> Result<(), InitError> {
    if STATE.get().is_some() {
        // Must only start once; re-activate if previously deinitialized.
        ACTIVE.store(true, Ordering::Release);
        return Ok(());
    }

    let dir = match &opts.dir {
        Some(d) => d.clone(),
        None => dir::default_dir()?.path,
    };
    let mut prefix = String::new();
    let _ = write!(prefix, "\"release\":");
    let _ = json_str(&mut prefix, &opts.release);
    let _ = write!(prefix, ",\"environment\":\"production\",\"tags\":{{");
    for (i, (k, v)) in [
        ("build-mode", &opts.build_mode),
        ("app-runtime", &opts.app_runtime),
        ("font-backend", &opts.font_backend),
        ("renderer", &opts.renderer),
    ]
    .iter()
    .enumerate()
    {
        if i > 0 {
            prefix.push(',');
        }
        let _ = json_str(&mut prefix, k);
        prefix.push(':');
        let _ = json_str(&mut prefix, v);
    }

    let state = State {
        dir: dir.as_os_str().as_encoded_bytes().to_vec(),
        prefix,
    };
    if STATE.set(state).is_err() {
        return Ok(());
    }
    ACTIVE.store(true, Ordering::Release);

    install_panic_hook();
    signal::install_handlers();
    log::debug!("crash capture initialized dir={}", dir.display());
    Ok(())
}

/// Process-wide deinitialization. Handlers stay registered but become
/// inert (a process that is shutting down must not write reports).
pub fn deinit() {
    ACTIVE.store(false, Ordering::Release);
}

// ---------------------------------------------------------------------------
// Event building. Must stay allocation-free: the signal handler uses it.

/// Escape `s` as a JSON string (including quotes).
pub(crate) fn json_str<W: fmt::Write>(w: &mut W, s: &str) -> fmt::Result {
    w.write_char('"')?;
    for c in s.chars() {
        match c {
            '"' => w.write_str("\\\"")?,
            '\\' => w.write_str("\\\\")?,
            '\n' => w.write_str("\\n")?,
            '\r' => w.write_str("\\r")?,
            '\t' => w.write_str("\\t")?,
            c if (c as u32) < 0x20 => write!(w, "\\u{:04x}", c as u32)?,
            c => w.write_char(c)?,
        }
    }
    w.write_char('"')
}

/// Days-since-epoch to (year, month, day) (Howard Hinnant's algorithm).
fn civil(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

/// `2024-08-29T02:38:57.607016Z`
pub(crate) fn write_rfc3339<W: fmt::Write>(w: &mut W, secs: i64, micros: u32) -> fmt::Result {
    let (y, mo, d) = civil(secs.div_euclid(86_400));
    let t = secs.rem_euclid(86_400);
    write!(
        w,
        "{y:04}-{mo:02}-{d:02}T{:02}:{:02}:{:02}.{micros:06}Z",
        t / 3600,
        t % 3600 / 60,
        t % 60
    )
}

pub(crate) fn now() -> (i64, u32) {
    match SystemTime::now().duration_since(UNIX_EPOCH) {
        Ok(d) => (d.as_secs() as i64, d.subsec_micros()),
        Err(_) => (0, 0),
    }
}

/// Format 16 random bytes as a v4 UUID (`8-4-4-4-12`, lowercase).
pub(crate) fn uuid_string(mut b: [u8; 16], out: &mut [u8; 36]) {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    b[6] = (b[6] & 0x0f) | 0x40;
    b[8] = (b[8] & 0x3f) | 0x80;
    let mut o = 0;
    for (i, byte) in b.iter().enumerate() {
        if matches!(i, 4 | 6 | 8 | 10) {
            out[o] = b'-';
            o += 1;
        }
        out[o] = HEX[(byte >> 4) as usize];
        out[o + 1] = HEX[(byte & 15) as usize];
        o += 2;
    }
}

/// What went wrong.
pub(crate) struct Crash<'a> {
    /// UUID with dashes.
    pub uuid: &'a str,
    pub secs: i64,
    pub micros: u32,
    /// Exception type: `panic` or the signal name.
    pub kind: &'a str,
    /// Human readable description.
    pub value: &'a str,
    /// Sentry mechanism type: `panic` or `signalhandler`.
    pub mechanism: &'a str,
    /// Signal info, registers and stack of the faulting thread (Darwin
    /// signal path; see `native.rs`).
    pub native: Option<&'a crate::native::Native>,
}

/// Write the event JSON (single line). Mirrors what Sentry produces after
/// Ghostty's `beforeSend`: the `thread-type` tag and `Dimensions` context
/// are only present when the crashing thread has a thread state.
pub(crate) fn write_event<W: fmt::Write>(w: &mut W, st: &State, c: &Crash<'_>) -> fmt::Result {
    w.write_str("{\"event_id\":\"")?;
    for ch in c.uuid.chars().filter(|c| *c != '-') {
        w.write_char(ch)?;
    }
    w.write_str("\",\"timestamp\":\"")?;
    write_rfc3339(w, c.secs, c.micros)?;
    w.write_str("\",\"platform\":\"native\",\"level\":\"fatal\",")?;
    w.write_str(&st.prefix)?;

    let ctx = context::current();
    if let Some((kind, _)) = &ctx {
        write!(w, ",\"thread-type\":\"{}\"", kind.as_str())?;
    }
    w.write_str("},\"contexts\":{")?;
    if let Some((_, Some(d))) = &ctx {
        let i = |v: u32| i32::try_from(v).unwrap_or(-1);
        write!(
            w,
            "\"Dimensions\":{{\"screen-width\":{},\"screen-height\":{},\"grid-columns\":{},\"grid-rows\":{},\"cell-width\":{},\"cell-height\":{}}}",
            i(d.screen_width),
            i(d.screen_height),
            i(d.grid_columns),
            i(d.grid_rows),
            i(d.cell_width),
            i(d.cell_height)
        )?;
    }
    w.write_str("},\"exception\":{\"values\":[{\"type\":")?;
    json_str(w, c.kind)?;
    w.write_str(",\"value\":")?;
    json_str(w, c.value)?;
    w.write_str(",\"mechanism\":{\"type\":")?;
    json_str(w, c.mechanism)?;
    w.write_str(",\"handled\":false")?;
    if let Some(n) = c.native {
        crate::native::write_mechanism_rest_and_stacktrace(w, n)?;
        w.write_str("}]}}")
    } else {
        w.write_str("}}]}}")
    }
}

/// Write the envelope header line and the event item header line (the event
/// payload follows). `len` is the event payload length.
pub(crate) fn write_envelope_head<W: fmt::Write>(
    w: &mut W,
    c: &Crash<'_>,
    len: usize,
) -> fmt::Result {
    w.write_str("{\"event_id\":\"")?;
    for ch in c.uuid.chars().filter(|c| *c != '-') {
        w.write_char(ch)?;
    }
    w.write_str("\",\"sent_at\":\"")?;
    write_rfc3339(w, c.secs, c.micros)?;
    write!(w, "\"}}\n{{\"type\":\"event\",\"length\":{len}}}\n")
}

pub(crate) fn report_file_name(uuid: &str) -> String {
    format!("{uuid}.{REPORT_EXTENSION}")
}

// ---------------------------------------------------------------------------
// Panic path (may allocate).

thread_local! {
    static IN_PANIC_REPORT: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

fn install_panic_hook() {
    let prev = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        if !IN_PANIC_REPORT.with(|f| f.replace(true)) {
            if let Some(st) = state() {
                let msg = if let Some(s) = info.payload().downcast_ref::<&str>() {
                    (*s).to_owned()
                } else if let Some(s) = info.payload().downcast_ref::<String>() {
                    s.clone()
                } else {
                    "Box<dyn Any>".to_owned()
                };
                let value = match info.location() {
                    Some(l) => format!("{msg} ({}:{}:{})", l.file(), l.line(), l.column()),
                    None => msg,
                };
                let backtrace = std::backtrace::Backtrace::force_capture().to_string();
                match write_report(st, "panic", &value, "panic", Some(&backtrace)) {
                    Ok(path) => {
                        log::warn!("crash report written to disk path={}", path.display());
                    }
                    Err(err) => log::warn!("failed to persist crash report err={err}"),
                }
            }
            IN_PANIC_REPORT.with(|f| f.set(false));
        }
        prev(info);
    }));
}

/// Build the envelope and write it to the crash dir (Zig: `Transport.sendInternal`).
fn write_report(
    st: &State,
    kind: &str,
    value: &str,
    mechanism: &str,
    backtrace: Option<&str>,
) -> io::Result<PathBuf> {
    let mut raw = [0u8; 16];
    signal::random_bytes(&mut raw);
    let mut u = [0u8; 36];
    uuid_string(raw, &mut u);
    let uuid = std::str::from_utf8(&u).unwrap_or_default();
    let (secs, micros) = now();
    let crash = Crash {
        uuid,
        secs,
        micros,
        kind,
        value,
        mechanism,
        native: None,
    };

    let mut event = String::new();
    write_event(&mut event, st, &crash).map_err(io::Error::other)?;
    let mut head = String::new();
    write_envelope_head(&mut head, &crash, event.len()).map_err(io::Error::other)?;
    let mut data = head.into_bytes();
    data.extend_from_slice(event.as_bytes());
    if let Some(bt) = backtrace {
        let mut a = Attachment::new("backtrace.txt", bt.as_bytes().to_vec());
        a.attachment_type = Some("event.attachment".into());
        let mut env = Envelope {
            headers: ObjectMap::new(),
            items: vec![Item::Attachment(a)],
        };
        let enc = env.items[0].encode().clone();
        data.push(b'\n');
        let mut h = enc.headers;
        h.insert("length".into(), enc.payload.len().into());
        data.extend_from_slice(serde_json::to_string(&h)?.as_bytes());
        data.push(b'\n');
        data.extend_from_slice(&enc.payload);
    }

    #[cfg(unix)]
    let dir = {
        use std::os::unix::ffi::OsStrExt as _;
        std::path::Path::new(std::ffi::OsStr::from_bytes(&st.dir))
    };
    // Windows paths are not arbitrary bytes; the dir was UTF-8 in practice.
    #[cfg(not(unix))]
    let dir_buf = PathBuf::from(String::from_utf8_lossy(&st.dir).into_owned());
    #[cfg(not(unix))]
    let dir = dir_buf.as_path();
    std::fs::create_dir_all(dir)?;
    let path = dir.join(report_file_name(uuid));
    std::fs::write(&path, data)?;
    Ok(path)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;
    use crate::envelope::ItemType;

    #[test]
    fn civil_dates() {
        let mut s = String::new();
        write_rfc3339(&mut s, 1_724_899_137, 607_016).unwrap();
        assert_eq!("2024-08-29T02:38:57.607016Z", s);
        let mut s = String::new();
        write_rfc3339(&mut s, 0, 0).unwrap();
        assert_eq!("1970-01-01T00:00:00.000000Z", s);
        let mut s = String::new();
        write_rfc3339(&mut s, 951_782_400, 1).unwrap(); // leap day 2000
        assert_eq!("2000-02-29T00:00:00.000001Z", s);
    }

    #[test]
    fn uuid_format() {
        let mut out = [0u8; 36];
        uuid_string([0xff; 16], &mut out);
        assert_eq!(
            "ffffffff-ffff-4fff-bfff-ffffffffffff",
            std::str::from_utf8(&out).unwrap()
        );
    }

    #[test]
    fn json_escape() {
        let mut s = String::new();
        json_str(&mut s, "a\"b\\c\n\u{1}").unwrap();
        assert_eq!("\"a\\\"b\\\\c\\n\\u0001\"", s);
    }

    #[test]
    fn event_and_report_parse_back() {
        let t = tempfile::tempdir().unwrap();
        let opts = Options {
            dir: Some(t.path().join("crash")),
            ..Options::default()
        };
        // Do not install global handlers in unit tests; build state only.
        let st = State {
            dir: opts
                .dir
                .as_ref()
                .unwrap()
                .as_os_str()
                .as_encoded_bytes()
                .to_vec(),
            prefix: "\"release\":\"1.3.1\",\"environment\":\"production\",\"tags\":{\"renderer\":\"opengl\"".into(),
        };
        let dims = context::SurfaceDims::new();
        dims.set(context::DimsSnapshot {
            screen_width: 800,
            screen_height: 600,
            grid_columns: 80,
            grid_rows: 24,
            cell_width: 10,
            cell_height: 25,
        });
        let _g = context::ThreadGuard::new(context::ThreadType::Renderer, Some(dims));
        let path = write_report(&st, "panic", "boom \"x\"", "panic", Some("bt\nline")).unwrap();
        assert!(path.to_string_lossy().ends_with(".ghosttycrash"));
        let data = std::fs::read(&path).unwrap();
        let mut env = Envelope::parse_slice(&data).unwrap();
        assert_eq!(2, env.items.len());
        assert_eq!(ItemType::Event, env.items[0].item_type());
        let Item::Encoded(ev) = &env.items[0] else {
            panic!()
        };
        let v: serde_json::Value = serde_json::from_slice(&ev.payload).unwrap();
        assert_eq!("renderer", v["tags"]["thread-type"]);
        assert_eq!(80, v["contexts"]["Dimensions"]["grid-columns"]);
        assert_eq!("boom \"x\"", v["exception"]["values"][0]["value"]);
        env.items[1].decode().unwrap();
        let Item::Attachment(a) = &env.items[1] else {
            panic!()
        };
        assert_eq!(b"bt\nline", a.payload.as_slice());
    }
}
