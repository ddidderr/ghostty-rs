# ghostty-rs

A Rust rewrite of the [Ghostty](https://ghostty.org) 1.3.1 terminal emulator
for Linux (GTK4 + libadwaita, OpenGL).

It aims for behavioural parity with Ghostty: the same configuration file
format, options, defaults and documentation; the same keybindings and
actions; the same `ghostty +action` CLI; the same terminal emulation
(including kitty keyboard/graphics protocols, OSC 8 hyperlinks, semantic
prompts, synchronized output, tmux control mode); the same shell
integration, themes and terminfo.

All of Ghostty's Zig code is ported: the Linux GTK app (verified at runtime
and pixel-identical to Ghostty 1.3.1), the `libghostty` and `libghostty-vt`
C APIs (same headers/ABI), WASM, crash reporting, and the macOS
(Metal/CoreText) and Windows (ConPTY) code paths. macOS and Windows are
compile-checked on Linux (`just check-macos`, `just check-windows`); the
Windows test suites also run under Wine (`just test-windows-wine`). On macOS
the Swift app runs natively on the Rust core and renders pixel-identical to
Ghostty 1.3.1 for a static test screen. The Swift macOS app is not part of the
port; it is a client of the libghostty C API.

### The macOS app (original Swift app on the Rust core)

Ghostty's macOS app stays the original Swift app, unchanged; it is vendored
from Ghostty 1.3.1 in `macos/` (plus `images/Ghostty.icon`, MIT, see
`macos/LICENSE`). It is wired to the Rust core instead of the Zig core: the
Rust `libghostty` exports the identical `include/ghostty.h` ABI.

Requirements: Xcode, `just`, `xz` (to unpack the gettext source that
`gettext-sys` builds from) and the Rust targets `aarch64-apple-darwin`,
`x86_64-apple-darwin`, `aarch64-apple-ios` and `aarch64-apple-ios-sim` (only
the host target for `native`). Then:

```sh
just macos-swift-app                     # ReleaseLocal; or: just macos-swift-app Debug
just macos-swift-app ReleaseLocal native # host architecture only (no iOS/x86_64 slices)
```

This builds the Rust core as `macos/GhosttyKit.xcframework`, installs the
resources into `zig-out/share` (what the Xcode project bundles) and runs
`xcodebuild`, mirroring `zig build -Demit-macos-app`; the app ends up in
`macos/build/<config>/Ghostty.app`. `DRY_RUN=1` previews the steps (works on
any host).

As in upstream, `cargo build` on macOS (and Windows) produces no terminal: the
`ghostty` binary is the helper CLI for `+actions` (`ghostty +version`,
`ghostty +list-fonts`, ...). The GTK app is only built on Linux and FreeBSD.

## Building and running

Requirements: Rust (edition 2024), GTK ≥ 4.14, libadwaita ≥ 1.5,
fontconfig, freetype, harfbuzz, Oniguruma, libepoxy, `blueprint-compiler`
and `glib-compile-resources` at build time.

```sh
cargo run                      # open a terminal window running your shell
cargo run -- -e htop           # run a command
cargo run -- +list-themes      # CLI actions work like Ghostty's
cargo run --profile production # optimized build (LTO, no debug assertions)
```

Binaries run from this checkout's `target/` (any profile) find `resources/`
(shell integration, themes, terminfo) automatically; installed binaries use
the installed resources or `GHOSTTY_RESOURCES_DIR`. Your existing Ghostty config
(`~/.config/ghostty/config[.ghostty]`) is used as-is.

## Installing

```sh
just install /usr/local          # binary, share/ghostty, terminfo, desktop files,
                                 # completions, man pages, translations, icons
just install /usr false true     # system-package layout (lib/systemd/user)
```

### Build-time resource directory

Set `GHOSTTY_BUILD_RESOURCES_DIR` while building or installing to embed an
explicit resource directory:

```sh
GHOSTTY_BUILD_RESOURCES_DIR=/usr/share/ghostty cargo build --profile production
GHOSTTY_BUILD_RESOURCES_DIR=/usr/share/ghostty cargo install --path . --profile production
GHOSTTY_BUILD_RESOURCES_DIR=../share/ghostty cargo build --profile production
```

Absolute paths are used directly. Relative paths resolve from the running
executable's directory, not the current working directory; `../share/ghostty`
allows moving an installed `bin/` and `share/` tree together. The directory
contains `themes/` and `shell-integration/`; these files still need to be installed
separately (`cargo install` only installs the executable).

A nonempty runtime `GHOSTTY_RESOURCES_DIR` overrides the embedded path in every
profile. The embedded path takes precedence over automatic resource discovery
and does not require a terminfo sentinel. An unset or empty build option keeps
the existing discovery rules, including the debug profile's preference for
installed resources over the runtime override. Changing or unsetting the build
option causes Cargo to rebuild resource lookup. Paths are literal: `~` and shell
variables are not expanded by Ghostty. Flatpak's sandbox resource layout remains
managed by its runtime integration.

## Testing

```sh
just test      # cargo test --workspace (~3000 tests)
just clippy    # clippy with -D warnings
```

Every Zig `test` block of the ported files was ported to a Rust test.
`crates/ghostty-core/tests` holds end-to-end tests that drive the real core,
terminal and a real pty through a fake app runtime. `ghostty-bench` mirrors
Ghostty's benchmark tool; terminal stream throughput is on par with the Zig
build (see `docs/rewrite/notes/perf-P1.md`).

## Verification

- `just difftest` — differential fuzzing of the terminal core against
  Ghostty's own Zig code (sets up Zig 0.15.2 under `target/`).
- `just visual-parity` — pixel comparison against the installed Ghostty in a
  headless nested weston.
- `just test-windows-wine` — runs the Windows test suites (incl. ConPTY /
  `cmd.exe` end-to-end tests) under Wine with mingw-w64.
- `just check-all` — native gate plus macOS/Windows/WASM checks.

## Documentation

- `docs/rewrite/ARCHITECTURE.md` – top-down decomposition, crate map, key
  design decisions, intentional omissions
- `docs/rewrite/STATUS.md` – work package history
- `docs/rewrite/DEVIANT_FIXES.md` – every intentional deviation from Ghostty
  (upstream bugs fixed instead of replicated; commits titled `fix-deviant(...)`)
- `docs/rewrite/KNOWN_DIFFERENCES.md` – remaining minor differences
- `docs/rewrite/notes/` – per-crate porting notes, deviations from Zig,
  audits and verification reports

## License

MIT, like Ghostty. Bundled fonts and themes carry their own licenses (see
`crates/ghostty-font/res/` and `resources/LICENSE-iTerm2-Color-Schemes`).
