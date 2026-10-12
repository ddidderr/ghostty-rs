//! Shell integration setup. Port of `termio/shell_integration.zig`.
//!
//! The shell scripts live in `resources/shell-integration/` at the workspace
//! root (copied verbatim from `src/shell-integration/`).

use std::path::Path;

use ghostty_os::{env::prepend_env_os, homedir, osbytes::EncodedOs, shell::ShellCommandBuilder};

/// The environment passed to the child (Zig `std.process.EnvMap`).
pub type EnvMap = ghostty_os::EnvMap;

/// Shell types we support
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Shell {
    Bash,
    Elvish,
    Fish,
    Nushell,
    Zsh,
}

impl Shell {
    pub const ALL: [Shell; 5] = [
        Shell::Bash,
        Shell::Elvish,
        Shell::Fish,
        Shell::Nushell,
        Shell::Zsh,
    ];

    /// The name of the shell (Zig `@tagName`), also the directory name inside
    /// `shell-integration/`.
    pub fn tag_name(self) -> &'static str {
        match self {
            Shell::Bash => "bash",
            Shell::Elvish => "elvish",
            Shell::Fish => "fish",
            Shell::Nushell => "nushell",
            Shell::Zsh => "zsh",
        }
    }
}

pub use ghostty_config::{command::Command, types_b::ShellIntegrationFeatures};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("unable to determine home directory: {0}")]
    Home(#[from] homedir::Error),
}

/// The result of setting up a shell integration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShellIntegration {
    /// The successfully-integrated shell.
    pub shell: Shell,

    /// The command to use to start the shell with the integration.
    /// In most cases this is identical to the command given but for
    /// bash in particular it may be different.
    pub command: Command,
}

/// `<resource_dir>/<rel>` as an OS string. `resource_dir` carries the
/// reversible config-string encoding (it may hold non-UTF-8 bytes), so it is
/// decoded before reaching the filesystem or the child environment.
fn resource_path(resource_dir: &EncodedOs, rel: &str) -> std::ffi::OsString {
    let mut p = resource_dir.to_os();
    p.push("/");
    p.push(rel);
    p
}

/// Set up the command execution environment for automatic
/// integrated shell integration and return a ShellIntegration
/// struct describing the integration.  If integration fails
/// (shell type couldn't be detected, etc.), this will return None.
pub fn setup(
    resource_dir: &EncodedOs,
    command: &Command,
    env: &mut EnvMap,
    force_shell: Option<Shell>,
) -> Result<Option<ShellIntegration>, Error> {
    let Some(shell) = force_shell.or_else(|| detect_shell(command)) else {
        return Ok(None);
    };

    let new_command = match shell {
        Shell::Bash => setup_bash(command, resource_dir, env)?,
        Shell::Nushell => setup_nushell(command, resource_dir, env),
        Shell::Zsh => setup_zsh(command, resource_dir, env),
        Shell::Elvish | Shell::Fish => {
            if !setup_xdg_data_dirs(resource_dir, env) {
                return Ok(None);
            }
            Some(command.clone())
        }
    };

    Ok(new_command.map(|command| ShellIntegration { shell, command }))
}

fn basename(path: &str) -> &str {
    // std.fs.path.basename: strips trailing separators.
    let trimmed = path.trim_end_matches('/');
    match trimmed.rfind('/') {
        Some(i) => &trimmed[i + 1..],
        None => trimmed,
    }
}

fn detect_shell(command: &Command) -> Option<Shell> {
    detect_shell_for(command, cfg!(target_vendor = "apple"))
}

