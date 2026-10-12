# compat-GC: ghostty-cli Windows branches and DiskCache locking

Ported (all `cfg(windows)` / cfg-gated, no unsafe, no new deps):
* `edit_config.rs`: Windows prints "not supported on Windows ... edit manually at <path>" to stderr and returns 1 (after the path lookup, before the editor lookup, as in Zig). The `exec` path is now `cfg(unix)` (`exec_editor`).
* `list_colors.rs` / `list_keybinds.rs`: both use `tui::window_size()`; on Windows it returns the fixed Zig default (120 cols x 24 rows) instead of querying the tty. Pixel size (1024x768) is not modelled (unused).
* `boo.rs`: Zig allows windows/macos/linux/freebsd and returns 1 elsewhere; mirrored with a `cfg!` check. Windows needs no other change.
* `ssh_cache/disk_cache.rs`: made the module compile on Windows (unix-only `mode`/`PermissionsExt` gated; `fixup_permissions` is a no-op on Windows as in Zig "Windows does not support chmod").

Locking deviation from the task brief: Zig does NOT lock on Windows (`if (comptime builtin.os.tag != .windows) file.tryLock(.exclusive)` and the matching unlock). So the faithful port is a no-op lock on Windows rather than LockFileEx; hence no windows-sys/unsafe. On unix the existing implementation already matched Zig: `File::try_lock` (flock LOCK_EX|LOCK_NB), WouldBlock is ignored (proceeds unlocked, as Zig ignores the bool), other errors -> CacheIsLocked, unlock on drop.

Tests: unix `lock_is_exclusive_until_dropped`; windows-only `lock_is_noop_on_windows` (compiled only); permission assertion split into unix-only `disk_cache_file_permissions`.

Verified: build/test(92)/clippy -D warnings on linux; `cargo check --target x86_64-pc-windows-gnu --tests -p ghostty-cli` and windows clippy clean (stub fontconfig.pc, RUSTC_WRAPPER= as in windows-WIN.md).
