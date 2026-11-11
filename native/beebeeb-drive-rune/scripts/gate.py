#!/usr/bin/env python3
"""gate.py: the frame gate. Board golden against the native render, element by element.

Usage: gate.py <board.json> <board.png> <native.json> <native.png> <out-dir> <stem>

Four checks, each over every keyed board element (scripts/keymap.js):

  geometry  the box: x, y, w, h within 1 px. A text's box is its line box, whose height is
            the font's and not the layout's, so a text is held to x, w (2 px: two shapers)
            and its vertical centre.
  pixels    the pixels INSIDE the board's box, cropped from both images:
              tile  both crops blurred (anti-aliasing differs between Chromium and Rune),
                    cut into 8 px tiles; the worst tile's mean difference. A wrong icon, a
                    missing thumbnail, a wrong fill, text at the other end of its cell.
              edge  the brightness profile across each of the four edges, unblurred; the
                    worst difference. A missing or extra hairline border or separator.
            The native crop may sit up to 1 px (2 device px) off: what geometry allows.
  text      the string, family, size, weight, letter spacing and colour.
  icons     WHICH glyph (the SVG path data), outlined or filled, its state name and colour.

Prints one table per failing check and the truth line:
  geometry: N checked, M failing · pixels: N checked, M failing · text: N checked, M failing · icons: N checked, M failing
Exit 0 only when every M is 0 and every N is above 0.

Also writes <stem>-side-by-side.png, <stem>-tiles.png (every 8 px tile of the whole frame
that differs: look there for what no key covers yet) and fail/<key>.png (board | native).
"""
import json
import math
import re
import shutil
import sys
from pathlib import Path

from PIL import Image, ImageChops, ImageDraw, ImageFilter, ImageStat

POS_TOL = 1.0          # px, boxes
TEXT_W_TOL = 2.0       # px, a text's width (two shapers)
SIZE_TOL = 0.05        # px, font size
TRACK_TOL = 0.06       # px, letter spacing
COLOR_TOL = 4          # 0-255 per channel, declared colours
BLUR = 1.2             # device px
TILE = 8               # device px
TILE_TOL = 26.0        # 0-255, worst tile mean of the blurred difference
EDGE_TOL = 9.0         # 0-255, worst edge-profile difference
SHIFT = 2              # device px the native crop may move
CORNER = 8             # px: the OS rounds the window's corners; the board's corner pixels are page


def load(path):
    with open(path, encoding="utf-8") as f:
        return json.load(f)


def chan_delta(a, b):
    if a is None or b is None:
        return 255
    return max(abs(int(x) - int(y)) for x, y in zip(a[:3], b[:3]))


def geometry(key, b, n):
    issues = []
    if b["kind"] == "text":
        dx, dw = abs(b["x"] - n["x"]), abs(b["w"] - n["w"])
        dc = abs((b["y"] + b["h"] / 2) - (n["y"] + n["h"] / 2))
        if dx > POS_TOL:
            issues.append(f"x {n['x']:.1f} (board {b['x']:.1f})")
        if dc > POS_TOL:
            issues.append(f"centre-y {n['y'] + n['h'] / 2:.1f} (board {b['y'] + b['h'] / 2:.1f})")
        if dw > TEXT_W_TOL:
            issues.append(f"w {n['w']:.1f} (board {b['w']:.1f})")
        return issues, max(dx, dc, dw)
    worst = 0.0
    for f in ("x", "y", "w", "h"):
        d = abs(b[f] - n[f])
        worst = max(worst, d)
        if d > POS_TOL:
            issues.append(f"{f} {n[f]:.1f} (board {b[f]:.1f})")
    return issues, worst


