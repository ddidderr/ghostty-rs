# Codex final completeness re-audit (2026-10-12, commit 6d309d5)

| Source classification | Files |
|---|---:|
| **Full** | **397** |
| **Partial** | **30** |
| **Missing** | **1** |
| **In-scope total** | **428** |
| `src/build/**/*.zig`, assessed through their outputs | 33 |
| **Total Zig inventory** | **461** |

Additionally, **8 artifact/distribution groups remain incomplete: 4 partial, 4 missing**. These are counted separately from source files.

Audited Rust commit: `6d309d5d16e2`. This was a static code audit; I did not run builds or tests. No files were modified, and the working tree remains clean. “Full” means a counterpart exists and this audit identified no concrete remaining functionality gap.

Zig paths below are relative to [ghostty/src](../../../../ghostty/src). `R/<crate>/…` means `ghostty-rs/crates/ghostty-<crate>/src/…`.

The remaining source gaps are:

| Zig file:line | Rust location | Status | Precisely what remains missing or different | Importance |
|---|---|---|---|---|
| `Surface.zig:645,2033,5992` | `R/core/surface/mod.rs:474,938,1367`; `R/core/surface/binding.rs:427` | Partial | Windows surface initialization assigns four fields absent from Windows `exec::Config`, including references to absent `RtPreExecInfo`/`RtPostForkInfo`. Screen-file writing imports Unix `OpenOptionsExt` unconditionally. Separately, `pwd()` and `PwdChange` convert preserved OSC 7 bytes using `from_utf8_lossy`, so non-UTF-8 working directories still do not survive the complete surface path. | **High**: Windows compilation; Medium: directory handling |
| `global.zig:120,164,198,224` | `R/core/global.rs:9,153,178,184` | Partial | Unconditional `Rlimit` imports/calls have no Windows definitions. Embedded initialization never calls the implemented Darwin `ensure_locale`. SIGPIPE handling assumes Rust executable startup has already ignored it; C/Swift hosts do not receive that initialization through `ghostty_init`. Crash initialization also leaves GTK/fontconfig/OpenGL defaults in embedded Metal builds. | **High**: Windows compilation; Medium: embedded initialization |
| `os/file.zig:7,12,51,59` | `R/os/file.rs:5,31,68,75` | Partial | Upstream supplies unsupported-platform rlimit fallbacks; Rust omits the types/functions entirely outside Unix, breaking shared-core consumers. `TMPDIR`/`TMP` values are also converted lossily before filesystem use. | **High** / Medium |
| `renderer/shadertoy.zig:161,254` | `R/renderer/shadertoy_sampler.rs:235,304`; `R/renderer/shadertoy.rs:38,312` | Partial | Sampler rewriting records one parameter layout per function **name**, so valid overloads with different sampler positions are rewritten incorrectly. Additionally, Metal output does not preserve the `iChannelTime[4]` std140 stride: Rust uploads 64 bytes for that array, but the selected Naga MSL writer emits four contiguous floats and ignores the array stride. Consequently, following uniform members are read at incorrect offsets. | Medium: custom shaders |
| `main_ghostty.zig:26` | Root `src/main.rs:24`; `R/os/args.rs:33` | Partial | Startup no longer panics on non-UTF-8 argv, but `collect_lossy()` replaces the bytes before action dispatch. Byte preservation remains incomplete. | Medium |
| `cli/args.zig:566,1366,1393` | `R/config/cli_args.rs:398,654,794` | Partial | Argument iteration, configuration-line decoding, and quoted Zig-string decoding use lossy UTF-8 conversion. For example, a decoded `\xFF` byte becomes U+FFFD rather than remaining `0xFF`. | Medium |
| `config/Config.zig:4067,4141` | `R/config/config.rs:4070,4100,4129` | Partial | The `xdg-terminal-exec` branch converts OS arguments lossily; normal CLI loading uses the lossy argument iterator. Current-directory conversion also loses bytes before path expansion. | Medium |
| `main_c.zig:105` | `R/lib/main_c.rs:79,90,102` | Partial | Supplied `ghostty_init(argc, argv)` arguments are converted lossily and stored privately for action dispatch. They are not supplied to subsequent configuration CLI loading. In a non-macOS C host, passing an argv different from the process argv therefore behaves differently from upstream. | Medium |
| `config/CApi.zig:54,66` | `R/lib/config_capi.rs:210,243,253` | Partial | `ghostty_config_load_cli_args` reads process arguments rather than the argv supplied to `ghostty_init`. `ghostty_config_load_file` converts its C filename lossily, preventing access to valid non-UTF-8 Unix paths. | Medium |
| `apprt/embedded.zig:430,531,1799` | `R/lib/embedded.rs:888,906,922,1596` | Partial | Surface working-directory, command, and environment options undergo lossy conversion. `ghostty_surface_text` also replaces invalid UTF-8 bytes before forwarding text, whereas upstream forwards the supplied byte slice. | Medium |
| `apprt/gtk/class/surface.zig:2633,2660` | `R/apprt-gtk/class/surface.rs:3378,3400` | Partial | File-drop handling converts `gio::File` paths lossily before shell escaping and pasting. Dropping a file with a non-UTF-8 filename therefore inserts a different path. The previously reported Snap filtering and child-environment capture defects are fixed. | Medium |
| `os/env.zig:83,102` | `R/os/env.rs:241,246` | Partial | `EnvMap` preserves OS bytes, but standalone `getenv`/`getenv_not_empty` use `std::env::var(...).ok()`. A present non-UTF-8 value is treated as absent; this affects callers such as XDG-directory discovery. | Medium |
| `os/homedir.zig:28` | `R/os/homedir.rs:45,76` | Partial | `$HOME` and shell-fallback output are converted lossily before returning a filesystem path. | Medium |
| `os/path.zig:16,27` | `R/os/path.rs:39` | Partial | `$PATH` is converted lossily before executable lookup. Executables in directories containing non-UTF-8 bytes cannot be found through that entry. | Medium |
| `os/passwd.zig:120,125,131` | `R/os/passwd.rs:39,40,115` | Partial | Native passwd shell/home paths and Flatpak `getent` output lose non-UTF-8 bytes. | Medium |
| `os/resourcesdir.zig:40,70` | `R/os/resourcesdir.rs:53,85,151` | Partial | A non-UTF-8 `GHOSTTY_RESOURCES_DIR` is ignored. Executable-relative resource discovery converts directory paths lossily before checking them. | Medium |
| `config/path.zig:201` | `R/config/path.rs:150` | Partial | Canonicalized paths are converted lossily. Even a UTF-8 config path can fail when a symlink resolves to a non-UTF-8 target. | Medium |
| `cli/validate_config.zig:61` | `R/cli/validate_config.rs:42` | Partial | The canonicalized config filename is converted lossily before loading, causing the same symlink-target failure in `+validate-config`. | Medium |
| `cli/list_themes.zig:154,158,162` | `R/cli/list_themes.rs:140` | Partial | Directory-entry names are converted lossily and then used to construct theme paths. Such themes cannot be correctly listed/selected. The previously missing preview features are implemented. | Medium |
| `lib/allocator.zig:78` | `R/vt-capi/alloc.rs:1,72` | Partial | The supplied C allocator controls wrapper allocations and selected buffers, but not interior Rust collections. It does not provide upstream’s allocator control over all parser-owned storage. | Medium: C allocator contract |
| `terminal/c/osc.zig:15,22` | `R/vt-capi/osc.rs:66,130` | Partial | The caller’s allocator is not propagated into the OSC parser or its title buffer: these allocate through Rust’s global allocator. Caller allocation limits/failure behavior consequently do not govern OSC payload storage. | Medium |
| `os/i18n.zig:88,115` | `R/os/i18n.rs:88` | Partial | Locale canonicalization substitutes hyphens and handles Chinese cases, but omits the other upstream libintl mappings. For example, upstream maps `pa-Guru` to `pa_IN`; Rust returns `pa_Guru`. | Medium |
| `crash/sentry.zig:81,259` | `R/crash/sentry.rs:109`; `R/crash/signal.rs:135,217` | Partial | Local reports, envelopes, and context hooks exist. Fatal-signal handling discards `siginfo` and machine context and emits an event without a native stack, registers, or minidump. This does not replace upstream’s Sentry-native/Breakpad capture. Panic backtraces do not cover this case. | Medium: enabled crash reporting |
| `build_config.zig:17,24,41` | `R/cli/build_info.rs:53`; `R/termio/stream_handler.rs:89`; `R/crash/sentry.rs:49` | Partial | Derived metadata exists for CLI reporting, but is not shared consistently. Terminal/child version reporting remains hardcoded, while crash backend metadata uses fixed defaults. A version override does not propagate to all consumers. | Low |
| `termio/Exec.zig:743` | `R/termio/exec.rs:848`; `R/termio/exec_windows.rs:633` | Partial | `TERM_PROGRAM_VERSION` uses the fixed `"1.3.1"` constant rather than the detected/overridden build version. Flatpak shutdown and crash-thread context gaps are fixed. | Low |
| `termio/stream_handler.zig:979,986` | `R/termio/stream_handler.rs:89,663` | Partial | XTVERSION reports the fixed `"1.3.1"` rather than the actual build version. | Low |
| `apprt/gtk/class/window.zig:1772,1789` | `R/apprt-gtk/class/window.rs:1665,1678` | Partial | Both About-dialog branches report `CARGO_PKG_VERSION`, currently `0.1.0`, rather than the Ghostty application version. The older-library dialog fallback itself is implemented. | Low |
| `os/cgroup.zig:6,25` | `R/os/cgroup.rs:24` | Partial | The returned cgroup path is converted lossily instead of preserving the bytes read from `/proc`. | Low |
| `benchmark/Benchmark.zig:41` | `R/bench/benchmark.rs:44` | Partial | Benchmark execution lacks the Darwin Instruments signpost interval emitted around upstream benchmark runs. | Low: profiling |
| `terminal/kitty/graphics_image.zig:372` | `R/terminal/kitty/graphics_image.rs`, `LoadingImage` | Partial | The debug-only `debugDump` facility for writing the current image payload to a diagnostic file has no counterpart. | Low: developer diagnostics |
| `tripwire.zig:107,159,182` | No counterpart; related omissions documented in `R/terminal/tabstops.rs:241` and `R/terminal/screen/tests/p5.rs:688` | **Missing** | Named failure injection, hit-count triggering, and verification of expected trips are absent. Associated allocation-failure tests are explicitly skipped. | Low: test infrastructure |

