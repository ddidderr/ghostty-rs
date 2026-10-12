//! The interactive theme preview of `+list-themes` (Zig: `Preview` in
//! list_themes.zig, built on libvaxis). This is a ratatui + crossterm
//! implementation with the same layout, key bindings and behaviour.
//!
//! Known differences from the vaxis version:
//!   * input is read with `termina` (not crossterm) so terminal responses
//!     such as the color-scheme report are visible; drawing still goes
//!     through ratatui's crossterm backend,
//!   * OSC 8 hyperlinks and the double/dashed/dotted/curly underline styles
//!     are re-emitted as raw escapes after each frame (see [`Overlay`]),
//!   * the color scheme is queried (`CSI ? 996 n`) and tracked
//!     (`CSI ? 2031 h`) (sent unconditionally; vaxis gates on DECRQM 2031 which
//!     termina cannot parse); it starts as light
//!     like the Zig code does, but `COLORFGBG` is used as a hint if set.

use std::{cell::RefCell, fmt::Write as _, io::Write};

use ghostty_config::{config::Config, types_a::TerminalColor};
use ratatui::{
    Frame,
    buffer::Buffer,
    crossterm::event::{
        Event,
        KeyCode,
        KeyEvent,
        KeyEventKind,
        KeyModifiers,
        MouseButton,
        MouseEvent,
        MouseEventKind,
    },
    layout::{Position, Rect},
    style::{Color, Modifier, Style},
};
use unicode_segmentation::UnicodeSegmentation as _;

use super::{
    ColorScheme,
    SMALL_LIST_THRESHOLD,
    ThemeListElement,
    absolute_path,
    should_include_theme,
};
use crate::error::Error;

const LOREM_IPSUM: &str = include_str!("../lorem_ipsum.txt");

/// Width of the theme list on the left.
const LIST_WIDTH: u16 = 32;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Normal,
    Help,
    Search,
    Save,
}

/// The terminal's color scheme (`vaxis.Color.Scheme`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scheme {
    Light,
    Dark,
}

/// A single line text input (`vaxis.widgets.TextInput`). The cursor is a
/// byte offset (always on a grapheme boundary) and all editing follows
/// vaxis: grapheme-wise left/right/delete, byte-wise (ASCII space) words.
#[derive(Debug, Clone, Default)]
pub struct TextInput {
    pub text: String,
    /// Cursor position in bytes.
    pub cursor: usize,
}

/// Byte length of the first grapheme of `s` (0 when empty).
fn first_grapheme_len(s: &str) -> usize {
    s.graphemes(true).next().map_or(0, str::len)
}

/// Byte length of the last grapheme of `s` (0 when empty).
fn last_grapheme_len(s: &str) -> usize {
    s.graphemes(true).next_back().map_or(0, str::len)
}

impl TextInput {
    pub fn clear(&mut self) {
        self.text.clear();
        self.cursor = 0;
    }

    fn move_left(&mut self, n: usize) {
        self.cursor = self.cursor.saturating_sub(n);
    }

    fn move_right(&mut self, n: usize) {
        self.cursor = (self.cursor + n).min(self.text.len());
    }

    /// Delete `n` bytes before the cursor (`growGapLeft`).
    fn delete_left(&mut self, n: usize) {
        let start = self.cursor.saturating_sub(n);
        self.text.drain(start..self.cursor);
        self.cursor = start;
    }

    /// Delete `n` bytes after the cursor (`growGapRight`).
    fn delete_right(&mut self, n: usize) {
        let end = (self.cursor + n).min(self.text.len());
        self.text.drain(self.cursor..end);
    }

    /// Bytes from the cursor to the end of the next word (`moveForwardWordwise`).
    fn forward_word_len(&self) -> usize {
        let second = &self.text.as_bytes()[self.cursor..];
        let start = second.iter().take_while(|&&b| b == b' ').count();
        second[start..]
            .iter()
            .position(|&b| b == b' ')
            .map_or(second.len(), |p| start + p)
    }

    /// Bytes from the cursor back to the start of the previous word
    /// (`moveBackwardWordwise`).
    fn backward_word_len(&self) -> usize {
        let first = self.text[..self.cursor].trim_end_matches(' ');
        let idx = first.rfind(' ').map_or(0, |last| last + 1);
        self.cursor - idx
    }

    /// Handle an editing key (`TextInput.update`). Returns true if it was
    /// consumed.
    pub fn update(&mut self, key: &KeyEvent) -> bool {
        // vaxis `Key.matches` compares the modifiers exactly (lock keys
        // aside), except that text input tolerates Shift.
        let m = key.modifiers;
        let none = m.is_empty();
        let only = |x: KeyModifiers| m == x;
        let ctrl = only(KeyModifiers::CONTROL);
        let alt = only(KeyModifiers::ALT);
        let code = key.code;
        let c = |ch: char| code == KeyCode::Char(ch);
        if none && code == KeyCode::Backspace {
            self.delete_left(last_grapheme_len(&self.text[..self.cursor]));
        } else if (none && code == KeyCode::Delete) || (ctrl && c('d')) {
            self.delete_right(first_grapheme_len(&self.text[self.cursor..]));
        } else if (none && code == KeyCode::Left) || (ctrl && c('b')) {
            self.move_left(last_grapheme_len(&self.text[..self.cursor]));
        } else if (none && code == KeyCode::Right) || (ctrl && c('f')) {
            self.move_right(first_grapheme_len(&self.text[self.cursor..]));
        } else if (ctrl && c('a')) || (none && code == KeyCode::Home) {
            self.cursor = 0;
        } else if (ctrl && c('e')) || (none && code == KeyCode::End) {
            self.cursor = self.text.len();
        } else if ctrl && c('k') {
            self.text.truncate(self.cursor);
        } else if ctrl && c('u') {
            self.delete_left(self.cursor);
        } else if alt && (c('b') || code == KeyCode::Left) {
            self.move_left(self.backward_word_len());
        } else if alt && (c('f') || code == KeyCode::Right) {
            self.move_right(self.forward_word_len());
        } else if (ctrl && c('w')) || (alt && code == KeyCode::Backspace) {
            // Move one word backward, then delete what was moved over.
            let n = self.backward_word_len();
            self.move_left(n);
            self.delete_right(n);
        } else if alt && c('d') {
            self.delete_right(self.forward_word_len());
        } else if let KeyCode::Char(ch) = code
            && (none || only(KeyModifiers::SHIFT))
        {
            self.text.insert(self.cursor, ch);
            self.cursor += ch.len_utf8();
        } else {
            return false;
        }
        true
    }
}

/// `zf.rank` with `.plain = true, .to_lower = true`: every token must match
/// the (ASCII-lowercased) name as a subsequence of bytes. Only whether a
/// rank exists is used by the preview, so the score is a simple span length
/// (lower is better).
#[must_use]
pub fn rank(name: &str, tokens: &[&str]) -> Option<f64> {
    let name: Vec<u8> = name.to_ascii_lowercase().into_bytes();
    let mut total = 0.0;
    for token in tokens {
        let token = token.as_bytes();
        if token.is_empty() {
            continue;
        }
        // Prefer a contiguous match.
        let contiguous = name.windows(token.len()).position(|w| w == token);
        if let Some(start) = contiguous {
            total += start as f64;
            continue;
        }
        let mut ti = 0;
        let mut first = None;
        let mut last = 0;
        for (i, c) in name.iter().enumerate() {
            if ti < token.len() && *c == token[ti] {
                first.get_or_insert(i);
                last = i;
                ti += 1;
            }
        }
        if ti < token.len() {
            return None;
        }
        total += 1000.0 + (last - first.unwrap_or(0)) as f64;
    }
    Some(total)
}

/// Standard base64 (for OSC 52).
fn base64(data: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::new();
    for chunk in data.chunks(3) {
        let n = (u32::from(chunk[0]) << 16)
            | (u32::from(*chunk.get(1).unwrap_or(&0)) << 8)
            | u32::from(*chunk.get(2).unwrap_or(&0));
        out.push(T[(n >> 18) as usize & 63] as char);
        out.push(T[(n >> 12) as usize & 63] as char);
        out.push(if chunk.len() > 1 {
            T[(n >> 6) as usize & 63] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            T[n as usize & 63] as char
        } else {
            '='
        });
    }
    out
}

/// Underline styles ratatui cannot express (`SGR 4:N`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ul {
    Double,
    Curly,
    Dotted,
    Dashed,
}

impl Ul {
    fn sgr(self) -> &'static str {
        match self {
            Ul::Double => "4:2",
            Ul::Curly => "4:3",
            Ul::Dotted => "4:4",
            Ul::Dashed => "4:5",
        }
    }
}

/// A recorded decoration of drawn text (absolute cell coordinates).
struct Deco {
    x: u16,
    y: u16,
    width: u16,
    link: Option<String>,
    ul: Option<Ul>,
}

/// A decorated run of cells, re-emitted as raw escapes after ratatui has
/// flushed the frame: text with an OSC 8 hyperlink and/or an extended
/// underline style. Each run is a single-style, single-row segment.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Overlay {
    pub x: u16,
    pub y: u16,
    /// The cell text of the run.
    pub text: String,
    pub style: Style,
    pub link: Option<String>,
    pub ul: Option<Ul>,
}

impl Overlay {
    fn capture(buf: &Buffer, d: &Deco) -> Overlay {
        let mut text = String::new();
        let mut skip = 0u16;
        let mut style = Style::default();
        for dx in 0..d.width {
            let Some(cell) = buf.cell((d.x + dx, d.y)) else {
                break;
            };
            if dx == 0 {
                style = cell.style();
            }
            if skip > 0 {
                skip -= 1;
                continue;
            }
            text.push_str(cell.symbol());
            skip = gw(cell.symbol()).saturating_sub(1);
        }
        Overlay {
            x: d.x,
            y: d.y,
            text,
            style,
            link: d.link.clone(),
            ul: d.ul,
        }
    }
}

