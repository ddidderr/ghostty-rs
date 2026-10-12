#!/usr/bin/env bash
# Prepare the Zig side of the differential fuzzer (idempotent).
#
#  1. Zig 0.15.2 (Ghostty 1.3.1's required version) -> target/zig-0.15.2/
#  2. A COPY of the reference Ghostty source        -> target/zig-src/
#     (the reference tree itself is never modified)
#  3. The Zig driver, built twice:
#       target/difftest-zig/bin/difftest_zig       ReleaseSafe (assertions on)
#       target/difftest-zig-fast/bin/difftest_zig  ReleaseFast (assertions off)
#
# Everything lives under target/ (never /tmp, which is RAM-backed).
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
REF="${GHOSTTY_ZIG_REF:-$ROOT/../ghostty}"
ZIGVER=0.15.2
ZIGDIR="$ROOT/target/zig-$ZIGVER"
ZIG="$ZIGDIR/zig-x86_64-linux-$ZIGVER/zig"

if [ ! -x "$ZIG" ]; then
  mkdir -p "$ZIGDIR"
  timeout 900 curl -sSfL -o "$ZIGDIR/zig.tar.xz" \
    "https://ziglang.org/download/$ZIGVER/zig-x86_64-linux-$ZIGVER.tar.xz"
  timeout 300 tar -C "$ZIGDIR" -xf "$ZIGDIR/zig.tar.xz"
  rm -f "$ZIGDIR/zig.tar.xz"
fi

if [ ! -f "$ROOT/target/zig-src/build.zig" ]; then
  mkdir -p "$ROOT/target/zig-src"
  timeout 600 rsync -a --exclude .git --exclude .zig-cache --exclude zig-cache \
    "$REF/" "$ROOT/target/zig-src/"
fi

cd "$ROOT/tools/difftest/zig"
for m in ReleaseSafe:difftest-zig ReleaseFast:difftest-zig-fast; do
  timeout 1800 "$ZIG" build -Doptimize="${m%%:*}" \
    --cache-dir "$ROOT/target/zig-cache/local" \
    --global-cache-dir "$ROOT/target/zig-cache/global" \
    --prefix "$ROOT/target/${m##*:}"
done
