//! Inter-process Communication to a running Ghostty instance from a separate
//! process (port of `ipc.zig`). The transport (D-Bus) lives in apprt-gtk.

/// Errors from IPC.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum Error {
    /// The IPC failed. If a function returns this error, it's expected that
    /// an a more specific error message will have been written to stderr (or
    /// otherwise shown to the user in an appropriate way).
    #[error("IPC failed")]
    IpcFailed,
}

/// Which instance to talk to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Target {
    /// Open up a new window in a custom instance of Ghostty.
    Class(String),

    /// Detect which instance to open a new window in.
    Detect,
}

/// The arguments to pass to Ghostty as the command.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewWindow {
    /// A list of command arguments to launch in the new window. If this is
    /// `None` the command configured in the config or the user's default
    /// shell should be launched.
    ///
    /// It is an error for this to be `Some`, but zero length.
    pub arguments: Option<Vec<Vec<u8>>>,
}

/// IPC actions.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    /// The arguments to pass to Ghostty as the command.
    NewWindow(NewWindow),
}

/// Mirrors `Action.Key`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActionKey {
    NewWindow,
}

impl Action {
    #[must_use]
    pub fn key(&self) -> ActionKey {
        match self {
            Action::NewWindow(_) => ActionKey::NewWindow,
        }
    }
}

impl Target {
    #[must_use]
    pub fn is_detect(&self) -> bool {
        matches!(self, Target::Detect)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn action_key() {
        let a = Action::NewWindow(NewWindow {
            arguments: Some(vec!["a".into()]),
        });
        assert_eq!(a.key(), ActionKey::NewWindow);
    }

    #[test]
    fn target_variants() {
        assert!(Target::Detect.is_detect());
        assert!(!Target::Class("x".into()).is_detect());
    }
}
