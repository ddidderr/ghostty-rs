#!/bin/bash
# Regenerates resources/terminfo (compiled xterm-ghostty) from the terminfo
# source embedded in ghostty-termio, like Ghostty's GhosttyResources.zig
# (`tic -x -o <dir> ghostty.terminfo`), then compares against the installed
# entry (if any) with `infocmp -x`.
set -eu
root=$(cd "$(dirname "$0")/.." && pwd)
src=$(mktemp)
trap 'rm -f "$src"' EXIT
cargo run -q -p ghostty-termio --example terminfo > "$src"
rm -rf "$root/resources/terminfo"
mkdir -p "$root/resources/terminfo"
tic -x -o "$root/resources/terminfo" "$src"
echo "wrote $root/resources/terminfo"
if infocmp -x xterm-ghostty >/dev/null 2>&1; then
  if diff <(infocmp -x xterm-ghostty | grep -v '^#') \
          <(TERMINFO="$root/resources/terminfo" infocmp -x xterm-ghostty | grep -v '^#') >/dev/null 2>&1; then
    echo "infocmp: identical to installed xterm-ghostty"
  else
    echo "infocmp: DIFFERS from installed xterm-ghostty" >&2
  fi
fi
