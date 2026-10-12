//! Global process state (port of `global.zig`).
//!
//! The Zig `GlobalState` also owns the allocator and the parsed CLI action;
//! in Rust the allocator is the global one and CLI action dispatch happens in
//! the root binary before this is initialized, so what remains is: logging,
//! signal setup, rlimits and the resources directory.

use std::io::Write as _;
use std::sync::OnceLock;

use ghostty_os::file::Rlimit;
use ghostty_os::resourcesdir::ResourcesDir;

/// Where logging should go (Zig: `GlobalState.Logging`, a packed struct
/// parsed from `GHOSTTY_LOG`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Logging {
    /// Whether to log to stderr.
    pub stderr: bool,
}

impl Default for Logging {
    fn default() -> Self {
        Logging { stderr: true }
    }
}

impl Logging {
    /// Parse the `GHOSTTY_LOG` value (Zig: `cli.args.parsePackedStruct`):
    /// a standalone boolean (`1/t/T/true/0/f/F/false`) sets every field,
    /// otherwise a comma separated list of field names where a `no-` prefix
    /// disables. Fields not mentioned keep their default (stderr on). Any
    /// invalid value (including an empty part) yields the default.
    #[must_use]
    pub fn parse(v: &str) -> Logging {
        match v {
            "1" | "t" | "T" | "true" => return Logging { stderr: true },
            "0" | "f" | "F" | "false" => return Logging { stderr: false },
            _ => {}
        }
        let mut out = Logging::default();
        for part in v.split(',') {
            let part = part.trim_matches([' ', '\t']);
            let (name, value) = match part.strip_prefix("no-") {
                Some(n) => (n, false),
                None => (part, true),
            };
            match name {
                "stderr" => out.stderr = value,
                // The only other field (macOS unified logging) is a no-op.
                "macos" => {}
                _ => return Logging::default(),
            }
        }
        out
    }
}

/// Maintains the Unix resource limits that we set for our process. This can
/// be used to restore the limits to their original values.
#[derive(Debug, Clone, Copy, Default)]
pub struct ResourceLimits {
    pub nofile: Option<Rlimit>,
}

impl ResourceLimits {
    #[must_use]
    pub fn init() -> ResourceLimits {
        ResourceLimits {
            // Maximize the number of file descriptors we can have open
            // because we can consume a lot of them if we make many terminals.
            nofile: ghostty_os::file::fix_max_files(),
        }
    }

    /// Restore the original limits (the child does this after fork).
    pub fn restore(&self) {
        if let Some(lim) = self.nofile {
            ghostty_os::file::restore_max_files(lim);
        }
    }
}

/// The global state (initialized once by [`init`]).
#[derive(Debug)]
pub struct GlobalState {
    pub logging: Logging,
    pub rlimits: ResourceLimits,

    /// The app resources directory, equivalent to zig-out/share when we
    /// build from source. Empty if we can't detect it.
    pub resources_dir: ResourcesDir,
}

static STATE: OnceLock<GlobalState> = OnceLock::new();

/// Options for [`init`].
#[derive(Clone, Copy, Default)]
pub struct InitOptions {
    /// A CLI action is being executed: disable stderr logging by default
    /// since the action writes to stderr/stdout itself.
    pub action: bool,

    /// The runtime-specific resources directory discovery (Zig:
    /// `apprt.runtime.resourcesDir`, e.g. GTK's Flatpak host path lookup).
    /// `None` uses the generic `ghostty_os::resourcesdir::resources_dir`.
    pub resources_dir: Option<fn() -> std::io::Result<ResourcesDir>>,
}

/// The global state. If [`init`] was never called, a default state (no
/// resources dir, no rlimit changes) is created.
pub fn state() -> &'static GlobalState {
    STATE.get_or_init(|| GlobalState {
        logging: Logging::default(),
        rlimits: ResourceLimits::default(),
        resources_dir: ResourcesDir::default(),
    })
}

