//! The interactive theme preview of `+list-themes` (Zig: `Preview` in
//! list_themes.zig, built on libvaxis). This is a ratatui + crossterm
//! implementation with the same layout, key bindings and behaviour.
//!
//! Known differences from the vaxis version:
//!   * hyperlinks (OSC 8) on theme names/paths are not emitted,
//!   * double/dashed/dotted/curly underlines are drawn as single underlines,
//!   * the terminal color scheme is not queried live: it defaults to light
//!     like the Zig code does, but `COLORFGBG` is used as a hint if set.

use std::cell::RefCell;
use std::io::Write;

use ghostty_config::config::Config;
use ghostty_config::types_a::TerminalColor;
use ratatui::buffer::Buffer;
use ratatui::crossterm::event::{
    self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseButton, MouseEvent,
    MouseEventKind,
};
use ratatui::layout::{Position, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::{Frame, crossterm};

use super::{
    ColorScheme, SMALL_LIST_THRESHOLD, ThemeListElement, absolute_path, should_include_theme,
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

/// A minimal single line text input (`vaxis.widgets.TextInput`).
#[derive(Debug, Clone, Default)]
pub struct TextInput {
    pub text: String,
    /// Cursor position in chars.
    pub cursor: usize,
}

impl TextInput {
    fn byte_idx(&self, chars: usize) -> usize {
        self.text
            .char_indices()
            .nth(chars)
            .map_or(self.text.len(), |(i, _)| i)
    }

    fn len(&self) -> usize {
        self.text.chars().count()
    }

    pub fn clear(&mut self) {
        self.text.clear();
        self.cursor = 0;
    }

    /// Handle an editing key. Returns true if it was consumed.
    pub fn update(&mut self, key: &KeyEvent) -> bool {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        let alt = key.modifiers.contains(KeyModifiers::ALT);
        match key.code {
            KeyCode::Char(c) if !ctrl && !alt => {
                let at = self.byte_idx(self.cursor);
                self.text.insert(at, c);
                self.cursor += 1;
            }
            KeyCode::Backspace => {
                if self.cursor > 0 {
                    let at = self.byte_idx(self.cursor - 1);
                    self.text.remove(at);
                    self.cursor -= 1;
                }
            }
            KeyCode::Delete | KeyCode::Char('d') if key.code == KeyCode::Delete || ctrl => {
                if self.cursor < self.len() {
                    let at = self.byte_idx(self.cursor);
                    self.text.remove(at);
                }
            }
            KeyCode::Left | KeyCode::Char('b') if key.code == KeyCode::Left || ctrl => {
                self.cursor = self.cursor.saturating_sub(1);
            }
            KeyCode::Right | KeyCode::Char('f') if key.code == KeyCode::Right || ctrl => {
                self.cursor = (self.cursor + 1).min(self.len());
            }
            KeyCode::Home => self.cursor = 0,
            KeyCode::Char('a') if ctrl => self.cursor = 0,
            KeyCode::End => self.cursor = self.len(),
            KeyCode::Char('e') if ctrl => self.cursor = self.len(),
            KeyCode::Char('u') if ctrl => {
                let at = self.byte_idx(self.cursor);
                self.text.drain(..at);
                self.cursor = 0;
            }
            KeyCode::Char('k') if ctrl => {
                let at = self.byte_idx(self.cursor);
                self.text.truncate(at);
            }
            KeyCode::Char('w') if ctrl => {
                let upto = self.byte_idx(self.cursor);
                let head = self.text[..upto].trim_end_matches(' ');
                let new_end = head.rfind(' ').map_or(0, |i| i + 1);
                self.text.drain(new_end..upto);
                self.cursor = self.text[..new_end].chars().count();
            }
            _ => return false,
        }
        true
    }
}

/// `zf.rank` with `.plain = true, .to_lower = true`: every token must match
/// the (lowercased) name as a subsequence. Only whether a rank exists is used
/// by the preview, so the score is a simple span length (lower is better).
#[must_use]
pub fn rank(name: &str, tokens: &[&str]) -> Option<f64> {
    let name: Vec<char> = name.to_lowercase().chars().collect();
    let mut total = 0.0;
    for token in tokens {
        let token: Vec<char> = token.chars().collect();
        if token.is_empty() {
            continue;
        }
        // Prefer a contiguous match.
        let contiguous = name
            .windows(token.len())
            .position(|w| w == token.as_slice());
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

/// Cached result of loading the currently previewed theme.
enum Loaded {
    Ok(Box<Config>),
    Err(String),
}

/// A pending side effect requested by the state (needs the terminal).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Effect {
    /// Copy the text to the clipboard (OSC 52).
    Copy(String),
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
    loaded: Option<(String, Loaded)>,
    /// Where `w` writes the auto theme file (overridable for tests).
    pub auto_theme_path: Option<String>,
    effects: Vec<Effect>,
}

fn initial_scheme() -> Scheme {
    // COLORFGBG is "fg;bg" (sometimes "fg;default;bg"); bg 0-6 and 8 are dark.
    if let Ok(v) = std::env::var("COLORFGBG") {
        if let Some(bg) = v.rsplit(';').next().and_then(|s| s.parse::<u8>().ok()) {
            return if bg <= 6 || bg == 8 {
                Scheme::Dark
            } else {
                Scheme::Light
            };
        }
    }
    Scheme::Light
}

impl Preview {
    #[must_use]
    pub fn new(themes: Vec<ThemeListElement>, theme_filter: ColorScheme) -> Preview {
        let mut p = Preview {
            should_quit: false,
            themes,
            filtered: vec![],
            current: 0,
            window: 0,
            hex: false,
            mode: Mode::Normal,
            color_scheme: initial_scheme(),
            text_input: TextInput::default(),
            theme_filter,
            mouse: None,
            list_height: 0,
            loaded: None,
            auto_theme_path: None,
            effects: vec![],
        };
        p.update_filtered();
        p
    }

    /// Recompute the filtered theme list (Zig: `updateFiltered`).
    pub fn update_filtered(&mut self) {
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
            // Errors are ignored here (Zig would abort the whole preview).
            let _ = theme_config.load_file(&absolute_path(&theme.path));
            if !should_include_theme(self.theme_filter, &theme_config) {
                continue;
            }
            if searching {
                theme.rank = rank(&theme.theme, &tokens);
                if theme.rank.is_some() {
                    self.filtered.push(i);
                }
            } else {
                self.filtered.push(i);
                theme.rank = None;
            }
        }

        if old == self.filtered {
            return;
        }

        if self.filtered.is_empty() {
            self.current = 0;
            self.window = 0;
            return;
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
    pub fn update(&mut self, event: &Event) {
        match event {
            Event::Key(key) if key.kind != KeyEventKind::Release => self.update_key(key),
            Event::Mouse(m) => self.mouse = Some(*m),
            _ => {}
        }
    }

    fn update_key(&mut self, key: &KeyEvent) {
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
                    self.update_filtered();
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
                        let name = t.theme.clone();
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
                    self.update_filtered();
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
                    return;
                }
                if ctrl && matches!(code, KeyCode::Char('x' | '/' | '_')) {
                    self.text_input.clear();
                    self.update_filtered();
                    return;
                }
                self.text_input.update(key);
                self.update_filtered();
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
        let win = Win { buf: &cell, area };
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
            theme_list.print(
                &theme.theme,
                match style {
                    St::Normal => self.ui_standard(),
                    St::Highlighted => self.ui_highlighted(),
                    St::Selected => self.ui_selected(),
                },
                row,
                2,
            );
            if style == St::Selected {
                let name_len = gw(&theme.theme);
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
                let col = gw(&self
                    .text_input
                    .text
                    .chars()
                    .take(self.text_input.cursor)
                    .collect::<String>());
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
                    let theme_line = format!("theme = {}", theme.theme);
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
        cursor
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
                let text = format!("Unable to open {} from:", theme.theme);
                child.print(
                    &text,
                    ui_err,
                    middle.saturating_sub(1),
                    (child.area.width / 2).saturating_sub(gw(&text) / 2),
                );
                child.print(
                    &theme.path,
                    ui_err,
                    middle,
                    (child.area.width / 2).saturating_sub(gw(&theme.path) / 2),
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
        // TODO: vaxis supports double/dashed/dotted/curly underlines.
        let standard_underline = standard.add_modifier(Modifier::UNDERLINED);

        let color = |i: usize| {
            let c = config.palette.value[i];
            Color::Rgb(c.r, c.g, c.b)
        };

        // Header.
        {
            let child = win.child(x_off, next_start, width, 4);
            child.fill(standard);
            child.print(
                &theme.theme,
                standard_bold_italic,
                1,
                (child.area.width / 2).saturating_sub(gw(&theme.theme) / 2),
            );
            child.print(
                &theme.path,
                standard,
                2,
                (child.area.width / 2).saturating_sub(gw(&theme.path) / 2),
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
                    &diag.to_string(),
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
            let _ = standard_underline;
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
                let style = match word {
                    "ipsum" => Style::default().fg(color(2)).bg(bg),
                    "consectetur" => standard_bold,
                    "reprehenderit" => standard_italic,
                    "Praesent" => standard_bold_italic,
                    "auctor" | "dui" | "erat" | "enim" | "odio" => {
                        // TODO: distinct underline styles (double, dashed,
                        // dotted, curly) are drawn as single underlines.
                        standard.add_modifier(Modifier::UNDERLINED)
                    }
                    _ => standard,
                };
                child.print(word, style, row, col);
                col = col.saturating_add(word_len).saturating_add(1);
            }
        }
    }
}

/// The Zig error name for display (`{}` of an error).
fn error_name(e: &ghostty_config::config::Error) -> String {
    use ghostty_config::config::Error;
    match e {
        Error::Io(io) => match io.kind() {
            std::io::ErrorKind::NotFound => "FileNotFound".to_string(),
            std::io::ErrorKind::PermissionDenied => "AccessDenied".to_string(),
            _ => io.to_string(),
        },
        e => e.to_string(),
    }
}

/// `resolveAutoThemePath`: `<config dir>/auto/theme.ghostty`.
pub fn resolve_auto_theme_path() -> Result<String, Error> {
    let main_cfg_path = ghostty_config::file_load::preferred_default_file_path()
        .map_err(|e| Error::Other(e.to_string()))?;
    let base_dir = std::path::Path::new(&main_cfg_path)
        .parent()
        .ok_or_else(|| Error::Other("BadPathName".to_string()))?;
    Ok(base_dir
        .join("auto")
        .join("theme.ghostty")
        .to_string_lossy()
        .into_owned())
}

/// `writeAutoThemeFile`.
pub fn write_auto_theme_file(path: Option<&str>, theme_name: &str) -> Result<(), Error> {
    let auto_path = match path {
        Some(p) => p.to_string(),
        None => resolve_auto_theme_path()?,
    };
    if let Some(dir) = std::path::Path::new(&auto_path).parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(&auto_path, format!("theme = {theme_name}\n"))?;
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
        if row >= self.area.height || col >= self.area.width {
            return col;
        }
        let x = self.area.x + col;
        let y = self.area.y + row;
        let max = usize::from(self.area.width - col);
        let (nx, _) = self.buf.borrow_mut().set_stringn(x, y, text, max, style);
        nx - self.area.x
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

/// Run the interactive preview (Zig: `preview`). Blocks until the user quits.
pub fn preview(themes: Vec<ThemeListElement>, theme_filter: ColorScheme) -> Result<(), Error> {
    let mut app = Preview::new(themes, theme_filter);
    let mut terminal = ratatui::try_init()?;
    let result = run_loop(&mut terminal, &mut app);
    ratatui::restore();
    result
}

fn run_loop(terminal: &mut ratatui::DefaultTerminal, app: &mut Preview) -> Result<(), Error> {
    use crossterm::event::{DisableMouseCapture, EnableMouseCapture};
    use crossterm::terminal::SetTitle;

    {
        let out = terminal.backend_mut();
        crossterm::execute!(
            out,
            SetTitle("\u{1f47b} Ghostty Theme Preview \u{1f47b}"),
            EnableMouseCapture
        )?;
    }

    let result = (|| -> Result<(), Error> {
        while !app.should_quit {
            terminal.draw(|f| draw_frame(app, f))?;
            // Block for an event, then drain everything that is pending.
            let first = event::read()?;
            app.update(&first);
            while event::poll(std::time::Duration::ZERO)? {
                let ev = event::read()?;
                app.update(&ev);
            }
            for effect in app.take_effects() {
                match effect {
                    Effect::Copy(text) => {
                        let out = terminal.backend_mut();
                        write!(out, "\x1b]52;c;{}\x1b\\", base64(text.as_bytes()))?;
                        out.flush()?;
                    }
                }
            }
        }
        Ok(())
    })();

    let _ = crossterm::execute!(terminal.backend_mut(), DisableMouseCapture);
    result
}

fn draw_frame(app: &mut Preview, f: &mut Frame<'_>) {
    let cursor = app.draw(f.buffer_mut());
    if let Some(pos) = cursor {
        f.set_cursor_position(pos);
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;
    use ghostty_config::theme::Location;

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
                    path: path.to_string_lossy().into_owned(),
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
        let mut p = Preview::new(themes, ColorScheme::All);
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
        p.update(&ch('j'));
        p.update(&key(KeyCode::Down));
        assert_eq!(2, p.current);
        p.update(&key(KeyCode::PageDown));
        assert_eq!(3, p.current);
        p.update(&ch('k'));
        assert_eq!(2, p.current);
        p.update(&key(KeyCode::PageUp));
        assert_eq!(0, p.current);
        p.update(&key(KeyCode::End));
        assert_eq!(3, p.current);
        p.update(&key(KeyCode::Home));
        assert_eq!(0, p.current);
        assert!(!p.should_quit);
        p.update(&ch('q'));
        assert!(p.should_quit);

        let mut p = app(&tmp);
        p.update(&ctrl('c'));
        assert!(p.should_quit);
        let mut p = app(&tmp);
        p.update(&key(KeyCode::Esc));
        assert!(p.should_quit);
    }

    #[test]
    fn filter_cycles_and_filters() {
        let tmp = tempfile::tempdir().unwrap();
        let mut p = app(&tmp);
        p.update(&ch('f'));
        assert_eq!(ColorScheme::Dark, p.theme_filter);
        let names: Vec<_> = p
            .filtered
            .iter()
            .map(|&i| p.themes[i].theme.as_str())
            .collect();
        assert_eq!(vec!["alpha", "gamma-dark"], names);
        p.update(&ch('f'));
        assert_eq!(ColorScheme::Light, p.theme_filter);
        assert_eq!(2, p.filtered.len());
        p.update(&ch('f'));
        assert_eq!(ColorScheme::All, p.theme_filter);
        assert_eq!(4, p.filtered.len());
    }

    #[test]
    fn search_mode() {
        let tmp = tempfile::tempdir().unwrap();
        let mut p = app(&tmp);
        p.update(&ch('/'));
        assert_eq!(Mode::Search, p.mode);
        p.update(&ch('G'));
        p.update(&ch('a'));
        p.update(&ch('m'));
        assert_eq!("Gam", p.text_input.text);
        assert_eq!(1, p.filtered.len());
        assert_eq!("gamma-dark", p.themes[p.filtered[0]].theme);
        p.update(&key(KeyCode::Backspace));
        p.update(&key(KeyCode::Backspace));
        p.update(&key(KeyCode::Backspace));
        assert_eq!(4, p.filtered.len());
        p.update(&ch('l'));
        p.update(&ch('t'));
        assert_eq!(1, p.filtered.len()); // "delta-light" is the only match for "lt"
        p.update(&ctrl('x'));
        assert_eq!("", p.text_input.text);
        assert_eq!(4, p.filtered.len());
        p.update(&key(KeyCode::Enter));
        assert_eq!(Mode::Normal, p.mode);
    }

    #[test]
    fn hex_toggle_and_help_mode() {
        let tmp = tempfile::tempdir().unwrap();
        let mut p = app(&tmp);
        p.update(&ch('x'));
        assert!(p.hex);
        p.update(&ch('d'));
        assert!(!p.hex);
        p.update(&key(KeyCode::F(1)));
        assert_eq!(Mode::Help, p.mode);
        // Keys are modal: 'j' does nothing in help.
        p.update(&ch('j'));
        assert_eq!(0, p.current);
        p.update(&key(KeyCode::Esc));
        assert_eq!(Mode::Normal, p.mode);
        assert!(!p.should_quit);
        p.update(&ch('?'));
        assert_eq!(Mode::Help, p.mode);
    }

    #[test]
    fn copy_and_save() {
        let tmp = tempfile::tempdir().unwrap();
        let mut p = app(&tmp);
        p.update(&ch('c'));
        p.update(&Event::Key(KeyEvent::new(
            KeyCode::Char('C'),
            KeyModifiers::SHIFT,
        )));
        let effects = p.take_effects();
        assert_eq!(Effect::Copy("alpha".into()), effects[0]);
        assert_eq!(
            Effect::Copy(tmp.path().join("alpha").to_string_lossy().into_owned()),
            effects[1]
        );

        let auto = tmp.path().join("cfg/auto/theme.ghostty");
        p.auto_theme_path = Some(auto.to_string_lossy().into_owned());
        p.update(&key(KeyCode::Enter));
        assert_eq!(Mode::Save, p.mode);
        p.update(&ch('w'));
        assert_eq!("theme = alpha\n", std::fs::read_to_string(&auto).unwrap());
        p.update(&key(KeyCode::Esc));
        assert_eq!(Mode::Normal, p.mode);
    }

    #[test]
    fn draw_layout() {
        let tmp = tempfile::tempdir().unwrap();
        let mut p = app(&tmp);
        p.update(&ch('j'));
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
    fn draw_help_and_save_and_search_overlays() {
        let tmp = tempfile::tempdir().unwrap();
        let mut p = app(&tmp);
        p.update(&ch('?'));
        let buf = render(&mut p, 120, 40);
        let all: String = (0..40)
            .map(|y| row_text(&buf, y))
            .collect::<Vec<_>>()
            .join("\n");
        assert!(all.contains("Cycle through theme filters."));
        assert!(all.contains('\u{256d}') && all.contains('\u{256f}'));

        p.update(&key(KeyCode::Esc));
        p.update(&key(KeyCode::Enter));
        let buf = render(&mut p, 120, 40);
        let all: String = (0..40)
            .map(|y| row_text(&buf, y))
            .collect::<Vec<_>>()
            .join("\n");
        assert!(all.contains("theme = alpha"));
        assert!(all.contains("config-file = ?auto/theme.ghostty"));

        p.update(&key(KeyCode::Esc));
        p.update(&ch('/'));
        p.update(&ch('b'));
        let mut buf = Buffer::empty(Rect::new(0, 0, 120, 40));
        let cursor = p.draw(&mut buf);
        assert!(cursor.is_some());
    }

    #[test]
    fn empty_filter_shows_no_theme_found() {
        let tmp = tempfile::tempdir().unwrap();
        let mut p = app(&tmp);
        p.update(&ch('/'));
        p.update(&ch('z'));
        p.update(&ch('z'));
        assert_eq!(0, p.filtered.len());
        let buf = render(&mut p, 80, 20);
        assert!(row_text(&buf, 9).contains("No theme found!"));
        // Navigation does not panic with an empty list.
        p.update(&key(KeyCode::Enter));
        p.update(&key(KeyCode::Esc));
        p.update(&key(KeyCode::End));
        p.update(&ch('j'));
        p.update(&ch('c'));
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
        p.update(&mouse(MouseEventKind::Up(MouseButton::Left), 2));
        render(&mut p, 120, 30);
        assert_eq!(2, p.current);
        p.update(&mouse(MouseEventKind::ScrollUp, 0));
        render(&mut p, 120, 30);
        assert_eq!(1, p.current);
        p.update(&mouse(MouseEventKind::ScrollDown, 0));
        render(&mut p, 120, 30);
        assert_eq!(2, p.current);
        // Click below the last theme does nothing.
        p.update(&mouse(MouseEventKind::Up(MouseButton::Left), 20));
        render(&mut p, 120, 30);
        assert_eq!(2, p.current);
    }

    #[test]
    fn window_scrolls_with_selection() {
        let tmp = tempfile::tempdir().unwrap();
        let names: Vec<String> = (0..40).map(|i| format!("theme{i:02}")).collect();
        let refs: Vec<(&str, &str)> = names.iter().map(|n| (n.as_str(), "#000000")).collect();
        let themes = make_themes(tmp.path(), &refs);
        let mut p = Preview::new(themes, ColorScheme::All);
        render(&mut p, 100, 10);
        p.update(&key(KeyCode::PageDown));
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

    #[test]
    fn base64_encoding() {
        assert_eq!("", base64(b""));
        assert_eq!("Zg==", base64(b"f"));
        assert_eq!("Zm8=", base64(b"fo"));
        assert_eq!("Zm9v", base64(b"foo"));
        assert_eq!("Zm9vYmFy", base64(b"foobar"));
    }
}
