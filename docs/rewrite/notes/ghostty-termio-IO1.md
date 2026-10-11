# ghostty-termio notes (agent IO1)

Linux only. Crate lints: `unsafe_code = "deny"` (workspace clippy table copied
into the crate's `[lints.clippy]`); `#[allow(unsafe_code)]` only on tiny items
with `// SAFETY:` (fork, execvpe, _exit, close, sigaction, borrow_raw).

## File -> module
- `pty.zig` -> `pty.rs` (`Pty`, `Winsize`, `Mode`, `child_pre_exec(master, slave)`)
- `Command.zig` (+ `apprt/gtk/{pre_exec,post_fork}.zig` cgroup logic, `cgroup.zig` fmtScope)
  -> `command.rs`
- `termio/shell_integration.zig` -> `shell_integration.rs`
- `terminfo/{main,Source,ghostty}.zig` -> `terminfo/{mod,source,ghostty}.rs`
  (`terminfo::ghostty` is a `const Source`; `encode`, `encode_to_string`,
  `xtgettcap_map`). Output verified byte-identical with the Zig `encode` (golden
  file `tests/data/xterm-ghostty.terminfo`, generated with `zig run`).
- Shell scripts copied verbatim to `resources/shell-integration/` (workspace root).

## Not ported (deferred; need ghostty-terminal / renderer)
Exec.zig, Termio.zig, Thread.zig, stream_handler.zig, mailbox.zig, message.zig,
backend.zig, Options.zig (all reference `terminal`/`renderer`/`apprt` types).

## Design / deviations
- Deps `ghostty-config` and `ghostty-terminal` REMOVED from Cargo.toml (note in file).
  Deps: ghostty-os, rustix (pty, termios, process, fs, stdio), nix (signal), libc, log, thiserror.
- `Command` does its own `fork` + `libc::execvpe` (not std::process::Command) so the
  child setup is exactly Ghostty's: dup2 stdio (CLOEXEC cleared), best-effort chdir
  (failure ignored), restore rlimit (`restore_rlimit: Option<Rlimit>` replaces
  `global_state.rlimits`), `os_pre_exec` then `rt_pre_exec` hooks (`Fn() -> Option<u8>`,
  `Some(code)` => `_exit(code)`; panics => exit 1), then execvpe. All allocation happens
  before fork. Exec failure: child prints the same "Requested executable not found" message
  (other errors: `errno=N` instead of the Zig error name) to stderr and `_exit(1)`; there
  is no `ExecFailedInChild` error because the child can never return into caller code.
  `exit_immediately(code)` is provided for hooks that want to exit (use instead of
  `std::process::exit`).
- Signal mask: Zig never reset the mask explicitly in Command either; only `Pty::child_pre_exec`
  resets 13 signal dispositions to SIG_DFL (same list as Zig), then setsid + TIOCSCTTY and
  closes slave+master. Close-all-fds is not done (Zig doesn't either; master is CLOEXEC).
- stdio fields are `Option<OwnedFd>` (callers pass `try_clone()`d fds); `wait(block)` returns
  `Result<Exit, Errno>`; non-blocking wait loops until a status is available (as Zig, with a
  `yield_now`). `setData/getData` dropped (closures capture state). Empty `args` => argv =
  [path] (what the Zig doc comment promises; Zig code itself passed an empty argv).
- Inherited env is snapshotted with `vars_os()` before fork (Zig passes `environ`).
- `Pty`: `slave: Option<OwnedFd>`; both fds close on drop (Zig never auto-closed the slave).
  Master opened CLOEXEC, slave not (like glibc openpty + explicit CLOEXEC). IUTF8 set.
- cgroup: `linux_cgroup_pre_exec(&RtPreExecInfo)` (child side, waits <=250ms) and
  `linux_cgroup_post_fork(cmd, &RtPostForkInfo, create_scope)` where `create_scope` is a
  closure standing in for the GIO D-Bus `cgroup.createScope`. `fmt_scope(pid)`.
  Local `LinuxCgroup`/`GtkSingleInstance`/`Rt*Info` types: TODO(integration) from ghostty-config.
- shell_integration: local `Command` (Shell/Direct) with `ArgIteratorGeneral` quoting rules,
  `ShellIntegrationFeatures` (same defaults) - TODO(integration) replace with config types.
  `EnvMap = HashMap<String,String>`. `setup(resource_dir, &Command, &mut EnvMap, Option<Shell>)`
  -> `Result<Option<ShellIntegration>, Error>` (Error only from `homedir::home`).
  Darwin `/bin/bash` special case dropped.

## Tests (52 total: 47 unit + 5 integration, all passing)
Zig tests ported (35): pty 1; Command 10 (createNullDelimitedEnvMap as `prepare()`, os/rt pre exec
1+2, rt post fork, redirect stdout, custom env, custom cwd, execve failure); shell_integration 21
(force shell, failure, detectShell, setup features, bash x8, xdg x3, nushell x3, zsh x3);
terminfo 3 (xtgettcap map, Source encode, ghostty encode).
Skipped: Windows/NullPty/Windows env block helpers, `refAllDecls` tests.
Extras: pty cloexec/IUTF8/mode, Exit decoding, signal exit, nonblocking wait, invalid cwd ignored,
fd leak check, cgroup mode matrices, arg splitter, bundled-resources check, terminfo golden vs Zig,
`tic -x` compile of the generated source (skipped if tic absent),
`tests/pty_command.rs`: real `/bin/sh -c 'echo hi; stty size'` on a 31x97 pty (output
`hi\r\n31 97\r\n`), controlling-tty check, resize check.

## Known gaps / TODO(integration)
- Config-derived types above; `restore_rlimit` from global state; D-Bus scope creation.
- `Pty::child_pre_exec` is meant to be called from Exec's `os_pre_exec` (see tests/pty_command.rs
  for the wiring). Flatpak passwd path in ghostty-os needs Pty (ghostty-os TODO) - now available.
- Hooks run after fork in a possibly multithreaded process; `linux_cgroup_pre_exec` allocates
  and sleeps (same as Zig) - fine on glibc.
