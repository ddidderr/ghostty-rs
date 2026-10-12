//! The individual benchmarks (ports of `benchmark/*.zig`) and the CLI action
//! dispatch (`benchmark/cli.zig`).
use crate::args::{Args, data_file, read_short};
use crate::benchmark::{Benchmark, BenchmarkFailed, Result, RunMode, RunResult, run};
use ghostty_terminal::{
    point::{Coordinate, Point},
    render::RenderState,
    screen_set::Key as ScreenKey,
    terminal::{Options as TermOptions, Terminal},
};
use ghostty_vt::{
    osc,
    parser::Parser,
    stream::{Action, Handler, Stream},
    utf8_decoder::Utf8Decoder,
};
use std::hint::black_box;
use std::io::Read;

const CHUNK: usize = 4096;

/// The available actions (benchmarks) of the CLI.
pub const ACTIONS: &[&str] = &[
    "codepoint-width",
    "grapheme-break",
    "screen-clone",
    "terminal-parser",
    "terminal-stream",
    "is-symbol",
    "osc-parser",
];

fn warn(what: &str, e: &dyn std::fmt::Display) -> BenchmarkFailed {
    log::warn!("{what} err={e}");
    BenchmarkFailed
}

/// Shared data-file handling: opened in setup, closed in teardown.
struct Data {
    path: Option<String>,
    f: Option<Box<dyn Read>>,
}

impl Data {
    fn new(path: Option<String>) -> Data {
        Data { path, f: None }
    }

    fn open(&mut self) -> Result {
        self.f =
            data_file(self.path.as_deref()).map_err(|e| warn("error opening data file", &e))?;
        Ok(())
    }

    /// Feed every 4096-byte chunk of the file to `func`.
    fn for_each_chunk(&mut self, mut func: impl FnMut(&[u8]) -> Result) -> Result {
        let Some(f) = self.f.as_mut() else {
            return Ok(());
        };
        let mut buf = [0u8; CHUNK];
        loop {
            let n = read_short(f.as_mut(), &mut buf)
                .map_err(|e| warn("error reading data file", &e))?;
            if n == 0 {
                return Ok(());
            }
            func(&buf[..n])?;
        }
    }
}

// ---------------------------------------------------------------------------
// terminal-stream

struct PrintHandler {
    t: Terminal,
}

impl Handler for PrintHandler {
    type Error = ghostty_terminal::terminal::Error;

    #[inline]
    fn vt(&mut self, action: Action<'_>) -> std::result::Result<(), Self::Error> {
        if let Action::Print(p) = action {
            self.t.print(p.cp)?;
        }
        Ok(())
    }
}

pub struct TerminalStream {
    data: Data,
    stream: Stream<PrintHandler>,
    /// Rust-only: use the full `stream_readonly` handler (applies every
    /// terminal-modifying action) instead of the print-only handler.
    full: bool,
}

impl TerminalStream {
    pub fn create(args: &mut Args) -> std::result::Result<Self, String> {
        let rows = args.take("terminal-rows", 80u16)?;
        let cols = args.take("terminal-cols", 120u16)?;
        let data = args.take_opt_string("data")?;
        let full = match args.take_opt_string("handler")?.as_deref() {
            None | Some("print") => false,
            Some("full") => true,
            Some(m) => return Err(format!("invalid value for --handler: {m}")),
        };
        let t = Terminal::new(TermOptions::new(cols, rows)).map_err(|e| format!("{e:?}"))?;
        Ok(TerminalStream {
            data: Data::new(data),
            stream: Stream::new(PrintHandler { t }),
            full,
        })
    }

    /// Total bytes processed so far is not tracked; see `data_len`.
    pub fn terminal(&self) -> &Terminal {
        &self.stream.handler.t
    }
}

impl Benchmark for TerminalStream {
    fn setup(&mut self) -> Result {
        self.stream.handler.t.full_reset();
        self.data.open()
    }

    fn teardown(&mut self) {
        self.data.f = None;
    }

