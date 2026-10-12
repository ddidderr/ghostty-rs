# Codex (gpt-6-astra) completeness audit (2026-10-12)

The rewrite is **not yet complete**. The static audit accounts for **354 in-scope Zig files: 319 full, 28 partial, 7 missing**. Another 107 files fall under the requested exclusions.

“Full” means the in-scope functionality has a corresponding implementation; it does not certify identical behavior under every input. I checked implementation code against the architecture and port notes. No files were modified, and no builds or tests were run.

References below use **Z** = [ghostty/src](../../../../ghostty/src), and **R/crate** = `ghostty-rs/crates/ghostty-<crate>/src`. Grouped rows count each named Zig file separately.

**Partial and missing source files**

| Zig file:line | Corresponding Rust module:line | Status | Verified gap | Importance |
|---|---|---|---|---|
| `termio/Exec.zig:1226`; `os/flatpak.zig:154,225` | `R/termio/exec.rs:211,1088`; `R/os/flatpak.rs:206` | Partial ×2 | Starting the Flatpak exit watcher consumes the command object. Closing the surface subsequently cannot send the upstream SIGHUP to the host process group; Rust explicitly logs that the watcher took the handle. | **High** |
| `apprt/gtk/class/surface.zig:1563,1592,1615` | `R/apprt-gtk/class/surface.rs:966` | Partial | `filterSnapPaths` is absent: Snap variables and Snap-prefixed path components are not removed before launching children. Additionally, `std::env::vars()` cannot preserve arbitrary Linux environment bytes and panics on non-UTF-8 entries; Zig uses a byte-preserving environment map. | Medium |
| `main_ghostty.zig:26` → `global.zig:99` | Root `src/main.rs:17` | Partial | Startup collects `std::env::args()` into strings, panicking on non-UTF-8 arguments before dispatch. Upstream argument processing accepts byte strings. The Rust OS argument helper correctly uses `args_os()`, but this entrypoint bypasses it. | Medium |
| `os/uri.zig:21` | `R/os/uri.rs:71,90` | Partial | Percent-decoding uses `String::from_utf8_lossy`, changing non-UTF-8 filename bytes into replacement characters. This reaches OSC 7 working-directory handling through `R/termio/stream_handler.rs:988`. | Medium |
| `input/Binding.zig:1921` | `R/input/binding/mod.rs:500` | Partial | Full Unicode case-folding is replaced by single-character lowercase conversion plus three special cases. Other case-fold equivalences and multi-codepoint folds are missing from keybinding comparison/hashing. | Medium |
| `terminal/StringMap.zig:55` | `R/terminal/string_map.rs:54` | Partial | Upstream sets a **whole-search** regex retry limit. Rust sets only a **per-match** retry limit, losing the aggregate search-work bound. | Medium |
| `renderer/shadertoy.zig:161` | `R/renderer/shadertoy.rs:201,252` | Partial | glslang compilation is replaced by Naga’s narrower GLSL frontend. The adapter rewrites the supplied `iChannel0` declaration, but not arbitrary combined-sampler types—for example, a user helper taking a `sampler2D` parameter. Those shaders lack the upstream compilation path. | Medium |
| `apprt/gtk/winproto/wayland.zig:150,200` | `R/apprt-gtk/winproto/wayland.rs:531` | Partial | Registry-global additions/removals after initialization are ignored. Upstream binds newly advertised supported globals and clears removed ones. Rust’s registry event handler is empty. | Low |
| `terminal/osc/parsers/osc9.zig:16,205` | `R/vt/osc/parsers/osc9.rs:13`; `R/vt/osc/mod.rs:432` | Partial | Rust applies the NUL-reservation capacity check before identifying the command. It therefore rejects full-buffer ConEmu control commands that upstream accepts without appending a NUL. | Low |
| `cli/list_themes.zig:306,522,683,1011` | `R/cli/list_themes/preview.rs:228,824,1183` | Partial | Interactive preview omits OSC 8 hyperlinks, terminal color-scheme query/update handling, and distinct double/dotted/dashed/curly underline previews. | Low |
| `build_config.zig:17,21,24` | `R/cli/build_info.rs:50`; root `src/main.rs:86` | Partial | Build metadata is not derived from the actual build: version defaults to `1.3.1`, commit is absent, channel is always stable, and build-mode/backend descriptions are fixed approximations. | Low |
| `apprt/gtk/gtk_version.zig:14`; `apprt/gtk/adw_version.zig:15` | `R/apprt-gtk/gtk_version.rs:12`; `adw_version.rs:6` | Partial ×2 | Compile-time library versions are hardcoded to GTK 4.14.0/libadwaita 1.5.0 instead of being obtained from the build headers. Runtime version queries are implemented. | Low |
| `apprt/gtk/class/application.zig:1247` | `R/apprt-gtk/css.rs:40` | Partial | CSS error logging omits the offending source bytes available through the upstream GTK ≥4.16 diagnostic branch; location and error text remain. | Low |
| `apprt/gtk/class/dialog.zig:21,39` | `R/apprt-gtk/class/dialog.rs:22` | Partial | Older-libadwaita `MessageDialog` presentation/closing fallback is absent; only `AlertDialog` remains. | Low |
| `apprt/gtk/class/clipboard_confirmation_dialog.zig:17,301` | `R/apprt-gtk/class/clipboard_confirmation_dialog.rs:35` | Partial | Older template/capability branch without `SwitchRow` is absent; the 1.4 template is selected unconditionally. | Low |
| `apprt/gtk/class/debug_warning.zig:14,42` | `R/apprt-gtk/class/debug_warning.rs:11` | Partial | Pre-Banner `gtk.Box`/1.2-template fallback is absent. | Low |
| `apprt/gtk/class/surface_child_exited.zig:15,211` | `R/apprt-gtk/class/surface_child_exited.rs` | Partial | Pre-Banner compatibility implementation is absent. | Low |
| `apprt/gtk/class/window.zig:1762,1780` | `R/apprt-gtk/class/window.rs:1630` | Partial | Older-library GTK about-dialog fallback is absent. These five compatibility omissions accompany Rust’s explicit GTK 4.14/libadwaita 1.5 minimum. | Low |
| `benchmark/CodepointWidth.zig:107,133` | `R/bench/benches.rs:297` | Partial | `wcwidth` mode logs a warning and exits without benchmarking libc. SIMD/table modes also share the same lookup implementation rather than measuring separate implementations. | Low |
| `benchmark/IsSymbol.zig:93,128` | `R/bench/benches.rs:383` | Partial | `uucode` and `table` modes use the same table backend; the independent uucode comparison is missing. | Low |
| `helpgen.zig:75,121,140` | `R/config/macros.rs`; `R/cli/ghostty.rs:122`; `R/input/binding/action.rs:1229` | Partial | Configuration help is derived from Rust field documentation. CLI help and keybinding-action help instead use checked-in text/manual tables; the corresponding source-documentation generators are absent. | Low |
| `crash/main.zig:8,16` | `R/cli/crash_report.rs` | Partial | Report-directory/listing functionality exists; crash initialization, teardown, and envelope integration do not. | Low |
| `crash/sentry.zig:50,81,172,259` | — | Missing | Crash-handler initialization, thread/surface metadata enrichment, crash filtering, and writing local `.ghosttycrash` reports are absent. | Low¹ |
| `crash/sentry_envelope.zig:38,221,331,377` | — | Missing | Envelope parsing/serialization, item decoding, and attachment encoding/decoding are absent. | Low¹ |
| `Surface.zig:6355`; `global.zig:146,189`; `renderer/Thread.zig:218`; `termio/Thread.zig:247` | `R/core/surface/*`; `R/core/global.rs`; `R/renderer/thread.rs`; `R/termio/thread.rs` | Partial ×4 | Their normal functionality is implemented, but crash-context construction, initialization/teardown, and thread-context hooks are omitted. `termio/Exec.zig:1260` also loses its read-thread crash context, in addition to its Flatpak gap above. | Low¹ |
| `extra/bash.zig:19,33`; `extra/zsh.zig:12,26`; `extra/fish.zig:12,26` | — | Missing ×3 | Shell-completion generators and their generated/installable completion files are absent. | Medium |
| `extra/vim.zig:5,6,16,42,60` | — | Missing | Vim/Neovim syntax, file detection, filetype plugin, and compiler integration are absent. | Low |
| `extra/sublime.zig:35,50` | — | Missing | Generated Sublime syntax and the same syntax installed for bat are absent. | Low |

