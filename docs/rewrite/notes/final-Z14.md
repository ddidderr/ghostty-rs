# Z14 (RV11 gaps)

1. ghostty-bench: `Args::take_enum` validates every occurrence of an enum option in order (empty value resets to default), used by codepoint-width, grapheme-break, is-symbol, screen-clone, terminal-stream `--handler` and synthetic `+osc --style`. Tests in args.rs / benches.rs.
2. Theme open (named and absolute) uses `to_syscall_path` so an interior NUL truncates the path like Zig's C string. Test `theme_absolute_interior_nul_opens_prefix`; checked against /usr/bin/ghostty (no diagnostics, loads).
3. New `ghostty_os::zig_error::{errno_name, io_error_name}` (Zig std posix errno mapping). Used by theme.rs, path.rs, config.rs (config-file open, `config::Error::Io` Display), file_load.rs logs, edit_config.rs, list_themes preview. Verified identical to oracle for FileNotFound, NotDir, AccessDenied, IsDir ("it is a directory" path), NameTooLong, SymLinkLoop.
