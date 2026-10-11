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