¹ Sentry is **optional and disabled by default on Linux upstream**, explicitly documented in `Z/build/Config.zig:158`. These omissions do not remove crash collection from the default Linux build.

**Assets, documentation, and installation**

These findings are separate from the source-file counts; requested generation/install behavior is included even where its Zig implementation lives under excluded `src/build`.

| Item | Status | Verified result / missing functionality | Importance |
|---|---|---|---|
| Man pages and generated documentation | **Missing** | **The Rust build/install path does not produce or install `ghostty.1` or `ghostty.5`.** There is no counterpart to `Z/build/mdgen/main_ghostty_1.zig:4`, `main_ghostty_5.zig:4`, or `mdgen.zig:21,51,86`. Upstream `Z/build/GhosttyDocs.zig:18` also generates Markdown/HTML; those generation/install steps are absent. Rust’s [install recipe](../../../justfile:41) contains none of these outputs. | Medium |
| Translations | **Missing** | Upstream’s 29 `.po` catalogs and translation template are absent. No equivalent of `Z/build/GhosttyI18n.zig:24` compiles and installs `.mo` files. Rust gettext lookup exists, but its standalone installation supplies no translated catalogs. | Medium |
| Shell completions | **Missing** | No bash/zsh/fish completion artifacts or install rules corresponding to `Z/build/GhosttyResources.zig:141,156,171`. Shell-integration scripts do not replace completions. | Medium |
| Editor syntax | **Missing** | No Vim/Neovim or bat installation corresponding to `Z/build/GhosttyResources.zig:186,226`; no Sublime syntax artifact. | Low |
| Terminfo distribution | **Partial** | Both compiled entries, `ghostty` and `xterm-ghostty`, exist and decode successfully with `infocmp`. Source generation and `tic` regeneration exist. Installation omits the textual `ghostty.terminfo` and `ghostty.termcap` outputs provided by `Z/build/GhosttyResources.zig:45,69`. | Low |
| Linux distribution/install modes | **Partial** | All seven `dist/linux` files are byte-identical. However, installation always selects ordinary D-Bus activation and `share/systemd/user`; it does not select the Flatpak service template, omit systemd for Flatpak, or select `lib/systemd/user` for system-package mode as `Z/build/GhosttyResources.zig:305,333` does. | Medium |

