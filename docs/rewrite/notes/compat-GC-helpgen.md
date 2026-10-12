# compat-GC: helpgen (ZIG/src/helpgen.zig)

Zig generates help strings at build time from doc comments (`helpgen.zig`,
exposed as `help_strings`). The Rust port does the same instead of keeping
checked-in tables.

## Design

* New crate `crates/ghostty-helpgen` (no dependencies), used as a
  `[build-dependencies]` of `ghostty-cli` and `ghostty-input`.
  * `doc_lines`: port of `extractDocComments`/`findCommonPrefix` (strip
    `///`, remove the common leading-space prefix of non-blank lines, keep
    blank lines and code fences verbatim).
  * `run_fn_doc`: doc comment of a module's `run` function (Zig `genActions`).
  * `enum_variant_docs`: `///` docs per variant of an enum (Zig
    `genKeybindActions`).
  * `*_match`: emit Rust `match self { ... }` bodies with escaped literals.
* `ghostty-cli/build.rs` reads the variants of `enum Action` in
  `src/ghostty.rs`, maps `ListFonts` -> `src/list_fonts.rs`, extracts the doc
  of `pub fn run`, writes `$OUT_DIR/action_help.rs`; `Action::help_text`
  `include!`s it. A missing doc comment fails the build (as in Zig).
  The former `src/help/*.txt` files are deleted.
* `ghostty-input/build.rs` extracts the docs of every variant of
  `enum Action` in `src/binding/action.rs` (now `///` instead of `//`),
  writes `$OUT_DIR/keybind_action_docs.rs`, included by `ActionKey::docs`
  (docs = lines joined by `\n` plus one trailing `\n`, as before;
  `help_string` strips one).
* Config help already derives from field docs through `config_struct!`
  (`ghostty-config/src/macros.rs`), so no generation was needed there.
* A line-based scanner is used rather than `syn`: it mirrors Zig's token
  based approach (doc run + following item) and avoids a heavy build dep.

## Verification

Outputs of `+help`, `+list-actions [--docs]` and every `+<action> --help`
were captured before and after: byte identical. Differences vs
`/usr/bin/ghostty` exist only for `+version --help` and `+new-window --help`
(unchanged from the baseline).

## Tests

`ghostty-helpgen` unit tests (marker stripping, common prefix, blank lines,
code fences, `////`, run-doc and enum-variant extraction, escaping).
