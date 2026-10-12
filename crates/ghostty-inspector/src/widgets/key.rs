//! Keyboard event inspector (ZIG/src/inspector/widgets/key.zig).

use std::collections::VecDeque;

use dear_imgui_rs::{InputTextFlags, TableColumnFlags, TableColumnWidth, TableFlags, Ui};
use ghostty_input::{Action, Key, KeyEvent, Mods, binding::Action as BindingAction};

/// The maximum number of recorded events (Zig: the ring grows by doubling up
/// to this capacity, after which the oldest event is dropped).
pub const MAX_EVENTS: usize = 50;

/// The recorded copy of the input event (Zig keeps an `input.KeyEvent` whose
/// `utf8` is owned by the inspector).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InputEvent {
    pub action: Action,
    pub key: Key,
    pub mods: Mods,
    pub consumed_mods: Mods,
    pub composing: bool,
    pub utf8: String,
    pub unshifted_codepoint: u32,
}

impl InputEvent {
    pub fn from_key_event(ev: &KeyEvent<'_>) -> Self {
        Self {
            action: ev.action,
            key: ev.key,
            mods: ev.mods,
            consumed_mods: ev.consumed_mods,
            composing: ev.composing,
            utf8: ev.utf8.to_owned(),
            unshifted_codepoint: ev.unshifted_codepoint,
        }
    }
}

/// Represents a recorded keyboard event.
#[derive(Debug, Clone)]
pub struct Event {
    /// The input event.
    pub event: InputEvent,

    /// The binding that was triggered as a result of this event.
    /// Multiple bindings are possible if they are chained.
    pub binding: Vec<BindingAction>,

    /// The data sent to the pty as a result of this keyboard event.
    pub pty: Vec<u8>,

    /// State for the inspector GUI. Do not set this unless you're the inspector.
    pub imgui_selected: bool,

    /// Unique ID used for the ImGui ID stack (Zig used the event pointer).
    id: u64,
}

fn next_event_id() -> u64 {
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(1);
    NEXT.fetch_add(1, Ordering::Relaxed)
}

fn action_name(a: Action) -> &'static str {
    match a {
        Action::Press => "press",
        Action::Release => "release",
        Action::Repeat => "repeat",
    }
}

/// The name of the key, or its UTF-8 for unidentified keys.
fn key_display(ev: &InputEvent) -> String {
    match ev.key {
        Key::Unidentified if !ev.utf8.is_empty() => ev.utf8.clone(),
        k => k.name().to_owned(),
    }
}

/// Formats pty data like the inspector shows it.
fn format_pty(data: &[u8]) -> String {
    use std::fmt::Write;
    let mut out = String::new();
    for &byte in data {
        // Print ESC special because its so common
        if byte == 0x1B {
            out.push_str("ESC ");
            continue;
        }

        // Print ASCII as-is
        if byte > 0x20 && byte < 0x7F {
            out.push(char::from(byte));
            continue;
        }

        // Everything else as a hex byte
        let _ = write!(out, "0x{byte:X} ");
    }
    out
}

/// Formats a UTF-8 string as a sequence of codepoints.
fn format_utf8(utf8: &str) -> String {
    use std::fmt::Write;
    let mut out = String::new();
    for ch in utf8.chars() {
        let _ = write!(out, "U+{:X} ", u32::from(ch));
    }
    out
}

impl Event {
    pub fn new(ev: &KeyEvent<'_>) -> Self {
        Self {
            event: InputEvent::from_key_event(ev),
            binding: Vec::new(),
            pty: Vec::new(),
            imgui_selected: false,
            id: next_event_id(),
        }
    }

