#!/usr/bin/env bash
# Minimise every failing case in a fails-<seed> dir and print a compact report.
# usage: triage.sh <fails-dir> [max-cases]
set -u
D=$1; N=${2:-30}
BIN="$(dirname "$0")/../../target/debug/difftest_fuzz"
for f in $(ls "$D"/*.bin | head -n "$N"); do
  n=$(basename "$f" .bin); IFS=- read -r _ sz sb <<<"$n"
  echo "== $n"
  timeout 300 "$BIN" min "${sz%x*}" "${sz#*x}" "${sb#sb}" "$f" | head -8
done
