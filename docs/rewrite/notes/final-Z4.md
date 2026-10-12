# Z4: artifact and distribution gaps

- Webdata (`GhosttyWebdata.zig`): `ghostty-build-data +webgen-{config,actions,commands}` (`src/webdata.rs`);
  `just webdata`, `just install-webdata PREFIX`. Byte-identical to Zig 0.15.2 output of a copy whose webgen mains
  gained `stdout.flush()`. Upstream's mains never flush the 2048-byte buffer, so the real Zig files are truncated.
  Rust emits the complete files. Also `+version` mode (build-info version string).
- libghostty / vt install: `scripts/install-lib.sh`, `just install-lib PREFIX PROFILE WHAT`; `just install` installs vt
  (as Zig's default install does). Layout, headers and `libghostty-vt.pc` match the Zig output (soname `.so.0` set via
  patchelf, symlinks `.so`, `.so.0`, `.so.0.1.0`). libghostty adds `ghostty.h` + `module.modulemap`.
- XCFramework: `scripts/xcframework.sh` / `just xcframework` (cargo staticlib per target, `lipo -create`,
  `xcodebuild -create-xcframework -headers crates/ghostty-lib/include`, iOS + simulator slices). Verified only with
  `DRY_RUN=1`: no macOS tools or SDK here; the darwin cross build was not attempted.
- App id: `just install` defaults to `--profile production` (ReleaseFast); other profiles get `-debug` id/name, like
  Zig Debug/ReleaseSafe. Desktop, D-Bus, systemd and metainfo files use it. DESTDIR install checked for production
  (`com.mitchellh.ghostty`, build mode .ReleaseFast) and release (`-debug`).
- i18n: `scripts/update-translations.sh` / `just update-translations` (xgettext with the same flags + `--keyword=tr`
  over blueprints and apprt-gtk Rust sources, py.pot, merge, msgmerge). msgids identical to upstream's .pot.
  Found and fixed: `surface_child_exited.rs` didn't translate "Command succeeded/failed" (now `tr`). Script accepts
  an OUT_DIR; po/ was not rewritten in place (location comments would churn).
- Dist: `scripts/dist.sh`, `just dist` / `just distcheck`: `ghostty-<version>.tar.gz` with `VERSION` and the prebuilt
  `crates/ghostty-apprt-gtk/dist/ghostty.gresource` (used by apprt-gtk/build.rs outside a git checkout, like Zig's
  `ghostty_resources.c/h`). Framedata is already committed. As in Zig, VERSION is for packagers (`GHOSTTY_VERSION_STRING`).
  Uses `git ls-files` (includes uncommitted files), not `git archive HEAD`. distcheck passes.
- Windows: `dist/windows/{ghostty.rc,ico,manifest}` copied; root `build.rs` uses `embed-resource` plus the GUI subsystem.
  The root binary can't cargo-check for Windows (GTK/wayland deps), so verified with a scratch crate using the same
  build.rs for x86_64-pc-windows-gnu: PE subsystem 2, manifest (PerMonitorV2, Common-Controls 6) present.
- Touched outside lane: apprt-gtk `build.rs` (prebuilt gresource) and `surface_child_exited.rs` (tr).
