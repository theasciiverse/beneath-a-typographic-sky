#!/usr/bin/env python3
"""Replay an asciicast v3 file in pyte and save the screen at given times
as PNGs that approximate the terminal (truecolour, Braille, blocks, text).
Usage: cast2png.py file.cast outdir t1 t2 ...   (needs pyte and Pillow)"""
import json, sys, os
import pyte
from PIL import Image, ImageDraw, ImageFont

cast, outdir, times = sys.argv[1], sys.argv[2], sorted(float(t) for t in sys.argv[3:])
os.makedirs(outdir, exist_ok=True)
lines = open(cast, encoding="utf-8").read().splitlines()
head = json.loads(lines[0])
cols, rows = head["term"]["cols"], head["term"]["rows"]
screen = pyte.Screen(cols, rows)
stream = pyte.Stream(screen)
cw, ch = 8, 16
try:
    font = ImageFont.truetype("DejaVuSansMono-Bold.ttf", ch - 3)
except OSError:
    font = ImageFont.load_default()

def colour(c, default):
    if c == "default":
        return default
    if isinstance(c, str) and len(c) == 6:
        return tuple(int(c[i:i + 2], 16) for i in (0, 2, 4))
    named = {"black": (0, 0, 0), "white": (220, 220, 220), "red": (200, 0, 0), "green": (0, 200, 0)}
    return named.get(c, default)

def save(t):
    img = Image.new("RGB", (cols * cw, rows * ch))
    d = ImageDraw.Draw(img)
    for y in range(rows):
        for x in range(cols):
            c = screen.buffer[y][x]
            fg, bg = colour(c.fg, (200, 200, 200)), colour(c.bg, (0, 0, 0))
            if c.reverse:
                fg, bg = bg, fg
            x0, y0 = x * cw, y * ch
            d.rectangle([x0, y0, x0 + cw - 1, y0 + ch - 1], fill=bg)
            o = ord(c.data[0]) if c.data else 32
            if 0x2800 <= o <= 0x28FF:
                bits = o - 0x2800
                for dx, dy, b in [(0,0,1),(0,1,2),(0,2,4),(1,0,8),(1,1,16),(1,2,32),(0,3,64),(1,3,128)]:
                    if bits & b:
                        px, py = x0 + dx * cw // 2 + cw // 8, y0 + dy * ch // 4 + ch // 16
                        d.rectangle([px, py, px + cw // 4, py + ch // 8], fill=fg)
            elif c.data == "▀":
                d.rectangle([x0, y0, x0 + cw - 1, y0 + ch // 2 - 1], fill=fg)
            elif c.data and c.data != " ":
                d.text((x0, y0 + 1), c.data, fill=fg, font=font)
    img.save(os.path.join(outdir, f"t{t:05.1f}.png"))

t, ti = 0.0, 0
for line in lines[1:]:
    e = json.loads(line)
    t += e[0]
    while ti < len(times) and t > times[ti]:
        save(times[ti]); ti += 1
    if e[1] == "o":
        stream.feed(e[2])
while ti < len(times):
    save(times[ti]); ti += 1