/// [`detect_shell`] for an explicit platform (`darwin == isDarwin()`).
fn detect_shell_for(command: &Command, darwin: bool) -> Option<Shell> {
    let mut it = command.arg_iterator();
    let arg0 = it.next()?;
    let exe = basename(&arg0);

    match exe {
        "bash" => {
            // Apple distributes their own patched version of Bash 3.2
            // on macOS that disables the ENV-based POSIX startup path.
            // This means we're unable to perform our automatic shell
            // integration sequence in this specific environment.
            //
            // If we're running "/bin/bash" on Darwin, we can assume
            // we're using Apple's Bash because /bin is non-writable
            // on modern macOS due to System Integrity Protection.
            if darwin && arg0 == "/bin/bash" {
                return None;
            }
            Some(Shell::Bash)
        }
        "elvish" => Some(Shell::Elvish),
        "fish" => Some(Shell::Fish),
        "nu" => Some(Shell::Nushell),
        "zsh" => Some(Shell::Zsh),
        _ => None,
    }
}

/// Set up the shell integration features environment variable.
pub fn setup_features(env: &mut EnvMap, features: ShellIntegrationFeatures, cursor_blink: bool) {
    // Sorted (case-insensitively) so that the output is deterministic.
    let fields: [(&str, bool); 6] = [
        ("cursor", features.cursor),
        ("path", features.path),
        ("ssh-env", features.ssh_env),
        ("ssh-terminfo", features.ssh_terminfo),
        ("sudo", features.sudo),
        ("title", features.title),
    ];

    let mut buf = String::new();
    for (name, enabled) in fields {
        if enabled {
            if !buf.is_empty() {
                buf.push(',');
            }
            buf.push_str(name);

            if name == "cursor" {
                buf.push_str(if cursor_blink { ":blink" } else { ":steady" });
            }
        }
    }

    if !buf.is_empty() {
        env.insert("GHOSTTY_SHELL_FEATURES", buf);
    }
}

/// Setup the bash automatic shell integration. This works by
/// starting bash in POSIX mode and using the ENV environment
/// variable to load our bash integration script. This prevents
/// bash from loading its normal startup files, which becomes
/// our script's responsibility (along with disabling POSIX
/// mode).
///
/// This returns a new shell command string that enables the integration
/// or None if integration failed.
fn setup_bash(
    command: &Command,
    resource_dir: &EncodedOs,
    env: &mut EnvMap,
) -> Result<Option<Command>, Error> {
    let mut cmd = ShellCommandBuilder::new();

    // Iterator that yields each argument in the original command line.
    let mut iter = command.arg_iterator();

    // Start accumulating arguments with the executable and initial flags.
    match iter.next() {
        Some(exe) => cmd.append_arg(&exe),
        None => return Ok(None),
    }
    cmd.append_arg("--posix");

    // Stores the list of intercepted command line flags that will be passed
    // to our shell integration script: --norc --noprofile
    // We always include at least "1" so the script can differentiate between
    // being manually sourced or automatically injected (from here).
    let mut inject = String::from("1");

    // Walk through the rest of the given arguments. If we see an option that
    // would require complex or unsupported integration behavior, we bail out
    // and skip loading our shell integration. Users can still manually source
    // the shell integration script.
    //
    // Unsupported options:
    //  -c          -c is always non-interactive
    //  --posix     POSIX mode (a la /bin/sh)
    let mut rcfile: Option<String> = None;
    while let Some(arg) = iter.next() {
        if arg == "--posix" {
            return Ok(None);
        } else if arg == "--norc" {
            inject.push_str(" --norc");
        } else if arg == "--noprofile" {
            inject.push_str(" --noprofile");
        } else if arg == "--rcfile" || arg == "--init-file" {
            rcfile = iter.next();
        } else if arg.len() > 1 && arg.starts_with('-') && !arg[1..].starts_with('-') {
            // '-c command' is always non-interactive
            if arg.contains('c') {
                return Ok(None);
            }
            cmd.append_arg(&arg);
        } else if arg == "-" || arg == "--" {
            // All remaining arguments should be passed directly to the shell
            // command. We shouldn't perform any further option processing.
            cmd.append_arg(&arg);
            for remaining in iter.by_ref() {
                cmd.append_arg(&remaining);
            }
            break;
        } else {
            cmd.append_arg(&arg);
        }
    }

    // Preserve an existing ENV value. We're about to overwrite it.
    if let Some(v) = env.get("ENV").map(std::ffi::OsStr::to_owned) {
        env.insert("GHOSTTY_BASH_ENV", v);
    }

    // Set our new ENV to point to our integration script.
    let script_path = resource_path(resource_dir, "shell-integration/bash/ghostty.bash");
    match std::fs::File::open(&script_path) {
        Ok(_) => {
            env.insert("ENV", script_path);
        }
        Err(err) => {
            log::warn!("unable to open {}: {err}", script_path.to_string_lossy());
            env.remove("GHOSTTY_BASH_ENV");
            return Ok(None);
        }
    }

    env.insert("GHOSTTY_BASH_INJECT", inject);
    if let Some(v) = rcfile {
        env.insert("GHOSTTY_BASH_RCFILE", v);
    }

    // In POSIX mode, HISTFILE defaults to ~/.sh_history, so unless we're
    // staying in POSIX mode (--posix), change it back to ~/.bash_history.
    if !env.contains_key("HISTFILE")
        && let Some(home) = homedir::home()?
    {
        // `home` is an encoded OS string: build the real OS value.
        let mut histfile = home.to_os();
        histfile.push("/.bash_history");
        env.insert("HISTFILE", histfile);
        env.insert("GHOSTTY_BASH_UNEXPORT_HISTFILE", "1");
    }

    // Return a copy of our modified command line to use as the shell command.
    Ok(Some(Command::Shell(cmd.written().to_owned())))
}