    fn step(&mut self) -> Result {
        if self.full {
            let mut s = self.stream.handler.t.vt_stream();
            return self.data.for_each_chunk(|chunk| {
                s.next_slice(chunk)
                    .map_err(|e| warn("error processing data file chunk", &format!("{e:?}")))
            });
        }
        let stream = &mut self.stream;
        self.data.for_each_chunk(|chunk| {
            stream
                .next_slice(chunk)
                .map_err(|e| warn("error processing data file chunk", &format!("{e:?}")))
        })
    }
}

// ---------------------------------------------------------------------------
// terminal-parser

pub struct TerminalParser {
    data: Data,
}

impl TerminalParser {
    pub fn create(args: &mut Args) -> std::result::Result<Self, String> {
        Ok(TerminalParser {
            data: Data::new(args.take_opt_string("data")?),
        })
    }
}

impl Benchmark for TerminalParser {
    fn setup(&mut self) -> Result {
        self.data.open()
    }

    fn teardown(&mut self) {
        self.data.f = None;
    }

    fn step(&mut self) -> Result {
        let mut p = Parser::new();
        self.data.for_each_chunk(|chunk| {
            for &c in chunk {
                drop(p.next(c));
            }
            Ok(())
        })
    }
}

// ---------------------------------------------------------------------------
// osc-parser

pub struct OscParser {
    data: Data,
    parser: osc::Parser,
}

impl OscParser {
    pub fn create(args: &mut Args) -> std::result::Result<Self, String> {
        Ok(OscParser {
            data: Data::new(args.take_opt_string("data")?),
            parser: osc::Parser::init(true),
        })
    }
}

impl Benchmark for OscParser {
    fn setup(&mut self) -> Result {
        self.data.open()?;
        self.parser.reset();
        Ok(())
    }

    fn teardown(&mut self) {
        self.data.f = None;
    }

    fn step(&mut self) -> Result {
        let Some(f) = self.data.f.as_mut() else {
            return Ok(());
        };
        let mut osc_buf = [0u8; 4096];
        loop {
            let mut len_buf = [0u8; 8];
            match read_short(f.as_mut(), &mut len_buf) {
                Ok(8) => {}
                Ok(_) => return Ok(()),
                Err(_) => return Err(BenchmarkFailed),
            }
            let len = usize::from_le_bytes(len_buf);
            if len > osc_buf.len() {
                return Err(BenchmarkFailed);
            }
            match read_short(f.as_mut(), &mut osc_buf[..len]) {
                Ok(n) if n == len => {}
                Ok(_) => return Ok(()),
                Err(_) => return Err(BenchmarkFailed),
            }
            self.parser.next_slice(&osc_buf[..len]);
            black_box(self.parser.end(Some(0x07)));
            self.parser.reset();
        }
    }
}

// ---------------------------------------------------------------------------
// codepoint-width

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WidthMode {
    Noop,
    Wcwidth,
    Simd,
    Table,
}

pub struct CodepointWidth {
    data: Data,
    mode: WidthMode,
}

impl CodepointWidth {
    pub fn create(args: &mut Args) -> std::result::Result<Self, String> {
        let mode = match args.take_opt_string("mode")?.as_deref() {
            None | Some("noop") => WidthMode::Noop,
            Some("wcwidth") => WidthMode::Wcwidth,
            Some("simd") => WidthMode::Simd,
            Some("table") => WidthMode::Table,
            Some(m) => return Err(format!("invalid value for --mode: {m}")),
        };
        Ok(CodepointWidth {
            data: Data::new(args.take_opt_string("data")?),
            mode,
        })
    }
}

/// Decode `chunk` and call `func` for each codepoint.
#[inline]
fn decode_each(d: &mut Utf8Decoder, chunk: &[u8], mut func: impl FnMut(u32)) {
    for &c in chunk {
        let (cp, consumed) = d.next(c);
        debug_assert!(consumed);
        if let Some(cp) = cp {
            func(cp);
        }
    }
}

impl Benchmark for CodepointWidth {
    fn setup(&mut self) -> Result {
        self.data.open()
    }

    fn teardown(&mut self) {
        self.data.f = None;
    }

