//! Detection of `+action` arguments (ZIG/src/cli/action.zig).

use std::fmt;

/// Errors from [`detect_iter`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DetectError {
    /// Multiple actions were detected. You can specify at most one
    /// action on the CLI otherwise the behavior desired is ambiguous.
    MultipleActions,

    /// An unknown action was specified.
    InvalidAction,
}

impl fmt::Display for DetectError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DetectError::MultipleActions => f.write_str("MultipleActions"),
            DetectError::InvalidAction => f.write_str("InvalidAction"),
        }
    }
}

impl std::error::Error for DetectError {}

/// The action enum E can implement `detect_special_case` to return this
/// enum in order to perform various special case actions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpecialCase<E> {
    /// Immediately return this action.
    Action(E),

    /// Return this action if no other action is found.
    Fallback(E),

    /// If there is no pending action (we haven't seen an action yet)
    /// then we should return no action. This is kind of weird but is
    /// a special case to allow "-e" in Ghostty.
    AbortIfNoAction,
}

/// An enum of actions that can be detected from CLI args.
pub trait ActionEnum: Sized + Copy {
    /// `std.meta.stringToEnum`.
    fn from_name(name: &str) -> Option<Self>;

    /// `E.detectSpecialCase`; the default has no special cases.
    fn detect_special_case(_arg: &str) -> Option<SpecialCase<Self>> {
        None
    }
}

/// Detect the action from the process CLI args (Zig: `detectArgs`).
pub fn detect_args<E: ActionEnum>() -> Result<Option<E>, DetectError> {
    detect_iter(ghostty_os::args::collect_lossy())
}

/// Detect the action from any iterator. Each iterator value should yield
/// a CLI argument such as "--foo".
pub fn detect_iter<E: ActionEnum, I, S>(iter: I) -> Result<Option<E>, DetectError>
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    let mut fallback: Option<E> = None;
    let mut pending: Option<E> = None;
    for arg in iter {
        let arg = arg.as_ref();

        // Allow handling of special cases.
        if let Some(special) = E::detect_special_case(arg) {
            match special {
                SpecialCase::Action(a) => return Ok(Some(a)),
                SpecialCase::Fallback(a) => fallback = Some(a),
                SpecialCase::AbortIfNoAction => {
                    if pending.is_none() {
                        return Ok(None);
                    }
                }
            }
        }

        // Commands must start with "+"
        let Some(name) = arg.strip_prefix('+') else {
            continue;
        };
        if pending.is_some() {
            return Err(DetectError::MultipleActions);
        }
        pending = Some(E::from_name(name).ok_or(DetectError::InvalidAction)?);
    }

    // If we have an action, we always return that action, even if we've
    // seen "--help" or "-h" because the action may have its own help text.
    if pending.is_some() {
        return Ok(pending);
    }

    // If we have no action but we have a fallback, then we return that.
    Ok(fallback)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;
    use ghostty_config::cli_args::split_args;

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum Plain {
        Foo,
        Bar,
        Baz,
    }

    impl ActionEnum for Plain {
        fn from_name(name: &str) -> Option<Self> {
            match name {
                "foo" => Some(Plain::Foo),
                "bar" => Some(Plain::Bar),
                "baz" => Some(Plain::Baz),
                _ => None,
            }
        }
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum Special {
        Foo,
        Bar,
    }

    /// Which special case `Special` reports for which argument.
    static SPECIAL_ARG: std::sync::Mutex<(&str, u8)> = std::sync::Mutex::new(("", 0));

    impl ActionEnum for Special {
        fn from_name(name: &str) -> Option<Self> {
            match name {
                "foo" => Some(Special::Foo),
                "bar" => Some(Special::Bar),
                _ => None,
            }
        }

        fn detect_special_case(arg: &str) -> Option<SpecialCase<Self>> {
            let (special, kind) = *SPECIAL_ARG.lock().unwrap();
            if arg != special {
                return None;
            }
            Some(match kind {
                0 => SpecialCase::Action(Special::Foo),
                1 => SpecialCase::Fallback(Special::Foo),
                _ => SpecialCase::AbortIfNoAction,
            })
        }
    }

    /// The special-case tests share a global, so they run in one test body
    /// each under this lock.
    static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    fn set_special(arg: &'static str, kind: u8) {
        *SPECIAL_ARG.lock().unwrap() = (arg, kind);
    }

    fn detect<E: ActionEnum>(s: &str) -> Result<Option<E>, DetectError> {
        detect_iter::<E, _, _>(split_args(s))
    }

    #[test]
    fn detect_direct_match() {
        assert_eq!(Some(Plain::Foo), detect::<Plain>("+foo").unwrap());
    }

    #[test]
    fn detect_invalid_match() {
        assert_eq!(Err(DetectError::InvalidAction), detect::<Plain>("+invalid"));
    }

    #[test]
    fn detect_multiple_actions() {
        assert_eq!(
            Err(DetectError::MultipleActions),
            detect::<Plain>("+foo +bar")
        );
    }

    #[test]
    fn detect_no_match() {
        assert_eq!(None, detect::<Plain>("--some-flag").unwrap());
        let _ = Plain::Baz;
    }

    #[test]
    fn detect_special_case_action() {
        let _g = LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        set_special("--special", 0);
        assert_eq!(
            Some(Special::Foo),
            detect::<Special>("--special +bar").unwrap()
        );
        assert_eq!(
            Some(Special::Foo),
            detect::<Special>("+bar --special").unwrap()
        );
        assert_eq!(Some(Special::Bar), detect::<Special>("+bar").unwrap());
    }

    #[test]
    fn detect_special_case_fallback() {
        let _g = LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        set_special("--special", 1);
        assert_eq!(Some(Special::Foo), detect::<Special>("--special").unwrap());
        assert_eq!(
            Some(Special::Bar),
            detect::<Special>("+bar --special").unwrap()
        );
        assert_eq!(
            Some(Special::Bar),
            detect::<Special>("--special +bar").unwrap()
        );
    }

    #[test]
    fn detect_special_case_abort_if_no_action() {
        let _g = LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        set_special("-e", 2);
        assert_eq!(None, detect::<Special>("-e").unwrap());
        assert_eq!(Some(Special::Foo), detect::<Special>("+foo -e").unwrap());
        assert_eq!(None, detect::<Special>("-e +bar").unwrap());
    }
}
