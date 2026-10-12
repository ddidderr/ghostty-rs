#!/usr/bin/env bash
# Assemble Ghostty.app from the Rust macOS app (crate ghostty-macos).
#
# Mirrors what Xcode does for the Swift app's `Ghostty` target
# (macos/Ghostty.xcodeproj): compile the executable, expand Ghostty-Info.plist
# (+ the INFOPLIST_KEY_* build settings), compile Assets.xcassets, copy
# Ghostty.sdef and the resources, sign with the entitlements. libghostty is NOT
# linked or embedded: the Rust app drives ghostty-lib in-process.
#
# Usage: scripts/macos-bundle.sh [--profile PROFILE] [--target TRIPLE]
#                                [--out DIR] [--debug] [--dock-tile-plugin PATH]
#                                [--sign IDENTITY] [--dry-run]
#   PROFILE  cargo profile (default: release; "dev" is the debug build)
#   TRIPLE   aarch64-apple-darwin or x86_64-apple-darwin (default: host)
#   --out    output directory (default: target/macos-app/<profile>)
#   --debug  use the debug identity: bundle id com.mitchellh.ghostty.debug,
#            display name "Ghostty[DEBUG]", GhosttyDebug.entitlements
#   --sign   codesign identity (default "-", ad hoc)
#
# It only runs for real on macOS (PlistBuddy, iconutil, sips, codesign, cargo for
# an Apple target). On any other host it is a dry run: every step is printed
# and the Swift-side inputs are checked to exist. DRY_RUN=1 forces that on macOS.
set -euo pipefail

root=$(cd "$(dirname "$0")/.." && pwd)
mac="$root/../ghostty/macos"
profile=release
target=
out=
debug=0
sign=-
plugin=
dry=${DRY_RUN:-0}

while [ $# -gt 0 ]; do
    case $1 in
    --profile) profile=$2; shift 2 ;;
    --target) target=$2; shift 2 ;;
    --out) out=$2; shift 2 ;;
    --debug) debug=1; shift ;;
    --sign) sign=$2; shift 2 ;;
    --dock-tile-plugin) plugin=$2; shift 2 ;;
    --dry-run) dry=1; shift ;;
    -h | --help) sed -n '2,/^set -e/p' "$0" | sed '$d'; exit 0 ;;
    *) echo "unknown argument: $1" >&2; exit 2 ;;
    esac
done

if [ "$(uname -s)" != Darwin ]; then
    dry=1
fi

run() {
    if [ "$dry" = 1 ]; then
        printf '+'
        printf ' %q' "$@"
        printf '\n'
    else
        "$@"
    fi
}

# Inputs from the Swift app tree (Xcode project settings in comments).
version=$(sed -n 's/^app-version = "\(.*\)"/\1/p' "$root/Cargo.toml")
[ -n "$version" ] || version=1.3.1
if [ "$debug" = 1 ]; then
    bundle_id=com.mitchellh.ghostty.debug
    display_name='Ghostty[DEBUG]'
    entitlements="$mac/GhosttyDebug.entitlements"
else
    bundle_id=com.mitchellh.ghostty
    display_name=Ghostty
    entitlements="$mac/Ghostty.entitlements"
fi
min_os=13.0 # MACOSX_DEPLOYMENT_TARGET of the Ghostty target
case $(uname -m) in arm64 | aarch64) host=aarch64-apple-darwin ;; *) host=x86_64-apple-darwin ;; esac
target=${target:-$host}
pdir=$profile
[ "$profile" = dev ] && pdir=debug
tdir=${CARGO_TARGET_DIR:-$root/target}
exe="$tdir/$target/$pdir/ghostty-macos-app"
out=${out:-$root/target/macos-app/$pdir}
app="$out/Ghostty.app"
contents="$app/Contents"
res="$contents/Resources"
plist="$contents/Info.plist"
pb=/usr/libexec/PlistBuddy

