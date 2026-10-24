# Vendored crates

Patched copies of crates.io dependencies, wired in via `[patch.crates-io]` in
the root `Cargo.toml`. Drop a crate here once upstream ships the fix.

- `gettext-sys` 0.27.0: the build script passes Rust's `aarch64-apple-ios-sim`
  triple to autoconf as `--host`, which `config.sub` rejects. The patch strips
  the `-sim` suffix (the simulator SDK is selected by the compiler flags), so
  the iOS simulator slice of `GhosttyKit.xcframework` builds. Otherwise
  unchanged.