fn sgr_color(c: Color, fg: bool) -> String {
    let (base, bright, ext, reset) = if fg {
        (30, 90, 38, 39)
    } else {
        (40, 100, 48, 49)
    };
    match c {
        Color::Reset => reset.to_string(),
        Color::Black => base.to_string(),
        Color::Red => (base + 1).to_string(),
        Color::Green => (base + 2).to_string(),
        Color::Yellow => (base + 3).to_string(),
        Color::Blue => (base + 4).to_string(),
        Color::Magenta => (base + 5).to_string(),
        Color::Cyan => (base + 6).to_string(),
        Color::Gray => (base + 7).to_string(),
        Color::DarkGray => bright.to_string(),
        Color::LightRed => (bright + 1).to_string(),
        Color::LightGreen => (bright + 2).to_string(),
        Color::LightYellow => (bright + 3).to_string(),
        Color::LightBlue => (bright + 4).to_string(),
        Color::LightMagenta => (bright + 5).to_string(),
        Color::LightCyan => (bright + 6).to_string(),
        Color::White => (bright + 7).to_string(),
        Color::Rgb(r, g, b) => format!("{ext};2;{r};{g};{b}"),
        Color::Indexed(i) => format!("{ext};5;{i}"),
    }
}

/// The raw escape output that draws `overlays` on top of the frame.
#[must_use]
pub fn render_overlays(overlays: &[Overlay]) -> String {
    let mut out = String::new();
    for o in overlays {
        let mut sgr: Vec<String> = vec!["0".to_owned()];
        if let Some(c) = o.style.fg.map(|c| sgr_color(c, true)) {
            sgr.push(c);
        }
        if let Some(c) = o.style.bg.map(|c| sgr_color(c, false)) {
            sgr.push(c);
        }
        if o.style.add_modifier.contains(Modifier::BOLD) {
            sgr.push("1".to_owned());
        }
        if o.style.add_modifier.contains(Modifier::ITALIC) {
            sgr.push("3".to_owned());
        }
        if o.style.add_modifier.contains(Modifier::UNDERLINED) {
            sgr.push("4".to_owned());
        }
        if let Some(ul) = o.ul {
            sgr.push(ul.sgr().to_owned());
        }
        let _ = write!(out, "\x1b[{};{}H", o.y + 1, o.x + 1);
        let _ = write!(out, "\x1b[{}m", sgr.join(";"));
        if let Some(link) = &o.link {
            let _ = write!(out, "\x1b]8;;{link}\x1b\\");
        }
        out.push_str(&o.text);
        if o.link.is_some() {
            out.push_str("\x1b]8;;\x1b\\");
        }
        out.push_str("\x1b[0m");
    }
    out
}

/// Cached result of loading the currently previewed theme.
enum Loaded {
    Ok(Box<Config>),
    Err(String),
}

/// A pending side effect requested by the state (needs the terminal).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Effect {
    /// Copy the (decoded) bytes to the clipboard (OSC 52).
    Copy(ghostty_os::osbytes::EncodedOs),
}

/// The preview state (Zig: `Preview`).
pub struct Preview {
    pub should_quit: bool,
    pub themes: Vec<ThemeListElement>,
    pub filtered: Vec<usize>,
    pub current: usize,
    pub window: usize,
    pub hex: bool,
    pub mode: Mode,
    pub color_scheme: Scheme,
    pub text_input: TextInput,
    pub theme_filter: ColorScheme,
    /// Last mouse event, consumed by the next draw.
    mouse: Option<MouseEvent>,
    /// The list height as of the last draw (for clamping the window).
    list_height: u16,
    loaded: Option<(ghostty_os::osbytes::EncodedOs, Loaded)>,
    /// Where `w` writes the auto theme file (overridable for tests).
    pub auto_theme_path: Option<std::path::PathBuf>,
    effects: Vec<Effect>,
    overlays: Vec<Overlay>,
}

impl Preview {
    pub fn new(themes: Vec<ThemeListElement>, theme_filter: ColorScheme) -> Result<Preview, Error> {
        let mut p = Preview {
            should_quit: false,
            themes,
            filtered: vec![],
            current: 0,
            window: 0,
            hex: false,
            mode: Mode::Normal,
            color_scheme: Scheme::Light,
            text_input: TextInput::default(),
            theme_filter,
            mouse: None,
            list_height: 0,
            loaded: None,
            auto_theme_path: None,
            effects: vec![],
            overlays: vec![],
        };
        p.update_filtered()?;
        Ok(p)
    }

    /// Recompute the filtered theme list (Zig: `updateFiltered`).
    pub fn update_filtered(&mut self) -> Result<(), Error> {
        let relative = self.current.saturating_sub(self.window);
        let selected: Option<String> = self
            .filtered
            .get(self.current)
            .map(|&i| self.themes[i].theme.clone());

        let old = self.filtered.clone();
        self.filtered.clear();

        let mut theme_config = Config::new_default();
        let lowered = self.text_input.text.to_ascii_lowercase();
        let tokens: Vec<&str> = lowered.split(' ').filter(|t| !t.is_empty()).collect();
        let searching = !self.text_input.text.is_empty();

        for (i, theme) in self.themes.iter_mut().enumerate() {
            theme_config.load_file(&absolute_path(&theme.path))?;
            if !should_include_theme(self.theme_filter, &theme_config) {
                continue;
            }
            if searching {
                theme.rank = rank(&theme.display_name(), &tokens);
                if theme.rank.is_some() {
                    self.filtered.push(i);
                }
            } else {
                self.filtered.push(i);
                theme.rank = None;
            }
        }

        if old == self.filtered {
            return Ok(());
        }

        if self.filtered.is_empty() {
            self.current = 0;
            self.window = 0;
            return Ok(());
        }

        let (current, window) = (|| {
            let Some(selected) = &selected else {
                return (0, 0);
            };
            for (i, &index) in self.filtered.iter().enumerate() {
                if &self.themes[index].theme == selected {
                    // Keep the relative position but ensure all search results are visible
                    let new_window = i.saturating_sub(relative);
                    // If the new window would hide some results at the top, adjust it
                    return (
                        i,
                        if self.filtered.len() <= SMALL_LIST_THRESHOLD {
                            0
                        } else {
                            new_window
                        },
                    );
                }
            }
            (0, 0)
        })();
        self.current = current;
        self.window = window;
        Ok(())
    }

    pub fn up(&mut self, count: usize) {
        if self.filtered.is_empty() {
            self.current = 0;
            return;
        }
        self.current = self.current.saturating_sub(count);
    }

    pub fn down(&mut self, count: usize) {
        if self.filtered.is_empty() {
            self.current = 0;
            return;
        }
        self.current += count;
        if self.current >= self.filtered.len() {
            self.current = self.filtered.len() - 1;
        }
    }

    fn selected(&self) -> Option<&ThemeListElement> {
        self.filtered.get(self.current).map(|&i| &self.themes[i])
    }

    /// Take the pending side effects.
    pub fn take_effects(&mut self) -> Vec<Effect> {
        std::mem::take(&mut self.effects)
    }

    /// Handle a terminal event (Zig: `update`).
    pub fn update(&mut self, event: &Event) -> Result<(), Error> {
        match event {
            Event::Key(key) if key.kind != KeyEventKind::Release => self.update_key(key)?,
            Event::Mouse(m) => self.mouse = Some(*m),
            _ => {}
        }
        Ok(())
    }

    fn update_key(&mut self, key: &KeyEvent) -> Result<(), Error> {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        let shift = key.modifiers.contains(KeyModifiers::SHIFT);
        let plain = key.modifiers.difference(KeyModifiers::SHIFT).is_empty();
        let code = key.code;
        let is_char = |c: char| code == KeyCode::Char(c) && plain && !shift;

        if ctrl && code == KeyCode::Char('c') {
            self.should_quit = true;
        }

        match self.mode {
            Mode::Normal => {
                if plain && (is_char('q') || code == KeyCode::Esc) {
                    self.should_quit = true;
                }
                if (plain && (is_char('?') || code == KeyCode::Char('?') || code == KeyCode::F(1)))
                    || (ctrl && code == KeyCode::Char('h'))
                {
                    self.mode = Mode::Help;
                }
                if is_char('/') {
                    self.mode = Mode::Search;
                }
                if plain && code == KeyCode::Enter {
                    self.mode = Mode::Save;
                }
                if ctrl && matches!(code, KeyCode::Char('x' | '/' | '_')) {
                    self.text_input.clear();
                    self.update_filtered()?;
                }
                if plain && code == KeyCode::Home {
                    self.current = 0;
                }
                if plain && code == KeyCode::End {
                    self.current = self.filtered.len().saturating_sub(1);
                }
                if plain && (matches!(code, KeyCode::Char('j' | '+') | KeyCode::Down)) {
                    self.down(1);
                }
                if plain && code == KeyCode::PageDown {
                    self.down(20);
                }
                if plain && (matches!(code, KeyCode::Char('k' | '-') | KeyCode::Up)) {
                    self.up(1);
                }
                if plain && code == KeyCode::PageUp {
                    self.up(20);
                }
                if plain && matches!(code, KeyCode::Char('h' | 'x')) && !shift {
                    self.hex = true;
                }
                if is_char('d') {
                    self.hex = false;
                }
                if is_char('c') {
                    if let Some(t) = self.selected() {
                        // Theme names are in the reversible OS-string encoding.
                        let name = ghostty_os::osbytes::EncodedOs::from_encoded(t.theme.clone());
                        self.effects.push(Effect::Copy(name));
                    }
                } else if code == KeyCode::Char('C')
                    || (code == KeyCode::Char('c') && shift && !ctrl)
                {
                    if let Some(t) = self.selected() {
                        let path = t.path.clone();
                        self.effects.push(Effect::Copy(path));
                    }
                }
                if is_char('f') {
                    self.theme_filter = match self.theme_filter {
                        ColorScheme::All => ColorScheme::Dark,
                        ColorScheme::Dark => ColorScheme::Light,
                        ColorScheme::Light => ColorScheme::All,
                    };
                    self.update_filtered()?;
                }
            }
            Mode::Help => {
                if is_char('q') {
                    self.should_quit = true;
                }
                if (plain
                    && (is_char('?')
                        || code == KeyCode::Char('?')
                        || code == KeyCode::Esc
                        || code == KeyCode::F(1)))
                    || (ctrl && code == KeyCode::Char('h'))
                {
                    self.mode = Mode::Normal;
                }
            }
            Mode::Search => {
                if plain && matches!(code, KeyCode::Esc | KeyCode::Enter) {
                    self.mode = Mode::Normal;
                    return Ok(());
                }
                if ctrl && matches!(code, KeyCode::Char('x' | '/' | '_')) {
                    self.text_input.clear();
                    self.update_filtered()?;
                    return Ok(());
                }
                self.text_input.update(key);
                self.update_filtered()?;
            }
            Mode::Save => {
                if is_char('q') {
                    self.should_quit = true;
                }
                if plain && matches!(code, KeyCode::Esc | KeyCode::Enter) {
                    self.mode = Mode::Normal;
                }
                if is_char('w') {
                    self.save_selected_theme();
                }
            }
        }
        Ok(())
    }