/// Setup automatic shell integration for shells that include
/// their modules from paths in `XDG_DATA_DIRS` env variable.
///
/// The shell-integration path is prepended to `XDG_DATA_DIRS`.
/// It is also saved in the `GHOSTTY_SHELL_INTEGRATION_XDG_DIR` variable
/// so that the shell can refer to it and safely remove this directory
/// from `XDG_DATA_DIRS` when integration is complete.
fn setup_xdg_data_dirs(resource_dir: &EncodedOs, env: &mut EnvMap) -> bool {
    // Get our path to the shell integration directory.
    let integ_path = resource_path(resource_dir, "shell-integration");
    if let Err(err) = std::fs::read_dir(&integ_path) {
        log::warn!("unable to open {}: {err}", integ_path.to_string_lossy());
        return false;
    }

    // Set an env var so we can remove this from XDG_DATA_DIRS later.
    // This happens in the shell integration config itself. We do this
    // so that our modifications don't interfere with other commands.
    env.insert("GHOSTTY_SHELL_INTEGRATION_XDG_DIR", integ_path.clone());

    // If no XDG_DATA_DIRS set use the default value as specified.
    // This ensures that the default directories aren't lost by setting
    // our desired integration dir directly. See #2711.
    // <https://specifications.freedesktop.org/basedir-spec/0.6/#variables>
    let key = "XDG_DATA_DIRS";
    let current = env
        .get(key)
        .unwrap_or_else(|| std::ffi::OsStr::new("/usr/local/share:/usr/share"));
    let new = prepend_env_os(current, &integ_path);
    env.insert(key, new);

    true
}

