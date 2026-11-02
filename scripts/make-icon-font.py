#!/usr/bin/env python3
"""Draw assets/icons.ttf: the two emoji egui's bundled fonts lack for IT
notes, as plain shapes in the style of its Noto Emoji.

  U+1F6DC 🛜 wireless  - the Wi-Fi fan (Unicode 15; no bundled font has it)
  U+1F5C4 🗄 file cabinet - drawn as a server rack (the bundled one is a
                            two-drawer filing cabinet)

Metrics match NotoEmoji-Regular (2048 units, advance 2600), so with the same
0.81 scale they sit and size like the other emoji. Needs fontTools:
  pip install fonttools && python3 scripts/make-icon-font.py
"""

import math
import pathlib

from fontTools.fontBuilder import FontBuilder
from fontTools.pens.ttGlyphPen import TTGlyphPen

UPM, ADVANCE, ASCENT, DESCENT = 2048, 2600, 2189, -600
OUT = pathlib.Path(__file__).resolve().parent.parent / "assets" / "icons.ttf"


def poly(pen, pts):
    pen.moveTo(pts[0])
    for p in pts[1:]:
        pen.lineTo(p)
    pen.closePath()


def rect(pen, x0, y0, x1, y1, hole=False):
    """Clockwise fills, anticlockwise cuts a hole (TrueType winding)."""
    pts = [(x0, y0), (x0, y1), (x1, y1), (x1, y0)]
    poly(pen, pts[::-1] if hole else pts)


def circle(pen, cx, cy, r, hole=False, n=32):
    pts = [
        (round(cx + r * math.cos(a)), round(cy + r * math.sin(a)))
        for a in (-2 * math.pi * i / n for i in range(n))
    ]
    poly(pen, pts[::-1] if hole else pts)


def arc(pen, cx, cy, r0, r1, half, n=24):
    """A band between radii r0 and r1, `half` radians either side of
    straight up, with round ends."""
    mid, cap = (r0 + r1) / 2, (r1 - r0) / 2
    pts = []
    # Outer edge, left to right (clockwise).
    for i in range(n + 1):
        a = math.pi / 2 + half - 2 * half * i / n
        pts.append((cx + r1 * math.cos(a), cy + r1 * math.sin(a)))
    # Right-hand round end.
    end = math.pi / 2 - half
    ex, ey = cx + mid * math.cos(end), cy + mid * math.sin(end)
    for i in range(1, 12):
        a = end - math.pi * i / 12
        pts.append((ex + cap * math.cos(a), ey + cap * math.sin(a)))
    # Inner edge, right to left.
    for i in range(n + 1):
        a = math.pi / 2 - half + 2 * half * i / n
        pts.append((cx + r0 * math.cos(a), cy + r0 * math.sin(a)))
    # Left-hand round end.
    start = math.pi / 2 + half
    sx, sy = cx + mid * math.cos(start), cy + mid * math.sin(start)
    for i in range(1, 12):
        a = start + math.pi - math.pi * i / 12
        pts.append((sx + cap * math.cos(a), sy + cap * math.sin(a)))
    poly(pen, [(round(x), round(y)) for x, y in pts])


def wireless():
    """The Wi-Fi fan: a dot and three arcs."""
    pen = TTGlyphPen(None)
    cx, cy = ADVANCE // 2, 10
    circle(pen, cx, cy, 190)
    for r0, r1 in ((500, 750), (950, 1200), (1400, 1650)):
        arc(pen, cx, cy, r0, r1, math.radians(45))
    return pen.glyph()


def rack():
    """A server rack: a frame on feet, four servers with lights."""
    pen = TTGlyphPen(None)
    x0, x1, y0, y1, t = 560, 2040, -230, 1800, 150
    rect(pen, x0, y0, x1, y1)
    rect(pen, x0 + t, y0 + t, x1 - t, y1 - t, hole=True)
    # Feet.
    rect(pen, x0 + 70, -340, x0 + 300, y0)
    rect(pen, x1 - 300, -340, x1 - 70, y0)
    # Four servers, each with two lights and a vent slot cut out.
    ix0, ix1 = x0 + t + 80, x1 - t - 80
    h, gap = 330, 90
    top = y1 - t - 70
    for k in range(4):
        u1 = top - k * (h + gap)
        u0 = u1 - h
        rect(pen, ix0, u0, ix1, u1)
        mid = (u0 + u1) / 2
        for lx in (ix0 + 150, ix0 + 340):
            circle(pen, lx, mid, 72, hole=True, n=16)
        rect(pen, ix1 - 560, mid - 50, ix1 - 120, mid + 50, hole=True)
    return pen.glyph()


def main():
    fb = FontBuilder(UPM, isTTF=True)
    order = [".notdef", "u1F6DC", "u1F5C4"]
    fb.setupGlyphOrder(order)
    fb.setupCharacterMap({0x1F6DC: "u1F6DC", 0x1F5C4: "u1F5C4"})
    fb.setupGlyf(
        {".notdef": TTGlyphPen(None).glyph(), "u1F6DC": wireless(), "u1F5C4": rack()}
    )
    metrics = {}
    for name in order:
        g = fb.font["glyf"][name]
        g.recalcBounds(fb.font["glyf"])
        metrics[name] = (ADVANCE, getattr(g, "xMin", 0))
    fb.setupHorizontalMetrics(metrics)
    fb.setupHorizontalHeader(ascent=ASCENT, descent=DESCENT)
    fb.setupNameTable({"familyName": "is-alive icons", "styleName": "Regular"})
    fb.setupOS2(
        sTypoAscender=ASCENT,
        sTypoDescender=DESCENT,
        usWinAscent=ASCENT,
        usWinDescent=-DESCENT,
    )
    fb.setupPost()
    fb.save(OUT)
    print(f"{OUT}: {OUT.stat().st_size} bytes")


if __name__ == "__main__":
    main()
