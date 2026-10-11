//! The CLI root (ZIG/src/cli/ghostty.zig): the [`Action`] enum, `+action`
//! dispatch and per-action `--help` output.

use std::io::Write;

use crate::action::{ActionEnum, DetectError, SpecialCase, detect_iter};
use crate::error::Error;

/// Special commands that can be invoked via CLI flags. These are all
/// invoked by using `+<action>` as a CLI flag. The only exception is
/// "version" which can be invoked additionally with `--version`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Action {
    /// Output the version and exit
    Version,

    /// Output help information for the CLI or configuration
    Help,

    /// List available fonts
    ListFonts,

    /// List available keybinds
    ListKeybinds,

    /// List available themes
    ListThemes,

    /// List named RGB colors
    ListColors,

    /// List keybind actions
    ListActions,

    /// Manage SSH terminfo cache for automatic remote host setup
    SshCache,

    /// Edit the config file in the configured terminal editor.
    EditConfig,

    /// Dump the config to stdout
    ShowConfig,

    // Validate passed config file
    ValidateConfig,

    // Show which font face Ghostty loads a codepoint from.
    ShowFace,

    // List, (eventually) view, and (eventually) send crash reports.
    CrashReport,

    // Boo!
    Boo,

    // Use IPC to tell the running Ghostty to open a new window.
    NewWindow,
}

impl Action {
    /// All actions in declaration order.
    pub const ALL: &'static [Action] = &[
        Action::Version,
        Action::Help,
        Action::ListFonts,
        Action::ListKeybinds,
        Action::ListThemes,
        Action::ListColors,
        Action::ListActions,
        Action::SshCache,
        Action::EditConfig,
        Action::ShowConfig,
        Action::ValidateConfig,
        Action::ShowFace,
        Action::CrashReport,
        Action::Boo,
        Action::NewWindow,
    ];

    /// The CLI name of the action (`@tagName`), e.g. `list-fonts`.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Action::Version => "version",
            Action::Help => "help",
            Action::ListFonts => "list-fonts",
            Action::ListKeybinds => "list-keybinds",
            Action::ListThemes => "list-themes",
            Action::ListColors => "list-colors",
            Action::ListActions => "list-actions",
            Action::SshCache => "ssh-cache",
            Action::EditConfig => "edit-config",
            Action::ShowConfig => "show-config",
            Action::ValidateConfig => "validate-config",
            Action::ShowFace => "show-face",
            Action::CrashReport => "crash-report",
            Action::Boo => "boo",
            Action::NewWindow => "new-window",
        }
    }

    /// `std.meta.stringToEnum`.
    #[must_use]
    pub fn from_name(name: &str) -> Option<Action> {
        Action::ALL.iter().copied().find(|a| a.name() == name)
    }

    /// Returns the filename associated with an action. This is a relative
    /// path from the root src/ directory of the Zig sources.
    #[must_use]
    pub fn file(self) -> String {
        format!("cli/{}.zig", self.name().replace('-', "_"))
    }

    /// The help text of the action (the doc comment of the Zig `run`
    /// function, i.e. `help_strings.Action.<name>`), without trailing newline.
    #[must_use]
    pub fn help_text(self) -> &'static str {
        match self {
            Action::Version => include_str!("help/version.txt"),
            Action::Help => include_str!("help/help.txt"),
            Action::ListFonts => include_str!("help/list-fonts.txt"),
            Action::ListKeybinds => include_str!("help/list-keybinds.txt"),
            Action::ListThemes => include_str!("help/list-themes.txt"),
            Action::ListColors => include_str!("help/list-colors.txt"),
            Action::ListActions => include_str!("help/list-actions.txt"),
            Action::SshCache => include_str!("help/ssh-cache.txt"),
            Action::EditConfig => include_str!("help/edit-config.txt"),
            Action::ShowConfig => include_str!("help/show-config.txt"),
            Action::ValidateConfig => include_str!("help/validate-config.txt"),
            Action::ShowFace => include_str!("help/show-face.txt"),
            Action::CrashReport => include_str!("help/crash-report.txt"),
            Action::Boo => include_str!("help/boo.txt"),
            Action::NewWindow => include_str!("help/new-window.txt"),
        }
    }

    /// Run the action. `args` is the full argument vector (including
    /// `argv[0]`). This returns the exit code to exit with.
    ///
    /// If the action requests its help text (`--help`/`-h`), the help text
    /// is printed to stdout and 0 is returned.
    pub fn run(self, args: &[String]) -> Result<u8, Error> {
        match self.run_main(args) {
            Err(Error::ActionHelpRequested) => {
                // Future note: for now we just output the help text directly
                // to stdout. In the future we can style this much prettier
                // for all commands by just changing this one place.
                let text = format!("{}\n", self.help_text());
                let mut stdout = std::io::stdout().lock();
                if let Err(e) = stdout.write_all(text.as_bytes()) {
                    log::warn!("failed to write help text: {e}\n");
                    return Ok(1);
                }
                if let Err(e) = stdout.flush() {
                    log::warn!("failed to flush help text: {e}\n");
                    return Ok(1);
                }
                Ok(0)
            }
            other => other,
        }
    }

    fn run_main(self, args: &[String]) -> Result<u8, Error> {
        match self {
            Action::Version => crate::version::run(args),
            Action::Help => crate::help::run(args),
            Action::ListFonts => crate::list_fonts::run(args),
            Action::ListKeybinds => crate::list_keybinds::run(args),
            Action::ListThemes => crate::list_themes::run(args),
            Action::ListColors => crate::list_colors::run(args),
            Action::ListActions => crate::list_actions::run(args),
            Action::SshCache => crate::ssh_cache::run(args),
            Action::EditConfig => crate::edit_config::run(args),
            Action::ShowConfig => crate::show_config::run(args),
            Action::ValidateConfig => crate::validate_config::run(args),
            Action::CrashReport => crate::crash_report::run(args),
            Action::ShowFace => crate::show_face::run(args),
            Action::Boo => crate::boo::run(args),
            Action::NewWindow => crate::new_window::run(args),
        }
    }
}

