#!/usr/bin/env python3
"""export-viewer-pages.py: the board's two sample PDF pages (DW-1505 "Studio plan.pdf" page 2,
DW-1506 "Service agreement.pdf" page 1) as PNG files at 2x, cut from the committed goldens.

The fixture of `frames::viewers` hands these to the PDF viewer as the page's pixels (a build that
ships has no fixture and draws a blank page). The find marks of DW-1506 are drawn by the viewer,
so they are taken out of the picture here: a mark is black at a known alpha over the page, which
is undone per pixel (c / (1 - alpha)).

Usage: scripts/export-viewer-pages.py   (writes src/frames/viewers/assets/page-*.png)
Last line: `pages: N written`.
"""
import os
from PIL import Image

HERE = os.path.dirname(os.path.abspath(__file__))
CRATE = os.path.dirname(HERE)
WHITE = (255, 255, 255)
# frame, the page's top in the golden (2x pixels), the marks (board points: x, y, w, h, alpha/255)
PAGES = [
    ("DW-1505", 204, [], "page-studio-plan-2.png"),
    ("DW-1506", 285, [(897.17, 304, 44.31, 15, 33), (721.91, 409, 44.31, 15, 82)], "page-service-agreement-1.png"),
]

written = 0
for frame, top, marks, name in PAGES:
    im = Image.open(os.path.join(CRATE, "golden", f"{frame}-board.png")).convert("RGB")
    px = im.load()
    xs = [x for x in range(680, im.width) if px[x, top + 40] == WHITE]
    x0, x1 = xs[0], xs[-1] + 1
    ys = [y for y in range(top, 1540) if px[x0 + 4, y] == WHITE]
    y0, y1 = ys[0], ys[-1] + 1
    for mx, my, mw, mh, alpha in marks:
        keep = 1 - alpha / 255
        for y in range(round(my * 2), round((my + mh) * 2)):
            for x in range(round(mx * 2), round((mx + mw) * 2)):
                c = px[x, y]
                if max(c) <= 255 * keep + 1.5:  # inside the mark's rounded box
                    px[x, y] = tuple(min(255, round(v / keep)) for v in c)
    page = im.crop((x0, y0, x1, y1))
    page.save(os.path.join(CRATE, "src", "frames", "viewers", "assets", name), optimize=True)
    print(f"{name}: {page.width}x{page.height} from {frame} at ({x0},{y0})")
    written += 1
print(f"pages: {written} written")
