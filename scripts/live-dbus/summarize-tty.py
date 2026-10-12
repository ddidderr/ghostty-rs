#!/usr/bin/env python3
"""Summarize the raw `cat -v` capture of child-theme.sh: per phase (split by
the `--- after` markers) the collapsed sequence of CSI ? 997 reports and the
last OSC 11 background answer."""
import re
import sys

text = open(sys.argv[1]).read()
parts = re.split(r"\n--- (after [^\n]*)\n", "\n" + text)
# parts: [startup, name1, body1, name2, body2, ...]; markers are written
# right before the action, so the body after a marker is its effect.
names = ["startup"] + parts[1::2]
chunks = [parts[0]] + parts[2::2]
for name, chunk in zip(names, chunks):
    reports = re.findall(r"\^\[\[\?997;(\d)n", chunk)
    collapsed = [r for i, r in enumerate(reports) if i == 0 or r != reports[i - 1]]
    bgs = re.findall(r"\^\[\]11;(rgb:[0-9a-f/]+)\^G", chunk)
    print(f"{name}: 997 reports={','.join(collapsed) or '-'} last bg={bgs[-1] if bgs else '-'}")
