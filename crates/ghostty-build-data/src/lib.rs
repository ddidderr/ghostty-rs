//! Build-data generators (port of `src/main_build_data.zig`, `src/extra/*.zig`
//! and `src/build/mdgen/*`).
//!
//! The binary prints the requested artifact to stdout; the `just install`
//! recipe (mirroring `GhosttyResources.zig` / `GhosttyDocs.zig`) captures it.

#![allow(clippy::too_many_lines)]
#![cfg_attr(test, allow(clippy::expect_used))]

use ghostty_cli::Action;
use ghostty_config::cli_args::FieldKind;
use ghostty_config::config::Config;
use ghostty_config::macros::ConfigStruct;

pub mod bash;
pub mod fish;
pub mod mdgen;
pub mod sublime;
pub mod vim;
pub mod zsh;

#[allow(clippy::needless_raw_string_hashes)]
mod chunks_bash;
#[allow(clippy::needless_raw_string_hashes)]
mod chunks_sublime;
#[allow(clippy::needless_raw_string_hashes)]
mod chunks_vim;
#[allow(clippy::needless_raw_string_hashes)]
mod chunks_zsh;

/// The modes of the generator (Zig: `main_build_data.Action`, plus the two
/// `mdgen` executables which are separate binaries upstream).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Bash,
    Fish,
    Zsh,
    Sublime,
    VimSyntax,
    VimFtdetect,
    VimFtplugin,
    VimCompiler,
    Terminfo,
    /// `ghostty(1)` man page markdown (Zig: `mdgen_ghostty_1`).
    Man1,
    /// `ghostty(5)` man page markdown (Zig: `mdgen_ghostty_5`).
    Man5,
}

impl Mode {
    pub const ALL: &'static [(&'static str, Mode)] = &[
        ("bash", Mode::Bash),
        ("fish", Mode::Fish),
        ("zsh", Mode::Zsh),
        ("sublime", Mode::Sublime),
        ("vim-syntax", Mode::VimSyntax),
        ("vim-ftdetect", Mode::VimFtdetect),
        ("vim-ftplugin", Mode::VimFtplugin),
        ("vim-compiler", Mode::VimCompiler),
        ("terminfo", Mode::Terminfo),
        ("mdgen-ghostty-1", Mode::Man1),
        ("mdgen-ghostty-5", Mode::Man5),
    ];

    /// Parse `+name` (Zig action syntax) or a bare `name`.
    #[must_use]
    pub fn parse(arg: &str) -> Option<Mode> {
        let name = arg.strip_prefix('+').unwrap_or(arg);
        Mode::ALL.iter().find(|(n, _)| *n == name).map(|(_, m)| *m)
    }

    /// Generate the output of this mode.
    #[must_use]
    pub fn generate(self) -> String {
        match self {
            Mode::Bash => bash::completions(),
            Mode::Fish => fish::completions(),
            Mode::Zsh => zsh::completions(),
            Mode::Sublime => sublime::syntax(),
            Mode::VimSyntax => vim::syntax(),
            Mode::VimFtdetect => vim::FTDETECT.to_string(),
            Mode::VimFtplugin => vim::FTPLUGIN.to_string(),
            Mode::VimCompiler => vim::COMPILER.to_string(),
            Mode::Terminfo => ghostty_termio::terminfo::ghostty.encode_to_string(),
            Mode::Man1 => mdgen::ghostty_1(),
            Mode::Man5 => mdgen::ghostty_5(),
        }
    }
}

/// A public (non-`_`) config field.
pub(crate) struct CfgField {
    pub name: &'static str,
    pub kind: FieldKind,
    pub help: String,
}

/// The non-internal fields of `Config` in declaration order.
pub(crate) fn config_fields() -> Vec<CfgField> {
    Config::FIELDS
        .iter()
        .filter(|f| !f.name.starts_with('_'))
        .map(|f| CfgField {
            name: f.name,
            kind: f.kind,
            help: f.help(),
        })
        .collect()
}

/// Space separated enum/packed names for a completion word list.
pub(crate) fn join_names(names: &[&str]) -> String {
    names.join(" ")
}

/// Space separated `name no-name` pairs (packed structs).
pub(crate) fn join_packed(names: &[&str]) -> String {
    names
        .iter()
        .map(|n| format!("{n} no-{n}"))
        .collect::<Vec<_>>()
        .join(" ")
}

/// All CLI actions in declaration order.
pub(crate) fn actions() -> &'static [Action] {
    Action::ALL
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_mode_generates_output() {
        for (name, mode) in Mode::ALL {
            assert!(!mode.generate().is_empty(), "{name}");
        }
        assert_eq!(Mode::parse("+bash"), Some(Mode::Bash));
        assert_eq!(Mode::parse("vim-syntax"), Some(Mode::VimSyntax));
        assert_eq!(Mode::parse("nope"), None);
    }

    #[test]
    fn completions_cover_config_and_actions() {
        let bash = bash::completions();
        assert!(bash.starts_with("_ghostty() {"));
        assert!(bash.contains("--font-family) _fonts ;;"));
        assert!(bash.contains("config+=\" '--maximize '\""));
        assert!(bash.contains("local list_themes=\""));
        assert!(bash.contains("topLevel+=\" +new-window\""));
        let zsh = zsh::completions();
        assert!(zsh.starts_with("#compdef ghostty"));
        assert!(zsh.contains("\"--working-directory=-:::{_files -/}\""));
        let fish = fish::completions();
        assert!(fish.starts_with("set -l commands \"+version +help"));
        assert!(fish.contains(" -l theme -r -f -a \"(ghostty +list-themes"));
    }

    #[test]
    fn editor_syntax_lists_config_keys() {
        assert!(vim::syntax().contains("\n\t\\ font-family"));
        assert!(sublime::syntax().contains("|font-family|"));
        assert!(!sublime::syntax().contains("|_"));
    }

    #[test]
    fn man_pages_have_sections() {
        let one = mdgen::ghostty_1();
        assert!(one.starts_with("% GHOSTTY(1) Version "));
        assert!(!one.contains("@@VERSION@@"));
        assert!(one.contains("**`+list-fonts`**\n\n:   "));
        assert!(one.contains("**`--font-family`**"));
        let five = mdgen::ghostty_5();
        assert!(five.contains("# KEYBIND ACTIONS"));
        assert!(five.contains("**`new_split`**"));
    }

    /// Every supported locale has a `.po` catalog and vice versa
    /// (Zig: `GhosttyI18n.zig` iterates `os/i18n_locales.zig`).
    #[test]
    fn po_catalogs_match_locales() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../po");
        let mut found: Vec<String> = std::fs::read_dir(dir)
            .expect("po dir")
            .filter_map(Result::ok)
            .filter_map(|e| {
                let n = e.file_name().into_string().ok()?;
                n.strip_suffix(".po").map(str::to_owned)
            })
            .collect();
        found.sort();
        let mut want: Vec<String> = ghostty_os::i18n_locales::LOCALES
            .iter()
            .map(|s| (*s).to_owned())
            .collect();
        want.sort();
        assert_eq!(found, want);
    }
}
