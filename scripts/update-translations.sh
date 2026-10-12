#!/usr/bin/env bash
# Update the translation catalogs in po/ (Zig: `zig build update-translations`,
# build/GhosttyI18n.zig `createUpdateStep`).
#
#   1. xgettext over the GTK blueprint files and the Rust sources of the GTK
#      apprt (Zig: src/apprt/gtk/**/*.zig) -> gtk.pot. The `tr(...)` function
#      of `ghostty_os::i18n` is the Rust counterpart of Zig's `_(...)`.
#   2. xgettext over the Nautilus extension -> py.pot.
#   3. xgettext merge (py.pot first, because of xgettext's charset behaviour
#      when merging) -> po/com.mitchellh.ghostty.pot.
#   4. msgmerge every po/<locale>.po against the new template.
#
# Usage: scripts/update-translations.sh [OUT_DIR]
#   OUT_DIR  where to write the .pot/.po files (default: po/, i.e. in place).
#            The existing catalogs are always read from po/.
set -euo pipefail

cd "$(dirname "$0")/.."
domain=com.mitchellh.ghostty
out=${1:-po}
mkdir -p "$out"
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT

gtk=crates/ghostty-apprt-gtk

# Blueprint files in the order of the GResource list (build.rs BLUEPRINTS,
# Zig: gresource.blueprints). The arguments to xgettext must be relative
# paths in the repo root so the location comments don't depend on where the
# checkout lives.
mapfile -t blueprints < <(
    sed -n 's/^[[:space:]]*(\([0-9]*\), \([0-9]*\), "\([^"]*\)"),.*/\1 \2 \3/p' "$gtk/build.rs" |
        while read -r major minor name; do
            echo "$gtk/resources/ui/$major.$minor/$name.blp"
        done
)
# Rust sources of the GTK apprt, sorted for a deterministic order.
mapfile -t sources < <(cd . && find "$gtk/src" -type f -name '*.rs' | LC_ALL=C sort)

xgettext \
    --language=C \
    --from-code=UTF-8 \
    --keyword=tr \
    --keyword=_ \
    --keyword=C_:1c,2 \
    -o "$tmp/gtk.pot" \
    "${blueprints[@]}" "${sources[@]}"

xgettext \
    --language=Python \
    --from-code=UTF-8 \
    -o "$tmp/py.pot" \
    dist/linux/ghostty_nautilus.py

xgettext \
    --add-comments=Translators \
    --package-name="$domain" \
    --msgid-bugs-address=m@mitchellh.com \
    '--copyright-holder="Mitchell Hashimoto, Ghostty contributors"' \
    -o - \
    "$tmp/py.pot" "$tmp/gtk.pot" >"$tmp/$domain.pot"

for po in po/*.po; do
    lang=$(basename "$po" .po)
    msgmerge --quiet --no-fuzzy-matching "$po" "$tmp/$domain.pot" >"$tmp/$lang.po"
done

# Only write once everything succeeded (Zig: UpdateSourceFiles).
cp "$tmp/$domain.pot" "$out/$domain.pot"
for po in po/*.po; do
    lang=$(basename "$po" .po)
    cp "$tmp/$lang.po" "$out/$lang.po"
done
echo "updated $out/$domain.pot and $(ls po/*.po | wc -l) catalogs"
