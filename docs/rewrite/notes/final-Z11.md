# Z11: RV9 gap groups (Ghostty 1.3.1 parity)

Fixes for the six gap groups in `review-rv9-codex.md`. No version change, no commit.

## Fixes

1. **Interior NUL in font names.** `fontconfig::Pattern::add_string_bytes`
   ends the value at the first NUL (the C-string handoff Zig does) instead of
   failing, so `font-family`, `font-style` and codepoint-map families no longer
   hit the `assert!` in `discovery::to_fc_pattern`.
   Test: `discovery::tests::interior_nul_in_font_names_ends_the_c_string`.
2. **Numeric grammars.** New `ghostty_os::zig_fmt`: exact ports of
   `std.fmt.parseInt`/`parseUnsigned` (underscores, `0b/0o/0x` in both cases,
   sign rules, `len > 2` prefix gate, Overflow vs InvalidCharacter) and
   `std.fmt.parseFloat` (grammar of `parse.zig` incl. `validUnderscores`, hex
   floats via a port of `convertHex` incl. the many-digits path, `inf`/
   `infinity`/`nan`). Decimal values are rounded by Rust's correctly rounded
   `FromStr` once the Zig grammar accepted the text. Ported all std tests.
   Used by: primitive int/float fields (base 0), palette key (u8, base 0, Zig
   error names), variation values, mouse-scroll bare float, quick-terminal
   `px`/`%`, window-padding (u32, base 10), duration (`parseUnsigned` u64),
   background-blur radius, codepoint/clipboard map hex (base 16), metric
   modifiers, keybind numeric parameters (base 10 / float), ssh-cache entry
   timestamp, `GIO_LAUNCHED_DESKTOP_FILE_PID`.
   Also found by differential runs: Zig `{d}` prints `nan` (Rust `NaN`);
   finalize clamps use Zig `@min/@max/clamp` NaN and +-0 semantics
   (`cli_args::zig_min/zig_max/zig_clamp`); a failed `background-blur` radius
   parse leaves `radius` (0 or the previous radius) like Zig's in-place union
   initialisation.
3. **C strings.** `config_capi` getters, C action titles/strings
   (`action_c::Keep`) and the embedded clipboard write callback now end the C
   string at the first NUL instead of deleting NUL bytes.
4. **GTK new-window args.** `--working-directory=`/`--title=` trim only
   `std.ascii.whitespace` (space, \t, \n, \r, VT, FF); U+00A0 etc. survive.
5. **Shell tokenizer.** New `ghostty_os::arg_iterator::ArgIteratorGeneral`
   (port of `std.process.ArgIteratorGeneral`, including `comments`/
   `single_quotes` options, `skip`, NUL terminates). `Command::arg_iterator`
   (and so shell-integration detection) use it: space, tab, CR and LF split.
   Ported the std "response file arg parsing" test plus separator/quote/NUL
   tests; termio test `detect_shell_splits_on_all_zig_separators`.
6. **Parser diagnostics.** UTF-8 validation moved to where Zig would notice it:
   metric modifiers (`InvalidFormat`), quick-terminal size (`MissingUnit`/
   `InvalidValue`), `link` (`NotImplemented`), colors/palette
   (`InvalidCharacter`), background-blur, keybind trigger (parsed after the
   action, like `Parser.init`) and action parameters (known action with bad
   bytes is `InvalidFormat`, unknown name `InvalidAction`).
   Other differences found: config-file open errors are printed as
   `error.FileNotFound` etc.; paths with interior NUL resolve/open as the C
   string prefix (`EncodedOs::to_syscall_path`) for relative path expansion,
   `config-file`, custom shaders, background image, gtk css.

## Differential harness

Script (not committed): for each of the 202 `Config.zig` fields x ~220 hostile
and valid values, run `/usr/bin/ghostty +validate-config --config-file=F` and
`+show-config` (value in `$XDG_CONFIG_HOME/ghostty/config`) against the Rust
binary and compare exit code, stdout and stderr. Final state: zero mismatches
except `theme` (resources path differs by install location),
`auto-update-channel` default (`tip` vs `stable`, build channel) and
`gtk-opengl-debug` (debug build).

Regression tests: `crates/ghostty-config/tests/zig_parity.rs`,
`ghostty_os::zig_fmt::tests`, `ghostty_os::arg_iterator::tests`,
`ghostty-config` command tests, lib/gtk tests listed above.

## Not covered / notes

- `ghostty-terminal` OSC/tmux/kitty parsers also call `parseInt`
  (`terminal/osc/parsers/*.zig`, `tmux/*.zig`, `kitty/*.zig`); they are outside
  the config/input scope of this task and still use Rust `str::parse`/
  `from_str_radix`. `zig_fmt` is available for a follow-up.
- Windows/macOS behaviour is covered by cross-checks only.
