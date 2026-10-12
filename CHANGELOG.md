# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to
[Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- Complete Rust rewrite of Ghostty 1.3.1 for Linux: VT parser and stream,
  terminal core (page list, screen, terminal, selection, formatter, search,
  kitty graphics, tmux control mode), font engine (fontconfig, freetype,
  harfbuzz, sprite font), OpenGL renderer with custom shader support,
  termio (pty, child process, shell integration, terminfo), configuration
  system with all options and docs, input and keybindings, core App/Surface,
  GTK4/libadwaita app runtime, terminal inspector, and all `ghostty +action`
  CLI commands.
- Bundled resources: shell integration, 463 themes, compiled terminfo,
  desktop integration files, translations.
- `ghostty-bench` (benchmarks) and `ghostty-build-data` (shell completions,
  Vim/Sublime/bat syntax, man pages).
- About 3000 tests, including all ported Zig tests and headless end-to-end
  tests of the core with a real pty.