The following asset checks passed:

- **Shell integration:** all **8/8 files byte-identical**, including hidden files; installation is wired.
- **Themes:** all **463/463 upstream themes byte-identical** against the pinned Zig dependency.
- **GTK:** all **18/18 in-scope UI blueprints** and **4/4 CSS files byte-identical**; Blueprint/GResource compilation is wired. Inspector assets were excluded.
- **Rendering/fonts/icons:** all **10/10 GLSL files**, all production embedded fonts, and all **7/7 installed icon source images** match their upstream sources.
- **Configuration documentation:** all **202 public configuration keys** are present; their documentation matches after whitespace normalization. The configuration-file template is byte-identical. Runtime configuration help exists despite the missing man-page pipeline.

**CLI action coverage**

All **15 runtime `ghostty +` actions** have Rust dispatch and implementations matching the list in `Z/cli/ghostty.zig:26`:

`version`, `help`, `list-fonts`, `list-keybinds`, `list-themes`, `list-colors`, `list-actions`, `ssh-cache`, `edit-config`, `show-config`, `validate-config`, `show-face`, `crash-report`, `boo`, `new-window`.

The qualifications are the theme-preview and version-metadata gaps above. Upstream `+crash-report` itself only lists reports, so its absent view/send functionality is not a Rust regression. Completion/syntax generation actions belong to upstream’s **build-data helper**, not this runtime action list.