    fn step(&mut self) -> Result {
        match self.mode {
            WidthMode::Noop => Ok(()),
            WidthMode::Wcwidth => {
                // libc wcwidth needs FFI, which this workspace forbids.
                log::warn!("wcwidth mode is not available in the Rust port");
                Ok(())
            }
            // The Rust port has no separate SIMD width function: both use
            // the lookup table.
            WidthMode::Simd | WidthMode::Table => {
                let mut d = Utf8Decoder::new();
                let table = ghostty_unicode::table();
                self.data.for_each_chunk(|chunk| {
                    decode_each(&mut d, chunk, |cp| {
                        // Same trick as terminal.rs.
                        black_box(if cp <= 0xFF { 1 } else { table.get(cp).width });
                    });
                    Ok(())
                })
            }
        }
    }
}

// ---------------------------------------------------------------------------
// grapheme-break

pub struct GraphemeBreak {
    data: Data,
    noop: bool,
}

impl GraphemeBreak {
    pub fn create(args: &mut Args) -> std::result::Result<Self, String> {
        let noop = match args.take_opt_string("mode")?.as_deref() {
            None | Some("table") => false,
            Some("noop") => true,
            Some(m) => return Err(format!("invalid value for --mode: {m}")),
        };
        Ok(GraphemeBreak {
            data: Data::new(args.take_opt_string("data")?),
            noop,
        })
    }
}

impl Benchmark for GraphemeBreak {
    fn setup(&mut self) -> Result {
        self.data.open()
    }

    fn teardown(&mut self) {
        self.data.f = None;
    }

    fn step(&mut self) -> Result {
        let mut d = Utf8Decoder::new();
        if self.noop {
            return self.data.for_each_chunk(|chunk| {
                for &c in chunk {
                    black_box(d.next(c));
                }
                Ok(())
            });
        }
        let mut state = ghostty_unicode::BreakState::Default;
        let mut cp1 = 0u32;
        self.data.for_each_chunk(|chunk| {
            decode_each(&mut d, chunk, |cp2| {
                black_box(ghostty_unicode::grapheme_break(cp1, cp2, &mut state));
                cp1 = cp2;
            });
            Ok(())
        })
    }
}

// ---------------------------------------------------------------------------
// is-symbol

pub struct IsSymbol {
    data: Data,
}

impl IsSymbol {
    pub fn create(args: &mut Args) -> std::result::Result<Self, String> {
        // Modes `uucode` and `table` both use the generated table here.
        match args.take_opt_string("mode")?.as_deref() {
            None | Some("uucode" | "table") => {}
            Some(m) => return Err(format!("invalid value for --mode: {m}")),
        }
        Ok(IsSymbol {
            data: Data::new(args.take_opt_string("data")?),
        })
    }
}

impl Benchmark for IsSymbol {
    fn setup(&mut self) -> Result {
        self.data.open()
    }

    fn teardown(&mut self) {
        self.data.f = None;
    }

    fn step(&mut self) -> Result {
        let mut d = Utf8Decoder::new();
        let table = ghostty_unicode::symbols_table::table();
        self.data.for_each_chunk(|chunk| {
            decode_each(&mut d, chunk, |cp| {
                black_box(table.get(cp));
            });
            Ok(())
        })
    }
}

// ---------------------------------------------------------------------------
// screen-clone

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CloneMode {
    Noop,
    Clone,
    Render,
}

pub struct ScreenClone {
    data: Data,
    mode: CloneMode,
    terminal: Terminal,
    /// Clones are kept alive so freeing is not benchmarked.
    keep: Vec<ghostty_terminal::screen::Screen>,
}

