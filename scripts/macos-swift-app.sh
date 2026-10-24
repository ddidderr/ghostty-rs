#!/usr/bin/env bash
# Build Ghostty's ORIGINAL Swift macOS app (unchanged) against the Rust core.
#
# Ghostty's Xcode project (macos/Ghostty.xcodeproj, vendored unchanged from
# Ghostty 1.3.1 together with images/Ghostty.icon) links
# `macos/GhosttyKit.xcframework` and bundles resources from `../zig-out/share`
# (see the Zig build's src/build/GhosttyXcodebuild.zig). The Rust libghostty
# exports the identical include/ghostty.h ABI, so this script produces both
# from the Rust workspace and then runs xcodebuild, exactly like
# `zig build -Demit-macos-app`:
#   1. scripts/xcframework.sh  -> macos/GhosttyKit.xcframework
#   2. `just install`          -> zig-out/share/{ghostty,terminfo,...}
#   3. xcodebuild -target Ghostty -configuration <CONFIG> (clean env, like Zig)
#
# Usage: scripts/macos-swift-app.sh [Release|Debug|ReleaseLocal]
# Runs only on macOS (Xcode required). DRY_RUN=1 prints the commands instead.
set -euo pipefail

cd "$(dirname "$0")/.."
src=$(pwd)
config=${1:-Release}
[ -d "$src/macos/Ghostty.xcodeproj" ] || {
    echo "error: $src/macos/Ghostty.xcodeproj not found" >&2
    exit 1
}

run() {
    if [ "${DRY_RUN:-}" = 1 ]; then
        printf '+'
        printf ' %q' "$@"
        printf '\n'
    else
        "$@"
    fi
}

if [ "${DRY_RUN:-}" != 1 ] && [ "$(uname -s)" != Darwin ]; then
    echo "error: building the macOS app requires macOS with Xcode (use DRY_RUN=1 to preview)" >&2
    exit 1
fi

# Debug configuration pairs with the dev profile (debug app id), like Zig.
profile=production
[ "$config" = Debug ] && profile=dev

# 1. The Rust core as GhosttyKit.xcframework where the Xcode project expects it.
run scripts/xcframework.sh universal "$profile" "$src/macos/GhosttyKit.xcframework"

# 2. Resources the Xcode project copies from ../zig-out/share.
run just install "$src/zig-out"

# 3. xcodebuild with a clean environment (only PATH), as GhosttyXcodebuild.zig.
run env -i PATH="$PATH" sh -c "cd '$src/macos' && xcodebuild -target Ghostty -configuration '$config'"

echo "Ghostty.app: $src/macos/build/$config/Ghostty.app"
