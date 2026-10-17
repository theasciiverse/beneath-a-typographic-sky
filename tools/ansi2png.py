#!/usr/bin/env python3
"""Render a `tmux capture-pane -e -p` dump (truecolour SGR, Braille, half
blocks, text) to a PNG that approximates what the terminal shows.
Usage: ansi2png.py capture.ans out.png [cell_w cell_h]"""
import re, sys
from PIL import Image, ImageDraw, ImageFont

src = open(sys.argv[1], encoding="utf-8", errors="replace").read()
cw = int(sys.argv[3]) if len(sys.argv) > 3 else 8
ch = int(sys.argv[4]) if len(sys.argv) > 4 else 16
lines = src.split("\n")
cols = 0
rows = []
for line in lines:
    fg, bg, bold = (200, 200, 200), (0, 0, 0), False
    cells = []
    i = 0
    while i < len(line):
        if line[i] == "\x1b":
            m = re.match(r"\x1b\[([0-9;]*)m", line[i:])
            if m:
                ps = [int(p) if p else 0 for p in m.group(1).split(";")] or [0]
                j = 0
                while j < len(ps):
                    p = ps[j]
                    if p == 0: fg, bg, bold = (200, 200, 200), (0, 0, 0), False
                    elif p == 1: bold = True
                    elif p == 38 and ps[j+1] == 2: fg = tuple(ps[j+2:j+5]); j += 4
                    elif p == 48 and ps[j+1] == 2: bg = tuple(ps[j+2:j+5]); j += 4
                    elif p == 39: fg = (200, 200, 200)
                    elif p == 49: bg = (0, 0, 0)
                    j += 1
                i += len(m.group(0)); continue
            m = re.match(r"\x1b\[[0-9;?]*[A-Za-z]", line[i:])
            if m: i += len(m.group(0)); continue
        cells.append((line[i], fg, bg))
        i += 1
    rows.append(cells)
    cols = max(cols, len(cells))
img = Image.new("RGB", (cols * cw, len(rows) * ch))
d = ImageDraw.Draw(img)
try:
    font = ImageFont.truetype("DejaVuSansMono-Bold.ttf", ch - 3)
except OSError:
    font = ImageFont.load_default()
for y, cells in enumerate(rows):
    for x, (c, fg, bg) in enumerate(cells):
        x0, y0 = x * cw, y * ch
        d.rectangle([x0, y0, x0 + cw - 1, y0 + ch - 1], fill=bg)
        o = ord(c)
        if 0x2800 <= o <= 0x28FF:
            bits = o - 0x2800
            order = [(0,0,1),(0,1,2),(0,2,4),(1,0,8),(1,1,16),(1,2,32),(0,3,64),(1,3,128)]
            for dx, dy, b in order:
                if bits & b:
                    px = x0 + dx * cw // 2 + cw // 8
                    py = y0 + dy * ch // 4 + ch // 16
                    d.rectangle([px, py, px + cw // 4, py + ch // 8], fill=fg)
        elif c == "▀":
            d.rectangle([x0, y0, x0 + cw - 1, y0 + ch // 2 - 1], fill=fg)
        elif c != " ":
            d.text((x0, y0 + 1), c, fill=fg, font=font)
img.save(sys.argv[2])
