#!/usr/bin/env python3
"""Pixel compare two PNG captures: stats + diff heatmap.

usage: compare.py REF.png RUST.png DIFF.png [--crop x,y,w,h] [--name NAME]
Prints one line: name, size, differing pixels (%), >8 / >32 levels, max delta, mean delta.
"""
import sys
import numpy as np
from PIL import Image


def main():
    a = sys.argv[1:]
    crop = None
    name = "all"
    pos = []
    i = 0
    while i < len(a):
        if a[i] == "--crop":
            crop = tuple(int(v) for v in a[i + 1].split(","))
            i += 2
        elif a[i] == "--name":
            name = a[i + 1]
            i += 2
        else:
            pos.append(a[i])
            i += 1
    ref = Image.open(pos[0]).convert("RGB")
    rs = Image.open(pos[1]).convert("RGB")
    if crop:
        x, y, w, h = crop
        ref, rs = ref.crop((x, y, x + w, y + h)), rs.crop((x, y, x + w, y + h))
    if ref.size != rs.size:
        print(f"{name}: SIZE MISMATCH {ref.size} vs {rs.size}")
        w, h = min(ref.size[0], rs.size[0]), min(ref.size[1], rs.size[1])
        ref, rs = ref.crop((0, 0, w, h)), rs.crop((0, 0, w, h))
    r = np.asarray(ref).astype(int)
    s = np.asarray(rs).astype(int)
    d = np.abs(r - s).max(axis=2)
    n = d.size
    diff = int((d > 0).sum())
    print(
        f"{name}: {ref.size[0]}x{ref.size[1]} differing={diff} ({100*diff/n:.3f}%) "
        f">8={100*(d>8).sum()/n:.3f}% >32={100*(d>32).sum()/n:.3f}% max={d.max()} mean={d.mean():.4f}"
    )
    # heatmap: dimmed reference with differences in red (intensity ~ delta)
    heat = (r * 0.25).astype(np.uint8)
    m = np.clip(d * 4, 0, 255).astype(np.uint8)
    heat[..., 0] = np.maximum(heat[..., 0], m)
    heat[..., 1] = np.where(m > 0, heat[..., 1] // 2, heat[..., 1])
    Image.fromarray(heat).save(pos[2])


main()