/// Set up automatic Nushell shell integration. This works by adding our
/// shell resource directory to the `XDG_DATA_DIRS` environment variable,
/// which Nushell will use to load `nushell/vendor/autoload/ghostty.nu`.
///
/// We then add `--execute 'use ghostty ...'` to the nu command line to
/// automatically enable our shell features.
fn setup_nushell(command: &Command, resource_dir: &EncodedOs, env: &mut EnvMap) -> Option<Command> {
    // Add our XDG_DATA_DIRS entry (for nushell/vendor/autoload/). This
    // makes our 'ghostty' module automatically available, even if any
    // of the later checks abort the rest of our automatic integration.
    if !setup_xdg_data_dirs(resource_dir, env) {
        return None;
    }

    let mut cmd = ShellCommandBuilder::new();
    let mut iter = command.arg_iterator();

    // Start accumulating arguments with the executable and initial flags.
    cmd.append_arg(&iter.next()?);

    // Tell nu to immediately "use" all of the exported functions in our
    // 'ghostty' module.
    //
    // We can consider making this more specific based on the set of
    // enabled shell features (e.g. `use ghostty sudo`). At the moment,
    // shell features are all runtime-guarded in the nushell script.
    cmd.append_arg("--execute 'use ghostty *'");

    // Walk through the rest of the given arguments. If we see an option that
    // would require complex or unsupported integration behavior, we bail out
    // and skip loading our shell integration. Users can still manually source
    // the shell integration module.
    //
    // Unsupported options:
    //  -c / --command      -c is always non-interactive
    //  --lsp               --lsp starts the language server
    while let Some(arg) = iter.next() {
        if arg == "--command" || arg == "--lsp" {
            return None;
        } else if arg.len() > 1 && arg.starts_with('-') && !arg[1..].starts_with('-') {
            if arg.contains('c') {
                return None;
            }
            cmd.append_arg(&arg);
        } else if arg == "-" || arg == "--" {
            // All remaining arguments should be passed directly to the shell
            // command. We shouldn't perform any further option processing.
            cmd.append_arg(&arg);
            for remaining in iter.by_ref() {
                cmd.append_arg(&remaining);
            }
            break;
        } else {
            cmd.append_arg(&arg);
        }
    }

    // Return a copy of our modified command line to use as the shell command.
    Some(Command::Shell(cmd.written().to_owned()))
}

/// Setup the zsh automatic shell integration. This works by setting
/// ZDOTDIR to our resources dir so that zsh will load our config. This
/// config then loads the true user config.
fn setup_zsh(command: &Command, resource_dir: &EncodedOs, env: &mut EnvMap) -> Option<Command> {
    // Preserve an existing ZDOTDIR value. We're about to overwrite it.
    if let Some(old) = env.get("ZDOTDIR").map(std::ffi::OsStr::to_owned) {
        env.insert("GHOSTTY_ZSH_ZDOTDIR", old);
    }

    // Set our new ZDOTDIR to point to our shell resource directory.
    let integ_path = resource_path(resource_dir, "shell-integration/zsh");
    if !Path::new(&integ_path).is_dir() {
        // Mirrors openDirAbsolute failing (missing or not a directory).
        log::warn!("unable to open {}", integ_path.to_string_lossy());
        return None;
    }
    env.insert("ZDOTDIR", integ_path);

    Some(command.clone())
}

#[cfg(test)]
mod tests {
    const NONE_FEATURES: ShellIntegrationFeatures = ShellIntegrationFeatures {
        cursor: false,
        sudo: false,
        title: false,
        ssh_env: false,
        ssh_terminfo: false,
        path: false,
    };

    use ghostty_os::TempDir;

    use super::*;

    /// Test helper that creates a temporary resources directory with shell
    /// integration paths.
    struct TmpResourcesDir {
        _tmp: TempDir,
        path: String,
        shell_path: String,
    }

    impl TmpResourcesDir {
        fn enc(&self) -> EncodedOs {
            EncodedOs::from_plain_text(&self.path)
        }

        fn new(shell: Shell) -> Self {
            let tmp = TempDir::init().unwrap();
            let path = std::fs::canonicalize(tmp.path())
                .unwrap()
                .to_string_lossy()
                .into_owned();
            let rel = format!("shell-integration/{}", shell.tag_name());
            std::fs::create_dir_all(format!("{path}/{rel}")).unwrap();
            let shell_path = format!("{path}/{rel}");
            if shell == Shell::Bash {
                std::fs::write(format!("{path}/shell-integration/bash/ghostty.bash"), "").unwrap();
            }
            Self {
                _tmp: tmp,
                path,
                shell_path,
            }
        }
    }

    fn sh(s: &str) -> Command {
        Command::Shell(s.to_owned())
    }

    fn shell_str(c: Option<&Command>) -> &str {
        match c.unwrap() {
            Command::Shell(s) => s,
            Command::Direct(_) => panic!("expected shell"),
        }
    }

