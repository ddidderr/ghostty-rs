# Distribution / build artifacts (X1)

## Build-data generator: `crates/ghostty-build-data`
Binary `ghostty-build-data +<mode>` (port of `main_build_data.zig`, `extra/*.zig`, `build/mdgen/*`).
Modes: bash zsh fish sublime vim-syntax vim-ftdetect vim-ftplugin vim-compiler terminfo,
plus mdgen-ghostty-1 / mdgen-ghostty-5 (separate executables upstream).
Static text chunks (`src/chunks_*.rs`) are extracted mechanically from the Zig multiline strings;
the mdgen header/footer templates are copied to `src/mdgen/`.
Verified byte-identical (version string set to the installed `1.3.1-arch2`) to the installed Ghostty:
bash/zsh/fish completions, 4 vim files, bat sublime syntax, `ghostty.1.md`, `ghostty.5.md`.
`pandoc --to man` of our markdown differs from the installed man pages only by the pandoc version
comment/escape style (3.6 vs 3.11).

Supporting changes:
- `ghostty-config`: `cli_args::FieldKind` + `ConfigField::KIND` (Zig `@typeInfo(field.type)` for completions;
  bool/?bool/enum/?enum/packed/RepeatablePath/optional/other) and `FieldInfo.kind`.
- `ghostty-cli`: `Action::option_fields()` (Zig `Action.options()`).
- `ghostty-input`: 4 keybind-action docs (`adjust_selection`, `write_scrollback_file`, `new_split`, `crash`) end in an
  empty `///` line in Zig; `ActionKey::docs()` now carries the extra `\n` and `ActionKey::help_string()` gives the exact
  Zig help string (docs minus one trailing newline). `+list-actions --docs` output unchanged.
- `helpgen.zig` (generating Zig source) has no counterpart: Rust reads docs from `ConfigStruct::FIELDS`,
  `Action::help_text()` and `ActionKey::docs()` directly.

## Man pages / docs
`just docs [OUT]` writes `ghostty.{1,5}.md`, and with pandoc installed `.html` and roff (`--standalone --from markdown --to man`).
Without pandoc only markdown is produced (warning); `just install` then installs docs markdown only.

## Translations
`po/` = Zig `po/*.po`, `com.mitchellh.ghostty.pot`, READMEs (29 locales). `just locales` runs `msgfmt` into
`<lang>/LC_MESSAGES/com.mitchellh.ghostty.mo`; install copies to `share/locale`. Domain matches
`ghostty_os::i18n::BUNDLE_ID` (`com.mitchellh.ghostty`); a test checks `po/*.po` == `i18n_locales::LOCALES`.
The `update` step (xgettext/msgmerge, Zig `GhosttyI18n.update_step`) is not ported (blueprints/Rust sources differ).

## `just install PREFIX FLATPAK SYSTEM_PACKAGE`
Adds completions (bash/zsh/fish), vim + nvim + bat files, docs, man pages, `.mo` files, `ghostty.terminfo`
and `ghostty.termcap` (Zig uses `infotocap`; skipped with a warning if absent).
`FLATPAK=true`: `dbus.service.flatpak.in`, no systemd unit, no 1024px icon.
`SYSTEM_PACKAGE=true`: systemd unit to `lib/systemd/user` (else `share/systemd/user`). Tested with DESTDIR + system-package.
Not mirrored: "(Debug)" naming/app-id for debug builds.

## Build metadata
`crates/ghostty-cli/build.rs` derives (mirrors `Config.zig` + `GitVersion.zig`): version = `workspace.metadata.ghostty.app-version`
(1.3.1), tagged `vX.Y.Z` -> plain, otherwise `X.Y.Z-<branch>+<hash>` (tip channel), no git -> `-dev+0000000`;
override with `GHOSTTY_VERSION_STRING`. Build mode from profile (`production` -> .ReleaseFast, `release` -> .ReleaseSafe
(debug-assertions on), `dev` -> .Debug). `BuildInfo::default()` consumes these.
`crates/ghostty-apprt-gtk/build.rs` emits `GHOSTTY_GTK_VERSION` / `GHOSTTY_ADW_VERSION` from pkg-config
(`gtk4`, `libadwaita-1`); `gtk_version`/`adw_version::COMPTIME_VERSION` parse them at compile time; `src/main.rs` uses them.

## Gate
build/test/fmt pass for touched crates. Workspace `clippy -D warnings` fails on pre-existing code in other crates
(`ghostty-onig` tests unwrap, `ghostty-unicode/build.rs` and `ghostty-os/src/uri.rs` format_push_string); touched crates are clean
with `--no-deps`.
