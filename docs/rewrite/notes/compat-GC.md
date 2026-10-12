# compat-GC: GTK/libadwaita compat fallbacks, bench modes, helpgen, CLI Windows

Sub-notes: `compat-GC-bench.md`, `compat-GC-helpgen.md`, `compat-GC-cli.md`.

## 1. GTK / libadwaita compatibility (crates/ghostty-apprt-gtk)

### Ghostty's own minimum (finding)
`ZIG/src/build/SharedDeps.zig` runs `blueprint.zig` over *every* entry of
`gresource.blueprints`, which refuses to compile a `<maj>.<min>/x.blp` if the
libadwaita headers are older than that version; the list contains `1.5/` files
(window, tab, command palette, ...). So Ghostty 1.3.1 cannot be *built* against
libadwaita < 1.5, and the Zig code calls `gtk_css_provider_load_from_bytes`
(4.12) and `gtk_im_context_activate_osk` (4.14) unguarded. Ghostty's real
minimum is therefore **GTK 4.14 / libadwaita 1.5**, which is what the crate
already required; I verified this by compiling with `gtk4` features `v4_8`/`v4_10`
(fails on `load_from_bytes`/`activate_osk`) and `v4_14` (compiles). The only
change to the requirement is dropping the unneeded `gnome_44` feature. In Zig the
fallbacks below are only reachable in a (hypothetical) build against older
headers; they are ported with identical selection logic.

### Version gating (gtk_version.rs, adw_version.rs)
Zig's `atLeast` = comptime header version AND runtime library version. Rust:
* cargo features are the "header version": `adw_1_3` < `adw_1_4` < `adw_1_5`
  and `gtk_4_16` (`gtk/v4_16`); all are default. `BUILD_LEVEL` /
  `const fn comptime_at_least` read them; `at_least` = `comptime_at_least &&
  runtime_at_least`; `runtime_at_least`/`runtime_until` unchanged.
  `COMPTIME_VERSION` (pkg-config) is kept for `+version` output.
* Types that Zig selects at comptime are selected with `cfg(feature = ...)`.
* Debug builds only: `GHOSTTY_DEBUG_FAKE_GTK_VERSION` / `GHOSTTY_DEBUG_FAKE_ADW_VERSION`
  (e.g. `1.4.0`) override the *runtime* version answers (`fake_version.rs`; no Zig
  counterpart) so runtime branches can be exercised on new libraries.
* Root `build.rs` links the binary with `-z lazy` (Rust defaults to `-z now`) so
  symbols only used behind a runtime check (GTK 4.16 `gtk_css_section_get_bytes`)
  don't stop an older library from loading the program, like Zig's lazy binding.

### Fallbacks ported
| Zig | Rust |
|---|---|
| `class/dialog.zig` `Parent = AlertDialog / MessageDialog`, `present`/`close` | `class/dialog.rs`: `DialogParent` + `present`/`close` per cfg `adw_1_5`; macros `dialog_subclass_wrapper!`/`impl_dialog_parent!` give the three subclasses (close-confirmation, config-errors, clipboard) the right `@extends` chain and `response` override for both parents |
| `clipboard_confirmation_dialog.zig` `can_remember`, template 1.4 vs 1.0 | `cfg_attr` template resource `ui/1.4` (adw_1_4) vs `ui/1.0`; `remember_choice` child only with SwitchRow; `CAN_REMEMBER`; `remember=false` without it |
| `debug_warning.zig` Parent `adw.Bin`/`gtk.Box`, template 1.3/1.2 | `class/debug_warning.rs` cfg `adw_1_3` |
| `surface_child_exited.zig` Banner / `SurfaceChildExitedNoop` | `class/surface_child_exited.rs`: noop widget (no template, `NOOP`, `close-request` emitted from `set_child_exited`); `Surface::child_exited` returns false for the noop, as in Zig. NB: Zig has no `1.2/surface-child-exited.blp` (the task text assumed one); the pre-Banner implementation is the Noop |
| `window.zig` `actionAbout` | `adw::AboutDialog` if `supports_dialogs()` else `gtk::show_about_dialog` |
| `application.zig` `signalCssParsingError` (GTK >= 4.16 branch) | `css.rs`: `domain code message` + `\n<source bytes>` (`CssSection::bytes`) behind `gtk_4_16` + runtime check, plain line otherwise; tested via `parsing_error_message` |
| blueprint variants | `build.rs` already compiles 1.0/1.2/1.3/1.4/1.5 variants (same list as `gresource.zig`) into the gresource; the class selects at build level (as Zig) |

### Verification
* `cargo clippy -p ghostty-apprt-gtk --all-targets -D warnings` clean with default
  features AND with `default = []` (temporarily edited), i.e. both the new and
  the fallback code compile.
* Default build: GTK app runs with the debug replay hook (paste dialog shows
  `GhosttyClipboardConfirmationDialog` with the remember switch; no GTK criticals).
* Fallback build (`default = []`, run via the replay hook): the paste confirmation
  and the close confirmation come up as `adw::MessageDialog`-based
  `GhosttyClipboardConfirmationDialog` / `GhosttyCloseConfirmationDialog`
  (transient for the window), the binary has no `BIND_NOW`.
* Workspace gate (excluding L1's in-progress `ghostty-lib`): `cargo clippy --workspace
  --all-targets -D warnings`, `cargo test --workspace`, `cargo +nightly fmt --all` pass.
  `+help`, `+list-actions [--docs]`, `+list-keybinds --help`, `+show-config --help`
  are byte-identical to `/usr/bin/ghostty`.

### Deviation found by the runtime check
Zig's `GhosttyCloseConfirmationDialog` registers a `close-request` signal; with
the `MessageDialog` (a `GtkWindow`) parent that name already exists in the class
ancestry and GObject aborts. Zig's fallback has the same latent bug (dead code
upstream). The fallback build here names it `dialog-close-request`
(`close_confirmation_dialog.rs::CLOSE_REQUEST`); default build unchanged.

### Not testable here
Real old libraries (GTK < 4.16 / libadwaita < 1.5) are not installed; only the
compile-time fallback selection and the version arithmetic are exercised.

## 2-4
See the sub-notes. Deviations vs the task text: Zig `CodepointWidth` has no
`uucode` mode (bench); Zig does no file locking on Windows, so the faithful
port is a no-op there (cli).
