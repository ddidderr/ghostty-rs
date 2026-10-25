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
#   2. `just install-resources` -> zig-out/share/{ghostty,terminfo,...}
#   3. xcodebuild -target Ghostty -configuration <CONFIG> (clean env, like Zig)
#
# Usage: scripts/macos-swift-app.sh [ReleaseLocal|Debug|Release] [universal|native]
# ReleaseLocal (default) is what `zig build` uses for release builds; Release
# is the CI configuration and needs Ghostty's signing identity.
# The second argument is the xcframework target (Zig: -Dxcframework-target,
# default universal); native builds only the host architecture.
# Runs only on macOS (Xcode required). DRY_RUN=1 prints the commands instead.
set -euo pipefail

cd "$(dirname "$0")/.."
src=$(pwd)
config=${1:-ReleaseLocal}
target=${2:-universal}
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
run scripts/xcframework.sh "$target" "$profile" "$src/macos/GhosttyKit.xcframework"

# 2. Resources the Xcode project copies from ../zig-out/share.
# Start clean so no stale files (e.g. a Linux-layout terminfo) get bundled.
run rm -rf "$src/zig-out/share"
run just install-resources "$src/zig-out"

# The app version is exactly upstream's: CFBundleShortVersionString is the
# release version and CFBundleVersion its build number (upstream CI sets both
# with PlistBuddy after building; build settings keep the signature valid).
meta() { sed -n "/^\[workspace.metadata.ghostty\]/,/^\[/s/^$1 = \"*\([^\"]*\)\"*$/\1/p" Cargo.toml; }
version=$(meta app-version)
build=$(meta app-build)

# 3. xcodebuild with a clean environment (only PATH), as GhosttyXcodebuild.zig.
# A native xcframework has only the host slice, so override the project's
# architectures with -arch (GhosttyXcodebuild.zig does the same).
arch=
if [ "$target" = native ]; then
    case $(uname -m) in
    arm64 | aarch64) arch="-arch arm64" ;;
    *) arch="-arch x86_64" ;;
    esac
fi
run env -i PATH="$PATH" sh -c "cd '$src/macos' && xcodebuild -target Ghostty -configuration '$config' $arch MARKETING_VERSION='$version' CURRENT_PROJECT_VERSION='$build'"

echo "Ghostty.app: $src/macos/build/$config/Ghostty.app"