for f in "$mac/Ghostty-Info.plist" "$mac/Ghostty.sdef" "$mac/Assets.xcassets" "$entitlements" \
    "$root/resources/terminfo" "$root/resources/themes" "$root/resources/shell-integration"; do
    [ -e "$f" ] || { echo "missing input: $f" >&2; exit 1; }
done

echo "== Ghostty.app ($version, $bundle_id, $target, profile $profile) -> $app"
[ "$dry" = 1 ] && echo "== dry run: nothing is built or written"

# 1. The executable (CFBundleExecutable = ghostty, like the Swift product name).
run cargo build --profile "$profile" -p ghostty-macos --bin ghostty-macos-app --target "$target"

run rm -rf "$app"
run mkdir -p "$contents/MacOS" "$res"
run cp "$exe" "$contents/MacOS/ghostty"

# 2. Info.plist: Ghostty-Info.plist plus the build settings Xcode folds in
#    (GENERATE_INFOPLIST_FILE = YES, INFOPLIST_KEY_*). NSMainNibFile is not set:
#    the Rust app builds MainMenu.xib in code (app/main_menu.rs).
run cp "$mac/Ghostty-Info.plist" "$plist"
plist_set() { # key type value
    if [ "$dry" = 1 ]; then
        printf '+ PlistBuddy -c %q %q\n' "Add :$1 $2 $3" "$plist"
    else
        "$pb" -c "Delete :$1" "$plist" 2>/dev/null || true
        "$pb" -c "Add :$1 $2 $3" "$plist"
    fi
}
plist_set CFBundleDevelopmentRegion string en
plist_set CFBundleExecutable string ghostty
plist_set CFBundleIdentifier string "$bundle_id"
plist_set CFBundleInfoDictionaryVersion string 6.0
plist_set CFBundleName string Ghostty
plist_set CFBundleDisplayName string "$display_name"
plist_set CFBundlePackageType string APPL
plist_set CFBundleShortVersionString string "$version"
plist_set CFBundleVersion string 1
plist_set CFBundleIconFile string AppIcon
plist_set CFBundleIconName string Ghostty
plist_set LSMinimumSystemVersion string "$min_os"
plist_set LSApplicationCategoryType string public.app-category.developer-tools
plist_set NSPrincipalClass string NSApplication
plist_set NSHumanReadableCopyright string ""
plist_set GhosttyBuild string "$version"
commit=$(git -C "$root" rev-parse --short HEAD 2>/dev/null || true)
plist_set GhosttyCommit string "$commit"
while IFS='|' read -r key text; do
    plist_set "$key" string "$text"
done <<'KEYS'
NSAppleEventsUsageDescription|A program running within Ghostty would like to use AppleScript.
NSAudioCaptureUsageDescription|A program running within Ghostty would like to access your system's audio.
NSBluetoothAlwaysUsageDescription|A program running within Ghostty would like to use Bluetooth.
NSCalendarsUsageDescription|A program running within Ghostty would like to access your Calendar.
NSCameraUsageDescription|A program running within Ghostty would like to use the camera.
NSContactsUsageDescription|A program running within Ghostty would like to access your Contacts.
NSLocalNetworkUsageDescription|A program running within Ghostty would like to access the local network.
NSLocationUsageDescription|A program running within Ghostty would like to access your location information.
NSMicrophoneUsageDescription|A program running within Ghostty would like to use your microphone.
NSMotionUsageDescription|A program running within Ghostty would like to access motion data.
NSPhotoLibraryUsageDescription|A program running within Ghostty would like to access your Photo Library.
NSRemindersUsageDescription|A program running within Ghostty would like to access your reminders.
NSSpeechRecognitionUsageDescription|A program running within Ghostty would like to use speech recognition.
NSSystemAdministrationUsageDescription|A program running within Ghostty requires elevated privileges.
KEYS
# NSLocationTemporaryUsageDescriptionDictionary (a dict, not a string)
if [ "$dry" = 1 ]; then
    printf '+ PlistBuddy -c %q %q\n' 'Add :NSLocationTemporaryUsageDescriptionDictionary dict' "$plist"
