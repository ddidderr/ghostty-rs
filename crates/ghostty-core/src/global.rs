//! Global process state (port of `global.zig`).
//!
//! The Zig `GlobalState` also owns the allocator and the parsed CLI action;
//! in Rust the allocator is the global one and CLI action dispatch happens in
//! the root binary before this is initialized, so what remains is: logging,
//! signal setup, rlimits and the resources directory.

use std::{io::Write as _, sync::OnceLock};

use ghostty_os::{file::Rlimit, resourcesdir::ResourcesDir};

/// Where logging should go (Zig: `GlobalState.Logging`, a packed struct
/// parsed from `GHOSTTY_LOG`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Logging {
    /// Whether to log to stderr.
    pub stderr: bool,

    /// Whether to log to macOS's unified logging. Enabled by default on
    /// macOS.
    pub macos: bool,
}

impl Default for Logging {
    fn default() -> Self {
        Logging {
            stderr: true,
            macos: cfg!(target_vendor = "apple"),
        }
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
        Logging::parse_with(v, Logging::default())
    }

    /// [`Logging::parse`] with an explicit default (lib mode defaults to no
    /// stderr logging).
    #[must_use]
    pub fn parse_with(v: &str, default: Logging) -> Logging {
        match v {
            "1" | "t" | "T" | "true" => {
                return Logging {
                    stderr: true,
                    macos: true,
                };
            }
            "0" | "f" | "F" | "false" => {
                return Logging {
                    stderr: false,
                    macos: false,
                };
            }
            _ => {}
        }
        let mut out = default;
        for part in v.split(',') {
            let part = part.trim_matches([' ', '\t']);
            let (name, value) = match part.strip_prefix("no-") {
                Some(n) => (n, false),
                None => (part, true),
            };
            match name {
                "stderr" => out.stderr = value,
                "macos" => out.macos = value,
                _ => return default,
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
    init_with(opts, Logging::default())
}

/// [`init`] for the embedded library (`ghostty_init`): Zig's
/// `GlobalState.Logging.stderr` defaults to `build_config.app_runtime !=
/// .none`, i.e. off for the lib artifact.
pub fn init_lib(opts: InitOptions) -> &'static GlobalState {
    init_with(
        opts,
        Logging {
            stderr: false,
            ..Logging::default()
        },
    )
}

fn init_with(opts: InitOptions, default_logging: Logging) -> &'static GlobalState {
    STATE.get_or_init(|| {
        let mut logging = default_logging;

        // If we have an action executing, we disable logging by default
        // since we write to stderr we don't want logs messing up our
        // output.
        if opts.action {
            logging.stderr = false;
        }

        // Env vars are useful for logging too because they are easy to set.
        if let Ok(v) = std::env::var("GHOSTTY_LOG") {
            logging = Logging::parse_with(&v, default_logging);
        }

        init_logger(logging);

        // SIGPIPE is ignored by the Rust runtime already (it sets SIG_IGN
        // before main), which is what Zig's `initSignals` does.

        // As early as possible, initialize our resource limits.
        let rlimits = ResourceLimits::init();

        // Initialize our crash reporting.
        if let Err(err) = ghostty_crash::init(&ghostty_crash::Options {
            release: ghostty_termio::stream_handler::VERSION_STRING.to_owned(),
            ..ghostty_crash::Options::default()
        }) {
            log::warn!("sentry init failed, no crash capture available err={err}");
        }

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
    /// macOS unified logging (`os_log`), only effective on Apple targets.
    macos: bool,
    max: log::LevelFilter,
}

/// The scope shown in log output: the crate name without the `ghostty_`
/// prefix (Zig scopes are `@tagName(scope)`).
fn log_scope(target: &str) -> &str {
    let target = target.strip_prefix("ghostty_").unwrap_or(target);
    target.split("::").next().unwrap_or(target)
}

/// The prefix used for macOS unified logging messages: nothing for the
/// default scope, `scope: ` otherwise.
#[cfg_attr(not(any(target_vendor = "apple", test)), allow(dead_code))]
fn macos_message(scope: &str, args: &std::fmt::Arguments<'_>) -> String {
    if scope.is_empty() || scope == "default" {
        format!("{args}")
    } else {
        format!("{scope}: {args}")
    }
}

/// Bundle id used as the unified logging subsystem
/// (`build_config.bundle_id`).
#[cfg(target_vendor = "apple")]
const BUNDLE_ID: &str = ghostty_os::i18n::BUNDLE_ID;

#[cfg(target_vendor = "apple")]
fn log_macos(record: &log::Record<'_>) {
    // On Mac, we use unified logging. To view this:
    //
    //   sudo log stream --level debug --predicate 'subsystem=="com.mitchellh.ghostty"'
    //
    // macOS logging is thread safe so no need for locks/mutexes.
    //
    // Convert our levels to Mac levels.
    let level = match record.level() {
        log::Level::Debug | log::Level::Trace => oslog::Level::Debug,
        log::Level::Info => oslog::Level::Info,
        log::Level::Warn => oslog::Level::Error,
        log::Level::Error => oslog::Level::Fault,
    };
    let scope = log_scope(record.target());

    // Initialize a logger. This is slow to do on every operation but we
    // shouldn't be logging too much.
    let logger = oslog::OsLog::new(BUNDLE_ID, scope);
    logger.with_level(level, &macos_message(scope, record.args()));
}

impl log::Log for Logger {
    fn enabled(&self, metadata: &log::Metadata<'_>) -> bool {
        (self.stderr || self.macos) && metadata.level() <= self.max
    }

    fn log(&self, record: &log::Record<'_>) {
        if !self.enabled(record.metadata()) {
            return;
        }

        #[cfg(target_vendor = "apple")]
        if self.macos {
            log_macos(record);
        }

        // skip if we are not logging to stderr
        if !self.stderr {
            return;
        }

        let level = match record.level() {
            log::Level::Error => "error",
            log::Level::Warn => "warning",
            log::Level::Info => "info",
            log::Level::Debug | log::Level::Trace => "debug",
        };
        let scope = log_scope(record.target());
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
        macos: logging.macos,
        max,
    });
    if log::set_boxed_logger(logger).is_ok() {
        log::set_max_level(max);
    }
}

/// Cleans up the global state. This doesn't _need_ to be called (Zig:
/// `GlobalState.deinit`); it deactivates crash capture ("flush crash logs").
pub fn deinit() {
    ghostty_crash::deinit();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn macos_log_message_prefix() {
        assert_eq!(
            macos_message("font", &format_args!("hi {}", 1)),
            "font: hi 1"
        );
        assert_eq!(macos_message("default", &format_args!("hi")), "hi");
        assert_eq!(log_scope("ghostty_font::collection"), "font");
        assert_eq!(log_scope("other::x"), "other");
    }

    #[test]
    fn logging_parse() {
        assert!(Logging::parse("stderr").stderr);
        assert!(!Logging::parse("no-stderr").stderr);
        // Unmentioned fields keep their default (stderr on).
        assert!(Logging::parse("macos").stderr);
        assert!(!Logging::parse("no-macos").macos);
        assert!(Logging::parse("macos").macos);
        assert!(Logging::parse("true").macos);
        assert!(!Logging::parse("0").macos);
        assert!(!Logging::parse("false").stderr);
        assert!(!Logging::parse("0").stderr);
        assert!(Logging::parse("true").stderr);
        // Empty / invalid values yield the default.
        assert!(Logging::parse("").stderr);
        assert!(Logging::parse("stderr,").stderr);
        assert_eq!(Logging::parse("garbage"), Logging::default());
    }
}