    fn save_selected_theme(&self) {
        let Some(theme) = self.selected() else {
            return;
        };
        let _ = write_auto_theme_file(self.auto_theme_path.as_deref(), &theme.theme);
    }

    // ---- colors ----

    fn ui_fg(&self) -> Color {
        match self.color_scheme {
            Scheme::Light => Color::Rgb(0, 0, 0),
            Scheme::Dark => Color::Rgb(0xff, 0xff, 0xff),
        }
    }
    fn ui_bg(&self) -> Color {
        match self.color_scheme {
            Scheme::Light => Color::Rgb(0xff, 0xff, 0xff),
            Scheme::Dark => Color::Rgb(0, 0, 0),
        }
    }
    fn ui_standard(&self) -> Style {
        Style::default().fg(self.ui_fg()).bg(self.ui_bg())
    }
    fn ui_hover_bg(&self) -> Color {
        match self.color_scheme {
            Scheme::Light => Color::Rgb(0xbb, 0xbb, 0xbb),
            Scheme::Dark => Color::Rgb(0x22, 0x22, 0x22),
        }
    }
    fn ui_highlighted(&self) -> Style {
        Style::default().fg(self.ui_fg()).bg(self.ui_hover_bg())
    }
    fn ui_selected(&self) -> Style {
        let bg = match self.color_scheme {
            Scheme::Light => Color::Rgb(0xaa, 0xaa, 0xaa),
            Scheme::Dark => Color::Rgb(0x33, 0x33, 0x33),
        };
        Style::default().fg(Color::Rgb(0, 0xaa, 0)).bg(bg)
    }
    fn ui_err(&self) -> Style {
        Style::default().fg(Color::Rgb(0xff, 0, 0)).bg(self.ui_bg())
    }

    // ---- drawing ----

    /// Draw the whole UI into `buf`. Returns the cursor position (search mode).
    pub fn draw(&mut self, buf: &mut Buffer) -> Option<Position> {
        let area = buf.area;
        let cell = RefCell::new(buf);
        let decos = RefCell::new(Vec::new());
        let win = Win {
            buf: &cell,
            decos: &decos,
            area,
        };
        win.clear();

        let theme_list = win.child(0, 0, LIST_WIDTH.min(area.width), area.height);
        self.list_height = theme_list.area.height;

        let mut highlight: Option<u16> = None;

        if let Some(mouse) = self.mouse.take() {
            if self.mode == Mode::Normal {
                match mouse.kind {
                    MouseEventKind::ScrollUp => self.up(1),
                    MouseEventKind::ScrollDown => self.down(1),
                    _ => {}
                }
                if mouse.column < theme_list.area.width && mouse.row < theme_list.area.height {
                    if mouse.kind == MouseEventKind::Up(MouseButton::Left) {
                        let selection = self.window + usize::from(mouse.row);
                        if selection < self.filtered.len() {
                            self.current = selection;
                        }
                    }
                    highlight = Some(mouse.row);
                }
            }
        }

        if self.filtered.is_empty() {
            self.current = 0;
            self.window = 0;
        } else {
            let height = usize::from(theme_list.area.height).max(1);
            let start = self.window;
            let end = self.window + height - 1;
            if self.current > end {
                self.window = self.current + 1 - height;
            }
            if self.current < start {
                self.window = self.current;
            }
            if self.window >= self.filtered.len() {
                self.window = self.filtered.len() - 1;
            }
        }

        let ui = self.ui_standard();
        theme_list.fill(ui);

        for row in 0..theme_list.area.height {
            let index = self.window + usize::from(row);
            if index >= self.filtered.len() {
                break;
            }
            let theme = &self.themes[self.filtered[index]];

            #[derive(PartialEq)]
            enum St {
                Normal,
                Highlighted,
                Selected,
            }
            let style = if index == self.current {
                St::Selected
            } else if highlight == Some(row) {
                St::Highlighted
            } else {
                St::Normal
            };

            if style == St::Selected {
                theme_list.print("\u{276f} ", self.ui_selected(), row, 0);
            }
            theme_list.print_ex(
                &theme.display_name(),
                match style {
                    St::Normal => self.ui_standard(),
                    St::Highlighted => self.ui_highlighted(),
                    St::Selected => self.ui_selected(),
                },
                row,
                2,
                Some(&theme.to_uri()),
                None,
            );
            if style == St::Selected {
                let name_len = gw(&theme.display_name());
                if name_len + 4 < LIST_WIDTH {
                    for i in (2 + name_len)..(LIST_WIDTH - 2) {
                        theme_list.print(" ", self.ui_selected(), row, i);
                    }
                }
                theme_list.print(
                    " \u{276e}",
                    self.ui_selected(),
                    row,
                    LIST_WIDTH.saturating_sub(2),
                );
            }
        }

        self.draw_preview(&win, LIST_WIDTH);

        let mut cursor = None;
        match self.mode {
            Mode::Normal => {}
            Mode::Help => {
                let (width, height) = (60u16, 22u16);
                let child = win.child_border(
                    (area.width / 2).saturating_sub(width / 2),
                    (area.height / 2).saturating_sub(height / 2),
                    width,
                    height,
                    self.ui_standard(),
                );
                let key_help: [(&str, &str); 19] = [
                    ("^C, q, ESC", "Quit."),
                    ("F1, ?, ^H", "Toggle help window."),
                    ("f", "Cycle through theme filters."),
                    ("k, \u{2191}", "Move up 1 theme."),
                    ("ScrollUp", "Move up 1 theme."),
                    ("PgUp", "Move up 20 themes."),
                    ("j, \u{2193}", "Move down 1 theme."),
                    ("ScrollDown", "Move down 1 theme."),
                    ("PgDown", "Move down 20 themes."),
                    ("h, x", "Show palette numbers in hexadecimal."),
                    ("d", "Show palette numbers in decimal."),
                    ("c", "Copy theme name to the clipboard."),
                    ("C", "Copy theme path to the clipboard."),
                    ("Home", "Go to the start of the list."),
                    ("End", "Go to the end of the list."),
                    ("/", "Start search."),
                    ("^X, ^/", "Clear search."),
                    ("\u{23ce}", "Save theme or close search window."),
                    ("w", "Write theme to auto config file."),
                ];
                for (i, (keys, help)) in key_help.iter().enumerate() {
                    let row = u16::try_from(i).unwrap_or(0) + 1;
                    child.print(keys, self.ui_standard(), row, 2);
                    child.print("\u{2014}", self.ui_standard(), row, 15);
                    child.print(help, self.ui_standard(), row, 17);
                }
            }
            Mode::Search => {
                let child = win.child_border(
                    20,
                    area.height.saturating_sub(5),
                    area.width.saturating_sub(40),
                    3,
                    self.ui_standard(),
                );
                child.print(&self.text_input.text, self.ui_standard(), 0, 0);
                let col = gw(&self.text_input.text[..self.text_input.cursor]);
                if col < child.area.width {
                    cursor = Some(Position::new(child.area.x + col, child.area.y));
                }
            }
            Mode::Save => {
                if let Some(theme) = self.selected() {
                    let (width, height) = (92u16, 17u16);
                    let child = win.child_border(
                        (area.width / 2).saturating_sub(width / 2),
                        (area.height / 2).saturating_sub(height / 2),
                        width,
                        height,
                        self.ui_standard(),
                    );
                    let theme_line = format!("theme = {}", theme.display_name());
                    let save_instructions: [&str; 14] = [
                        "To apply this theme, add the following line to your Ghostty configuration:",
                        "",
                        &theme_line,
                        "",
                        "Save the configuration file and then reload it to apply the new theme.",
                        "",
                        "Or press 'w' to write an auto theme file to your system's preferred default config path.",
                        "Then add the following line to your Ghostty configuration and reload:",
                        "",
                        "config-file = ?auto/theme.ghostty",
                        "",
                        "For more details on configuration and themes, visit the Ghostty documentation:",
                        "",
                        "https://ghostty.org/docs/config/reference",
                    ];
                    for (i, instruction) in save_instructions.iter().enumerate() {
                        child.print(
                            instruction,
                            self.ui_standard(),
                            u16::try_from(i).unwrap_or(0) + 1,
                            2,
                        );
                    }
                }
            }
        }

        // Collect the decorations (hyperlinks, extended underlines) with the
        // final cell contents/styles for the raw-escape overlay pass.
        let decos = decos.into_inner();
        let buf = cell.into_inner();
        self.overlays = decos
            .iter()
            .map(|d| Overlay::capture(buf, d))
            .collect::<Vec<_>>();
        cursor
    }

    /// Hyperlink / extended-underline overlays of the last [`Self::draw`].
    #[must_use]
    pub fn overlays(&self) -> &[Overlay] {
        &self.overlays
    }

    fn load_current(&mut self) -> Option<&Loaded> {
        let theme = self.selected()?.clone();
        let stale = self.loaded.as_ref().is_none_or(|(p, _)| p != &theme.path);
        if stale {
            let mut config = Config::new_default();
            let loaded = match config.load_file(&absolute_path(&theme.path)) {
                Ok(()) => Loaded::Ok(Box::new(config)),
                Err(e) => Loaded::Err(error_name(&e)),
            };
            self.loaded = Some((theme.path, loaded));
        }
        self.loaded.as_ref().map(|(_, l)| l)
    }

