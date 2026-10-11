#!/usr/bin/env bash
# Compare number of Zig tests vs Rust tests per area.
set -euo pipefail
ZIG=$(cd "$(dirname "$0")/.." && pwd)/../ghostty/src
RS=$(cd "$(dirname "$0")/.." && pwd)/crates
printf "%-12s %6s\n" area zig_tests
for d in terminal font renderer input config termio cli os datastruct unicode apprt; do
  printf "%-12s %6s\n" "$d" "$(grep -rh '^test "' "$ZIG/$d" | wc -l)"
done
echo
printf "%-22s %6s\n" crate rust_tests
for c in "$RS"/*/; do
  printf "%-22s %6s\n" "$(basename "$c")" "$(grep -rhE '^\s*#\[(test|gtk::test)\]' "$c" 2>/dev/null | wc -l)"
done