    /// Returns a label that can be used for this event.
    pub fn label(&self) -> String {
        let mut out = String::new();
        match self.event.action {
            Action::Press => out.push_str("Press: "),
            Action::Release => out.push_str("Release: "),
            Action::Repeat => out.push_str("Repeat: "),
        }

        if self.event.mods.shift {
            out.push_str("Shift+");
        }
        if self.event.mods.ctrl {
            out.push_str("Ctrl+");
        }
        if self.event.mods.alt {
            out.push_str("Alt+");
        }
        if self.event.mods.super_ {
            out.push_str("Super+");
        }

        // Write our key. If we have an invalid key we attempt to write
        // the utf8 associated with it if we have it to handle non-ascii.
        out.push_str(&key_display(&self.event));

        // Deadkey
        if self.event.composing {
            out.push_str(" (composing)");
        }

        out
    }

    /// Render this event in the inspector GUI.
    pub fn render(&self, ui: &Ui) {
        let Some(_t) = ui.begin_table_with_flags("##event", 2, TableFlags::NONE) else {
            return;
        };

        if !self.binding.is_empty() {
            ui.table_next_row();
            ui.table_set_column_index(0);
            ui.text("Triggered Binding");
            ui.table_set_column_index(1);

            let height = {
                let item_count = self.binding.len().min(5) as f32;
                let padding = ui.clone_style().frame_padding()[1] * 2.0;
                ui.text_line_height_with_spacing() * item_count + padding
            };
            if let Some(_list) = ui
                .list_box_config("##bindings")
                .size([0.0, height])
                .begin(ui)
            {
                for action in &self.binding {
                    ui.selectable(action.key().name());
                }
            }
        }

        {
            ui.table_next_row();
            ui.table_set_column_index(0);
            ui.text("Encoding to Pty");
            ui.table_set_column_index(1);
            if self.pty.is_empty() {
                ui.text_disabled("(no data)");
            } else {
                let mut buf = format_pty(&self.pty);
                ui.input_text("##pty", &mut buf)
                    .flags(InputTextFlags::READ_ONLY)
                    .build();
            }
        }

        {
            ui.table_next_row();
            ui.table_set_column_index(0);
            ui.text("Action");
            ui.table_set_column_index(1);
            ui.text(action_name(self.event.action));
        }
        {
            ui.table_next_row();
            ui.table_set_column_index(0);
            ui.text("Key");
            ui.table_set_column_index(1);
            ui.text(self.event.key.name());
        }
        if !self.event.mods.empty() {
            ui.table_next_row();
            ui.table_set_column_index(0);
            ui.text("Mods");
            ui.table_set_column_index(1);
            if self.event.mods.shift {
                ui.text("shift ");
            }
            if self.event.mods.ctrl {
                ui.text("ctrl ");
            }
            if self.event.mods.alt {
                ui.text("alt ");
            }
            if self.event.mods.super_ {
                ui.text("super ");
            }
        }
        if self.event.composing {
            ui.table_next_row();
            ui.table_set_column_index(0);
            ui.text("Composing");
            ui.table_set_column_index(1);
            ui.text("true");
        }
        {
            ui.table_next_row();
            ui.table_set_column_index(0);
            ui.text("UTF-8");
            ui.table_set_column_index(1);
            if self.event.utf8.is_empty() {
                ui.text_disabled("(empty)");
            } else {
                let mut buf = format_utf8(&self.event.utf8);
                ui.input_text("##utf8", &mut buf)
                    .flags(InputTextFlags::READ_ONLY)
                    .build();
            }
        }
    }
}

fn mods_tooltip(mods: &Mods) -> String {
    let mut parts = Vec::new();
    if mods.shift {
        parts.push("Shift");
    }
    if mods.ctrl {
        parts.push("Ctrl");
    }
    if mods.alt {
        parts.push("Alt");
    }
    if mods.super_ {
        parts.push("Super");
    }
    parts.join("+")
}

/// Keyboard event stream inspector widget.
#[derive(Debug, Default)]
pub struct Stream {
    pub events: VecDeque<Event>,
}

impl Stream {
    pub fn new() -> Self {
        Self {
            events: VecDeque::with_capacity(MAX_EVENTS),
        }
    }