def text_check(b, n):
    issues = []
    if n.get("text") != b["text"]:
        issues.append(f"text {n.get('text')!r} (board {b['text']!r})")
    if n.get("family") != b["family"]:
        issues.append(f"family {n.get('family')} (board {b['family']})")
    if n.get("size") is None or abs(n["size"] - b["size"]) > SIZE_TOL:
        issues.append(f"size {n.get('size')} (board {b['size']:g})")
    if n.get("weight") != b["weight"]:
        issues.append(f"weight {n.get('weight')} (board {b['weight']})")
    if abs((n.get("tracking") or 0.0) - b.get("tracking", 0.0)) > TRACK_TOL:
        issues.append(f"tracking {n.get('tracking')} (board {b.get('tracking')})")
    if chan_delta(b["color"], n.get("color")) > COLOR_TOL:
        issues.append(f"colour {n.get('color')} (board {b['color'][:3]})")
    return issues


def norm_path(d):
    return re.sub(r"\s+", " ", d.strip())


def icon_check(b, n):
    issues = []
    if b.get("mark"):
        # The brand mark draws in its own colours; which glyph it is, is its path data.
        if [norm_path(d) for d in n.get("icon") or []] != [norm_path(d) for d in b["icon"]]:
            issues.append(f"glyph {n.get('iconName')} is not the board's mark")
        return issues
    if [norm_path(d) for d in n.get("icon") or []] != [norm_path(d) for d in b["icon"]]:
        issues.append(f"glyph {n.get('iconName')} (board path {b['icon'][0][:28]}…)")
    if bool(n.get("filled")) != bool(b.get("filled")):
        issues.append(f"filled {bool(n.get('filled'))} (board {bool(b.get('filled'))})")
    if "state" in b and n.get("state") != b["state"]:
        issues.append(f"state {n.get('state')} (board {b['state']})")
    if chan_delta(b["color"], n.get("color")) > COLOR_TOL:
        issues.append(f"colour {n.get('color')} (board {b['color'][:3]})")
    return issues