The Metal layout finding follows the actual dependency implementation: Naga 30.0.1’s GLSL frontend gives the scalar array a 16-byte stride, while its MSL writer’s array branch ignores `stride` and emits a plain scalar array. `msl_from_compiled` applies no correction. This is separate from the sampler-overload limitation.

The remaining artifact and distribution gaps are:

| Zig producer:line / output | Rust location | Status | Precisely what remains missing or different | Importance |
|---|---|---|---|---|
| `build/GhosttyWebdata.zig:45,76,107` — `config.mdx`, `actions.mdx`, `commands.mdx` | `crates/ghostty-build-data/src/lib.rs`, mode dispatch | **Missing** | No generator or installation counterpart for the three website-reference MDX outputs. Man-page generation does not produce them. | Medium |
| `build/GhosttyXCFramework.zig:62`; `build/GhosttyLib.zig:118` — universal library and `GhosttyKit.xcframework` | `crates/ghostty-lib/Cargo.toml:9` | **Missing** | Per-target static/shared libraries exist, but universal-library assembly and XCFramework packaging do not. This concerns the C library deliverable, not the excluded Swift app. | Medium |
| `build/GhosttyLib.zig:142,148` — installed native library/header | `crates/ghostty-lib/Cargo.toml`; `crates/ghostty-lib/include/`; `justfile` | Partial | Library outputs and the header exist, but there is no equivalent prefix installation of the library and `include/ghostty.h`. | Medium |
| `build/GhosttyLibVt.zig:103,130` — native VT distribution | `crates/ghostty-vt-capi/build.rs`; `justfile:30`; crate-local `include/` | Partial | No `libghostty-vt.pc` output or prefix installation of the VT library/header tree. The recipe copies unversioned library names inside `target`; it does not reproduce the upstream installed library distribution. | Medium |
| `build/GhosttyResources.zig:267,274` — Linux desktop/D-Bus identity | `justfile:102,153`; `crates/ghostty-apprt-gtk/src/lib.rs:25`; root `Cargo.toml`, release profile | Partial | Installation hardcodes the ordinary application ID, while the default release recipe enables debug assertions and the runtime selects `com.mitchellh.ghostty-debug`. Desktop/D-Bus resources therefore identify a different application. Flatpak and system-package placement branches are now implemented. | Medium |
| `build/GhosttyI18n.zig:68,159,181,189` — extracted/merged translation catalogs | `justfile:80`; `po/` | Partial | Existing catalogs and `.mo` compilation/install are present. Source extraction, POT regeneration, and PO merging have no counterpart. | Low |
| `build/GhosttyDist.zig:20,83,92` — versioned source distribution | `justfile`; workspace packaging configuration | **Missing** | No producer for the equivalent versioned source-release tarball, including embedded version information and prepared distribution resources. | Low |
| `build/GhosttyExe.zig:50` — Windows executable resources | Root `build.rs`; `dist/` | **Missing** | No equivalent embedding of the upstream Windows icon, version-resource data, or manifest specifying DPI awareness and common-controls activation. | Low |

