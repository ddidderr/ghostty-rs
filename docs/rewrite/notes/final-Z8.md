# Z8: RV6 remaining gap groups

1. crash: `request_abort_dump` returns `exception::abort_reply_ok(mach_msg rcv result)`; timeout/error falls back to the signal-path report. `MachException::minidump_subcode` keeps the subcode for EXC_BAD_ACCESS and EXC_SOFTWARE/EXC_SOFT_SIGNAL (signal number). Tests in `exception.rs` (platform independent); mach glue checked by `just check-macos`.
2. Default `SHELL`: `getenv_os` + `shell_from_env` (EncodedOs encoding) in `config.rs`; test `shell_from_env_round_trips_bytes`.
3. macOS exec search: `exec_candidates` works on bytes with the raw `var_os("PATH")` bytes (default path only when PATH is unset); tests `exec_candidates_*`.
4. Windows env overrides: `apply_env_overrides` decodes keys/values (`config_str_to_os`); `env_overrides_are_decoded` (cfg(windows), compiled by check-windows).
5. `EnvMap` on Windows compares keys as WTF-16 units upcased per unit (Unicode simple mapping, single BMP result only, surrogates unchanged); `windows_key_tests` run everywhere.
6. `+show-config` writes `config_str_to_bytes(formatted)` (`format_config_bytes`) so raw bytes are emitted; UTF-8 output unchanged. Test `show_config_round_trips_non_utf8_and_fdd0_paths`.
7. Locale: full `ensureLocale` chain now in `ghostty-os/src/locale.rs` `posix` module (all unix; libc dep moved to cfg(unix); cocoa step stays Apple-only). GTK `init_locale` now delegates to it (unsafe removed there). Chain tested via a mock `Sys` trait. `core::global::init_lib` already calls `ensure_locale`, so the C API gets it.