    #[test]
    fn force_shell() {
        let mut env = EnvMap::new();
        for shell in Shell::ALL {
            let res = TmpResourcesDir::new(shell);
            let result = setup(&res.enc(), &sh("sh"), &mut env, Some(shell)).unwrap();
            assert_eq!(shell, result.unwrap().shell);
        }
    }

    #[test]
    fn shell_integration_failure() {
        let mut env = EnvMap::new();
        let result = setup(
            &EncodedOs::from_encoded("/nonexistent"),
            &sh("sh"),
            &mut env,
            None,
        )
        .unwrap();
        assert!(result.is_none());
        assert_eq!(0, env.len());
    }

    #[test]
    fn detect_shell_cases() {
        assert_eq!(detect_shell(&sh("sh")), None);
        assert_eq!(detect_shell(&sh("bash")), Some(Shell::Bash));
        assert_eq!(detect_shell(&sh("elvish")), Some(Shell::Elvish));
        assert_eq!(detect_shell(&sh("fish")), Some(Shell::Fish));
        assert_eq!(detect_shell(&sh("nu")), Some(Shell::Nushell));
        assert_eq!(detect_shell(&sh("zsh")), Some(Shell::Zsh));

        // Apple's patched /bin/bash can't use the ENV-based integration.
        assert_eq!(detect_shell_for(&sh("/bin/bash"), true), None);
        assert_eq!(detect_shell_for(&sh("/bin/bash"), false), Some(Shell::Bash));
        assert_eq!(
            detect_shell_for(&sh("/opt/homebrew/bin/bash"), true),
            Some(Shell::Bash)
        );

        assert_eq!(detect_shell(&sh("bash -c 'command'")), Some(Shell::Bash));
        assert_eq!(detect_shell(&sh("\"/a b/bash\"")), Some(Shell::Bash));
        assert_eq!(detect_shell(&sh("/usr/bin/zsh")), Some(Shell::Zsh));
        assert_eq!(
            detect_shell(&Command::Direct(vec!["/bin/fish".into()])),
            Some(Shell::Fish)
        );
        assert_eq!(detect_shell(&sh("")), None);
    }

    #[test]
    fn arg_iterator_shell_and_direct() {
        let v: Vec<_> = sh("echo hello world").arg_iterator().collect();
        assert_eq!(v, ["echo", "hello", "world"]);
        let v: Vec<_> = Command::Direct(vec!["echo".into(), "hello world".into()])
            .arg_iterator()
            .collect();
        assert_eq!(v, ["echo", "hello world"]);
        let v: Vec<_> = sh("a  \"b c\"\td\\\"e").arg_iterator().collect();
        assert_eq!(v, ["a", "b c", "d\"e"]);
    }

    #[test]
    fn setup_features_all_variants() {
        // all features enabled
        let mut env = EnvMap::new();
        setup_features(
            &mut env,
            ShellIntegrationFeatures {
                cursor: true,
                sudo: true,
                title: true,
                ssh_env: true,
                ssh_terminfo: true,
                path: true,
            },
            true,
        );
        assert_eq!(
            "cursor:blink,path,ssh-env,ssh-terminfo,sudo,title",
            env.get("GHOSTTY_SHELL_FEATURES").unwrap().to_str().unwrap()
        );

        // all features disabled
        let mut env = EnvMap::new();
        setup_features(&mut env, NONE_FEATURES, true);
        assert!(!env.contains_key("GHOSTTY_SHELL_FEATURES"));

        // mixed features
        let mut env = EnvMap::new();
        setup_features(
            &mut env,
            ShellIntegrationFeatures {
                sudo: true,
                ssh_env: true,
                ..NONE_FEATURES
            },
            true,
        );
        assert_eq!(
            "ssh-env,sudo",
            env.get("GHOSTTY_SHELL_FEATURES").unwrap().to_str().unwrap()
        );

        // blinking cursor
        let only_cursor = ShellIntegrationFeatures {
            cursor: true,
            ..NONE_FEATURES
        };
        let mut env = EnvMap::new();
        setup_features(&mut env, only_cursor, true);
        assert_eq!(
            "cursor:blink",
            env.get("GHOSTTY_SHELL_FEATURES").unwrap().to_str().unwrap()
        );

        // steady cursor
        let mut env = EnvMap::new();
        setup_features(&mut env, only_cursor, false);
        assert_eq!(
            "cursor:steady",
            env.get("GHOSTTY_SHELL_FEATURES").unwrap().to_str().unwrap()
        );
    }

