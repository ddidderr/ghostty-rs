#!/usr/bin/env bash
# Build + clippy the wasm32 targets (Ghostty's wasm build and libghostty-vt
# for wasm), then run the node-based wasm tests.
set -euo pipefail
cd "$(dirname "$0")/.."
cargo clippy --target wasm32-unknown-unknown -p ghostty-wasm -p ghostty-vt-capi \
  -p ghostty-font -p ghostty-config -p ghostty-terminal -p ghostty-vt --lib -- -D warnings
cargo build --target wasm32-unknown-unknown --release -p ghostty-wasm -p ghostty-vt-capi
cargo test -p ghostty-wasm -p ghostty-vt-capi --test node --test wasm_examples
