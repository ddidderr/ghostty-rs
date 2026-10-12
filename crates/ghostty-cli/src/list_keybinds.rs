//! `+list-keybinds` (ZIG/src/cli/list_keybinds.zig), including its pretty
//! (tty) output.

use std::{
    cmp::Ordering,
    io::{IsTerminal, Write},
};

use ghostty_config::{
    config::Config,
    config_struct,
    formatter::EntryFormatter,
    keybinds::Keybinds,
};
use ghostty_input::binding::{
    Action as BindingAction,
    Trigger,
    TriggerKey,
    set::{Set, Value},
};

use crate::{
    error::Error,
    tui::{self, Color, PrettySession, Style, gwidth},
};

config_struct! {
    pub struct Options {
        /// If `true`, print out the default keybinds instead of the ones configured
        /// in the config file.
        "default" => default: bool = false,

        /// If `true`, print out documentation about the action associated with the
        /// keybinds.
        "docs" => docs: bool = false,

        /// If `true`, print without formatting even if printing to a tty
        "plain" => plain: bool = false,
    }
}
cli_options!(Options, help);

/// The `list-keybinds` command is used to list all the available keybinds for
/// Ghostty.
///
/// When executed without any arguments this will list the current keybinds
/// loaded by the config file. If no config file is found or there aren't any
/// changes to the keybinds it will print out the default ones configured for
/// Ghostty
///
/// Flags:
///
///   * `--default`: will print out all the default keybinds
///
///   * `--docs`: currently does nothing, intended to print out documentation
///     about the action associated with the keybinds
///
///   * `--plain`: will disable formatting and make the output more
///     friendly for Unix tooling. This is default when not printing to a tty.
pub fn run(args: &[impl AsRef<[u8]>]) -> Result<u8, Error> {
    let mut opts = Options::default();
    crate::parse_options(&mut opts, args)?;

    let config = if opts.default {
        Config::new_default()
    } else {
        Config::load()?
    };

    let stdout = std::io::stdout();
    if tui::CAN_PRETTY_PRINT && !opts.plain && stdout.is_terminal() {
        return pretty_print(&config.keybind);
    }

    let text = format_plain(&config.keybind, opts.docs)?;
    let mut out = std::io::BufWriter::new(stdout.lock());
    out.write_all(text.as_bytes())?;
    // Don't forget to flush!
    out.flush()?;
    Ok(0)
}

/// The plain `keybind = ...` listing.
pub fn format_plain(keybinds: &Keybinds, docs: bool) -> Result<String, Error> {
    let mut text = String::new();
    let mut f = EntryFormatter::new("keybind", &mut text);
    keybinds.format_entry_docs(&mut f, docs)?;
    Ok(text)
}

/// A flattened binding: a chain of triggers (the sequence) and the actions
/// (more than one if chained).
#[derive(Debug, Clone)]
pub struct ChordBinding {
    pub table_name: Option<String>,
    pub triggers: Vec<Trigger>,
    pub actions: Vec<BindingAction>,
}

fn mod_count(t: &Trigger) -> usize {
    usize::from(t.mods.super_)
        + usize::from(t.mods.ctrl)
        + usize::from(t.mods.shift)
        + usize::from(t.mods.alt)
}

fn key_order(t: &Trigger) -> i64 {
    match t.key {
        TriggerKey::Physical(k) => i64::from(k as i32),
        TriggerKey::Unicode(c) => i64::from(c),
        TriggerKey::CatchAll => i64::from(i32::MAX),
    }
}

impl ChordBinding {
    /// Order keybinds based on various properties
    ///    1. Default bindings before table bindings (tables grouped at end)
    ///    2. Longest chord sequence
    ///    3. Most active modifiers
    ///    4. Alphabetically by active modifiers
    ///    5. Trigger key order
    ///    6. Within tables, sort by table name
    /// These properties propagate through chorded keypresses
    ///
    /// Adapted from Binding.lessThan
    #[must_use]
    pub fn less_than(&self, rhs: &ChordBinding) -> bool {
        let lhs_has_table = self.table_name.is_some();
        let rhs_has_table = rhs.table_name.is_some();

        if lhs_has_table != rhs_has_table {
            return !lhs_has_table;
        }

        if let (Some(l), Some(r)) = (&self.table_name, &rhs.table_name) {
            let cmp = l.as_bytes().cmp(r.as_bytes());
            if cmp != Ordering::Equal {
                return cmp == Ordering::Less;
            }
        }

        let lhs_len = self.triggers.len();
        let rhs_len = rhs.triggers.len();
        debug_assert!(lhs_len != 0);
        debug_assert!(rhs_len != 0);

        if lhs_len != rhs_len {
            return lhs_len > rhs_len;
        }

        let lhs_count: usize = self.triggers.iter().map(mod_count).sum();
        let rhs_count: usize = rhs.triggers.iter().map(mod_count).sum();
        if lhs_count != rhs_count {
            return lhs_count > rhs_count;
        }

        for (l, r) in self.triggers.iter().zip(&rhs.triggers) {
            let l_int = l.mods.int();
            let r_int = r.mods.int();
            if l_int != r_int {
                return l_int > r_int;
            }
        }

        let mut l_iter = self.triggers.iter();
        let mut r_iter = rhs.triggers.iter();
        loop {
            let (Some(l), Some(r)) = (l_iter.next(), r_iter.next()) else {
                // The Zig code treats this as unreachable (the final
                // iteration always returns).
                return false;
            };
            // We want catch_all to sort last.
            let lhs_key = key_order(l);
            let rhs_key = key_order(r);

            if l_iter.len() == 0 || r_iter.len() == 0 {
                return lhs_key < rhs_key;
            }
            if lhs_key != rhs_key {
                return lhs_key < rhs_key;
            }
        }
    }
}

