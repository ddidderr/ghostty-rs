# Z17: RV14 findings

1. `+show-face --cp` is `Option<U21>` (new `ghostty_config::cli_args::U21`, Zig `u21`); out of range gives an
   `invalid value` diagnostic and exit 1 (no panic). Audit of Zig int-typed CLI/config fields against Rust
   (Config.zig, cli/*.zig, nested types): all other widths already match.
2. `--string` is `Option<EncodedOs>` (raw bytes at option parse). Invalid UTF-8 is detected after the `--cp`
   lookup: `Unable to parse string as unicode: error.InvalidUtf8` (no newline), exit 1.
3. New `crates/ghostty-cli/src/zig_writer.rs` models `File.writer(&buf)`: fixed buffer, drain-on-overflow,
   explicit flush, so stdout/stderr interleave in Zig's flush order (`+show-face` creates both writers on
   stdout; stdout is flushed first, then stderr; verified against the oracle on pipes). Anything still
   buffered is flushed on drop. Applied to every Zig action using `File.writer`: show-face, help (+ any
   `--help`), version, validate-config, show-config, list-actions, list-keybinds (plain), list-fonts,
   list-themes, ssh-cache, crash-report, edit-config, new-window.
4. Windows basename: already Zig 0.15.2 (colon only at byte index 1); `λ:theme` stays whole. Added test.
5. `ghostty_os::osbytes::zig_is_absolute{,_posix,_windows}` + `EncodedOs::is_zig_absolute` (0.15.2
   `isAbsolute`: `\x`, `/x`, `X:\`, `X:/`). Used by theme open, config path expand, file_load, Config
   asserts, surface mouse path, GTK bell path (every Zig `isAbsolute` site).
6. `xev::available(IoUring)` is exactly `IoUring::new(256).is_ok()`. The termio reactor now handles a ring
   without `EXT_ARG` by using `IORING_OP_TIMEOUT` (like libxev timers); test forces that path.

## Upstream bugs fixed (not replicated)

- Positional output overwrite: `std.fs.File.writer` (Zig 0.15.2 `fs/File.zig` `Writer.init`/`drain`)
  `pwrite`s at the writer's own offset starting at 0. With stdout a regular file, `+show-face`
  (`cli/show_face.zig:68-73`, two writers on `File.stdout()`) overwrites its own output, and any action
  (`cli/help.zig:34`, `cli/validate_config.zig:43`, ...) overwrites pre-existing file content; `Writer.end`
  (`validate_config.zig:46`, `show_config.zig:84`) also truncates it. Rust always appends at the current end.
- Never-flushed buffer: `cli/list_themes.zig:124-126,129-131,171` writes "Could not find the Ghostty
  resources directory" / "No themes found..." to a 4096 byte stderr buffer that is never flushed, so the
  messages are lost. Rust flushes it when the action ends.