    fn draw_preview(&mut self, win: &Win<'_, '_>, x_off: u16) {
        let area = win.area;
        let x_off = x_off.min(area.width);
        let width = area.width - x_off;
        let ui_err = self.ui_err();
        let ui_std = self.ui_standard();
        let hex = self.hex;

        if self.filtered.is_empty() {
            let child = win.child(0, 0, area.width, area.height);
            child.fill(ui_std);
            child.print(
                "No theme found!",
                ui_std,
                (area.height / 2).saturating_sub(1),
                (area.width / 2).saturating_sub(7),
            );
            return;
        }

        let Some(theme) = self.selected().cloned() else {
            return;
        };
        let config = match self.load_current() {
            Some(Loaded::Ok(c)) => c.clone(),
            Some(Loaded::Err(err)) => {
                let child = win.child(x_off, 0, width, area.height);
                child.fill(ui_std);
                let middle = child.area.height / 2;
                let text = format!("Unable to open {} from:", theme.display_name());
                child.print(
                    &text,
                    ui_err,
                    middle.saturating_sub(1),
                    (child.area.width / 2).saturating_sub(gw(&text) / 2),
                );
                child.print_ex(
                    &theme.path.to_text_lossy(),
                    ui_err,
                    middle,
                    (child.area.width / 2).saturating_sub(gw(&theme.path.to_text_lossy()) / 2),
                    Some(&theme.to_uri()),
                    None,
                );
                child.print(
                    err,
                    ui_err,
                    middle + 1,
                    (child.area.width / 2).saturating_sub(gw(err) / 2),
                );
                return;
            }
            None => return,
        };

        let mut next_start: u16 = 0;

        let rgb = |c: ghostty_config::types_a::Color| Color::Rgb(c.r, c.g, c.b);
        let term_rgb = |c: &Option<TerminalColor>, default: Color| match c {
            Some(TerminalColor::Color(c)) => rgb(*c),
            _ => default,
        };
        let fg = rgb(config.foreground);
        let bg = rgb(config.background);
        let cursor_fg = term_rgb(&config.cursor_text, bg);
        let cursor_bg = term_rgb(&config.cursor_color, fg);
        let selection_fg = term_rgb(&config.selection_foreground, bg);
        let selection_bg = term_rgb(&config.selection_background, fg);
        let cursor = Style::default().fg(cursor_fg).bg(cursor_bg);
        let standard_selection = Style::default().fg(selection_fg).bg(selection_bg);
        let standard = Style::default().fg(fg).bg(bg);
        let standard_bold = standard.add_modifier(Modifier::BOLD);
        let standard_italic = standard.add_modifier(Modifier::ITALIC);
        let standard_bold_italic = standard.add_modifier(Modifier::BOLD | Modifier::ITALIC);
        // Single underline is a plain SGR 4; the other styles (double,
        // dashed, dotted, curly) are re-emitted as raw `SGR 4:N` overlays.

        let color = |i: usize| {
            let c = config.palette.value[i];
            Color::Rgb(c.r, c.g, c.b)
        };

        // Header.
        {
            let child = win.child(x_off, next_start, width, 4);
            child.fill(standard);
            let uri = theme.to_uri();
            child.print_ex(
                &theme.display_name(),
                standard_bold_italic,
                1,
                (child.area.width / 2).saturating_sub(gw(&theme.display_name()) / 2),
                Some(&uri),
                None,
            );
            child.print_ex(
                &theme.path.to_text_lossy(),
                standard,
                2,
                (child.area.width / 2).saturating_sub(gw(&theme.path.to_text_lossy()) / 2),
                Some(&uri),
                None,
            );
            next_start += child.area.height;
        }

        // Diagnostics.
        if !config.diagnostics.items().is_empty() {
            let n = u16::try_from(config.diagnostics.items().len()).unwrap_or(u16::MAX);
            let child = win.child(x_off, next_start, width, 2 + n);
            let text = "Problems were encountered trying to load the theme:";
            child.print(
                text,
                ui_err,
                0,
                (child.area.width / 2).saturating_sub(gw(text) / 2),
            );
            for (i, diag) in config.diagnostics.items().iter().enumerate() {
                child.print(
                    &ghostty_os::osbytes::config_str_to_text(&diag.to_string()),
                    ui_err,
                    2 + u16::try_from(i).unwrap_or(0),
                    2,
                );
            }
            next_start += 2 + n;
        }

        // Palette.
        {
            let child = win.child(x_off, next_start, width, 6);
            child.fill(standard);
            for i in 0..16usize {
                let r = u16::try_from(i / 8).unwrap_or(0);
                let c = u16::try_from(i % 8).unwrap_or(0);
                let text = if hex {
                    format!(" {i:02x}")
                } else {
                    format!("{i:3}")
                };
                child.print(&text, standard, 3 * r, c * 8);
                let block = Style::default().fg(color(i)).bg(bg);
                child.print("\u{2588}\u{2588}\u{2588}\u{2588}", block, 3 * r, 4 + c * 8);
                child.print(
                    "\u{2588}\u{2588}\u{2588}\u{2588}",
                    block,
                    3 * r + 1,
                    4 + c * 8,
                );
            }
            next_start += child.area.height;
        }

        // Code sample.
        {
            let child = win.child(x_off, next_start, width, 24);
            let bold = Style::default().fg(fg).bg(bg).add_modifier(Modifier::BOLD);
            let c = |i: usize| Style::default().fg(color(i)).bg(bg);
            let (color1, color2, color3, color4, color5, color6) =
                (c(1), c(2), c(3), c(4), c(5), c(6));
            let color6ul = color6.add_modifier(Modifier::UNDERLINED);
            let (color10, color12, color238) = (c(10), c(12), c(238));
            child.fill(standard);

            let rule = |row: u16, head: &str| {
                child.print_segments(&[(head, color238)], row, 2);
                if child.area.width > 10 {
                    for col in 10..child.area.width {
                        child.print("\u{2500}", color238, row, col);
                    }
                }
            };

            child.print_segments(
                &[
                    ("\u{2192}", color2),
                    (" ", standard),
                    ("bat", color4),
                    (" ", standard),
                    ("ziggzagg.zig", color6ul),
                ],
                0,
                2,
            );
            rule(
                1,
                "\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{252c}",
            );
            child.print_segments(
                &[
                    ("       \u{2502} ", color238),
                    ("File: ", standard),
                    ("ziggzagg.zig", bold),
                ],
                2,
                2,
            );
            rule(
                3,
                "\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{253c}",
            );
            child.print_segments(
                &[
                    ("   1   \u{2502} ", color238),
                    ("const", color5),
                    (" std ", standard),
                    ("= @import", color5),
                    ("(", standard),
                    ("\"std\"", color10),
                    (");", standard),
                ],
                4,
                2,
            );
            child.print_segments(&[("   2   \u{2502}", color238)], 5, 2);
            child.print_segments(
                &[
                    ("   3   \u{2502} ", color238),
                    ("pub ", color5),
                    ("fn ", color12),
                    ("main", color2),
                    ("() ", standard),
                    ("!", color5),
                    ("void", color12),
                    (" {", standard),
                ],
                6,
                2,
            );
            child.print_segments(
                &[
                    ("   4   \u{2502}     ", color238),
                    ("const ", color5),
                    ("stdout ", standard),
                    ("=", color5),
                    (" std.Io.getStdOut().writer();", standard),
                ],
                7,
                2,
            );
            child.print_segments(
                &[
                    ("   5   \u{2502}     ", color238),
                    ("var ", color5),
                    ("i:", standard),
                    (" usize", color12),
                    (" =", color5),
                    (" 1", color4),
                    (";", standard),
                ],
                8,
                2,
            );
            child.print_segments(
                &[
                    ("   6   \u{2502}     ", color238),
                    ("while ", color5),
                    ("(i ", standard),
                    ("<= ", color5),
                    ("16", color4),
                    (") : (i ", standard),
                    ("+= ", color5),
                    ("1", color4),
                    (") {", standard),
                ],
                9,
                2,
            );
            child.print_segments(
                &[
                    ("   7   \u{2502}         ", color238),
                    ("if ", color5),
                    ("(i ", standard),
                    ("% ", color5),
                    ("15 ", color4),
                    ("== ", color5),
                    ("0", color4),
                    (") {", standard),
                ],
                10,
                2,
            );
            child.print_segments(
                &[
                    ("   8   \u{2502}             ", color238),
                    ("try ", color5),
                    ("stdout.writeAll(", standard),
                    ("\"ZiggZagg", color10),
                    ("\\n", color12),
                    ("\"", color10),
                    (");", standard),
                ],
                11,
                2,
            );
            child.print_segments(
                &[
                    ("   9   \u{2502}         ", color238),
                    ("} ", standard),
                    ("else if ", color5),
                    ("(i ", standard),
                    ("% ", color5),
                    ("3 ", color4),
                    ("== ", color5),
                    ("0", color4),
                    (") {", standard),
                ],
                12,
                2,
            );
            child.print_segments(
                &[
                    ("  10   \u{2502}             ", color238),
                    ("try ", color5),
                    ("stdout.writeAll(", standard),
                    ("\"Zigg", color10),
                    ("\\n", color12),
                    ("\"", color10),
                    (");", standard),
                ],
                13,
                2,
            );
            child.print_segments(
                &[
                    ("  11   \u{2502}         ", color238),
                    ("} ", standard),
                    ("else if ", color5),
                    ("(i ", standard),
                    ("% ", color5),
                    ("5 ", color4),
                    ("== ", color5),
                    ("0", color4),
                    (") {", standard),
                ],
                14,
                2,
            );
            child.print_segments(
                &[
                    ("  12   \u{2502}             ", color238),
                    ("try ", color5),
                    ("stdout.writeAll(", standard),
                    ("\"Zagg", color10),
                    ("\\n", color12),
                    ("\"", color10),
                    (");", standard),
                ],
                15,
                2,
            );
            child.print_segments(
                &[
                    ("  13   \u{2502}         ", color238),
                    ("} ", standard),
                    ("else ", color5),
                    ("{", standard),
                ],
                16,
                2,
            );
            child.print_segments(
                &[
                    ("  14   \u{2502}             ", color238),
                    ("try ", color5),
                    ("stdout.print(\"{d}\\n\", .{i})", standard_selection),
                    (";", cursor),
                ],
                17,
                2,
            );
            child.print_segments(
                &[("  15   \u{2502}         ", color238), ("}", standard)],
                18,
                2,
            );
            child.print_segments(
                &[("  16   \u{2502}     ", color238), ("}", standard)],
                19,
                2,
            );
            child.print_segments(&[("  17   \u{2502} ", color238), ("}", standard)], 20, 2);
            rule(
                21,
                "\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2534}",
            );
            child.print_segments(
                &[
                    ("ghostty ", color6),
                    ("on ", standard),
                    ("\u{e0a0} main ", color4),
                    ("[+] ", color1),
                    ("via ", standard),
                    ("\u{e0a0} v0.13.0 ", color3),
                    ("via ", standard),
                    ("\u{f313}  impure (ghostty-env)", color4),
                ],
                22,
                2,
            );
            child.print_segments(
                &[
                    ("\u{2726} ", color4),
                    ("at ", standard),
                    ("10:36:15 ", color3),
                    ("\u{2192}", color2),
                ],
                23,
                2,
            );
            next_start += child.area.height;
        }

        if next_start < area.height {
            let child = win.child(x_off, next_start, width, area.height - next_start);
            child.fill(standard);
            let mut words = LOREM_IPSUM.split([' ', '\n']);
            let mut row: u16 = 1;
            let mut col: u16 = 2;
            while row < child.area.height {
                let word = if let Some(w) = words.next() {
                    w
                } else {
                    words = LOREM_IPSUM.split([' ', '\n']);
                    words.next().unwrap_or("")
                };
                let word_len = u16::try_from(word.len()).unwrap_or(u16::MAX);
                if usize::from(col) + word.len() > usize::from(child.area.width) {
                    row += 1;
                    col = 2;
                }
                let mut ul = None;
                let style = match word {
                    "ipsum" => Style::default().fg(color(2)).bg(bg),
                    "consectetur" => standard_bold,
                    "reprehenderit" => standard_italic,
                    "Praesent" => standard_bold_italic,
                    "auctor" => standard.add_modifier(Modifier::UNDERLINED),
                    "dui" => {
                        ul = Some(Ul::Double);
                        standard
                    }
                    "erat" => {
                        ul = Some(Ul::Dashed);
                        standard
                    }
                    "enim" => {
                        ul = Some(Ul::Dotted);
                        standard
                    }
                    "odio" => {
                        ul = Some(Ul::Curly);
                        standard
                    }
                    _ => standard,
                };
                child.print_ex(word, style, row, col, None, ul);
                col = col.saturating_add(word_len).saturating_add(1);
            }
        }
    }
}

