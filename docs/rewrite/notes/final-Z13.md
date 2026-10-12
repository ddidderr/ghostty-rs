# Z13: RV10 codex gaps

1. action_c.rs: key-table name, mouse-over-link and open-url use `Keep::counted_bytes` (full bytes + NUL, len always equals the allocation). Audited config_capi/embedded: other pointer+len pairs already match. Test `counted_buffers_hold_exactly_len_bytes`.
2. bench/args.rs keeps every occurrence in order (empty resets default, first error wins, parseBool spellings); lib/benchmark.rs uses `ghostty_os::arg_iterator::ArgIteratorGeneral`.
3. termio `window_title` takes bytes; OSC 7 forwards the raw path.
4. apprt `DesktopNotification`/`MouseOverLink` carry `EncodedOs`; core passes raw bytes; C actions decode them. GTK uses lossy text at the UI boundary.
5. Diagnostic getter decodes the internal encoding (`cstring_value`).
6. `StringError::Utf8CannotEncodeSurrogateHalf`.
7. NaN: both formatters use `format_float_d`.
8. `AppRuntime` + `Config::finalize_for`; process-wide `app_runtime()` (default Gtk on Linux/BSD, None elsewhere); embedded lib / wasm set None. termio installs the cgroup pre-exec/post-fork hooks only for Gtk (Zig `@hasDecl(apprt.runtime, "pre_exec")`).

Gate: workspace tests pass; clippy clean except ghostty-terminal/difftest (DF in-progress edits), which also stops `just check-macos`/`check-windows` at ghostty-terminal; check-wasm passes.