    #[test]
    fn default_features() {
        let mut env = EnvMap::new();
        setup_features(&mut env, ShellIntegrationFeatures::default(), true);
        assert_eq!(
            "cursor:blink,path,title",
            env.get("GHOSTTY_SHELL_FEATURES").unwrap().to_str().unwrap()
        );
    }

    #[test]
    fn bash() {
        let res = TmpResourcesDir::new(Shell::Bash);
        let mut env = EnvMap::new();

        let command = setup_bash(&sh("bash"), &res.enc(), &mut env).unwrap();
        assert_eq!("bash --posix", shell_str(command.as_ref()));
        assert_eq!(
            "1",
            env.get("GHOSTTY_BASH_INJECT").unwrap().to_str().unwrap()
        );
        assert_eq!(
            format!("{}/ghostty.bash", res.shell_path),
            env.get("ENV").unwrap().to_str().unwrap()
        );
    }

    #[test]
    fn bash_unsupported_options() {
        let res = TmpResourcesDir::new(Shell::Bash);
        for cmdline in [
            "bash --posix",
            "bash --rcfile script.sh --posix",
            "bash --init-file script.sh --posix",
            "bash -c script.sh",
            "bash -ic script.sh",
        ] {
            let mut env = EnvMap::new();
            assert!(
                setup_bash(&sh(cmdline), &res.enc(), &mut env)
                    .unwrap()
                    .is_none()
            );
            assert_eq!(0, env.len());
        }
    }

    #[test]
    fn bash_inject_flags() {
        let res = TmpResourcesDir::new(Shell::Bash);

        let mut env = EnvMap::new();
        let command = setup_bash(&sh("bash --norc"), &res.enc(), &mut env).unwrap();
        assert_eq!("bash --posix", shell_str(command.as_ref()));
        assert_eq!(
            "1 --norc",
            env.get("GHOSTTY_BASH_INJECT").unwrap().to_str().unwrap()
        );

        let mut env = EnvMap::new();
        let command = setup_bash(&sh("bash --noprofile"), &res.enc(), &mut env).unwrap();
        assert_eq!("bash --posix", shell_str(command.as_ref()));
        assert_eq!(
            "1 --noprofile",
            env.get("GHOSTTY_BASH_INJECT").unwrap().to_str().unwrap()
        );
    }

    #[test]
    fn bash_rcfile() {
        let res = TmpResourcesDir::new(Shell::Bash);
        let mut env = EnvMap::new();

        let command = setup_bash(&sh("bash --rcfile profile.sh"), &res.enc(), &mut env).unwrap();
        assert_eq!("bash --posix", shell_str(command.as_ref()));
        assert_eq!(
            "profile.sh",
            env.get("GHOSTTY_BASH_RCFILE").unwrap().to_str().unwrap()
        );

        let command = setup_bash(&sh("bash --init-file profile.sh"), &res.enc(), &mut env).unwrap();
        assert_eq!("bash --posix", shell_str(command.as_ref()));
        assert_eq!(
            "profile.sh",
            env.get("GHOSTTY_BASH_RCFILE").unwrap().to_str().unwrap()
        );
    }