For completeness, the source inventory partitions as follows. The gap table above contains every file counted partial or missing.

| Zig area | Full | Partial | Missing | Rust counterpart area |
|---|---:|---:|---:|---|
| Root files | 21 | 5 | 1 | Root entrypoint; `core`, `termio`, `cli`, `lib`, `wasm`, `bench`, `build-data`, `helpgen`; standard-library replacements |
| `apprt/` | 51 | 3 | 0 | `apprt`, `apprt-gtk`, `lib/embedded`, WASM runtime |
| `benchmark/` | 11 | 1 | 0 | `bench`; `lib/benchmark` |
| `cli/` | 20 | 3 | 0 | `cli`; `config/cli_args`, diagnostics and comma splitting |
| `config/` | 15 | 3 | 0 | `config`; `lib/config_capi`; WASM config exports |
| `crash/` | 3 | 1 | 0 | `crash` |
| `datastruct/` | 11 | 0 | 0 | `datastruct`, safe index/collection replacements |
| `extra/` | 5 | 0 | 0 | `build-data` |
| `font/` | 47 | 0 | 0 | `font`, including CoreText and platform backends |
| `input/` | 16 | 0 | 0 | `input` |
| `inspector/` | 13 | 0 | 0 | `inspector`; embedded inspector bridge |
| `lib/` | 8 | 1 | 0 | `vt-capi`, ABI types and allocation helpers |
| `os/` | 25 | 8 | 0 | `os`, including Darwin, Windows and WASM modules |
| `renderer/` | 34 | 1 | 0 | `renderer`, including Metal and OpenGL |
| `simd/` | 6 | 0 | 0 | VT/Unicode routines and equivalent library/scalar implementations |
| `stb/` | 1 | 0 | 0 | Font rasterization counterpart |
| `synthetic/` | 9 | 0 | 0 | `bench/synthetic` and generator executable |
| `terminal/` | 83 | 2 | 0 | `terminal`, `vt`, `vt-capi` |
| `terminfo/` | 3 | 0 | 0 | `termio/terminfo` |
| `termio/` | 7 | 2 | 0 | `termio` |
| `unicode/` | 8 | 0 | 0 | `unicode` and table generation |
| **Total** | **397** | **30** | **1** | |

Of the previous audit’s **35 flagged files, 25 now rate full and 10 remain partial**. None remains wholly missing. Rechecked fixes include Flatpak signalling, Snap filtering, byte-preserving environment capture and URI decoding, full case-folding, whole-search regex limits, Wayland registry updates, OSC 9 capacity handling, theme-preview features, GTK/adwaita fallbacks and version detection, independent benchmark modes, help generation, crash envelopes/context hooks, and completion/syntax generators.

Other artifact counterparts are present: man-page Markdown/HTML/roff generation, compiled translations, completion and editor-syntax outputs, terminfo/termcap, GTK resources, Unicode/help tables, shell integration, themes, fonts, icons, shaders, compressed animation frames, benchmark/synthetic executables, and WASM outputs. Runtime Metal compilation and native GResource embedding are behaviorally equivalent substitutions, not missing artifacts.

The public headers have definitions for **all 88 libghostty functions and all 50 libghostty-vt functions**. Their presence does not close the behavioral and allocator gaps identified above.