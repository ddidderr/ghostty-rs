# Z10: byte-typed key text and preedit

`KeyEvent.utf8` is now `&[u8]` (Zig `[]const u8`) end to end; the Z9 "documented limit" for C key `text` / preedit is gone.

- `ghostty-input`: `KeyEvent.utf8: &[u8]`; `key_encode` writes the bytes unchanged. Where Zig validates with `Utf8View.init ... catch` (plain-text control scan, kitty alternates, modify-other-keys, ctrl CSI-u, kitty associated text, binding unicode lookup) invalid UTF-8 takes the same "no UTF-8" fallback (`utf8_chars`). Legacy/kitty "send text directly" paths write the raw bytes.
- `ghostty-core`: `OwnedKeyEvent.utf8: Vec<u8>`; `Surface::preedit_callback(Option<&[u8]>)` (and the `RuntimeSurface` trait in `ghostty-apprt`). Preedit bytes are decoded lossily (U+FFFD) for the renderer codepoints only. Deviation: Zig returns an error on invalid preedit; we render replacement chars instead (no panic).
- `ghostty-lib`: `ghostty_surface_key` copies `text` as raw bytes; `ghostty_surface_preedit` passes bytes. `ghostty_surface_text` was already bytes.
- `ghostty-vt-capi`: `ghostty_key_event_*` no longer truncates invalid UTF-8 to its valid prefix; passes bytes through.
- Inspector key widget shows invalid text lossily (display only). GTK callers pass `as_bytes()` of valid UTF-8.
- Tests: `ghostty-lib/tests/surface.rs::key_text_and_preedit_are_bytes_not_lossy_utf8` (key text `ff fe` reaches the pty), `ghostty-core/tests/e2e_ime.rs::preedit_with_invalid_utf8_is_replaced_for_display_only`, `ghostty-input` `invalid_utf8_*` encoder tests. Existing tests only changed mechanically (`"x"` -> `"x".as_bytes()`).

## Orchestrator follow-up
Preedit with invalid UTF-8 now matches Zig exactly instead of rendering
U+FFFD: `preedit_callback` clears the prior preedit, marks it dirty and
returns an error (Zig: `try std.unicode.Utf8View.init(text)`); the C API
logs it like `embedded.zig`. Test: `preedit_with_invalid_utf8_is_an_error_and_clears_prior`.
