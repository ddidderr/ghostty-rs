#!/usr/bin/env python3
"""zoom.py DIR x,y,w,h scale OUT : side-by-side ref | rust | diff crop, upscaled (nearest)."""
import sys
from PIL import Image
d, box, sc, out = sys.argv[1], [int(v) for v in sys.argv[2].split(",")], int(sys.argv[3]), sys.argv[4]
x, y, w, h = box
ims = [Image.open(f"{d}/{n}.png").convert("RGB").crop((x, y, x + w, y + h)).resize((w * sc, h * sc), Image.NEAREST) for n in ("ref", "rust", "diff")]
o = Image.new("RGB", (w * sc, h * sc * 3 + 8), (255, 0, 255))
for i, im in enumerate(ims):
    o.paste(im, (0, i * (h * sc + 4)))
o.save(out)
