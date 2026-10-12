#!/bin/sh
# Prints links on known rows, then reports the text area size (CSI 18 t).
esc=$(printf '\033'); bel=$(printf '\007')
stty raw -echo
printf '%s[H%s[2J' "$esc" "$esc"
printf 'https://example.org/plain\r\n'
printf '%s]8;;https://example.com/osc8%s\\osc8-link%s]8;;%s\\\r\n' "$esc" "$esc" "$esc" "$esc"
printf 'file:///etc/hostname\r\n'
printf '/etc/hostname\r\n'
printf '%s[18t' "$esc"
exec cat -v >>"$OUTF"
