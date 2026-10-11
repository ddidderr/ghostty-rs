# ghostty-termio notes (agent IO2): Termio, Thread, Exec, stream handler

Builds on `ghostty-termio-IO1.md` (pty, Command, shell_integration, terminfo). Linux only.
Deps added: ghostty-config, ghostty-terminal, ghostty-vt, ghostty-renderer, ghostty-apprt
(path dep; it is not in `[workspace.dependencies]`), ghostty-datastruct, crossbeam-channel,
rustix features `event`,`pipe`.

## File -> module
| Zig | Rust |
|---|---|
| Options.zig | `options.rs` (`Options`) |
| backend.zig | `backend.rs` (`Backend`, `ThreadData`, `Config`, `Kind`) |
| message.zig | `message.rs` (`Message`, `SizeReport`, `WriteReq`) |
| mailbox.zig | `mailbox.rs` (`Mailbox`, bounded crossbeam queue of 64 + eventfd) |
| Termio.zig | `termio.rs` (`Termio`, `Output`, `DerivedConfig`, `ThreadData`) |
| Thread.zig | `thread.rs` (`Thread`) |
| Exec.zig | `exec.rs` (`Exec`, `Config`, `ThreadData`, read thread, Subprocess, `exec_command`) |
| stream_handler.zig | `stream_handler.rs` (`StreamHandler`, `TerminalStream`, `HandlerState`) |
| (xev.Async) | `wakeup.rs` (`Wakeup`, eventfd) |
| (renderer.State, apprt/renderer mailboxes) | `state.rs` (`RendererState`), `sinks.rs` (`SurfaceMailbox`, `RendererMailbox`) |
IO1 stand-ins replaced by real types: `shell_integration::{Command, ShellIntegrationFeatures}` are
re-exports of `ghostty_config::{command::Command, types_b::ShellIntegrationFeatures}`;
`command::{LinuxCgroup, GtkSingleInstance}` re-export `ghostty_config::types_b`; `RtPreExecInfo::init(&Config)`
/ `RtPostForkInfo::init(&Config)` added. NOTE config enum `Default` is the first variant; our info structs keep
`Default` = `single-instance` / `detect` (the config defaults). `Detect` must be resolved by the apprt before
spawning (as in Zig GTK) or the child exits 127 in `linux_cgroup_pre_exec`; tests use `LinuxCgroup::Never`.

## Threading model
Per surface: (1) the **IO thread** (`Thread::thread_main(termio)`) owns `Termio`; (2) the **pty read thread**
("io-reader", spawned in `Exec::thread_enter`) reads the pty and calls `Output::process_output`; (3) the renderer
thread and (4) the surface/core thread are outside this crate.
* libxev -> one `poll(2)` loop in `Thread::run`: mailbox eventfd, stop eventfd, child pidfd (fallback: 50ms
  `waitpid(WNOHANG)` poll; Flatpak: channel+eventfd), pty `POLLOUT` (only while writes are queued) and deadlines
  (selection-scroll 15ms, resize coalesce 25ms, sync-output reset 1000ms, termios poll 200ms).
* The `Terminal` lives in `Arc<RendererState>` (`Mutex<Terminal>`), shared with the renderer like Zig's
  `renderer.State`. Lock order: terminal mutex, then the stream mutex inside `Output`.
* Mailbox full: Zig unlocks the renderer mutex mid-handler and blocks. Impossible with an owned guard, so
  `HandlerState` keeps an in-order *outbox*; `Output::process_output` delivers it (blocking) after dropping the
  locks. `Mailbox::send_locked(msg, &mutex, guard)` gives other callers the Zig unlock/relock behaviour.
* Writes: `ThreadData` keeps a `VecDeque<Vec<u8>>`, written non-blocking; `POLLOUT` polled on `EAGAIN`.
  Backpressure to producers is the 64-slot mailbox. Linefeed mode replaces `\r` by `\r\n`.
* Stop: notify the handle from `Thread::stop_handle()`; `Exec::thread_exit` kills the process group (SIGHUP loop
  with `waitpid(WNOHANG)`, unless the exit watcher already reaped it), writes the quit pipe and joins the reader.
* Start failure (`pty` open, input file, spawn): the message is printed on the terminal (cursor hidden) and the
  thread keeps draining the mailbox (dropping messages) until stopped, like Zig.