/// The Zig error name for display (`{}` of an error).
fn error_name(e: &ghostty_config::config::Error) -> String {
    use ghostty_config::config::Error;
    match e {
        Error::Io(io) => ghostty_os::zig_error::io_error_name(io)
            .trim_start_matches("error.")
            .to_owned(),
        e => e.to_string(),
    }
}

/// `resolveAutoThemePath`: `<config dir>/auto/theme.ghostty`.
pub fn resolve_auto_theme_path() -> Result<std::path::PathBuf, Error> {
    let main_cfg_path = ghostty_config::file_load::preferred_default_file_path()
        .map_err(|e| Error::Other(e.to_string()))?
        .to_path()?;
    let base_dir = main_cfg_path
        .parent()
        .ok_or_else(|| Error::Other("BadPathName".to_string()))?;
    Ok(base_dir.join("auto").join("theme.ghostty"))
}

/// `writeAutoThemeFile`.
pub fn write_auto_theme_file(
    path: Option<&std::path::Path>,
    theme_name: &str,
) -> Result<(), Error> {
    let auto_path = match path {
        Some(p) => p.to_path_buf(),
        None => resolve_auto_theme_path()?,
    };
    if let Some(dir) = auto_path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(
        &auto_path,
        ghostty_os::osbytes::config_str_to_bytes(&format!("theme = {theme_name}\n")),
    )?;
    Ok(())
}

// ---- A tiny window abstraction over a ratatui buffer (vaxis.Window) ----

/// Display width of a string.
fn gw(s: &str) -> u16 {
    crate::tui::gwidth(s)
}

/// A clipped, offset view onto the buffer (`vaxis.Window`). All coordinates
/// passed to its methods are relative to the window.
#[derive(Clone, Copy)]
struct Win<'a, 'b> {
    buf: &'a RefCell<&'b mut Buffer>,
    decos: &'a RefCell<Vec<Deco>>,
    area: Rect,
}

impl Win<'_, '_> {
    /// `Window.clear`.
    fn clear(&self) {
        self.reset_area(self.area);
    }

    fn reset_area(&self, area: Rect) {
        let mut buf = self.buf.borrow_mut();
        for y in area.top()..area.bottom() {
            for x in area.left()..area.right() {
                if let Some(c) = buf.cell_mut((x, y)) {
                    c.reset();
                }
            }
        }
    }

    /// `Window.fill` with a blank cell of the given style.
    fn fill(&self, style: Style) {
        // Anything previously drawn here is overwritten, including its
        // hyperlink/underline decorations.
        let a = self.area;
        self.decos.borrow_mut().retain(|d| {
            !(d.y >= a.top() && d.y < a.bottom() && d.x < a.right() && d.x + d.width > a.left())
        });
        let mut buf = self.buf.borrow_mut();
        for y in self.area.top()..self.area.bottom() {
            for x in self.area.left()..self.area.right() {
                if let Some(c) = buf.cell_mut((x, y)) {
                    c.reset();
                    c.set_symbol(" ");
                    c.set_style(style);
                }
            }
        }
    }

    /// `Window.child` (without border); clipped to this window.
    fn child(&self, x_off: u16, y_off: u16, width: u16, height: u16) -> Self {
        let x = self.area.x.saturating_add(x_off).min(self.area.right());
        let y = self.area.y.saturating_add(y_off).min(self.area.bottom());
        let w = width.min(self.area.right() - x);
        let h = height.min(self.area.bottom() - y);
        Win {
            buf: self.buf,
            decos: self.decos,
            area: Rect::new(x, y, w, h),
        }
    }

    /// `Window.child` with a border on all sides; returns the inner window.
    fn child_border(&self, x_off: u16, y_off: u16, width: u16, height: u16, style: Style) -> Self {
        let outer = self.child(x_off, y_off, width, height);
        outer.fill(style);
        let a = outer.area;
        if a.width >= 2 && a.height >= 2 {
            let mut buf = self.buf.borrow_mut();
            let mut put = |x: u16, y: u16, s: &str| {
                if let Some(c) = buf.cell_mut((x, y)) {
                    c.set_symbol(s);
                    c.set_style(style);
                }
            };
            for x in (a.left() + 1)..(a.right() - 1) {
                put(x, a.top(), "\u{2500}");
                put(x, a.bottom() - 1, "\u{2500}");
            }
            for y in (a.top() + 1)..(a.bottom() - 1) {
                put(a.left(), y, "\u{2502}");
                put(a.right() - 1, y, "\u{2502}");
            }
            put(a.left(), a.top(), "\u{256d}");
            put(a.right() - 1, a.top(), "\u{256e}");
            put(a.left(), a.bottom() - 1, "\u{2570}");
            put(a.right() - 1, a.bottom() - 1, "\u{256f}");
        }
        outer.child(1, 1, a.width.saturating_sub(2), a.height.saturating_sub(2))
    }

    /// `Window.printSegment` without wrapping (text is clipped). Returns the
    /// column after the text.
    fn print(&self, text: &str, style: Style, row: u16, col: u16) -> u16 {
        self.print_ex(text, style, row, col, None, None)
    }

    /// `Window.printSegment` with an optional OSC 8 hyperlink and an optional
    /// extended underline style (ratatui cannot express either, so they are
    /// recorded and re-emitted as raw escapes after the frame is flushed).
    fn print_ex(
        &self,
        text: &str,
        style: Style,
        row: u16,
        col: u16,
        link: Option<&str>,
        ul: Option<Ul>,
    ) -> u16 {
        if row >= self.area.height || col >= self.area.width {
            return col;
        }
        let x = self.area.x + col;
        let y = self.area.y + row;
        let max = usize::from(self.area.width - col);
        let nx = self.set_graphemes(x, y, text, max, style);
        if (link.is_some() || ul.is_some()) && nx > x {
            self.decos.borrow_mut().push(Deco {
                x,
                y,
                width: nx - x,
                link: link.map(str::to_owned),
                ul,
            });
        }
        nx - self.area.x
    }

    /// Write `text` grapheme by grapheme, each sized with `vaxis.gwidth`
    /// (not ratatui's width), stopping when a grapheme no longer fits.
    fn set_graphemes(&self, x: u16, y: u16, text: &str, max: usize, style: Style) -> u16 {
        let mut buf = self.buf.borrow_mut();
        let mut remaining = max.min(usize::from(buf.area.right().saturating_sub(x)));
        let mut cx = x;
        for g in text.graphemes(true) {
            if g.contains(char::is_control) {
                continue;
            }
            let w = gw(g);
            if w == 0 {
                continue;
            }
            let Some(rest) = remaining.checked_sub(usize::from(w)) else {
                break;
            };
            remaining = rest;
            buf[(cx, y)].set_symbol(g).set_style(style);
            for i in 1..w {
                buf[(cx + i, y)].reset();
            }
            cx += w;
        }
        cx
    }

    /// `Window.print`: several segments one after another.
    fn print_segments(&self, segments: &[(&str, Style)], row: u16, col: u16) -> u16 {
        let mut col = col;
        for (text, style) in segments {
            col = self.print(text, *style, row, col);
        }
        col
    }
}

// ---- The terminal driver ----

/// Convert a `termina` key event to the crossterm event type `Preview`
/// handles. Keys the preview has no use for map to `None`.
fn convert_key(k: termina::event::KeyEvent) -> Option<KeyEvent> {
    use termina::event::{KeyCode as T, KeyEventKind as TK, Modifiers as TM};
    let code = match k.code {
        T::Char(c) => KeyCode::Char(c),
        T::Enter => KeyCode::Enter,
        T::Backspace => KeyCode::Backspace,
        T::Tab => KeyCode::Tab,
        T::Escape => KeyCode::Esc,
        T::Left => KeyCode::Left,
        T::Right => KeyCode::Right,
        T::Up => KeyCode::Up,
        T::Down => KeyCode::Down,
        T::Home => KeyCode::Home,
        T::End => KeyCode::End,
        T::BackTab => KeyCode::BackTab,
        T::PageUp => KeyCode::PageUp,
        T::PageDown => KeyCode::PageDown,
        T::Insert => KeyCode::Insert,
        T::Delete => KeyCode::Delete,
        T::Function(n) => KeyCode::F(n),
        _ => return None,
    };
    let mut modifiers = KeyModifiers::NONE;
    for (t, c) in [
        (TM::SHIFT, KeyModifiers::SHIFT),
        (TM::ALT, KeyModifiers::ALT),
        (TM::CONTROL, KeyModifiers::CONTROL),
        (TM::SUPER, KeyModifiers::SUPER),
        (TM::HYPER, KeyModifiers::HYPER),
        (TM::META, KeyModifiers::META),
    ] {
        if k.modifiers.contains(t) {
            modifiers |= c;
        }
    }
    let kind = match k.kind {
        TK::Press => KeyEventKind::Press,
        TK::Release => KeyEventKind::Release,
        TK::Repeat => KeyEventKind::Repeat,
    };
    let mut ev = KeyEvent::new(code, modifiers);
    ev.kind = kind;
    Some(ev)
}