else
    "$pb" -c 'Add :NSLocationTemporaryUsageDescriptionDictionary dict' "$plist" || true
fi
# NSServices menu item titles use $(INFOPLIST_KEY_CFBundleDisplayName).
if [ "$dry" = 1 ]; then
    echo "+ substitute \$(INFOPLIST_KEY_CFBundleDisplayName) -> $display_name in $plist"
else
    sed -i '' "s/\$(INFOPLIST_KEY_CFBundleDisplayName)/$display_name/g" "$plist"
fi

# 3. Dock tile plugin (a separate bundle target in Xcode; ported by MA4).
if [ -n "$plugin" ]; then
    run mkdir -p "$contents/PlugIns"
    run cp -R "$plugin" "$contents/PlugIns/DockTilePlugin.plugin"
else
    echo "-- no --dock-tile-plugin: dropping NSDockTilePlugIn from Info.plist"
    if [ "$dry" = 1 ]; then
        printf '+ PlistBuddy -c %q %q\n' 'Delete :NSDockTilePlugIn' "$plist"
    else
        "$pb" -c 'Delete :NSDockTilePlugIn' "$plist" || true
    fi
fi

# 4. AppleScript definition (OSAScriptingDefinition = Ghostty.sdef).
run cp "$mac/Ghostty.sdef" "$res/Ghostty.sdef"

# 5. Icons from Assets.xcassets (actool equivalent). AppIcon.icns for the app;
#    the alternate/custom icon images are copied as plain resources so
#    `Bundle.image(forResource:)` finds them.
iconset="$out/AppIcon.iconset"
src="$mac/Assets.xcassets/AppIconImage.imageset/macOS-AppIcon-1024px.png"
run rm -rf "$iconset"
run mkdir -p "$iconset"
for s in 16 32 128 256 512; do
    run sips -z $s $s "$src" --out "$iconset/icon_${s}x${s}.png"
    run sips -z $((s * 2)) $((s * 2)) "$src" --out "$iconset/icon_${s}x${s}@2x.png"
done
run iconutil -c icns "$iconset" -o "$res/AppIcon.icns"
run cp "$src" "$res/AppIconImage.png"
for img in "$mac"/Assets.xcassets/Alternate\ Icons/*.imageset; do
    name=$(basename "$img" .imageset)
    run cp "$img/macOS-AppIcon-1024px.png" "$res/$name.png"
done
for img in "$mac"/Assets.xcassets/Custom\ Icon/*.imageset; do
    name=$(basename "$img" .imageset)
    png=$(cd "$img" 2>/dev/null && ls ./*.png 2>/dev/null | head -n1 || true)
    [ -n "$png" ] && run cp "$img/${png#./}" "$res/$name.png"
done
run cp "$mac/Assets.xcassets/ResetZoom.imageset/ResetZoom.pdf" "$res/ResetZoom.pdf"

# 6. Ghostty resources (shell integration, themes, terminfo). The lookup done by
#    ghostty-os `resources_dir()` on macOS: Contents/Resources/terminfo/78/xterm-ghostty
#    is the sentinel and the resources dir is Contents/Resources/ghostty.
run mkdir -p "$res/ghostty" "$res/terminfo/78" "$res/terminfo/67"
run cp -R "$root/resources/shell-integration" "$res/ghostty/shell-integration"
run cp -R "$root/resources/themes" "$res/ghostty/themes"
run cp "$root/resources/terminfo/x/xterm-ghostty" "$res/terminfo/78/xterm-ghostty"
run cp "$root/resources/terminfo/g/ghostty" "$res/terminfo/67/ghostty"

# 7. Sign (entitlements of the Xcode target).
run codesign --force --deep --sign "$sign" --entitlements "$entitlements" "$app"

echo "== done: $app"
