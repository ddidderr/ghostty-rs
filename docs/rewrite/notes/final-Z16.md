# Z16: RV13 P3 fixes

## 1. auto-update-channel
`Config::finalize` now defaults the field from `ghostty_build_info::RELEASE_CHANNEL` (Zig:
`build_config.release_channel`); ghostty-config gained the dep. Tests: default follows the build
channel, explicit value kept. `+show-config` for this tip build prints `tip` like /usr/bin/ghostty.

## 2. async-backend
* `ghostty_os::xev` (new): libxev `Dynamic` selection: `Backend`, `candidates` (Linux: io_uring, epoll;
  others single), `backend()` (process global, default = last candidate = epoll), `available`
  (io_uring: `IoUring::new(256)` like libxev, plus we require EXT_ARG and POLL_ADD/POLL_REMOVE probes),
  `detect()`, `prefer()`.
* `ghostty-core` global init: detect (warn on failure) and `libxev default backend=..` info log (global.zig).
  `+version` prints the real backend instead of `n/a`.
* GTK `Application::startup` calls `startup_xev` -> `apply_async_backend` (application.zig startupXev):
  `auto` no-op, else `prefer`, logs `libxev manual backend=` / `libxev manual backend failed, using default=`.
* `ghostty_termio::reactor` (new): `Reactor` = `Epoll` (persistent epoll, registrations diffed per wait),
  `IoUring` (`io-uring` crate: one-shot POLL_ADD per interest, POLL_REMOVE on drop, EXT_ARG timeout;
  unsafe only for SQ push with SAFETY comment) or `Poll` (poll(2), used where libxev has one fixed backend,
  i.e. macOS kqueue). `Thread::new()` uses `xev::backend()`; `Thread::with_backend` for tests. The IO
  loop and the post-failure drain loop run on it.
* Not changed: the renderer and search threads use crossbeam channels (not fd loops), and the pty reader
  is a dedicated blocking thread (as in Zig), so there is nothing to select there.
* Tests: reactor unit tests (timeout, level trigger, drop/re-add, cross-thread wake, fd reuse) per backend;
  `tests/exec_integration` now runs the whole 21-test suite 3x (poll, epoll, io_uring; io_uring skips with
  a SKIP message if unavailable). In this sandbox io_uring IS available and was exercised.
  (Fixed a test-only pid-file collision between suites.)

## 3. Windows zig_error
Installed Zig std is 0.16 (not 0.15.2) so the table follows 0.15.2 `windows.OpenFile` from knowledge of
its NTSTATUS arms, translated to Win32 codes (see the doc table in `os/zig_error.rs`): sharing violation,
access denied, user-mapped-file -> AccessDenied; invalid name -> BadPathName; BAD_NETPATH/BAD_NET_NAME ->
NetworkNotFound; NOT_READY -> NoDevice; PIPE_BUSY -> PipeBusy; ALREADY_EXISTS -> PathAlreadyExists;
ERROR_DIRECTORY -> NotDir; 206 -> NameTooLong. Removed CreateProcess-only entries (FileBusy, InvalidName,
InvalidExe, SystemResources, NoSpaceLeft) which file opens never produce (Unexpected). Table is a
host-independent fn so it is tested on Linux. Caveat: 0.16 retries sharing violations then FileBusy; the
oracle is 0.15.2 behaviour (AccessDenied), as requested.

## 4. basename
`config/theme.rs`: exact `ComponentIterator` + `parsePathWindows` port (`basename_windows`, `basename_posix`):
any 1-3 byte code point + `:` is a drive root (so `1:theme` -> `theme`), UNC/local-device roots. Tests
include all `testBasenameWindows` cases from Zig std.
