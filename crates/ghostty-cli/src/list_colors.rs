//! `+list-colors` (ZIG/src/cli/list_colors.zig).

use std::{
    cmp::Ordering,
    io::{IsTerminal, Write},
};

use ghostty_config::config_struct;

use crate::{
    error::Error,
    tui::{self, Color, PrettySession, Style},
};

config_struct! {
    pub struct Options {
        /// If `true`, print without formatting even if printing to a tty
        "plain" => plain: bool = false,
    }
}
cli_options!(Options, help);

/// All X11 color names (with their original spelling) and values. Like the
/// case-insensitive Zig map the first spelling of a name wins.
#[must_use]
pub fn x11_colors() -> Vec<(String, (u8, u8, u8))> {
    ghostty_vt::x11_color::names()
        .iter()
        .map(|(n, c)| ((*n).to_string(), (c.r, c.g, c.b)))
        .collect()
}

/// `std.ascii.orderIgnoreCase`.
fn order_ignore_case(a: &str, b: &str) -> Ordering {
    let a = a.bytes().map(|c| c.to_ascii_lowercase());
    let b = b.bytes().map(|c| c.to_ascii_lowercase());
    a.cmp(b)
}

/// The colors sorted by name, ignoring case.
#[must_use]
pub fn sorted_colors() -> Vec<(String, (u8, u8, u8))> {
    let mut colors = x11_colors();
    colors.sort_by(|a, b| order_ignore_case(&a.0, &b.0));
    colors
}

/// The `list-colors` command is used to list all the named RGB colors in
/// Ghostty.
///
/// Flags:
///
///   * `--plain`: will disable formatting and make the output more
///     friendly for Unix tooling. This is default when not printing to a tty.
pub fn run(args: &[impl AsRef<[u8]>]) -> Result<u8, Error> {
    let mut opts = Options::default();
    crate::parse_options(&mut opts, args)?;

    let colors = sorted_colors();

    let stdout = std::io::stdout();
    if tui::CAN_PRETTY_PRINT && !opts.plain && stdout.is_terminal() {
        return pretty_print(&colors);
    }

    let mut out = std::io::BufWriter::new(stdout.lock());
    write_plain(&mut out, &colors)?;
    out.flush()?;
    Ok(0)
}

/// The plain `name = #rrggbb` listing.
pub fn write_plain(out: &mut dyn Write, colors: &[(String, (u8, u8, u8))]) -> std::io::Result<()> {
    for (name, (r, g, b)) in colors {
        writeln!(out, "{name} = #{r:02x}{g:02x}{b:02x}")?;
    }
    Ok(())
}

fn pretty_print(colors: &[(String, (u8, u8, u8))]) -> Result<u8, Error> {
    let mut session = PrettySession::new_raw(std::io::BufWriter::new(std::io::stdout().lock()))?;
    let result = pretty_rows(&mut session, colors);
    let fin = session.finish();
    result?;
    fin?;
    Ok(0)
}

fn pretty_rows<W: Write>(
    session: &mut PrettySession<W>,
    colors: &[(String, (u8, u8, u8))],
) -> std::io::Result<()> {
    let width = session.width;
    let max_name_len = colors.iter().map(|(n, _)| n.len()).max().unwrap_or(0);

    // max name length plus " = #RRGGBB XX" plus "  " gutter between columns
    let column_size = max_name_len + 15;
    // add two to take into account lack of gutter after last column
    let columns = ((usize::from(width) + 2) / column_size).max(1);

    let step = colors.len() / columns + 1;
    for i in 0..step {
        let mut row = session.row();
        let mut col = 0u16;

        for j in 0..columns {
            let k = i + step * j;
            let Some((name, (r, g, b))) = colors.get(k) else {
                continue;
            };

            let style1 = Style {
                fg: Color::Rgb(*r, *g, *b),
                ..Style::default()
            };
            let style2 = Style {
                fg: Color::Rgb(*r, *g, *b),
                bg: Color::Rgb(*r, *g, *b),
                ..Style::default()
            };
            let plain = Style::default();

            // name of the color
            col = row.print_segment(name, plain, col).col;
            // push the color data to the end of the column
            for _ in 0..(max_name_len - name.len()) {
                col = row.print_segment(" ", plain, col).col;
            }
            col = row.print_segment(" = ", plain, col).col;
            // rgb triple
            col = row
                .print_segment(&format!("#{r:02x}{g:02x}{b:02x}"), style1, col)
                .col;
            col = row.print_segment(" ", plain, col).col;
            // colored block
            col = row.print_segment("  ", style2, col).col;
            // add the gutter if needed
            if j + 1 < columns {
                col = row.print_segment("  ", plain, col).col;
            }
        }

        // clear the rest of the line
        while col != 0 && col < width {
            col = row.print_segment(" ", Style::default(), col).col;
        }

        session.print_row(&row)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;

    #[test]
    fn plain_listing_matches_ghostty_format() {
        let colors = sorted_colors();
        assert_eq!(782, colors.len());
        let mut out = Vec::new();
        write_plain(&mut out, &colors[..2]).unwrap();
        assert_eq!(
            "alice blue = #f0f8ff\nAliceBlue = #f0f8ff\n",
            String::from_utf8(out).unwrap()
        );
    }

    #[test]
    fn sort_is_case_insensitive() {
        assert_eq!(Ordering::Less, order_ignore_case("alice blue", "AliceBlue"));
        assert_eq!(Ordering::Less, order_ignore_case("Zed", "zee"));
        assert_eq!(Ordering::Equal, order_ignore_case("ABC", "abc"));
    }

    #[test]
    fn pretty_rows_fill_width() {
        let colors = sorted_colors();
        let mut buf = Vec::new();
        let mut session = PrettySession::new(&mut buf).unwrap();
        session.width = 100;
        pretty_rows(&mut session, &colors[..10]).unwrap();
        drop(session);
        let s = String::from_utf8(buf).unwrap();
        assert!(s.starts_with("\x1b[?2027h\r\x1b[m\x1b[J\x1b[?25l\x1b[?2026h\x1b[m"));
        let max_len = colors[..10].iter().map(|(n, _)| n.len()).max().unwrap();
        let columns = (102 / (max_len + 15)).max(1);
        assert_eq!(10 / columns + 1, s.matches("\r\n").count());
    }
}