impl ActionEnum for Action {
    fn from_name(name: &str) -> Option<Self> {
        Action::from_name(name)
    }

    fn detect_special_case(arg: &str) -> Option<SpecialCase<Self>> {
        // If we see a "-e" and we haven't seen a command yet, then
        // we are done looking for commands. This special case enables
        // `ghostty -e ghostty +command`. If we've seen a command we
        // still want to keep looking because
        // `ghostty +command -e +command` is invalid.
        if arg == "-e" {
            return Some(SpecialCase::AbortIfNoAction);
        }

        // Special case, --version always outputs the version no
        // matter what, no matter what other args exist.
        if arg == "--version" {
            return Some(SpecialCase::Action(Action::Version));
        }

        // --help matches "help" but if a subcommand is specified
        // then we match the subcommand.
        if arg == "--help" || arg == "-h" {
            return Some(SpecialCase::Fallback(Action::Help));
        }

        None
    }
}

impl std::fmt::Display for Action {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.name())
    }
}

/// Detect the CLI action from `args` (full argv, `argv[0]` included).
pub fn detect(args: &[String]) -> Result<Option<Action>, DetectError> {
    detect_iter::<Action, _, _>(args.iter())
}

/// The CLI entrypoint logic of `main_ghostty.zig` for actions: detect the
/// action from `args` (the full argv), print the errors Ghostty prints for
/// invalid invocations and run the action.
///
/// Returns `None` if no action was requested (the caller should launch the
/// terminal), or the exit code the process should terminate with.
#[must_use]
pub fn run_from_args(args: &[String]) -> Option<u8> {
    let action = match detect(args) {
        Ok(a) => a?,
        Err(err) => {
            let mut stderr = std::io::stderr().lock();
            let _ = match err {
                DetectError::MultipleActions => stderr.write_all(
                    b"Error: multiple CLI actions specified. You must specify only one\n\
                      action starting with the `+` character.\n",
                ),
                DetectError::InvalidAction => stderr.write_all(
                    b"Error: unknown CLI action specified. CLI actions are specified with\n\
                      the '+' character.\n\n\
                      All valid CLI actions can be listed with `ghostty +help`\n",
                ),
            };
            let _ = stderr.flush();
            return Some(1);
        }
    };

    log::info!("executing CLI action={action}");
    Some(match action.run(args) {
        Ok(code) => code,
        Err(err) => {
            log::error!("CLI action failed error={err}");
            1
        }
    })
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;
    use ghostty_config::cli_args::split_args;

    fn detect_str(s: &str) -> Result<Option<Action>, DetectError> {
        detect_iter::<Action, _, _>(split_args(s))
    }

    #[test]
    fn parse_action_none() {
        assert_eq!(None, detect_str("--a=42 --b --b-f=false").unwrap());
    }

    #[test]
    fn parse_action_version() {
        for s in [
            "--a=42 --b --b-f=false --version",
            "--version --a=42 --b --b-f=false",
            "--c=84 --d --version --a=42 --b --b-f=false",
        ] {
            assert_eq!(Some(Action::Version), detect_str(s).unwrap());
        }
    }

    #[test]
    fn parse_action_plus() {
        for s in [
            "--a=42 --b --b-f=false +version",
            "+version --a=42 --b --b-f=false",
            "--c=84 --d +version --a=42 --b --b-f=false",
        ] {
            assert_eq!(Some(Action::Version), detect_str(s).unwrap());
        }
    }

    #[test]
    fn parse_action_plus_ignores_e() {
        assert_eq!(None, detect_str("--a=42 -e +version").unwrap());
        assert_eq!(
            Err(DetectError::MultipleActions),
            detect_str("+list-fonts --a=42 -e +version")
        );
    }

    #[test]
    fn help_fallback() {
        assert_eq!(Some(Action::Help), detect_str("--help").unwrap());
        assert_eq!(Some(Action::Help), detect_str("-h").unwrap());
        assert_eq!(
            Some(Action::ListThemes),
            detect_str("+list-themes --help").unwrap()
        );
    }

    #[test]
    fn names_round_trip() {
        for a in Action::ALL {
            assert_eq!(Some(*a), Action::from_name(a.name()));
            assert_ne!("", a.help_text());
            assert!(a.file().starts_with("cli/"));
        }
        assert_eq!("cli/list_fonts.zig", Action::ListFonts.file());
    }
}