    #[test]
    fn bash_histfile() {
        let res = TmpResourcesDir::new(Shell::Bash);

        // HISTFILE unset (needs a home dir; set HOME so it is deterministic)
        temp_env::with_var("HOME", Some("/home/test"), || {
            let mut env = EnvMap::new();
            setup_bash(&sh("bash"), &res.enc(), &mut env).unwrap();
            assert!(
                env.get("HISTFILE")
                    .unwrap()
                    .to_str()
                    .unwrap()
                    .ends_with(".bash_history")
            );
            assert_eq!(
                "1",
                env.get("GHOSTTY_BASH_UNEXPORT_HISTFILE")
                    .unwrap()
                    .to_str()
                    .unwrap()
            );
        });

        // HISTFILE set
        let mut env = EnvMap::new();
        env.insert("HISTFILE", "my_history");
        setup_bash(&sh("bash"), &res.enc(), &mut env).unwrap();
        assert_eq!("my_history", env.get("HISTFILE").unwrap().to_str().unwrap());
        assert!(!env.contains_key("GHOSTTY_BASH_UNEXPORT_HISTFILE"));
    }

    #[test]
    fn bash_env() {
        let res = TmpResourcesDir::new(Shell::Bash);
        let mut env = EnvMap::new();
        env.insert("ENV", "env.sh");

        setup_bash(&sh("bash"), &res.enc(), &mut env).unwrap();
        assert_eq!(
            "env.sh",
            env.get("GHOSTTY_BASH_ENV").unwrap().to_str().unwrap()
        );
        assert_eq!(
            format!("{}/ghostty.bash", res.shell_path),
            env.get("ENV").unwrap().to_str().unwrap()
        );
    }

    #[test]
    fn bash_additional_arguments() {
        let res = TmpResourcesDir::new(Shell::Bash);
        let mut env = EnvMap::new();

        let command = setup_bash(&sh("bash - --arg file1 file2"), &res.enc(), &mut env).unwrap();
        assert_eq!(
            "bash --posix - --arg file1 file2",
            shell_str(command.as_ref())
        );

        let command = setup_bash(&sh("bash -- --arg file1 file2"), &res.enc(), &mut env).unwrap();
        assert_eq!(
            "bash --posix -- --arg file1 file2",
            shell_str(command.as_ref())
        );
    }

    #[test]
    fn bash_missing_resources() {
        let tmp = TempDir::init().unwrap();
        let dir = std::fs::canonicalize(tmp.path()).unwrap();
        let mut env = EnvMap::new();
        assert!(
            setup_bash(&sh("bash"), &EncodedOs::from_path(&dir), &mut env)
                .unwrap()
                .is_none()
        );
        assert_eq!(0, env.len());
    }

    #[test]
    fn xdg_empty_xdg_data_dirs() {
        let res = TmpResourcesDir::new(Shell::Fish);
        let mut env = EnvMap::new();

        assert!(setup_xdg_data_dirs(&res.enc(), &mut env));
        assert_eq!(
            format!("{}/shell-integration", res.path),
            env.get("GHOSTTY_SHELL_INTEGRATION_XDG_DIR")
                .unwrap()
                .to_str()
                .unwrap()
        );
        assert_eq!(
            format!("{}/shell-integration:/usr/local/share:/usr/share", res.path),
            env.get("XDG_DATA_DIRS").unwrap().to_str().unwrap()
        );
    }

    #[test]
    fn xdg_existing_xdg_data_dirs() {
        let res = TmpResourcesDir::new(Shell::Fish);
        let mut env = EnvMap::new();
        env.insert("XDG_DATA_DIRS", "/opt/share");

        assert!(setup_xdg_data_dirs(&res.enc(), &mut env));
        assert_eq!(
            format!("{}/shell-integration", res.path),
            env.get("GHOSTTY_SHELL_INTEGRATION_XDG_DIR")
                .unwrap()
                .to_str()
                .unwrap()
        );
        assert_eq!(
            format!("{}/shell-integration:/opt/share", res.path),
            env.get("XDG_DATA_DIRS").unwrap().to_str().unwrap()
        );
    }

    #[test]
    fn xdg_missing_resources() {
        let tmp = TempDir::init().unwrap();
        let dir = std::fs::canonicalize(tmp.path()).unwrap();
        let mut env = EnvMap::new();
        assert!(!setup_xdg_data_dirs(&EncodedOs::from_path(&dir), &mut env));
        assert_eq!(0, env.len());
    }