/// The plain (unstyled) text of a trigger, used for width measurement
/// (the Zig `iterateBindings` width computation).
fn trigger_width(t: &Trigger) -> u16 {
    let mut buf = String::new();
    if t.mods.super_ {
        buf.push_str("super + ");
    }
    if t.mods.ctrl {
        buf.push_str("ctrl  + ");
    }
    if t.mods.alt {
        buf.push_str("alt   + ");
    }
    if t.mods.shift {
        buf.push_str("shift + ");
    }
    buf.push_str(&key_text(t.key));
    gwidth(&buf)
}

fn key_text(key: TriggerKey) -> String {
    match key {
        TriggerKey::Physical(k) => k.name().to_string(),
        TriggerKey::Unicode(c) => {
            char::from_u32(c).map_or_else(|| "\u{fffd}".to_string(), |c| c.to_string())
        }
        TriggerKey::CatchAll => "catch_all".to_string(),
    }
}

/// Flatten a binding set, recursively expanding leader keys into chords.
/// Returns the bindings and the widest chord.
#[must_use]
pub fn iterate_bindings(set: &Set) -> (Vec<ChordBinding>, u16) {
    let mut widest_chord: u16 = 0;
    let mut bindings = vec![];
    for (key, value) in &set.bindings {
        let trigger = key.0;
        let width = trigger_width(&trigger);

        match value {
            Value::Leader(leader) => {
                // Recursively iterate on the set of bindings for this leader key
                let (mut sub_bindings, max_width) = iterate_bindings(leader);

                // Prepend the current keybind onto the list of sub-binds
                for nb in &mut sub_bindings {
                    nb.triggers.insert(0, trigger);
                }

                // Add the longest sub-bind width to the current bind width along with a padding
                // of 5 for the '  >  ' spacer
                widest_chord = widest_chord.max(width + max_width + 5);
                bindings.extend(sub_bindings);
            }
            Value::Leaf(leaf) => {
                widest_chord = widest_chord.max(width);
                bindings.push(ChordBinding {
                    table_name: None,
                    triggers: vec![trigger],
                    actions: vec![leaf.action.clone()],
                });
            }
            Value::LeafChained(leaf) => {
                widest_chord = widest_chord.max(width);
                bindings.push(ChordBinding {
                    table_name: None,
                    triggers: vec![trigger],
                    actions: leaf.actions.clone(),
                });
            }
        }
    }

    (bindings, widest_chord)
}

/// Collect and sort all bindings (default set and key tables) for the pretty
/// listing. Returns the bindings, the widest chord and the widest table
/// prefix (`table/`).
#[must_use]
pub fn collect_bindings(keybinds: &Keybinds) -> (Vec<ChordBinding>, u16, u16) {
    // Collect default bindings, recursively flattening chords
    let (mut bindings, mut widest_chord) = iterate_bindings(&keybinds.set);

    // Collect key table bindings
    let mut widest_table_prefix: u16 = 0;
    for (table_name, table_set) in &keybinds.tables {
        let (mut table_bindings, table_width) = iterate_bindings(table_set);
        for b in &mut table_bindings {
            b.table_name = Some(table_name.to_text_lossy());
        }

        bindings.extend(table_bindings);
        widest_chord = widest_chord.max(table_width);
        widest_table_prefix =
            widest_table_prefix.max(gwidth(&table_name.to_text_lossy()) + gwidth("/"));
    }

    // std.mem.sort is stable.
    bindings.sort_by(|a, b| {
        if a.less_than(b) {
            Ordering::Less
        } else if b.less_than(a) {
            Ordering::Greater
        } else {
            Ordering::Equal
        }
    });
    (bindings, widest_chord, widest_table_prefix)
}

fn pretty_print(keybinds: &Keybinds) -> Result<u8, Error> {
    let mut session = PrettySession::new_raw(std::io::BufWriter::new(std::io::stdout().lock()))?;
    let result = pretty_rows(&mut session, keybinds);
    let fin = session.finish();
    result?;
    fin?;
    Ok(0)
}