    /// Append an event, dropping the oldest one when full.
    pub fn record(&mut self, ev: Event) {
        if self.events.len() >= MAX_EVENTS {
            self.events.pop_front();
        }
        self.events.push_back(ev);
    }

    pub fn draw(&mut self, ui: &Ui, open: bool) {
        if !open {
            return;
        }

        if self.events.is_empty() {
            ui.text("No recorded key events. Press a key with the terminal focused to record it.");
            return;
        }

        if ui.button("Clear") {
            self.events.clear();
        }

        ui.separator();

        let table_flags = TableFlags::BORDERS | TableFlags::RESIZABLE | TableFlags::SCROLL_Y;

        let Some(_t) = ui.begin_table_with_flags(
            "table_key_events",
            6,
            dear_imgui_rs::TableOptions::default()
                .flags(table_flags)
                .sizing_policy(dear_imgui_rs::TableSizingPolicy::FixedFit),
        ) else {
            return;
        };

        ui.table_setup_scroll_freeze(0, 1);
        let fixed = |name: &str, w: f32| {
            ui.table_setup_column(
                name,
                TableColumnFlags::NONE,
                Some(TableColumnWidth::Fixed(w)),
            );
        };
        fixed("Action", 80.0);
        fixed("Key", 160.0);
        fixed("Mods", 150.0);
        fixed("UTF-8", 80.0);
        ui.table_setup_column(
            "PTY Encoding",
            TableColumnFlags::NONE,
            Some(TableColumnWidth::Stretch(0.0)),
        );
        ui.table_setup_column(
            "Binding",
            TableColumnFlags::NONE,
            Some(TableColumnWidth::Stretch(0.0)),
        );
        ui.table_headers_row();

        for ev in self.events.iter().rev() {
            let _id = ui.push_id(ev.id as i32);

            ui.table_next_row();
            let row_min_y = ui.cursor_screen_pos()[1];

            // Set row background color based on action
            ui.table_set_row_bg0_color_u32(action_color(ev.event.action));

            // Action column with colored text
            ui.table_set_column_index(0);
            let action_text_color = match ev.event.action {
                Action::Press => [0.4, 1.0, 0.4, 1.0],   // Green
                Action::Release => [0.6, 0.6, 1.0, 1.0], // Blue
                Action::Repeat => [1.0, 1.0, 0.4, 1.0],  // Yellow
            };
            ui.text_colored(action_text_color, action_name(ev.event.action));

            // Key column with consistent key coloring
            ui.table_set_column_index(1);
            let key_name = key_display(&ev.event);
            let key_rgba = key_color(ev.event.key);
            let key_color = [
                (key_rgba & 0xFF) as f32 / 255.0,
                ((key_rgba >> 8) & 0xFF) as f32 / 255.0,
                ((key_rgba >> 16) & 0xFF) as f32 / 255.0,
                1.0,
            ];
            ui.text_colored(key_color, key_name);

            // Composing indicator
            if ev.event.composing {
                ui.same_line();
                ui.text_colored([1.0, 0.6, 0.0, 1.0], "*");
                if ui.is_item_hovered() {
                    ui.set_tooltip("Composing (dead key)");
                }
            }

            // Mods
            ui.table_set_column_index(2);
            if ev.event.mods.empty() {
                ui.text_disabled("-");
            } else {
                let mut any_hovered = false;
                for (set, label) in [
                    (ev.event.mods.shift, "S"),
                    (ev.event.mods.ctrl, "C"),
                    (ev.event.mods.alt, "A"),
                    (ev.event.mods.super_, "M"),
                ] {
                    if set {
                        ui.small_button(label);
                        any_hovered = any_hovered || ui.is_item_hovered();
                        ui.same_line();
                    }
                }
                ui.new_line();

                if any_hovered {
                    ui.set_tooltip(mods_tooltip(&ev.event.mods));
                }
            }

            // UTF-8
            ui.table_set_column_index(3);
            if ev.event.utf8.is_empty() {
                ui.text_disabled("-");
            } else {
                ui.text(format_utf8(&ev.event.utf8));
            }

            // PTY
            ui.table_set_column_index(4);
            if ev.pty.is_empty() {
                ui.text_disabled("-");
            } else {
                ui.text(format_pty(&ev.pty));
            }

            // Binding
            ui.table_set_column_index(5);
            if ev.binding.is_empty() {
                ui.text_disabled("-");
            } else {
                let names: Vec<&str> = ev.binding.iter().map(|a| a.key().name()).collect();
                ui.text(names.join(", "));
            }

            // Row hover highlight
            let row_max_y = ui.cursor_screen_pos()[1];
            let mouse_pos = ui.mouse_pos();
            if mouse_pos[1] >= row_min_y && mouse_pos[1] < row_max_y {
                ui.table_set_row_bg1_color_u32(0x1AFFFFFF);
            }
        }
    }
}