    #[test]
    fn nushell() {
        let res = TmpResourcesDir::new(Shell::Nushell);
        let mut env = EnvMap::new();

        let command = setup_nushell(&sh("nu"), &res.enc(), &mut env);
        assert_eq!("nu --execute 'use ghostty *'", shell_str(command.as_ref()));
        assert_eq!(
            format!("{}/shell-integration", res.path),
            env.get("GHOSTTY_SHELL_INTEGRATION_XDG_DIR")
                .unwrap()
                .to_str()
                .unwrap()
        );
        assert!(
            env.get("XDG_DATA_DIRS")
                .unwrap()
                .to_str()
                .unwrap()
                .starts_with(&format!("{}/shell-integration", res.path))
        );
    }

    #[test]
    fn nushell_unsupported_options() {
        let res = TmpResourcesDir::new(Shell::Nushell);
        for cmdline in [
            "nu --command exit",
            "nu --lsp",
            "nu -c script.sh",
            "nu -ic script.sh",
        ] {
            let mut env = EnvMap::new();
            assert!(setup_nushell(&sh(cmdline), &res.enc(), &mut env).is_none());
            assert!(env.contains_key("XDG_DATA_DIRS"));
            assert!(env.contains_key("GHOSTTY_SHELL_INTEGRATION_XDG_DIR"));
        }
    }

    #[test]
    fn nushell_missing_resources() {
        let tmp = TempDir::init().unwrap();
        let dir = std::fs::canonicalize(tmp.path()).unwrap();
        let mut env = EnvMap::new();
        assert!(setup_nushell(&sh("nu"), &EncodedOs::from_path(&dir), &mut env).is_none());
        assert_eq!(0, env.len());
    }

    #[test]
    fn zsh() {
        let res = TmpResourcesDir::new(Shell::Zsh);
        let mut env = EnvMap::new();

        let command = setup_zsh(&sh("zsh"), &res.enc(), &mut env);
        assert_eq!("zsh", shell_str(command.as_ref()));
        assert_eq!(
            res.shell_path,
            env.get("ZDOTDIR").unwrap().to_str().unwrap()
        );
        assert!(!env.contains_key("GHOSTTY_ZSH_ZDOTDIR"));
    }

    #[test]
    fn zsh_zdotdir() {
        let res = TmpResourcesDir::new(Shell::Zsh);
        let mut env = EnvMap::new();
        env.insert("ZDOTDIR", "$HOME/.config/zsh");

        let command = setup_zsh(&sh("zsh"), &res.enc(), &mut env);
        assert_eq!("zsh", shell_str(command.as_ref()));
        assert_eq!(
            res.shell_path,
            env.get("ZDOTDIR").unwrap().to_str().unwrap()
        );
        assert_eq!(
            "$HOME/.config/zsh",
            env.get("GHOSTTY_ZSH_ZDOTDIR").unwrap().to_str().unwrap()
        );
    }

    #[test]
    fn zsh_missing_resources() {
        let tmp = TempDir::init().unwrap();
        let dir = std::fs::canonicalize(tmp.path()).unwrap();
        let mut env = EnvMap::new();
        assert!(setup_zsh(&sh("zsh"), &EncodedOs::from_path(&dir), &mut env).is_none());
        assert_eq!(0, env.len());
    }

    #[test]
    fn bundled_resources_exist() {
        // The shipped scripts that setup() points shells at.
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../resources");
        let root_enc = EncodedOs::from_path(&root);
        let mut env = EnvMap::new();
        let r = setup(&root_enc, &sh("bash"), &mut env, None)
            .unwrap()
            .unwrap();
        assert_eq!(r.shell, Shell::Bash);
        assert!(
            env.get("ENV")
                .unwrap()
                .to_str()
                .unwrap()
                .ends_with("shell-integration/bash/ghostty.bash")
        );
        assert!(
            root.join("shell-integration/nushell/vendor/autoload/ghostty.nu")
                .is_file()
        );
    }
}
