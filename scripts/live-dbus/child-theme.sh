#!/bin/sh
# Enables color-scheme reporting (mode 2031), asks for the current scheme
# (CSI ? 996 n) and keeps asking for the background colour (OSC 11).
esc=$(printf '\033'); bel=$(printf '\007')
stty raw -echo
printf '%s[?2031h%s[?996n' "$esc" "$esc"
( while :; do printf '%s]11;?%s' "$esc" "$bel"; sleep 0.7; done ) &
exec cat -v >>"$OUTF"
