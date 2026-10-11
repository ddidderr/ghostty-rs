# ghostty-cli port notes (C1)

Crate `crates/ghostty-cli` (deps: ghostty-config, ghostty-input, ghostty-os; plus
ratatui/crossterm for TUIs, flate2 for boo frames, unicode-width). Run end to end with
`cargo run -p ghostty-cli --example cli -- +list-actions --docs` (the example behaves
like `ghostty` for actions via `ghostty_cli::run_from_args`, which also reproduces
main_ghostty.zig's "multiple/unknown action" errors). Do not edit the root crate:
wire `run_from_args(&std::env::args().collect::<Vec<_>>())` into its main later.

## API
Every action module exposes `pub fn run(args: &[String]) -> Result<u8, Error>`; `args` is the
full argv (argv[0] skipped, `+action` args ignored, like `args.argsIterator`). `--help`/`-h`
yields `Error::ActionHelpRequested`, which `Action::run` turns into the help text (stdout, exit 0).
`Action::{ALL,name,from_name,file,help_text,run}`, `action::{detect_iter,ActionEnum,SpecialCase}`.

## File -> module
| Zig | Rust |
|---|---|
| cli.zig / action.zig | `action.rs` (generic detection + 7 tests) |
| ghostty.zig | `ghostty.rs` (`Action`, dispatch, help, `run_from_args`; 5 tests) |
| help.zig, version.zig | `help.rs`, `version.rs` (+ `build_info.rs`) |
| list_actions/list_colors/list_keybinds | same names; pretty output via `tui.rs` |
| list_themes.zig | `list_themes.rs` (listing) + `list_themes/preview.rs` (ratatui TUI) |
| show_config/validate_config/edit_config/crash_report | same names |
| ssh_cache.zig + ssh-cache/{Entry,DiskCache} | `ssh_cache.rs`, `ssh_cache/{entry,disk_cache}.rs` |
| boo.zig (+ build/framegen/frames) | `boo.rs`, `boo_frames.deflate` (235 frames, raw deflate of frames joined by \x01) |
| tui.zig | `tui.rs` (+ a small vaxis `prettyPrint` clone) |
| list_fonts/show_face/new_window | stubs: Options + help + `Error::NotYetAvailable` (TODO(integration)) |

Help texts (`help/*.txt`) are the `run` doc comments of the Zig files, extracted with the
helpgen rules (common-indent strip); `--help` output is byte-identical to Ghostty 1.3.1.

## Oracle verification (Ghostty 1.3.1 at /usr/bin/ghostty)
Byte-identical (stdout, stderr, exit code): `+list-actions` (+`--docs`, `--help`), `+list-keybinds`
(`--plain`, `--default`, `--docs`; also with a custom config containing chords, chains, tables,
catch_all, unicode keys), `+list-themes --plain` (`--path`, `--color=dark|light`; with
`GHOSTTY_RESOURCES_DIR=/usr/share/ghostty`), `+help`, `--help`, `-h`, `+validate-config` (ok and
missing file), `+crash-report`, `+ssh-cache`, bad action / multiple actions / bad flag errors,
`--help` of every action, `+show-config` (except the two known config deviations: `auto-update-channel`
tip vs stable, `gtk-opengl-debug` debug build). **Pretty (tty) output of `+list-colors` and
`+list-keybinds` is byte-identical in a pty** for widths 20..200 (including line wrapping and
the first-row/teardown sequences of libvaxis); verified with a pty harness.

Known differences:
* `+list-colors --plain`: Ghostty 1.3.1 never flushes its 4 KiB stdout buffer on this path, so its
  output is truncated at 617 of 782 lines. We print all 782 (first 617 identical).
* `+list-colors` in a tty narrower than one column: Zig divides by zero; we clamp to one column.
* `+version` differs in version/build info (placeholders in `build_info`; TODO(integration): root
  crate / apprt-gtk should call `build_info::set` with the real GTK/libadwaita versions, commit
  and channel). "Zig version" row became "Rust version", "libxev" row prints `n/a`.
* `+edit-config` exec failure prints `error.FileNotFound`/`error.AccessDenied` for those kinds,
  otherwise the Rust io error text.
* `+new-window`: the Zig parse hook swallows `--help`; ported as is. IPC not available.

## Interactive `+list-themes` (ratatui)
Same layout as the vaxis version: 32-column list with `❯ name ❮` selection, preview (header, 16
color swatches with decimal/hex numbers, 24-line code sample, lorem ipsum filler, diagnostics /
load-error panels), help (60x22), search (3-high box at bottom) and save (92x17) overlays, all
key bindings (q/Esc/^C, ?/F1/^H, /, ^X ^/, j/k/arrows, PgUp/PgDn (20), Home/End, h/x/d hex
toggle, c / C copy name/path via OSC 52, f cycle filter, Enter save dialog, w write
`auto/theme.ghostty`), mouse wheel / click / hover. Differences: no OSC 8 hyperlinks on names,
double/dashed/dotted/curly underlines drawn as single, color scheme not queried live (defaults to
light like Zig; `COLORFGBG` used as a hint), theme load errors during filtering are ignored
(Zig aborts), `zf` fuzzy ranking replaced by a subsequence matcher (only matching, not order, is
used). Logic (filtering, navigation, windowing, search, save) is unit tested; rendering is tested
against a ratatui `Buffer`.

## Tests
Ported every Zig test in the ported files: action.zig (7 detect tests), ghostty.zig (5 parse
action tests), ssh-cache/Entry.zig (6), DiskCache.zig (4 operation tests + isValidHost,
isValidUser, isValidCacheKey). Skipped: none. 86 tests total including new ones (help/version
output, show-config, validate-config, crash-report, ssh-cache action, list-colors/keybinds/themes,
tui row rendering vs vaxis bytes, boo frame parsing of all 235 frames, preview state machine,
new-window arg hook).

## TODO(integration) / gaps
* `list-fonts`, `show-face` (need ghostty-font), `new-window` (GTK IPC): return `Error::NotYetAvailable`.
* `build_info::set` for real version/GTK info.
* ghostty-vt `x11_color` lacks a key listing and loses name case: `list_colors` re-parses
  `ghostty-vt/src/res/rgb.txt` through a relative `include_str!` path.
* ghostty-input `ActionKey::docs()` has an extra trailing blank line for 4 actions
  (adjust_selection, ...) compared to Ghostty's help strings; `list_actions::generate_plaintext`
  is a local copy of `helpgen_actions::generate` that normalises trailing newlines. Fix the data in
  ghostty-input and switch back to `helpgen_actions::generate`.
* crash-report only lists (sentry is not ported). `ssh-cache` locking uses `File::try_lock`;
  like Zig, a lock held by another process is ignored (the boolean result is unused upstream).
* `Config::load` and `finalize` are called twice in `validate-config` (as in Zig).

## Integration pass: list-fonts / show-face / new-window (wired)
* `list_fonts.rs`, `show_face.rs` now use `ghostty-font` (fontconfig `Discover`, `SharedGridSet`, `SharedGrid::get_index`,
  `Collection::get_face`); ghostty-cli depends on ghostty-font and ghostty-apprt. Output is byte-identical to
  /usr/bin/ghostty for `+list-fonts` (plain, `--family`, `--bold`) and `+show-face` (`--cp`, `--string`, `--presentation`).
  Like Zig, show-face writes its "stderr" messages to stdout.
* `new_window.rs`: `set_ipc_handler(fn(&ipc::Target, &ipc::Action) -> Result<bool, String>)` (OnceLock). The root binary must
  register it (map `ghostty_apprt_gtk::ipc::perform_ipc`'s `Err(IpcFailed)` to `Err(new_window::IPC_FAILED.into())`).
  Without a handler: prints "+new-window is not supported on this platform." and exits 1 (apprt none behaviour).
* Still open: `build_info::set`.
