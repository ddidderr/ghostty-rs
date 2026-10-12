//! Differential-testing support: the canonical terminal dump (Rust twin of
//! `tools/difftest/zig/src/main.zig`) and the deterministic input generator.
//!
//! The dump format must stay byte-identical to the Zig driver's output.

pub mod inputgen;

use std::fmt::Write as _;

use ghostty_terminal::{
    hyperlink::HyperlinkId,
    point::{Coordinate, Point},
    screen::Screen,
    screen_set::Key,
    style::{Color, Style},
    terminal::{Options, Terminal},
};
use ghostty_vt::modes::ENTRIES;

fn color(w: &mut String, c: Color) {
    match c {
        Color::None => w.push('n'),
        Color::Palette(p) => {
            let _ = write!(w, "p{p}");
        }
        Color::Rgb(r) => {
            let _ = write!(w, "r{:02x}{:02x}{:02x}", r.r, r.g, r.b);
        }
    }
}

fn style_str(w: &mut String, s: &Style) {
    color(w, s.fg_color);
    w.push(',');
    color(w, s.bg_color);
    w.push(',');
    color(w, s.underline_color);
    let f = &s.flags;
    let bits = u16::from(f.bold)
        | (u16::from(f.italic) << 1)
        | (u16::from(f.faint) << 2)
        | (u16::from(f.blink) << 3)
        | (u16::from(f.inverse) << 4)
        | (u16::from(f.invisible) << 5)
        | (u16::from(f.strikethrough) << 6)
        | (u16::from(f.overline) << 7)
        | ((f.underline as u16) << 8);
    let _ = write!(w, ",{:x}", bits & 0x7ff);
}

fn charset_str(w: &mut String, c: ghostty_terminal::screen::CharsetState) {
    let _ = write!(
        w,
        "gl={} gr={} ss={} g={},{},{},{}",
        c.gl as u8,
        c.gr as u8,
        c.single_shift.map_or(-1, |s| i32::from(s as u8)),
        c.charsets.g0 as u8,
        c.charsets.g1 as u8,
        c.charsets.g2 as u8,
        c.charsets.g3 as u8,
    );
}

fn dump_screen(w: &mut String, name: &str, s: &Screen) {
    let _ = writeln!(w, "[screen {name}]");
    let c = &s.cursor;
    let _ = write!(
        w,
        "cursor {} {} wrap={} prot={} cstyle={} style=",
        c.x,
        c.y,
        u8::from(c.pending_wrap),
        u8::from(c.protected),
        c.cursor_style as u8
    );
    style_str(w, &c.style);
    let _ = writeln!(w, " hl={}", u8::from(c.hyperlink_id != 0));
    if let Some(sc) = &s.saved_cursor {
        let _ = write!(
            w,
            "saved {} {} wrap={} prot={} origin={} style=",
            sc.x,
            sc.y,
            u8::from(sc.pending_wrap),
            u8::from(sc.protected),
            u8::from(sc.origin)
        );
        style_str(w, &sc.style);
        w.push(' ');
        charset_str(w, sc.charset);
        w.push('\n');
    } else {
        w.push_str("saved none\n");
    }
    w.push_str("charset ");
    charset_str(w, s.charset);
    w.push('\n');
    let _ = writeln!(w, "protected_mode {}", s.protected_mode as u8);
    let _ = writeln!(w, "kitty_kb {}", s.kitty_keyboard.current().int());
    let pages = &s.pages;
    let _ = writeln!(w, "rows {}", pages.total_rows);
    for y in 0..pages.total_rows {
        let pt = Point::Screen(Coordinate { x: 0, y: y as u32 });
        let pin = pages.pin(pt).expect("pin");
        let (row, _) = pin.row_and_cell(pages).expect("row");
        let _ = write!(
            w,
            "row {y} w={} wc={} sp={} kv={} |",
            u8::from(row.wrap()),
            u8::from(row.wrap_continuation()),
            row.semantic_prompt() as u8,
            u8::from(row.kitty_virtual_placeholder()),
        );
        for x in 0..pages.cols {
            let cpin = ghostty_terminal::page_list::Pin { x, ..pin };
            let (_, cl) = cpin.row_and_cell(pages).expect("cell");
            let st = cpin.style(pages).expect("style");
            let tag = cl.content_tag() as u8;
            let plain = tag == 0
                && cl.codepoint() == 0
                && cl.wide() as u8 == 0
                && !cl.protected()
                && !cl.hyperlink()
                && cl.semantic_content() as u8 == 0
                && st == Style::default();
            if plain {
                continue;
            }
            let _ = write!(w, " {x}:{tag}:");
            match tag {
                0 | 1 => {
                    let _ = write!(w, "{:x}", cl.codepoint());
                }
                2 => {
                    let _ = write!(w, "{}", cl.color_palette());
                }
                _ => {
                    let r = cl.color_rgb();
                    let _ = write!(w, "{:02x}{:02x}{:02x}", r.r, r.g, r.b);
                }
            }
            if tag == 1 {
                match cpin.grapheme(pages) {
                    Some(g) => {
                        for cp in g {
                            let _ = write!(w, "+{cp:x}");
                        }
                    }
                    None => w.push_str("+?"),
                }
            }
            let _ = write!(
                w,
                ":{}:{}{}{}:",
                cl.wide() as u8,
                if cl.protected() { "p" } else { "" },
                if cl.hyperlink() { "h" } else { "" },
                cl.semantic_content() as u8,
            );
            style_str(w, &st);
            if cl.hyperlink() {
                let page = &pages.node(cpin.node).expect("node").data;
                let idx = page
                    .cell_index(usize::from(cpin.x), usize::from(cpin.y))
                    .expect("idx");
                match page
                    .lookup_hyperlink(idx)
                    .and_then(|id| page.hyperlink_set.get(id))
                {
                    Some(h) => {
                        let _ = write!(w, " @{}", String::from_utf8_lossy(&h.uri));
                        match &h.id {
                            HyperlinkId::Explicit(e) => {
                                let _ = write!(w, "@E:{}", String::from_utf8_lossy(e));
                            }
                            HyperlinkId::Implicit(n) => {
                                let _ = write!(w, "@I:{n}");
                            }
                        }
                    }
                    None => w.push_str(" @?"),
                }
            }
        }
        w.push('\n');
    }
}

