#!/usr/bin/env python3
"""Generate the File Provider badge placeholder PNGs (task 1699).

Deterministic 128x128 RGBA PNGs: a flat filled circle on a transparent
background, one file per sync-state badge. Bytes are stable across runs
(no timestamps), so re-runs produce no diff churn. These are placeholders
pending a design pass (deviation D3 in the task 1699 record).

Usage: python3 scripts/gen-badge-pngs.py [outdir]
Writes badge-<name>.png for each entry in BADGES.
"""

import struct
import sys
import zlib

SIZE = 128
RADIUS = 52.0
CENTER = (SIZE - 1) / 2.0

# name -> (R, G, B)
BADGES = {
    "error": (0xE5, 0x48, 0x4D),
    "conflict": (0xF5, 0xA6, 0x23),
    "trashing": (0x8E, 0x8E, 0x93),
    "uploading": (0x3B, 0x82, 0xF6),
    "downloading": (0x34, 0xC7, 0x59),
    "added": (0x7C, 0x5C, 0xE0),
}

# LD-0113 Amendment 8a: the Backups root's "Read-only" badge, a padlock in a neutral grey (Amber is
# for the encryption state only). name -> (R, G, B)
LOCKS = {
    "read-only": (0x6B, 0x6B, 0x70),
}


def chunk(tag: bytes, data: bytes) -> bytes:
    return (
        struct.pack(">I", len(data))
        + tag
        + data
        + struct.pack(">I", zlib.crc32(tag + data) & 0xFFFFFFFF)
    )


def circle_png(rgb) -> bytes:
    r, g, b = rgb
    ihdr = struct.pack(">IIBBBBB", SIZE, SIZE, 8, 6, 0, 0, 0)
    raw = bytearray()
    inner = RADIUS**2
    edge = (RADIUS + 1.0) ** 2
    for y in range(SIZE):
        raw.append(0)  # filter type 0 per scanline
        for x in range(SIZE):
            dx = x - CENTER
            dy = y - CENTER
            d2 = dx * dx + dy * dy
            if d2 <= inner:
                alpha = 255
            elif d2 < edge:
                # simple 1px anti-alias ramp
                alpha = int(255 * (edge - d2) / (edge - inner))
            else:
                alpha = 0
            raw += bytes((r, g, b, alpha))
    return (
        b"\x89PNG\r\n\x1a\n"
        + chunk(b"IHDR", ihdr)
        + chunk(b"IDAT", zlib.compress(bytes(raw), 9))
        + chunk(b"IEND", b"")
    )


def in_lock(x: int, y: int) -> bool:
    """A padlock on the 128x128 grid: a rounded body, a shackle ring over it, a keyhole cut out."""
    # Body: x 30..98, y 58..108, corner radius 8.
    if 30 <= x <= 98 and 58 <= y <= 108:
        cx = min(max(x, 38), 90)
        cy = min(max(y, 66), 100)
        if (x - cx) ** 2 + (y - cy) ** 2 > 64:
            return False
        # Keyhole: a circle at (64, 78) with radius 7 and a slot below it.
        if (x - 64) ** 2 + (y - 78) ** 2 <= 49 or (61 <= x <= 67 and 78 <= y <= 94):
            return False
        return True
    # Shackle: the upper half of a ring centred on (64, 58), radii 16..26.
    if y <= 58:
        d2 = (x - 64) ** 2 + (y - 58) ** 2
        return 256 <= d2 <= 676
    return False


def lock_png(rgb) -> bytes:
    r, g, b = rgb
    ihdr = struct.pack(">IIBBBBB", SIZE, SIZE, 8, 6, 0, 0, 0)
    raw = bytearray()
    for y in range(SIZE):
        raw.append(0)  # filter type 0 per scanline
        for x in range(SIZE):
            raw += bytes((r, g, b, 255 if in_lock(x, y) else 0))
    return (
        b"\x89PNG\r\n\x1a\n"
        + chunk(b"IHDR", ihdr)
        + chunk(b"IDAT", zlib.compress(bytes(raw), 9))
        + chunk(b"IEND", b"")
    )


def main() -> int:
    outdir = sys.argv[1] if len(sys.argv) > 1 else "BeebeebFileProvider/Resources"
    import os

    os.makedirs(outdir, exist_ok=True)
    for name, rgb in BADGES.items():
        path = os.path.join(outdir, f"badge-{name}.png")
        with open(path, "wb") as fh:
            fh.write(circle_png(rgb))
        print(f"wrote {path}")
    for name, rgb in LOCKS.items():
        path = os.path.join(outdir, f"badge-{name}.png")
        with open(path, "wb") as fh:
            fh.write(lock_png(rgb))
        print(f"wrote {path}")
    return 0


if __name__ == "__main__":
    sys.exit(main())