class Pixels:
    def __init__(self, board_png, native_png, width, height):
        self.board = Image.open(board_png).convert("RGB")
        self.native = Image.open(native_png).convert("RGB")
        self.size_issue = None
        if self.board.size != self.native.size:
            self.size_issue = f"render is {self.native.size[0]}x{self.native.size[1]}, golden is {self.board.size[0]}x{self.board.size[1]}"
            return
        self.scale = self.board.size[0] / width
        blur = ImageFilter.GaussianBlur(BLUR)
        self.board_blur = self.board.filter(blur)
        self.native_blur = self.native.filter(blur)
        self.board_l = self.board.convert("L")
        self.native_l = self.native.convert("L")

    def device_box(self, b):
        s = self.scale
        x0, y0 = math.floor(b["x"] * s + 1e-6), math.floor(b["y"] * s + 1e-6)
        x1, y1 = math.ceil((b["x"] + b["w"]) * s - 1e-6), math.ceil((b["y"] + b["h"]) * s - 1e-6)
        w, h = self.board.size
        return max(0, x0), max(0, y0), min(w, max(x1, x0 + 1)), min(h, max(y1, y0 + 1))

    @staticmethod
    def worst_tile(a, b):
        diff = ImageChops.difference(a, b)
        w, h = diff.size
        tiles = diff.resize((max(1, round(w / TILE)), max(1, round(h / TILE))), Image.BOX)
        return max(max(px) for px in tiles.getdata())

    def tile(self, box):
        x0, y0, x1, y1 = box
        w, h = self.board.size
        a = self.board_blur.crop(box)
        best = 255.0
        for dy in range(-SHIFT, SHIFT + 1):
            for dx in range(-SHIFT, SHIFT + 1):
                if x0 + dx < 0 or y0 + dy < 0 or x1 + dx > w or y1 + dy > h:
                    continue
                best = min(best, self.worst_tile(a, self.native_blur.crop((x0 + dx, y0 + dy, x1 + dx, y1 + dy))))
                if best == 0:
                    return 0.0
        return float(best)

    def _profile(self, img, box, side):
        """Mean brightness of the lines parallel to one edge, from 3 px outside to 6 px inside."""
        x0, y0, x1, y1 = box
        w, h = img.size
        span_x = (x0 + (x1 - x0) // 5, max(x0 + (x1 - x0) // 5 + 1, x1 - (x1 - x0) // 5))
        span_y = (y0 + (y1 - y0) // 5, max(y0 + (y1 - y0) // 5 + 1, y1 - (y1 - y0) // 5))
        depth = min(6, (y1 - y0) // 2 if side in "tb" else (x1 - x0) // 2)
        out = []
        for o in range(-4, depth + 2):
            if side == "t":
                line = (span_x[0], y0 + o, span_x[1], y0 + o + 1)
            elif side == "b":
                line = (span_x[0], y1 - 1 - o, span_x[1], y1 - o)
            elif side == "l":
                line = (x0 + o, span_y[0], x0 + o + 1, span_y[1])
            else:
                line = (x1 - 1 - o, span_y[0], x1 - o, span_y[1])
            if line[0] < 0 or line[1] < 0 or line[2] > w or line[3] > h:
                out.append(None)
            else:
                out.append(ImageStat.Stat(img.crop(line)).mean[0])
        return out

    def edge(self, box):
        worst = 0.0
        for side in "tblr":
            pb = self._profile(self.board_l, box, side)
            pn = self._profile(self.native_l, box, side)
            best = 255.0
            for s in (-1, 0, 1):
                d = 0.0
                for i in range(1, len(pb) - 1):
                    a, b = pb[i], pn[i + s]
                    if a is None or b is None:
                        continue
                    d = max(d, abs(a - b))
                best = min(best, d)
            worst = max(worst, best)
        return worst

    def crop_pair(self, box, path, pad=6, zoom=3):
        x0, y0, x1, y1 = box
        w, h = self.board.size
        big = (max(0, x0 - pad), max(0, y0 - pad), min(w, x1 + pad), min(h, y1 + pad))
        a, b = self.board.crop(big), self.native.crop(big)
        z = zoom if a.size[0] * zoom <= 1400 else 1
        a = a.resize((a.size[0] * z, a.size[1] * z), Image.NEAREST)
        b = b.resize((b.size[0] * z, b.size[1] * z), Image.NEAREST)
        out = Image.new("RGB", (a.size[0] + b.size[0] + 8, a.size[1]), (255, 0, 255))
        out.paste(a, (0, 0))
        out.paste(b, (a.size[0] + 8, 0))
        out.save(path)

    def frame_tiles(self, path):
        """Every tile of the whole frame, no shift: the map of what differs anywhere."""
        diff = ImageChops.difference(self.board_blur, self.native_blur)
        w, h = diff.size
        tw, th = w // TILE, h // TILE
        tiles = diff.resize((tw, th), Image.BOX)
        over = 0
        shade = self.native.copy().convert("RGBA")
        draw = ImageDraw.Draw(shade, "RGBA")
        c = int(CORNER * self.scale / TILE) + 1
        for i, px in enumerate(tiles.getdata()):
            tx, ty = i % tw, i // tw
            if (tx < c or tx >= tw - c) and (ty < c or ty >= th - c):
                continue
            if max(px) > TILE_TOL:
                over += 1
                draw.rectangle((tx * TILE, ty * TILE, tx * TILE + TILE - 1, ty * TILE + TILE - 1), fill=(255, 0, 0, 110))
        shade.convert("RGB").save(path)
        return tw * th, over


def table(title, rows):
    if not rows:
        return
    print(f"\n{title}")
    print(f"  {'key':<28} detail")
    for key, detail in rows:
        print(f"  {key:<28} {detail}")


def main():
    if len(sys.argv) != 7:
        print(__doc__)
        return 64
    board_json, board_png, native_json, native_png, out_dir, stem = sys.argv[1:]
    out = Path(out_dir)
    out.mkdir(parents=True, exist_ok=True)
    fail_dir = out / "fail"
    if fail_dir.exists():
        shutil.rmtree(fail_dir)
    fail_dir.mkdir()

    board = load(board_json)
    native = load(native_json)
    b_els, n_els = board["elements"], native["elements"]
    px = Pixels(board_png, native_png, board["width"], board["height"])
    if px.size_issue:
        print(f"gate: {px.size_issue}")
        return 2

    geo_fail, pix_fail, text_fail, icon_fail = [], [], [], []
    geo_n = pix_n = text_n = icon_n = 0
    corner = lambda b: (b["x"] < CORNER or b["x"] + b["w"] > board["width"] - CORNER) and (b["y"] < CORNER or b["y"] + b["h"] > board["height"] - CORNER)

    for key, b in b_els.items():
        n = n_els.get(key)
        geo_n += 1
        if n is None:
            geo_fail.append((999.0, key, "MISSING in the native tree"))
        else:
            issues, worst = geometry(key, b, n)
            if issues:
                geo_fail.append((worst, key, "; ".join(issues)))

        if b["kind"] == "text":
            text_n += 1
            issues = ["MISSING in the native tree"] if n is None else text_check(b, n)
            if issues:
                text_fail.append((key, "; ".join(issues)))
        elif b["kind"] == "icon":
            icon_n += 1
            issues = ["MISSING in the native tree"] if n is None else icon_check(b, n)
            if issues:
                icon_fail.append((key, "; ".join(issues)))

        pix_n += 1
        box = px.device_box(b)
        tile = px.tile(box)
        # A box that reaches a window corner holds the page behind the board's rounded frame.
        edge = 0.0 if corner(b) else px.edge(box)
        if tile > TILE_TOL or edge > EDGE_TOL:
            parts = []
            if tile > TILE_TOL:
                parts.append(f"tile {tile:.0f} > {TILE_TOL:.0f}")
            if edge > EDGE_TOL:
                parts.append(f"edge {edge:.0f} > {EDGE_TOL:.0f}")
            pix_fail.append((max(tile / TILE_TOL, edge / EDGE_TOL), key, "; ".join(parts)))
            px.crop_pair(box, fail_dir / f"{re.sub(r'[^A-Za-z0-9_.-]+', '_', key)}.png")

    geo_fail.sort(key=lambda r: -r[0])
    pix_fail.sort(key=lambda r: -r[0])
    table("geometry failures (worst first)", [(k, d) for _, k, d in geo_fail])
    table("pixel failures (worst first; crops in fail/<key>.png, board | native)", [(k, d) for _, k, d in pix_fail])
    table("text failures", text_fail)
    table("icon failures", icon_fail)

    side = Image.new("RGB", (px.board.size[0] * 2 + 16, px.board.size[1]), (255, 0, 255))
    side.paste(px.board, (0, 0))
    side.paste(px.native, (px.board.size[0] + 16, 0))
    side.save(out / f"{stem}-side-by-side.png")
    total_tiles, over_tiles = px.frame_tiles(out / f"{stem}-tiles.png")

    extra = sorted(set(n_els) - set(b_els))
    print(f"\nframe tiles (whole frame, unshifted, informative): {total_tiles} tiles, {over_tiles} over {TILE_TOL:.0f} -> {out / (stem + '-tiles.png')}")
    print(f"native keys the board does not export: {len(extra)}")
    line = (f"geometry: {geo_n} checked, {len(geo_fail)} failing · pixels: {pix_n} checked, {len(pix_fail)} failing · "
            f"text: {text_n} checked, {len(text_fail)} failing · icons: {icon_n} checked, {len(icon_fail)} failing")
    print(f"\n{stem} {line}")
    (out / f"{stem}-gate.txt").write_text(f"{stem} {line}\n", encoding="utf-8")
    failing = len(geo_fail) + len(pix_fail) + len(text_fail) + len(icon_fail)
    if min(geo_n, pix_n, text_n, icon_n) == 0:
        print("gate: a check ran over 0 elements: that is a RED, not a pass")
        return 1
    return 1 if failing else 0


if __name__ == "__main__":
    sys.exit(main())