/// Feed `input` to a fresh terminal through the read-only stream and return
/// the canonical dump.
pub fn dump(input: &[u8], cols: u16, rows: u16, max_scrollback: usize) -> String {
    let mut opts = Options::new(cols, rows);
    opts.max_scrollback = max_scrollback;
    let mut t = Terminal::new(opts).expect("terminal");
    {
        let mut s = t.vt_stream();
        s.next_slice(input).expect("stream");
    }
    dump_terminal(&t)
}

/// Feed `input` and ignore stream errors (they are reported to the caller as
/// `Err`, not a crash). Used by the Rust-only panic hunt.
pub fn drive_lenient(input: &[u8], cols: u16, rows: u16, max_scrollback: usize) -> String {
    let mut opts = Options::new(cols, rows);
    opts.max_scrollback = max_scrollback;
    let mut t = Terminal::new(opts).expect("terminal");
    let _ = t.vt_stream().next_slice(input);
    dump_terminal(&t)
}

/// Canonical dump of an already-driven terminal.
pub fn dump_terminal(t: &Terminal) -> String {
    let mut w = String::new();
    let _ = writeln!(w, "size {} {}", t.cols, t.rows);
    let _ = writeln!(
        w,
        "active {}",
        match t.screens.active_key {
            Key::Primary => "primary",
            Key::Alternate => "alternate",
        }
    );
    let sr = &t.scrolling_region;
    let _ = writeln!(
        w,
        "region t={} b={} l={} r={}",
        sr.top, sr.bottom, sr.left, sr.right
    );
    w.push_str("modes");
    for e in ENTRIES {
        if t.modes.get(e.mode) {
            let _ = write!(w, " {}", e.name);
        }
    }
    w.push_str("\nsaved_modes");
    for e in ENTRIES {
        if t.modes.saved.get(e.mode) {
            let _ = write!(w, " {}", e.name);
        }
    }
    w.push_str("\ntabstops");
    for x in 0..usize::from(t.cols) {
        if t.tabstops.get(x) {
            let _ = write!(w, " {x}");
        }
    }
    let f = &t.flags;
    let _ = writeln!(
        w,
        "\nflags srp={} mok2={} mev={} mfmt={} msc={}",
        f.shell_redraws_prompt as u8,
        u8::from(f.modify_other_keys_2),
        f.mouse_event as u8,
        f.mouse_format as u8,
        f.mouse_shift_capture as u8
    );
    let _ = writeln!(w, "status_display {}", t.status_display as u8);
    let _ = writeln!(w, "mouse_shape {}", t.mouse_shape as u8);
    let _ = writeln!(w, "pwd {}", String::from_utf8_lossy(&t.pwd));
    match t.previous_char {
        Some(pc) => {
            let _ = writeln!(w, "prev {pc:x}");
        }
        None => w.push_str("prev none\n"),
    }
    for (n, d) in [
        ("foreground", &t.colors.foreground),
        ("background", &t.colors.background),
        ("cursor", &t.colors.cursor),
    ] {
        let _ = write!(w, "color {n}");
        for (tag, v) in [("o", d.override_), ("d", d.default)] {
            match v {
                Some(o) => {
                    let _ = write!(w, " {tag}={:02x}{:02x}{:02x}", o.r, o.g, o.b);
                }
                None => {
                    let _ = write!(w, " {tag}=-");
                }
            }
        }
        w.push('\n');
    }
    w.push_str("palette");
    for (i, c) in t.colors.palette.current.iter().enumerate() {
        let o = &t.colors.palette.original[i];
        if (c.r, c.g, c.b) != (o.r, o.g, o.b) {
            let _ = write!(w, " {i}={:02x}{:02x}{:02x}", c.r, c.g, c.b);
        }
    }
    w.push('\n');
    if let Some(s) = t.screens.get(Key::Primary) {
        dump_screen(&mut w, "primary", s);
    }
    if let Some(s) = t.screens.get(Key::Alternate) {
        dump_screen(&mut w, "alternate", s);
    }
    w.push_str("[plain]\n");
    w.push_str(&t.plain_string().expect("plain"));
    w.push_str("\n[end]\n");
    w
}
