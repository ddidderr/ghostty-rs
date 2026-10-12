# Z18 notes

## A. Deviant fix (separable)
- `crates/ghostty-apprt-gtk/src/ipc/new_window.rs`: `arguments_variant` now validates (UTF-8, no NUL) and returns `Error::IpcFailed` with a stderr message instead of `process::abort()` (Zig `new_window.zig:49-53`). Validation runs before connecting to the bus. Test `invalid_dbus_strings_are_rejected_without_abort` replaces the SIGABRT subprocess test.
- `docs/rewrite/DEVIANT_FIXES.md`: row 31.

## B. Parity
1. `crates/ghostty-cli/src/show_face.rs`: `write_diagnostic` / `write_lookup` write Zig's exact fragments (literal, `cli:`, digits, `:`, key, `: `, message, `\n`; hex padding separate from digits). Oracle (pipe) check: n = 0..44 copies of `--font-size=bad` x cp {65, 7, 0x1F600}: 0 diffs (includes the 23x case). Tests: `diagnostics_drain_at_zig_fragment_boundaries`, `lookup_line_pads_hex_and_drains_per_fragment`.
2. `crates/ghostty-os/src/open.rs`: stdin inherited; `LD_LIBRARY_PATH` removed only with the new `snap` cargo feature (default off; forwarded by root `Cargo.toml` `snap`); stdout/stderr bounded to 50 KiB each (overflow = error, like `collectOutput`; child is still reaped, unlike Zig). Tests: `opener_inherits_stdin`, `snap_env_is_stripped_only_for_snap_builds`, `output_collection_is_bounded_per_stream`.
3. `show_face.rs`: `run_args` takes a config loader; tests use explicit args (`test_config`), `run` uses `Config::load`.

## C. DEVIANT_FIXES.md corrections
Row 30 (offset wording) and the wasm, VT parser buffer, config strings, `+version` rows reworded; removed: invalid preedit, `initial_input` escaping, `wcwidth` bench, D-Bus escape encoding (no longer deviations).
