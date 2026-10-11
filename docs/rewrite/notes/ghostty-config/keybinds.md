# Keybinds (fork C)

- `Config.Keybinds` -> `crates/ghostty-config/src/keybinds.rs`.
- Struct: `set: Set`, `tables: Vec<(String, Set)>` (insertion ordered), `chain_target: ChainTarget`.
- `init()` = Linux/GTK default table only (darwin branches dropped), same insertion order as Zig.
- `ConfigField`: `parse_cli` (clear, empty -> init, `name/` tables, `chain=` target tracking), `reset` -> `init()`, `format_entry` / `format_entry_docs`.
- `PartialEq` follows Zig `equal`/`equalSet` (ignores reverse map and chain state).
- Formatting uses `ghostty_input::binding::set::Value::format_entries` through an adapter to `EntryFormatter`.
- Errors from input's `binding::Error` map to `ArgError::Other("InvalidFormat"|"InvalidAction")`.
- Tests: 31 ported/added (all Zig tests in range ported, 0 skipped; plus `reset_calls_init`, `default_keybinds_format_roundtrip`).

Quirks
- `format_entry_docs(docs=true)`: Zig looks up help via `@tagName(v)` of the set Value (leader/leaf/leaf_chained), which never matches a keybind action, so only a blank line is emitted. Preserved.
- `Keybinds::default()` is EMPTY (Zig `.{}`); Config::default must call `init()`.
- Input's `Set` has no PartialEq; equality is implemented locally.
