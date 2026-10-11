//! Conditional configuration state (port of `config/conditional.zig`).
//!
//! Conditionals in Ghostty configuration are based on a static, typed
//! state of the world instead of a dynamic key-value set.

/// The theme of the underlying OS desktop environment.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Hash)]
pub enum Theme {
    #[default]
    Light,
    Dark,
}

impl Theme {
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            Theme::Light => "light",
            Theme::Dark => "dark",
        }
    }
}

/// The target OS (Zig: `std.Target.Os.Tag`); only the Linux build is
/// supported, but the other tags are kept so matching by name works.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Os {
    Linux,
    Macos,
    Windows,
    Other,
}

impl Os {
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            Os::Linux => "linux",
            Os::Macos => "macos",
            Os::Windows => "windows",
            Os::Other => "other",
        }
    }

    /// The OS of the current build.
    #[must_use]
    pub fn current() -> Os {
        if cfg!(target_os = "linux") {
            Os::Linux
        } else if cfg!(target_os = "macos") {
            Os::Macos
        } else if cfg!(target_os = "windows") {
            Os::Windows
        } else {
            Os::Other
        }
    }
}

/// The static, typed state conditionals are matched against.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct State {
    /// The theme of the underlying OS desktop environment.
    pub theme: Theme,
    /// The target OS of the current build.
    pub os: Os,
}

impl Default for State {
    fn default() -> Self {
        Self {
            theme: Theme::Light,
            os: Os::current(),
        }
    }
}

impl State {
    /// Tests the conditional against the state and returns true if it matches.
    #[must_use]
    pub fn match_(&self, cond: &Conditional) -> bool {
        let value = match cond.key {
            Key::Theme => self.theme.as_str(),
            Key::Os => self.os.as_str(),
        };
        match cond.op {
            Op::Eq => value == cond.value,
            Op::Ne => value != cond.value,
        }
    }
}

/// An enum of the available conditional configuration keys.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Key {
    Theme,
    Os,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Op {
    Eq,
    Ne,
}

/// A single conditional that can be true or false.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Conditional {
    pub key: Key,
    pub op: Op,
    pub value: String,
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn conditional_enum_match() {
        let state = State {
            theme: Theme::Dark,
            ..State::default()
        };
        let c = |op, v: &str| Conditional {
            key: Key::Theme,
            op,
            value: v.to_string(),
        };
        assert!(state.match_(&c(Op::Eq, "dark")));
        assert!(!state.match_(&c(Op::Ne, "dark")));
        assert!(state.match_(&c(Op::Ne, "light")));
    }
}
