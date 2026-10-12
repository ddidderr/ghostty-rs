# Crash handling (CR)

New crate `crates/ghostty-crash` (workspace member; `unsafe_code = "deny"`, allowed only in `signal.rs` and `context.rs` with SAFETY comments).

## Ported
- `crash/dir.zig` -> `dir.rs` (`default_dir`, `Dir`, `ReportIterator`, `Report`). `ghostty-cli` `+crash-report` now lists via it (`crash_report::list_reports/default_dir` delegate).
- `crash/sentry_envelope.zig` -> `envelope.rs` (parse/serialize/Attachment/Item/ItemType); all 9 Zig tests ported + error/decode/round-trip tests.
- `crash/sentry.zig` -> `sentry.rs` + `signal.rs` + `context.rs`: `init`/`deinit`, crash dir (`$XDG_STATE_HOME/ghostty/crash`), `<uuid-v4>.ghosttycrash` filename, event with release/tags (build-mode, app-runtime, font-backend, renderer, thread-type) and `Dimensions` context (screen/grid/cell as i32, -1 on overflow), panic hook, SIGSEGV/SIGBUS/SIGILL/SIGFPE/SIGABRT handlers.
- `crash/main.zig` -> `lib.rs` re-exports.
- Hooks: `Surface::crash_guard` (Main) in the CoreSurface callbacks + occlusion/refresh/text; renderer `Thread::thread_main` (Renderer); termio `Thread::thread_main` and the exec read thread (Io, as in Zig); `global::init` -> `ghostty_crash::init`, `global::deinit` (called from `main`).

## Design
- Feature `sentry` (default off, like `-Dsentry`) on the root binary -> `ghostty-crash/sentry`. `ghostty_crash::init()` is a no-op without it. `ghostty_crash::install()` always installs (used by tests).
- Panic path may allocate: envelope = header line + `event` item + `backtrace.txt` attachment. Signal path: no allocation, static scratch buffer, only `open/write/mkdir/getrandom/raise`; writes header + event item (no attachment), then restores SIG_DFL and re-raises so exit status/core dump are unchanged. Uses the thread's existing altstack (Rust installs one) or a leaked 64K one.
- Surface "pointer" -> `Arc<SurfaceDims>` (atomics) that the surface refreshes on every callback/resize and shares with the renderer and IO threads (`Thread::crash_dims` pub field) and the read thread (`current_dims()`).

## Deviations
- Zig's `Attachment.encode` never writes the `type` header (unparseable output); we add `"type":"attachment"` if absent.
- Crash dir is resolved at init (not at crash time) since the signal path can't read the environment.
- Panic reports are written on any panic (Zig panics abort and are caught by sentry as a signal). No sentry DB/cache dir, no sessions, no macOS cache dir, no init thread.
- Read thread tag is `io` (Zig behaviour), not a separate `read` type.
- Signal-targeted thread context is only exact for synchronous faults/abort; process-directed signals may hit another thread.

## Tests
`cargo test -p ghostty-crash` (envelope, dir, event/report parse-back, context) and root `tests/crash.rs` (child process: panic, SIGSEGV, stack-overflow fault on altstack, abort; checks envelope contents and that `ghostty +crash-report` lists the file).

## Known gaps
No minidump/native stack in signal reports; no `+crash-report` viewing (upstream doesn't either).