/// Returns row background color for an action (ABGR format for ImGui)
fn action_color(action: Action) -> u32 {
    match action {
        Action::Press => 0x1A4A6F4A,   // Muted sage green
        Action::Release => 0x1A6A5A5A, // Muted slate gray
        Action::Repeat => 0x1A4A5A6F,  // Muted warm brown
    }
}

/// Generate a consistent color for a key based on its enum value.
/// Uses HSV color space with fixed saturation and value for pleasing colors.
fn key_color(key: Key) -> u32 {
    let key_int = key as u32;
    let hue = key_int.wrapping_mul(47) as f32 / 256.0;
    hsv_to_rgba(hue, 0.5, 0.9, 1.0)
}

/// Convert HSV (hue 0-1, saturation 0-1, value 0-1) to RGBA u32.
fn hsv_to_rgba(h: f32, s: f32, v: f32, a: f32) -> u32 {
    let i = (h * 6.0) as u32;
    let f = h * 6.0 - i as f32;
    let p = v * (1.0 - s);
    let q = v * (1.0 - f * s);
    let t = v * (1.0 - (1.0 - f) * s);

    let (r, g, b) = match i % 6 {
        0 => (v, t, p),
        1 => (q, v, p),
        2 => (p, v, t),
        3 => (p, q, v),
        4 => (t, p, v),
        _ => (v, p, q),
    };

    let ri = (r * 255.0) as u32;
    let gi = (g * 255.0) as u32;
    let bi = (b * 255.0) as u32;
    let ai = (a * 255.0) as u32;

    (ai << 24) | (bi << 16) | (gi << 8) | ri
}

#[cfg(test)]
mod tests {
    use super::*;

    fn press(key: Key, utf8: &str) -> Event {
        Event::new(&KeyEvent {
            action: Action::Press,
            key,
            utf8,
            ..KeyEvent::default()
        })
    }

    #[test]
    fn event_label() {
        let mut ev = press(Key::KeyA, "a");
        ev.event.mods.ctrl = true;
        ev.event.mods.shift = true;
        assert_eq!(ev.label(), "Press: Shift+Ctrl+key_a");

        let ev = press(Key::Unidentified, "é");
        assert_eq!(ev.label(), "Press: é");
    }

    #[test]
    fn pty_formatting() {
        assert_eq!(format_pty(b"\x1b[A"), "ESC [A");
        assert_eq!(format_pty(&[0x0d, b'a', 0x20]), "0xD a0x20 ");
    }

    #[test]
    fn utf8_formatting() {
        assert_eq!(format_utf8("aé"), "U+61 U+E9 ");
    }

    #[test]
    fn ring_drops_oldest() {
        let mut s = Stream::new();
        for _ in 0..(MAX_EVENTS + 5) {
            s.record(press(Key::KeyA, "a"));
        }
        assert_eq!(s.events.len(), MAX_EVENTS);
    }

    #[test]
    fn hsv_matches_zig() {
        // pure red, value 1
        assert_eq!(hsv_to_rgba(0.0, 1.0, 1.0, 1.0), 0xFF00_00FF);
    }
}
