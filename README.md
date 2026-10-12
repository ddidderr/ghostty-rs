# ghostty-rs

A Rust rewrite of the [Ghostty](https://ghostty.org) 1.3.1 terminal emulator
for Linux (GTK4 + libadwaita, OpenGL).

It aims for behavioural parity with Ghostty: the same configuration file
format, options, defaults and documentation; the same keybindings and
actions; the same `ghostty +action` CLI; the same terminal emulation
(including kitty keyboard/graphics protocols, OSC 8 hyperlinks, semantic
prompts, synchronized output, tmux control mode); the same shell
integration, themes and terminfo.

Out of scope: macOS, Windows, WASM, the embedded `libghostty` C API, and
Sentry crash reporting (disabled by default on Linux upstream).

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

Development builds find `resources/` (shell integration, themes, terminfo)
automatically. Your existing Ghostty config
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

## Documentation

- `docs/rewrite/ARCHITECTURE.md` – top-down decomposition, crate map, key
  design decisions, intentional omissions
- `docs/rewrite/STATUS.md` – work package history
- `docs/rewrite/notes/` – per-crate porting notes, deviations from Zig,
  audits and verification reports

## License

MIT, like Ghostty. Bundled fonts and themes carry their own licenses (see
`crates/ghostty-font/res/` and `resources/themes/`).