impl ScreenClone {
    pub fn create(args: &mut Args) -> std::result::Result<Self, String> {
        let mode = match args.take_opt_string("mode")?.as_deref() {
            None | Some("clone") => CloneMode::Clone,
            Some("noop") => CloneMode::Noop,
            Some("render") => CloneMode::Render,
            Some(m) => return Err(format!("invalid value for --mode: {m}")),
        };
        let rows = args.take("terminal-rows", 80u16)?;
        let cols = args.take("terminal-cols", 120u16)?;
        let data = args.take_opt_string("data")?;
        let terminal = Terminal::new(TermOptions::new(cols, rows)).map_err(|e| format!("{e:?}"))?;
        Ok(ScreenClone {
            data: Data::new(data),
            mode,
            terminal,
            keep: Vec::new(),
        })
    }
}

impl Benchmark for ScreenClone {
    fn setup(&mut self) -> Result {
        self.terminal.full_reset();

        // Force a style on every single row.
        let rows = self.terminal.rows;
        let mut s = self.terminal.vt_stream();
        let fail = |e: &dyn std::fmt::Debug| warn("error processing setup data", &format!("{e:?}"));
        s.next_slice(b"\x1b[48;2;20;40;60m").map_err(|e| fail(&e))?;
        for _ in 0..rows - 1 {
            s.next_slice(b"hello\r\n").map_err(|e| fail(&e))?;
        }
        s.next_slice(b"hello").map_err(|e| fail(&e))?;

        // Setup our terminal state from the data file (not benchmarked).
        self.data.open()?;
        self.data.for_each_chunk(|chunk| {
            s.next_slice(chunk)
                .map_err(|e| warn("error processing data file chunk", &format!("{e:?}")))
        })?;
        self.data.f = None;
        Ok(())
    }

    fn step(&mut self) -> Result {
        // We loop because it's so fast that a single run doesn't capture it.
        match self.mode {
            CloneMode::Noop => {
                for _ in 0..1000 {
                    black_box(self.terminal.screens.active());
                }
            }
            CloneMode::Clone => {
                for _ in 0..1000 {
                    let s = self.terminal.screens.active();
                    let copy = s
                        .clone_screen(Point::Viewport(Coordinate { x: 0, y: 0 }), None)
                        .map_err(|e| warn("error cloning screen", &format!("{e:?}")))?;
                    self.keep.push(black_box(copy));
                }
            }
            CloneMode::Render => {
                let mut state = RenderState::new();
                let fail = |e: &dyn std::fmt::Debug| {
                    warn("error updating render state", &format!("{e:?}"))
                };
                // The first run is dominated by allocation; ignore it.
                state.update(&mut self.terminal).map_err(|e| fail(&e))?;
                for _ in 0..1000 {
                    // Forces a full rebuild because it thinks our screen changed.
                    state.screen = ScreenKey::Alternate;
                    state.update(&mut self.terminal).map_err(|e| fail(&e))?;
                    black_box(&state);
                }
            }
        }
        Ok(())
    }

    fn teardown(&mut self) {
        self.keep.clear();
    }
}

// ---------------------------------------------------------------------------
// CLI

/// Run the named action with the given (already `+action`-free) arguments.
/// When `time` is set the measured result is returned.
pub fn main_action(action: &str, mut args: Args) -> std::result::Result<RunResult, String> {
    fn go<B: Benchmark>(
        b: std::result::Result<B, String>,
        args: Args,
    ) -> std::result::Result<RunResult, String> {
        let mut b = b?;
        args.finish()?;
        run(&mut b, RunMode::Once).map_err(|e| e.to_string())
    }
    // Options are consumed by `create`; leftovers are errors.
    let a = &mut args;
    match action {
        "screen-clone" => {
            let b = ScreenClone::create(a);
            go(b, args)
        }
        "terminal-stream" => {
            let b = TerminalStream::create(a);
            go(b, args)
        }
        "codepoint-width" => {
            let b = CodepointWidth::create(a);
            go(b, args)
        }
        "grapheme-break" => {
            let b = GraphemeBreak::create(a);
            go(b, args)
        }
        "terminal-parser" => {
            let b = TerminalParser::create(a);
            go(b, args)
        }
        "is-symbol" => {
            let b = IsSymbol::create(a);
            go(b, args)
        }
        "osc-parser" => {
            let b = OscParser::create(a);
            go(b, args)
        }
        _ => Err(format!("invalid action: +{action}")),
    }
}