fn convert_mouse(m: termina::event::MouseEvent) -> MouseEvent {
    use termina::event::{MouseButton as TB, MouseEventKind as TK};
    let button = |b: TB| match b {
        TB::Left => MouseButton::Left,
        TB::Right => MouseButton::Right,
        TB::Middle => MouseButton::Middle,
    };
    let kind = match m.kind {
        TK::Down(b) => MouseEventKind::Down(button(b)),
        TK::Up(b) => MouseEventKind::Up(button(b)),
        TK::Drag(b) => MouseEventKind::Drag(button(b)),
        TK::Moved => MouseEventKind::Moved,
        TK::ScrollDown => MouseEventKind::ScrollDown,
        TK::ScrollUp => MouseEventKind::ScrollUp,
        TK::ScrollLeft => MouseEventKind::ScrollLeft,
        TK::ScrollRight => MouseEventKind::ScrollRight,
    };
    MouseEvent {
        kind,
        column: m.column,
        row: m.row,
        modifiers: KeyModifiers::NONE,
    }
}

/// Feed one terminal input event to the preview. Handles the terminal's
/// color-scheme report (`CSI ? 997 ; 1|2 n`, sent in answer to
/// `CSI ? 996 n` and on every change while `CSI ? 2031 h` is active), which
/// vaxis delivers as its `color_scheme` event.
fn handle_terminal_event(app: &mut Preview, ev: termina::Event) -> Result<(), Error> {
    use termina::escape::csi::{Csi, Mode, ThemeMode};
    match ev {
        termina::Event::Key(k) => {
            if let Some(k) = convert_key(k) {
                app.update(&Event::Key(k))?;
            }
        }
        termina::Event::Mouse(m) => app.update(&Event::Mouse(convert_mouse(m)))?,
        termina::Event::Csi(Csi::Mode(Mode::ReportTheme(t))) => {
            app.color_scheme = match t {
                ThemeMode::Dark => Scheme::Dark,
                ThemeMode::Light => Scheme::Light,
            };
        }
        _ => {}
    }
    Ok(())
}

/// Run the interactive preview (Zig: `preview`). Blocks until the user quits.
pub fn preview(themes: Vec<ThemeListElement>, theme_filter: ColorScheme) -> Result<(), Error> {
    let mut app = Preview::new(themes, theme_filter)?;
    run_loop(&mut app)
}

fn run_loop(app: &mut Preview) -> Result<(), Error> {
    use ratatui::{Terminal as RatTerminal, backend::CrosstermBackend};
    use termina::{
        PlatformTerminal,
        Terminal as _,
        escape::csi::{Csi, DecPrivateMode, DecPrivateModeCode, Mode},
    };

    let mut tty = PlatformTerminal::new()?;
    tty.enter_raw_mode()?;

    // Alternate screen, title, mouse reporting.
    write!(
        tty,
        "\x1b[?1049h\x1b[?2027h\x1b]2;\u{1f47b} Ghostty Theme Preview \u{1f47b}\x1b\\\
         \x1b[?1000h\x1b[?1002h\x1b[?1003h\x1b[?1006h"
    )?;

    // Request the current color scheme (`CSI ? 996 n`) and subscribe to
    // changes (`CSI ? 2031 h`). vaxis gates this on a DECRQM 2031 answer,
    // but termina cannot parse that report, so (like most TUIs) both are
    // sent unconditionally: terminals without support ignore them.
    let theme_mode = DecPrivateMode::Code(DecPrivateModeCode::Theme);
    write!(
        tty,
        "{}{}",
        Csi::Mode(Mode::QueryTheme),
        Csi::Mode(Mode::SetDecPrivateMode(theme_mode)),
    )?;
    tty.flush()?;

    let mut terminal = RatTerminal::new(CrosstermBackend::new(std::io::stdout()))?;
    let result = (|| -> Result<(), Error> {
        while !app.should_quit {
            let mut cursor = None;
            terminal.draw(|f| cursor = draw_frame(app, f))?;

            // Hyperlinks and extended underlines ratatui can't express.
            if !app.overlays().is_empty() {
                let backend = terminal.backend_mut();
                write!(backend, "{}", render_overlays(app.overlays()))?;
                if let Some(pos) = cursor {
                    write!(backend, "\x1b[{};{}H", pos.y + 1, pos.x + 1)?;
                }
                backend.flush()?;
            }

            // Block for an event, then drain everything that is pending.
            let first = tty.read(|_| true)?;
            handle_terminal_event(app, first)?;
            while tty.poll(|_| true, Some(std::time::Duration::ZERO))? {
                let ev = tty.read(|_| true)?;
                handle_terminal_event(app, ev)?;
            }
            for effect in app.take_effects() {
                match effect {
                    Effect::Copy(text) => {
                        let out = terminal.backend_mut();
                        write!(out, "\x1b]52;c;{}\x1b\\", base64(&text.to_bytes()))?;
                        out.flush()?;
                    }
                }
            }
        }
        Ok(())
    })();

    let _ = terminal.show_cursor();
    let _ = write!(
        tty,
        "{}\x1b[?1006l\x1b[?1003l\x1b[?1002l\x1b[?1000l\x1b[?2027l\x1b[?1049l",
        Csi::Mode(Mode::ResetDecPrivateMode(theme_mode)),
    );
    let _ = tty.flush();
    let _ = tty.enter_cooked_mode();
    result
}

