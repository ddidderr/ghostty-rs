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
Windows test suites also run under Wine (`just test-windows-wine`), macOS has
not been run natively. The Swift macOS app is not part of the port; it is a
client of the libghostty C API.

### The macOS app (original Swift app on the Rust core)

Ghostty's macOS app stays the original Swift app, unchanged. It is wired to
the Rust core instead of the Zig core: the Rust `libghostty` exports the
identical `include/ghostty.h` ABI. On a Mac with Xcode:

```sh
just macos-swift-app ../ghostty          # Release; or: just macos-swift-app ../ghostty Debug
```

This builds the Rust core as `GhosttyKit.xcframework` into
`<ghostty>/macos/`, installs the resources into `<ghostty>/zig-out/share`
(what the Xcode project bundles) and runs `xcodebuild`, mirroring
`zig build -Demit-macos-app`. Not runnable on this Linux machine
(`DRY_RUN=1` previews the steps).

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

Dev-profile builds (`cargo run`) find `resources/` (shell integration,
themes, terminfo) automatically; other profiles use the installed
resources or `GHOSTTY_RESOURCES_DIR`. Your existing Ghostty config
(`~/.config/ghostty/config[.ghostty]`) is used as-is.

## Installing

```sh
just install /usr/local          # binary, share/ghostty, terminfo, desktop files,
                                 # completions, man pages, translations, icons
just install /usr false true     # system-package layout (lib/systemd/user)
```

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
