# Guide for porting agents

You are porting part of Ghostty 1.3.1 (Zig) to Rust. Read
`docs/rewrite/ARCHITECTURE.md` first.

- Zig source (read-only reference): `../../../ghostty/src`
- Rust workspace: `../..`

## Rules

1. **Stay in your lane.** Only edit the crate/modules assigned to you. If you
   need something from another crate that doesn't exist yet, write a minimal
   local version, mark it `// TODO(integration): ...`, and mention it in your
   report. Never rewrite other agents' modules. You MAY add dependencies to your
   own crate's `Cargo.toml` (prefer well-known crates; use `cargo add -p <crate>`).
2. **Faithful port — except genuine bugs.** If the Zig code has a real
   memory-safety bug or undefined behaviour (use-after-free, out-of-bounds,
   leaks, stale pointers, infinite loops, integer overflow UB), do NOT
   replicate it: implement the evidently intended, safe behaviour, add a test,
   and record it under "Upstream bugs fixed" in your notes (Zig file:line,
   what was wrong, what the Rust does). Everything else stays faithful.
   Original rule 2:
   **Faithful port.** Preserve behaviour, edge cases, defaults and naming
   (snake_case of the Zig names). Port doc comments (they are user-visible for
   config/actions). Don't "improve" semantics. Idiomatic Rust structure is fine
   (enums for tagged unions, traits for comptime interfaces, `Result` for error
   unions) as long as behaviour matches.
3. **Port the tests.** Every Zig `test "name"` in files you port becomes a
   `#[test] fn name_snake_case()` in a `#[cfg(test)] mod tests` of the
   corresponding Rust module. Tests that are about Zig-only concerns
   (allocator failure injection, comptime, C ABI, Metal/macOS) may be skipped —
   list them in your notes file. Add extra tests when behaviour is subtle.
4. **No `unsafe`** unless your crate is explicitly allowed (renderer, termio,
   apprt-gtk), and then only minimal + `// SAFETY:` comments.
5. **Quality gate before you finish** (must all pass for your crate):
   - `cargo build -p <crate>`
   - `cargo test -p <crate>`
   - `cargo clippy -p <crate> --all-targets -- -D warnings`
   - `cargo fmt -p <crate>` (rustfmt.toml in repo root; if nightly-only options
     warn, ignore the warnings)
   No `todo!()`/`unimplemented!()` left in code paths that are reachable,
   unless documented in your notes as deferred.
6. **Do NOT run `git commit`** or any git command that changes state. The
   orchestrator commits.
7. Other agents work concurrently in other crates of the same workspace. If the
   *workspace* build breaks because of another crate, use `-p <crate>` and
   ignore it. Cargo may block briefly on the build-directory lock; that's normal.
8. **Write notes** to `docs/rewrite/notes/<crate>-<topic>.md`: what was ported
   (file → module), skipped tests with reasons, deviations, TODO(integration)
   items, known gaps.
9. **Final report** (your last message): ≤ 25 lines — modules done, test counts
   (ported / skipped), quality-gate status, open issues. The orchestrator reads
   only this, so be precise and honest about anything incomplete.

## Conventions

- `pub mod` per Zig file; file names snake_case (`PageList.zig` → `page_list.rs`).
- Error types: `thiserror` is fine; `anyhow` only in binaries/CLI.
- Logging: the `log` crate (`log::warn!` etc.) where Zig uses `std.log`.
- Integer types: keep the Zig widths (u16 for cell counts `CellCountInt`, etc.).
- Zig `packed struct` bitfields → plain structs unless memory layout matters
  (documented in ARCHITECTURE for `Cell`/`Row`).
- Prefer `std` collections; `hashbrown`/`smallvec`/`bitflags` OK.