## API for the core Surface agent
```rust
let mailbox = Mailbox::new()?;                          // clone: producers (surface) + consumer (IO thread)
let mut ecfg = exec::Config::new(env_map, config.term.clone());   // command, env_override, shell_integration(+features),
ecfg.command = config.command.clone(); /* working_directory, resources_dir, cursor_blink, rt_*_info, create_scope */
let termio = Termio::init(Options {
    size,                                   // renderer::size::Size (screen, cell, padding)
    full_config: &config,                   // only read during init (scrollback, grapheme width, input)
    config: DerivedConfig::new(&config),
    backend: Backend::new(backend::Config::Exec(ecfg))?,
    mailbox: mailbox.clone(),
    renderer_wakeup,                        // Wakeup the renderer thread polls (termio notifies it after output/messages)
    renderer_mailbox: Arc<dyn RendererMailbox>,   // push(renderer::Message, Timeout): gets Resize, ResetCursorBlink
    surface_mailbox: Arc<dyn SurfaceMailbox>,     // blanket impl for apprt::surface::Mailbox<S, A>
})?;
let state: Arc<RendererState> = termio.renderer_state().clone();   // hand to the renderer: state.lock() -> Terminal
let thread = Thread::new()?; let stop = thread.stop_handle();
let join = std::thread::Builder::new().name("io".into()).spawn(move || thread.thread_main(termio))?;
// later: mailbox.send(Message::...); mailbox.notify();  (or termio.queue_message before moving it)
// shutdown: stop.notify(); join.join();
```
Messages the IO thread accepts (`message::Message`): `ColorSchemeReport{force}`, `Crash`, `ChangeConfig(Box<DerivedConfig>)`,
`Inspector(bool)` (flag only), `Resize(Size)` (coalesced 25ms), `SizeReport(..)`, `ClearScreen{history}`,
`ScrollViewport`, `SelectionScroll(bool)`, `JumpToPrompt(isize)`, `StartSynchronizedOutput`, `LinefeedMode(bool)`,
`Focused(bool)`, `Write(WriteReq)` (`Message::write_req(&[u8])`, `Message::write_stable(&'static [u8])`).
Surface messages sent (apprt `surface::Message`): `ChildExited{exit_code, runtime_ms}` (exit code; signal number if
killed), `PasswordInput(bool)` (termios poll, only while focused), `SelectionScrollTick(bool)`, `SetTitle`, `ReportTitle`,
`SetMouseShape`, `ClipboardRead/Write`, `PwdChange`, `ColorChange`, `DesktopNotification`, `ProgressReport`,
`RingBell`, `StartCommand`, `StopCommand(Some(code))`.
Renderer messages sent: `Resize(Size)` (Forever) and `ResetCursorBlink` (Instant, at most every 500ms); plus
`renderer_wakeup.notify()` after output, mailbox drains and resets.
**wait-after-command and abnormal-exit detection are NOT in termio** (Zig `Surface.zig`): the surface compares
`runtime_ms` with `abnormal-command-exit-runtime` and decides whether to close; the IO thread simply keeps running
(writes ignored, resize/focus still processed) until stopped. `Backend.childExitedAbnormally` in backend.zig has no
Exec implementation upstream and is not ported.
Manual use without a child: `Termio::process_output(bytes)` / `Output::process_output`.

## Tests
Unit 102 total in the crate (IO1: 47 incl. updated), IO2 new: wakeup 2, mailbox 4, message 2, exec 4,
stream_handler 41 (DA1/DA2/DSR/CPR/origin CPR, XTVERSION, ENQ, DECRQM, kitty kbd query, OSC 4/10/11/12 queries in
16/8-bit/none, OSC 21, XTGETTCAP, DECRQSS sgr/decscusr/decstbm/decslrm/invalid, size reports, title, OSC 7 local/remote/reset,
OSC 52, notifications, progress, OSC 133, cursor-style default/blink override, mode side effects, outbox ordering, ...).
Integration `tests/exec_integration.rs` (21): real `sh -c 'printf hello\e[31mred'` (contents, SGR, exit code 0),
exit code, signal exit, exec failure, echo/cat writes, 256 KiB write backpressure, linefeed mode, query reply round
trip (DA1, CSI 18 t), initial `input`, missing input file error screen + drain, resize coalescing -> pty size,
sync-output timer reset, password-input detection, selection-scroll timer, focus events, clear-screen, stop kills
process group, writes after exit, ChangeConfig, manual `process_output`.
Zig tests: message.zig 2 (`refAllDecls` skipped; size test replaced by a growth guard), Exec.zig execCommand 7 ->
Linux subset ported (shell command, direct command, config freed; passwd variants collapse because the passwd
argument only mattered on macOS; 2 darwin tests skipped). Zig has no tests in Termio/Thread/stream_handler/mailbox.

## Deviations / TODO(integration)
* `RendererState` is defined here (renderer `State.zig` deferred); no `inspector`/`preedit`/`mouse`. Inspector
  slow-path (byte-at-a-time) is not ported; `TerminalStream::process_byte` exists for it.
* kitty image storage (`set_limit`, `delete`) is a stub in ghostty-terminal: `image_storage_limit` is not applied;
  clear-screen only marks kitty images dirty. `Terminal::kitty_graphics` returns None, so no APC replies yet.
* tmux control mode DCS: parsed by ghostty-vt but ignored here (`terminal.tmux.Viewer` not ported).
* Message layout differs: three Zig write variants are one `Message::Write(WriteReq)`; `ChangeConfig` owns a boxed
  `DerivedConfig` (no allocator); `size_of::<Message>() == 40` is replaced by a growth guard.
* Mailbox is a crossbeam bounded channel (the datastruct `BlockingQueue` cannot return the rejected message).
* Process exit code: normal exit -> status, killed by signal -> signal number (libxev behaviour).
* Drain loop continues after a failing message (Zig stopped the drain at the first error).
* Read thread: on `POLLHUP` it does one more drain of the fd before exiting (Zig exits at once and could lose
  trailing output). `getpgid` retry loop capped at ~5s (Zig loops forever).
* Flatpak host-command path is ported but untestable here. `FlatpakHostCommand::wait_callback` consumes the command,
  so `stop()` cannot signal it afterwards (TODO(ghostty-os): allow `signal` after `wait_callback`).
* `Config.create_scope` (cgroup D-Bus call) and `restore_rlimit` must be supplied by apprt-gtk / core.
* macOS `login(1)` / Windows branches of `execCommand` not ported (Linux only).
* ghostty_terminal `StreamHandler` writes `Terminal` API through `&mut Terminal` borrowed per read; parser state
  (`Parser`, `Utf8Decoder`) persists in `TerminalStream`.