fn pretty_rows<W: Write>(
    session: &mut PrettySession<W>,
    keybinds: &Keybinds,
) -> std::io::Result<()> {
    let (bindings, widest_chord, widest_table_prefix) = collect_bindings(keybinds);

    // Set up styles for each modifier
    let fg = |i: u8| Style {
        fg: Color::Index(i),
        ..Style::default()
    };
    let super_style = fg(1);
    let ctrl_style = fg(2);
    let alt_style = fg(3);
    let shift_style = fg(4);
    let table_style = fg(8);
    let plain = Style::default();
    let sep_style = Style {
        bold: true,
        fg: Color::Index(6),
        ..Style::default()
    };
    let dim_style = Style {
        dim: true,
        ..Style::default()
    };
    let arg_style = fg(5);

    // Print the list
    for bind in &bindings {
        let mut row = session.row();
        let mut col: u16 = 0;

        if let Some(name) = &bind.table_name {
            col = row.print_segment(name, table_style, col).col;
            col = row.print_segment("/", table_style, col).col;
        }

        for (i, trigger) in bind.triggers.iter().enumerate() {
            if trigger.mods.super_ {
                col = row.print_segment("super", super_style, col).col;
                col = row.print_segment(" + ", plain, col).col;
            }
            if trigger.mods.ctrl {
                col = row.print_segment("ctrl ", ctrl_style, col).col;
                col = row.print_segment(" + ", plain, col).col;
            }
            if trigger.mods.alt {
                col = row.print_segment("alt  ", alt_style, col).col;
                col = row.print_segment(" + ", plain, col).col;
            }
            if trigger.mods.shift {
                col = row.print_segment("shift", shift_style, col).col;
                col = row.print_segment(" + ", plain, col).col;
            }
            col = row.print_segment(&key_text(trigger.key), plain, col).col;

            // Print a separator between chorded keys
            if i + 1 < bind.triggers.len() {
                col = row.print_segment("  >  ", sep_style, col).col;
            }
        }

        let mut action_col: u16 = widest_table_prefix + widest_chord + 3;
        for (i, act) in bind.actions.iter().enumerate() {
            if i > 0 {
                action_col = row.print_segment(", ", dim_style, action_col).col;
            }

            let action = act.to_string();
            // If our action has an argument, we print the argument in a different color
            if let Some(idx) = action.find(':') {
                action_col = row
                    .print(
                        &[
                            (&action[..idx], plain),
                            (&action[idx..=idx], dim_style),
                            (&action[idx + 1..], arg_style),
                        ],
                        action_col,
                    )
                    .col;
            } else {
                action_col = row.print_segment(&action, plain, action_col).col;
            }
        }
        let _ = col;
        session.print_row(&row)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;

    fn default_keybinds() -> Keybinds {
        Config::new_default().keybind
    }

    #[test]
    fn plain_default_listing_has_entries() {
        let text = format_plain(&default_keybinds(), false).unwrap();
        assert!(
            text.contains("keybind = ctrl+shift+c=copy_to_clipboard:mixed\n"),
            "{text}"
        );
        assert!(text.lines().all(|l| l.starts_with("keybind = ")));
    }

    #[test]
    fn empty_keybinds() {
        let kb = Keybinds::default();
        assert_eq!("keybind = \n", format_plain(&kb, false).unwrap());
    }

    #[test]
    fn collected_bindings_are_sorted() {
        let (bindings, widest, table_prefix) = collect_bindings(&default_keybinds());
        assert!(bindings.len() > 50);
        assert!(widest > 0);
        assert_eq!(0, table_prefix);
        // Longest sequences come first, and ordering is a strict order.
        for w in bindings.windows(2) {
            assert!(!w[1].less_than(&w[0]), "{:?} vs {:?}", w[0], w[1]);
        }
        assert!(
            bindings
                .windows(2)
                .all(|w| w[0].triggers.len() >= w[1].triggers.len())
        );
    }

    #[test]
    fn chords_and_tables() {
        let mut kb = Keybinds::default();
        kb.set.parse_and_put("ctrl+a>b=new_window").unwrap();
        kb.set.parse_and_put("ctrl+a>c=new_tab").unwrap();
        kb.set.parse_and_put("x=close_surface").unwrap();
        let (bindings, widest, _) = collect_bindings(&kb);
        assert_eq!(3, bindings.len());
        assert_eq!(2, bindings[0].triggers.len());
        assert_eq!(1, bindings[2].triggers.len());
        // "ctrl  + a" (9) + "  >  " (5) + "b" (1)
        assert_eq!(15, widest);
    }

    #[test]
    fn pretty_rows_output() {
        let mut kb = Keybinds::default();
        kb.set
            .parse_and_put("super+ctrl+shift+arrow_down=resize_split:down,10")
            .unwrap();
        kb.set.parse_and_put("paste=paste_from_clipboard").unwrap();
        let mut buf = Vec::new();
        let mut session = PrettySession::new(&mut buf).unwrap();
        session.width = 100;
        pretty_rows(&mut session, &kb).unwrap();
        drop(session);
        let s = String::from_utf8(buf).unwrap();
        assert!(
            s.contains(
                "\x1b[31msuper\x1b[39m + \x1b[32mctrl \x1b[39m + \x1b[34mshift\x1b[39m + arrow_down"
            ),
            "{s:?}"
        );
        assert!(
            s.contains("resize_split\x1b[2m:\x1b[35m\x1b[22mdown,10\r\n"),
            "{s:?}"
        );
        assert!(s.contains("paste\x1b["), "{s:?}");
    }
}