fn draw_frame(app: &mut Preview, f: &mut Frame<'_>) -> Option<Position> {
    let cursor = app.draw(f.buffer_mut());
    if let Some(pos) = cursor {
        f.set_cursor_position(pos);
    }
    cursor
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use ghostty_config::theme::Location;

    use super::*;

    fn make_themes(dir: &std::path::Path, names: &[(&str, &str)]) -> Vec<ThemeListElement> {
        names
            .iter()
            .map(|(name, bg)| {
                let path = dir.join(name);
                std::fs::write(
                    &path,
                    format!("background = {bg}\nforeground = #eeeeee\npalette = 1=#ff0000\n"),
                )
                .unwrap();
                ThemeListElement {
                    location: Location::Resources,
                    path: ghostty_os::osbytes::EncodedOs::from_path(&path),
                    theme: (*name).to_string(),
                    rank: None,
                }
            })
            .collect()
    }

    fn key(code: KeyCode) -> Event {
        Event::Key(KeyEvent::new(code, KeyModifiers::NONE))
    }
    fn ch(c: char) -> Event {
        key(KeyCode::Char(c))
    }
    fn ctrl(c: char) -> Event {
        Event::Key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL))
    }

    fn app(tmp: &tempfile::TempDir) -> Preview {
        let themes = make_themes(
            tmp.path(),
            &[
                ("alpha", "#000000"),
                ("beta", "#ffffff"),
                ("gamma-dark", "#101010"),
                ("delta-light", "#f0f0f0"),
            ],
        );
        let mut p = Preview::new(themes, ColorScheme::All).unwrap();
        p.color_scheme = Scheme::Light;
        p
    }

    fn render(p: &mut Preview, w: u16, h: u16) -> Buffer {
        let mut buf = Buffer::empty(Rect::new(0, 0, w, h));
        p.draw(&mut buf);
        buf
    }

    fn row_text(buf: &Buffer, y: u16) -> String {
        (0..buf.area.width)
            .map(|x| buf[(x, y)].symbol().to_string())
            .collect()
    }

    #[test]
    fn navigation_and_quit() {
        let tmp = tempfile::tempdir().unwrap();
        let mut p = app(&tmp);
        assert_eq!(4, p.filtered.len());
        p.update(&ch('j')).unwrap();
        p.update(&key(KeyCode::Down)).unwrap();
        assert_eq!(2, p.current);
        p.update(&key(KeyCode::PageDown)).unwrap();
        assert_eq!(3, p.current);
        p.update(&ch('k')).unwrap();
        assert_eq!(2, p.current);
        p.update(&key(KeyCode::PageUp)).unwrap();
        assert_eq!(0, p.current);
        p.update(&key(KeyCode::End)).unwrap();
        assert_eq!(3, p.current);
        p.update(&key(KeyCode::Home)).unwrap();
        assert_eq!(0, p.current);
        assert!(!p.should_quit);
        p.update(&ch('q')).unwrap();
        assert!(p.should_quit);

        let mut p = app(&tmp);
        p.update(&ctrl('c')).unwrap();
        assert!(p.should_quit);
        let mut p = app(&tmp);
        p.update(&key(KeyCode::Esc)).unwrap();
        assert!(p.should_quit);
    }

    #[test]
    fn filter_cycles_and_filters() {
        let tmp = tempfile::tempdir().unwrap();
        let mut p = app(&tmp);
        p.update(&ch('f')).unwrap();
        assert_eq!(ColorScheme::Dark, p.theme_filter);
        let names: Vec<_> = p
            .filtered
            .iter()
            .map(|&i| p.themes[i].theme.as_str())
            .collect();
        assert_eq!(vec!["alpha", "gamma-dark"], names);
        p.update(&ch('f')).unwrap();
        assert_eq!(ColorScheme::Light, p.theme_filter);
        assert_eq!(2, p.filtered.len());
        p.update(&ch('f')).unwrap();
        assert_eq!(ColorScheme::All, p.theme_filter);
        assert_eq!(4, p.filtered.len());
    }

    #[test]
    fn search_mode() {
        let tmp = tempfile::tempdir().unwrap();
        let mut p = app(&tmp);
        p.update(&ch('/')).unwrap();
        assert_eq!(Mode::Search, p.mode);
        p.update(&ch('G')).unwrap();
        p.update(&ch('a')).unwrap();
        p.update(&ch('m')).unwrap();
        assert_eq!("Gam", p.text_input.text);
        assert_eq!(1, p.filtered.len());
        assert_eq!("gamma-dark", p.themes[p.filtered[0]].theme);
        p.update(&key(KeyCode::Backspace)).unwrap();
        p.update(&key(KeyCode::Backspace)).unwrap();
        p.update(&key(KeyCode::Backspace)).unwrap();
        assert_eq!(4, p.filtered.len());
        p.update(&ch('l')).unwrap();
        p.update(&ch('t')).unwrap();
        assert_eq!(1, p.filtered.len()); // "delta-light" is the only match for "lt"
        p.update(&ctrl('x')).unwrap();
        assert_eq!("", p.text_input.text);
        assert_eq!(4, p.filtered.len());
        p.update(&key(KeyCode::Enter)).unwrap();
        assert_eq!(Mode::Normal, p.mode);
    }

    #[test]
    fn hex_toggle_and_help_mode() {
        let tmp = tempfile::tempdir().unwrap();
        let mut p = app(&tmp);
        p.update(&ch('x')).unwrap();
        assert!(p.hex);
        p.update(&ch('d')).unwrap();
        assert!(!p.hex);
        p.update(&key(KeyCode::F(1))).unwrap();
        assert_eq!(Mode::Help, p.mode);
        // Keys are modal: 'j' does nothing in help.
        p.update(&ch('j')).unwrap();
        assert_eq!(0, p.current);
        p.update(&key(KeyCode::Esc)).unwrap();
        assert_eq!(Mode::Normal, p.mode);
        assert!(!p.should_quit);
        p.update(&ch('?')).unwrap();
        assert_eq!(Mode::Help, p.mode);
    }

    #[test]
    fn copy_and_save() {
        let tmp = tempfile::tempdir().unwrap();
        let mut p = app(&tmp);
        p.update(&ch('c')).unwrap();
        p.update(&Event::Key(KeyEvent::new(
            KeyCode::Char('C'),
            KeyModifiers::SHIFT,
        )))
        .unwrap();
        let effects = p.take_effects();
        assert_eq!(
            Effect::Copy(ghostty_os::osbytes::EncodedOs::from_encoded("alpha")),
            effects[0]
        );
        assert_eq!(
            Effect::Copy(ghostty_os::osbytes::EncodedOs::from_path(
                &tmp.path().join("alpha")
            )),
            effects[1]
        );

        let auto = tmp.path().join("cfg/auto/theme.ghostty");
        p.auto_theme_path = Some(auto.clone());
        p.update(&key(KeyCode::Enter)).unwrap();
        assert_eq!(Mode::Save, p.mode);
        p.update(&ch('w')).unwrap();
        assert_eq!("theme = alpha\n", std::fs::read_to_string(&auto).unwrap());
        p.update(&key(KeyCode::Esc)).unwrap();
        assert_eq!(Mode::Normal, p.mode);
    }

    #[test]
    fn theme_preview_renders_plain_unicode_names() {
        let tmp = tempfile::tempdir().unwrap();
        let mut p = app(&tmp);
        let name = "Alpha\u{fdd0}é";
        p.themes[0].theme = ghostty_os::osbytes::EncodedOs::from_plain_text(name).into_encoded();
        render(&mut p, 120, 60);
        assert!(p.overlays().iter().any(|overlay| overlay.text == name));
        assert!(
            !p.overlays()
                .iter()
                .any(|overlay| overlay.text.contains("\u{fdd0}\u{fdd0}"))
        );
    }

    #[test]
    fn draw_layout() {
        let tmp = tempfile::tempdir().unwrap();
        let mut p = app(&tmp);
        p.update(&ch('j')).unwrap();
        let buf = render(&mut p, 120, 60);
        // List on the left: selected item has the marker and the arrow at col 30.
        let r1 = row_text(&buf, 1);
        assert!(r1.starts_with("\u{276f} beta"), "{r1:?}");
        assert!(r1.contains("\u{276e}"), "{r1:?}");
        assert!(row_text(&buf, 0).starts_with("  alpha"));
        // Preview: theme name centered in the header, palette numbers, code sample.
        let preview_header = row_text(&buf, 1);
        assert!(preview_header.contains("beta"));
        let all: String = (0..60)
            .map(|y| row_text(&buf, y))
            .collect::<Vec<_>>()
            .join("\n");
        assert!(all.contains("  0"));
        assert!(all.contains("ziggzagg.zig"));
        assert!(all.contains("const std = @import(\"std\");"));
        assert!(all.contains("Lorem ipsum"));
        // The palette swatches use the theme palette color 1.
        let swatch = (32..120)
            .find(|&x| buf[(x, 4)].fg == Color::Rgb(255, 0, 0))
            .is_some();
        assert!(swatch);
        // Background of the preview is the theme background.
        assert_eq!(Color::Rgb(255, 255, 255), buf[(100, 0)].bg);
    }

    #[test]
    fn hyperlinks_and_underline_overlays() {
        let tmp = tempfile::tempdir().unwrap();
        let mut p = app(&tmp);
        let _ = render(&mut p, 120, 60);
        let ov = p.overlays().to_vec();
        // List entries (4) + header name + header path are hyperlinks.
        let links: Vec<&Overlay> = ov.iter().filter(|o| o.link.is_some()).collect();
        assert!(links.len() >= 6, "{links:?}");
        let alpha = links
            .iter()
            .find(|o| o.text == "alpha" && o.x == 2)
            .unwrap();
        // (On Windows the path is `C:\...`; Zig's `toUri` does not rewrite it,
        // the backslashes are percent-encoded.)
        let (prefix, suffix) = if cfg!(windows) {
            ("file://", "%5Calpha")
        } else {
            ("file:///", "/alpha")
        };
        assert!(alpha.link.as_deref().unwrap().starts_with(prefix));
        assert!(alpha.link.as_deref().unwrap().ends_with(suffix));
        // The five lorem ipsum words get their own underline overlays.
        let uls: Vec<(String, Ul)> = ov
            .iter()
            .filter_map(|o| o.ul.map(|u| (o.text.clone(), u)))
            .collect();
        for (word, ul) in [
            ("dui", Ul::Double),
            ("erat", Ul::Dashed),
            ("enim", Ul::Dotted),
            ("odio", Ul::Curly),
        ] {
            assert!(uls.contains(&(word.to_owned(), ul)), "{word}: {uls:?}");
        }
        // And the raw escapes carry OSC 8 and the 4:N styles.
        let raw = render_overlays(&ov);
        assert!(raw.contains(&format!("\x1b]8;;{prefix}")));
        assert!(raw.contains("\x1b]8;;\x1b\\"));
        for sgr in ["4:2", "4:3", "4:4", "4:5"] {
            assert!(raw.contains(sgr), "{sgr}");
        }
    }

    #[test]
    fn popup_drops_covered_overlays() {
        let tmp = tempfile::tempdir().unwrap();
        let mut p = app(&tmp);
        p.update(&ch('?')).unwrap();
        let _ = render(&mut p, 120, 40);
        // The help window covers the middle of the screen: no overlay may
        // intersect it.
        let (w, h) = (60u16, 22u16);
        let (x0, y0) = (60 - w / 2, 20 - h / 2);
        for o in p.overlays() {
            let inside = o.y >= y0 && o.y < y0 + h && o.x < x0 + w && o.x + 1 > x0;
            assert!(!inside, "{o:?}");
        }
    }

    #[test]
    fn terminal_theme_report_sets_scheme() {
        let tmp = tempfile::tempdir().unwrap();
        let mut p = app(&tmp);
        let mut parser = termina::Parser::default();
        parser.parse(b"\x1b[?997;1n", false);
        handle_terminal_event(&mut p, parser.pop().unwrap()).unwrap();
        assert_eq!(Scheme::Dark, p.color_scheme);
        parser.parse(b"\x1b[?997;2n", false);
        handle_terminal_event(&mut p, parser.pop().unwrap()).unwrap();
        assert_eq!(Scheme::Light, p.color_scheme);
    }

    #[test]
    fn termina_key_conversion() {
        use termina::event::{KeyCode as T, Modifiers};
        let k = convert_key(termina::event::KeyEvent::new(
            T::Char('x'),
            Modifiers::CONTROL,
        ))
        .unwrap();
        assert_eq!(KeyCode::Char('x'), k.code);
        assert!(k.modifiers.contains(KeyModifiers::CONTROL));
        assert_eq!(KeyCode::Esc, convert_key(T::Escape.into()).unwrap().code);
        assert!(convert_key(T::CapsLock.into()).is_none());
    }

    #[test]
    fn draw_help_and_save_and_search_overlays() {
        let tmp = tempfile::tempdir().unwrap();
        let mut p = app(&tmp);
        p.update(&ch('?')).unwrap();
        let buf = render(&mut p, 120, 40);
        let all: String = (0..40)
            .map(|y| row_text(&buf, y))
            .collect::<Vec<_>>()
            .join("\n");
        assert!(all.contains("Cycle through theme filters."));
        assert!(all.contains('\u{256d}') && all.contains('\u{256f}'));

        p.update(&key(KeyCode::Esc)).unwrap();
        p.update(&key(KeyCode::Enter)).unwrap();
        let buf = render(&mut p, 120, 40);
        let all: String = (0..40)
            .map(|y| row_text(&buf, y))
            .collect::<Vec<_>>()
            .join("\n");
        assert!(all.contains("theme = alpha"));
        assert!(all.contains("config-file = ?auto/theme.ghostty"));

        p.update(&key(KeyCode::Esc)).unwrap();
        p.update(&ch('/')).unwrap();
        p.update(&ch('b')).unwrap();
        let mut buf = Buffer::empty(Rect::new(0, 0, 120, 40));
        let cursor = p.draw(&mut buf);
        assert!(cursor.is_some());
    }

    #[test]
    fn empty_filter_shows_no_theme_found() {
        let tmp = tempfile::tempdir().unwrap();
        let mut p = app(&tmp);
        p.update(&ch('/')).unwrap();
        p.update(&ch('z')).unwrap();
        p.update(&ch('z')).unwrap();
        assert_eq!(0, p.filtered.len());
        let buf = render(&mut p, 80, 20);
        assert!(row_text(&buf, 9).contains("No theme found!"));
        // Navigation does not panic with an empty list.
        p.update(&key(KeyCode::Enter)).unwrap();
        p.update(&key(KeyCode::Esc)).unwrap();
        p.update(&key(KeyCode::End)).unwrap();
        p.update(&ch('j')).unwrap();
        p.update(&ch('c')).unwrap();
        assert_eq!(0, p.current);
    }

    #[test]
    fn unreadable_theme_shows_error() {
        let tmp = tempfile::tempdir().unwrap();
        let mut p = app(&tmp);
        std::fs::remove_file(tmp.path().join("alpha")).unwrap();
        let buf = render(&mut p, 120, 30);
        let all: String = (0..30)
            .map(|y| row_text(&buf, y))
            .collect::<Vec<_>>()
            .join("\n");
        assert!(all.contains("Unable to open alpha from:"), "{all}");
        assert!(all.contains("FileNotFound"));
    }

    #[test]
    fn mouse_click_selects_and_wheel_scrolls() {
        let tmp = tempfile::tempdir().unwrap();
        let mut p = app(&tmp);
        let mouse = |kind, row| {
            Event::Mouse(MouseEvent {
                kind,
                column: 3,
                row,
                modifiers: KeyModifiers::NONE,
            })
        };
        p.update(&mouse(MouseEventKind::Up(MouseButton::Left), 2))
            .unwrap();
        render(&mut p, 120, 30);
        assert_eq!(2, p.current);
        p.update(&mouse(MouseEventKind::ScrollUp, 0)).unwrap();
        render(&mut p, 120, 30);
        assert_eq!(1, p.current);
        p.update(&mouse(MouseEventKind::ScrollDown, 0)).unwrap();
        render(&mut p, 120, 30);
        assert_eq!(2, p.current);
        // Click below the last theme does nothing.
        p.update(&mouse(MouseEventKind::Up(MouseButton::Left), 20))
            .unwrap();
        render(&mut p, 120, 30);
        assert_eq!(2, p.current);
    }

    #[test]
    fn window_scrolls_with_selection() {
        let tmp = tempfile::tempdir().unwrap();
        let names: Vec<String> = (0..40).map(|i| format!("theme{i:02}")).collect();
        let refs: Vec<(&str, &str)> = names.iter().map(|n| (n.as_str(), "#000000")).collect();
        let themes = make_themes(tmp.path(), &refs);
        let mut p = Preview::new(themes, ColorScheme::All).unwrap();
        render(&mut p, 100, 10);
        p.update(&key(KeyCode::PageDown)).unwrap();
        let buf = render(&mut p, 100, 10);
        assert_eq!(20, p.current);
        assert_eq!(11, p.window);
        assert!(row_text(&buf, 9).starts_with("\u{276f} theme20"));
    }

    #[test]
    fn rank_is_subsequence() {
        assert!(rank("Catppuccin Mocha", &["cat", "moc"]).is_some());
        assert!(rank("Catppuccin Mocha", &["cpm"]).is_some());
        assert!(rank("Catppuccin Mocha", &["xyz"]).is_none());
        assert!(rank("abc", &["cba"]).is_none());
        assert!(rank("abc", &[]).is_some());
    }

    #[test]
    fn text_input_editing() {
        let mut t = TextInput::default();
        for c in "héllo wörld".chars() {
            t.update(&KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE));
        }
        assert_eq!("héllo wörld", t.text);
        t.update(&KeyEvent::new(KeyCode::Left, KeyModifiers::NONE));
        t.update(&KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE));
        assert_eq!("héllo wörd", t.text);
        t.update(&KeyEvent::new(KeyCode::Char('w'), KeyModifiers::CONTROL));
        assert_eq!("héllo d", t.text);
        t.update(&KeyEvent::new(KeyCode::Home, KeyModifiers::NONE));
        t.update(&KeyEvent::new(KeyCode::Delete, KeyModifiers::NONE));
        assert_eq!("éllo d", t.text);
        t.update(&KeyEvent::new(KeyCode::Char('k'), KeyModifiers::CONTROL));
        assert_eq!("", t.text);
    }

    fn typed(t: &mut TextInput, text: &str) {
        for c in text.chars() {
            t.update(&KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE));
        }
    }

    fn press(t: &mut TextInput, code: KeyCode, m: KeyModifiers) -> bool {
        t.update(&KeyEvent::new(code, m))
    }

    #[test]
    fn text_input_word_navigation_matches_vaxis() {
        let alt = KeyModifiers::ALT;
        let mut t = TextInput::default();
        typed(&mut t, "blue red");
        press(&mut t, KeyCode::Char('b'), alt);
        typed(&mut t, "X");
        assert_eq!("blue Xred", t.text);

        // Alt+B skips trailing spaces; Alt+Left is the same.
        let mut t = TextInput::default();
        typed(&mut t, "one two   ");
        press(&mut t, KeyCode::Left, alt);
        assert_eq!(4, t.cursor);
        press(&mut t, KeyCode::Char('b'), alt);
        assert_eq!(0, t.cursor);
        // Alt+F: skips leading spaces then stops before the next space.
        press(&mut t, KeyCode::Char('f'), alt);
        assert_eq!(3, t.cursor);
        press(&mut t, KeyCode::Right, alt);
        assert_eq!(7, t.cursor);
        press(&mut t, KeyCode::Char('f'), alt);
        assert_eq!(10, t.cursor);

        // Alt+D deletes forward to the end of the next word.
        let mut t = TextInput::default();
        typed(&mut t, "one two three");
        press(&mut t, KeyCode::Home, KeyModifiers::NONE);
        press(&mut t, KeyCode::Char('d'), alt);
        assert_eq!(" two three", t.text);
        press(&mut t, KeyCode::Char('d'), alt);
        assert_eq!(" three", t.text);

        // Ctrl+W and Alt+Backspace delete the previous word.
        let mut t = TextInput::default();
        typed(&mut t, "one two three");
        press(&mut t, KeyCode::Char('w'), KeyModifiers::CONTROL);
        assert_eq!("one two ", t.text);
        press(&mut t, KeyCode::Backspace, alt);
        assert_eq!("one ", t.text);
        assert_eq!(4, t.cursor);

        // Ctrl+D / Ctrl+B / Ctrl+F / Ctrl+A / Ctrl+E / Ctrl+U / Ctrl+K.
        let ctrl = KeyModifiers::CONTROL;
        let mut t = TextInput::default();
        typed(&mut t, "abcdef");
        press(&mut t, KeyCode::Char('b'), ctrl);
        press(&mut t, KeyCode::Char('b'), ctrl);
        press(&mut t, KeyCode::Char('d'), ctrl);
        assert_eq!("abcdf", t.text);
        press(&mut t, KeyCode::Char('f'), ctrl);
        assert_eq!(5, t.cursor);
        press(&mut t, KeyCode::Char('a'), ctrl);
        press(&mut t, KeyCode::Char('f'), ctrl);
        press(&mut t, KeyCode::Char('k'), ctrl);
        assert_eq!("a", t.text);
        typed(&mut t, "bc");
        press(&mut t, KeyCode::Char('b'), ctrl);
        press(&mut t, KeyCode::Char('u'), ctrl);
        assert_eq!("c", t.text);
        assert_eq!(0, t.cursor);
        press(&mut t, KeyCode::Char('e'), ctrl);
        assert_eq!(1, t.cursor);

        // Modifiers must match exactly: Ctrl+Left is not a plain Left and
        // Alt+x is not text.
        let mut t = TextInput::default();
        typed(&mut t, "ab");
        assert!(!press(&mut t, KeyCode::Left, ctrl));
        assert!(!press(&mut t, KeyCode::Char('x'), alt));
        assert_eq!("ab", t.text);
        assert_eq!(2, t.cursor);
    }

    #[test]
    fn text_input_is_grapheme_aware() {
        let none = KeyModifiers::NONE;
        // Backspace removes the whole cluster (e + combining acute).
        let mut t = TextInput::default();
        typed(&mut t, "xe\u{301}");
        press(&mut t, KeyCode::Backspace, none);
        assert_eq!("x", t.text);

        // Left/Right/Delete move over a whole ZWJ emoji sequence.
        let astronaut = "\u{1f469}\u{200d}\u{1f680}";
        let mut t = TextInput::default();
        typed(&mut t, &format!("a{astronaut}b"));
        press(&mut t, KeyCode::Left, none);
        press(&mut t, KeyCode::Left, none);
        assert_eq!(1, t.cursor);
        press(&mut t, KeyCode::Delete, none);
        assert_eq!("ab", t.text);

        let mut t = TextInput::default();
        typed(&mut t, &format!("{astronaut}{astronaut}"));
        press(&mut t, KeyCode::Home, none);
        press(&mut t, KeyCode::Right, none);
        assert_eq!(astronaut.len(), t.cursor);
    }

    #[test]
    fn rank_folds_ascii_only() {
        // A non-ASCII capital is not folded on either side.
        assert!(rank("\u{c4}", &["\u{c4}"]).is_some());
        assert!(rank("\u{c4}", &["\u{e4}"]).is_none());
        assert!(rank("\u{e4}", &["\u{c4}"]).is_none());
        // ASCII is folded in the name (the query is lowered by the caller).
        assert!(rank("Catppuccin", &["cat"]).is_some());
    }

    #[test]
    fn search_with_non_ascii_query_matches_like_zig() {
        let tmp = tempfile::tempdir().unwrap();
        let themes = make_themes(
            tmp.path(),
            &[("\u{c4}rger", "#000000"), ("plain", "#000000")],
        );
        let mut p = Preview::new(themes, ColorScheme::All).unwrap();
        p.update(&ch('/')).unwrap();
        p.update(&ch('\u{c4}')).unwrap();
        assert_eq!(1, p.filtered.len());
        p.update(&key(KeyCode::Backspace)).unwrap();
        p.update(&ch('\u{e4}')).unwrap();
        assert_eq!(0, p.filtered.len());
    }

    #[test]
    fn initial_scheme_is_light_regardless_of_colorfgbg() {
        let tmp = tempfile::tempdir().unwrap();
        let themes = make_themes(tmp.path(), &[("a", "#000000")]);
        let p = temp_env::with_var("COLORFGBG", Some("15;0"), || {
            Preview::new(themes, ColorScheme::All).unwrap()
        });
        assert_eq!(Scheme::Light, p.color_scheme);
    }

    #[test]
    fn load_file_error_propagates() {
        let tmp = tempfile::tempdir().unwrap();
        let mut themes = make_themes(tmp.path(), &[("a", "#000000")]);
        // A theme file with an unreadable path: a path below a regular file.
        let bad = tmp.path().join("a").join("nope");
        themes[0].path = ghostty_os::osbytes::EncodedOs::from_path(&bad);
        let result = Preview::new(themes, ColorScheme::All);
        assert!(result.is_err());
    }

    #[test]
    fn base64_encoding() {
        assert_eq!("", base64(b""));
        assert_eq!("Zg==", base64(b"f"));
        assert_eq!("Zm8=", base64(b"fo"));
        assert_eq!("Zm9v", base64(b"foo"));
        assert_eq!("Zm9vYmFy", base64(b"foobar"));
    }
}
