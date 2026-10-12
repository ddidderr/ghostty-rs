set positional-arguments

run *args:
    cargo run -- "$@"

build:
    cargo build

build-release:
    cargo build --release

build-production:
    cargo build --profile production

fmt:
    cargo +nightly fmt
    tombi format
    fd -tf -e md -x prettier --write --prose-wrap always --print-width 80
    rumdl check --flavor commonmark --fix
    just --fmt

_fix:
    cargo fix --workspace --all-targets --all-features
    cargo clippy --fix --workspace --all-targets --all-features

fix: _fix fmt

clippy:
    cargo clippy --workspace --all-targets --all-features -- -D warnings

test:
    cargo test --workspace --all-targets --all-features

clean:
    cargo clean

# Install bin + share/ghostty (shell-integration, themes) + share/terminfo +
# desktop file, D-Bus/systemd/metainfo and icons, mirroring Ghostty's
# GhosttyResources.zig. Usage: `just install /usr/local` (default PREFIX
# /usr/local; honours DESTDIR). Builds the release profile.
install PREFIX="/usr/local": build-release
    #!/usr/bin/env bash
    set -euo pipefail
    P="${DESTDIR:-}{{ PREFIX }}"
    install -Dm755 target/release/ghostty "$P/bin/ghostty"
    mkdir -p "$P/share/ghostty" "$P/share/terminfo"
    cp -R resources/shell-integration resources/themes "$P/share/ghostty/"
    find "$P/share/ghostty/shell-integration" -name '*.md' -delete
    find "$P/share/ghostty/themes" -name '*.md' -delete
    cp -R resources/terminfo/. "$P/share/terminfo/"
    gen() { # template out
        mkdir -p "$(dirname "$2")"
        sed -e 's|@NAME@|Ghostty|g' -e 's|@APPID@|com.mitchellh.ghostty|g' \
            -e "s|@GHOSTTY@|{{ PREFIX }}/bin/ghostty|g" "$1" > "$2"
    }
    gen dist/linux/app.desktop.in "$P/share/applications/com.mitchellh.ghostty.desktop"
    gen dist/linux/dbus.service.in "$P/share/dbus-1/services/com.mitchellh.ghostty.service"
    gen dist/linux/systemd.service.in "$P/share/systemd/user/app-com.mitchellh.ghostty.service"
    gen dist/linux/com.mitchellh.ghostty.metainfo.xml.in "$P/share/metainfo/com.mitchellh.ghostty.metainfo.xml"
    install -Dm644 dist/linux/ghostty_dolphin.desktop "$P/share/kio/servicemenus/com.mitchellh.ghostty.desktop"
    install -Dm755 dist/linux/ghostty_nautilus.py "$P/share/nautilus-python/extensions/ghostty.py"
    ic="$P/share/icons/hicolor"
    for pair in 16:16x16 32:32x32 128:128x128 256:256x256 512:512x512 1024:1024x1024 \
                32:16x16@2 64:32x32@2 256:128x128@2 512:256x256@2; do
        install -Dm644 "dist/icons/${pair%%:*}.png" "$ic/${pair##*:}/apps/com.mitchellh.ghostty.png"
    done
    echo "installed to $P"

# Regenerate resources/terminfo from the terminfo source in ghostty-termio.
terminfo:
    scripts/gen-terminfo.sh
