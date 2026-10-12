#!/usr/bin/env bash
# Iteration build of the Rust binary used by live-dbus-test.sh / visual-parity.
cd "$(dirname "$0")/../.." || exit 1
CARGO_TARGET_DIR=target/vp CARGO_PROFILE_PRODUCTION_LTO=off CARGO_PROFILE_PRODUCTION_CODEGEN_UNITS=16 \
  CARGO_PROFILE_PRODUCTION_INCREMENTAL=true exec cargo build --profile production "$@"