/// Initialize the global state: logging, rlimits, resources dir and i18n.
/// Call once, early (Zig: `GlobalState.init`).
pub fn init(opts: InitOptions) -> &'static GlobalState {
    STATE.get_or_init(|| {
        let mut logging = Logging::default();

        // If we have an action executing, we disable logging by default
        // since we write to stderr we don't want logs messing up our
        // output.
        if opts.action {
            logging.stderr = false;
        }

        // Env vars are useful for logging too because they are easy to set.
        if let Ok(v) = std::env::var("GHOSTTY_LOG") {
            logging = Logging::parse(&v);
        }

        init_logger(logging);

        // SIGPIPE is ignored by the Rust runtime already (it sets SIG_IGN
        // before main), which is what Zig's `initSignals` does.

        // As early as possible, initialize our resource limits.
        let rlimits = ResourceLimits::init();

        // Find our resources directory once for the app so every launch
        // hereafter can use this cached value.
        let discover = opts
            .resources_dir
            .unwrap_or(ghostty_os::resourcesdir::resources_dir);
        let resources_dir = match discover() {
            Ok(d) => d,
            Err(err) => {
                log::warn!("failed to find resources dir err={err}");
                ResourcesDir::default()
            }
        };

        // Setup i18n
        if let Some(v) = resources_dir.app()
            && let Err(err) = ghostty_os::i18n::init(v)
        {
            log::warn!("failed to init i18n, translations will not be available err={err}");
        }

        GlobalState {
            logging,
            rlimits,
            resources_dir,
        }
    })
}

/// The `log` backend, formatted like Zig's `logFn`:
/// `info(scope): message` on stderr.
struct Logger {
    stderr: bool,
    max: log::LevelFilter,
}

impl log::Log for Logger {
    fn enabled(&self, metadata: &log::Metadata<'_>) -> bool {
        self.stderr && metadata.level() <= self.max
    }

    fn log(&self, record: &log::Record<'_>) {
        if !self.enabled(record.metadata()) {
            return;
        }

        let level = match record.level() {
            log::Level::Error => "error",
            log::Level::Warn => "warning",
            log::Level::Info => "info",
            log::Level::Debug | log::Level::Trace => "debug",
        };
        let target = record.target();
        let scope = target
            .strip_prefix("ghostty_")
            .unwrap_or(target)
            .split("::")
            .next()
            .unwrap_or(target);
        let _ = writeln!(
            std::io::stderr().lock(),
            "{level}({scope}): {}",
            record.args()
        );
    }

    fn flush(&self) {}
}

fn init_logger(logging: Logging) {
    // Our log level is always at least info in every build mode; debug
    // builds (or RUST_LOG=debug) also show debug messages.
    let max = match std::env::var("RUST_LOG").ok().as_deref() {
        Some("trace") => log::LevelFilter::Trace,
        Some("debug") => log::LevelFilter::Debug,
        Some("warn") => log::LevelFilter::Warn,
        Some("error") => log::LevelFilter::Error,
        Some("info") => log::LevelFilter::Info,
        _ if cfg!(debug_assertions) => log::LevelFilter::Debug,
        _ => log::LevelFilter::Info,
    };
    let logger = Box::new(Logger {
        stderr: logging.stderr,
        max,
    });
    if log::set_boxed_logger(logger).is_ok() {
        log::set_max_level(max);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn logging_parse() {
        assert!(Logging::parse("stderr").stderr);
        assert!(!Logging::parse("no-stderr").stderr);
        // Unmentioned fields keep their default (stderr on).
        assert!(Logging::parse("macos").stderr);
        assert!(!Logging::parse("false").stderr);
        assert!(!Logging::parse("0").stderr);
        assert!(Logging::parse("true").stderr);
        // Empty / invalid values yield the default.
        assert!(Logging::parse("").stderr);
        assert!(Logging::parse("stderr,").stderr);
        assert_eq!(Logging::parse("garbage"), Logging::default());
    }
}
