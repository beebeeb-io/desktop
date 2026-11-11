#!/usr/bin/env python3
"""Pixel-compare a golden board PNG against a Rune headless render.

Usage: compare.py <golden.png> <rune.png> <output-dir> <frame-id>

Writes to output-dir:
  <frame-id>-side-by-side.png
  <frame-id>-diff.png
  <frame-id>-compare.md  (summary with per-region breakdown)

Exit code: 0 if overall diff% <= threshold, 1 otherwise.

Text antialiasing tolerance: Chromium sub-pixel and Rune's swash rasteriser
produce different sub-pixel coverage. A per-channel tolerance of 32 (out of
255) catches layout/colour differences without being dominated by text edges.
"""
import sys
from pathlib import Path
from PIL import Image, ImageDraw, ImageFont

PIXEL_TOLERANCE = 32  # per-channel absolute difference threshold
THRESHOLD_PCT = 15.0  # overall pass/fail threshold

# DW-0401 regions at 2x scale (2560x1600 for 1280x800 at scale 2)
REGIONS_DW0401 = {
    "caption":      (0,    0,    2560, 64),
    "sidebar":      (0,    64,   420,  1544),
    "toolbar":      (420,  64,   2560, 144),
    "list_header":  (420,  144,  2560, 208),
    "rows":         (420,  208,  1960, 1544),
    "detail_pane":  (1960, 208,  2560, 1544),
    "footer":       (0,    1544, 2560, 1600),
}


def diff_pixels(img_a: Image.Image, img_b: Image.Image, tolerance: int) -> int:
    """Count pixels where any channel differs by more than tolerance."""
    assert img_a.size == img_b.size, f"Size mismatch: {img_a.size} vs {img_b.size}"
    w, h = img_a.size
    pa = img_a.load()
    pb = img_b.load()
    count = 0
    for y in range(h):
        for x in range(w):
            a = pa[x, y]
            b = pb[x, y]
            # Compare RGB channels (ignore alpha if present)
            for c in range(min(len(a), len(b), 3)):
                if abs(a[c] - b[c]) > tolerance:
                    count += 1
                    break
    return count


def make_diff_image(img_a: Image.Image, img_b: Image.Image, tolerance: int) -> Image.Image:
    """Create a difference visualisation: red where pixels differ."""
    assert img_a.size == img_b.size
    w, h = img_a.size
    diff = Image.new("RGB", (w, h), (0, 0, 0))
    pd = diff.load()
    pa = img_a.load()
    pb = img_b.load()
    for y in range(h):
        for x in range(w):
            a = pa[x, y]
            b = pb[x, y]
            differs = False
            for c in range(min(len(a), len(b), 3)):
                if abs(a[c] - b[c]) > tolerance:
                    differs = True
                    break
            if differs:
                pd[x, y] = (255, 0, 0)
            else:
                # Dim version of the golden
                r, g, b_ch = a[0], a[1], a[2]
                pd[x, y] = (r // 3, g // 3, b_ch // 3)
    return diff


def region_diff(img_a: Image.Image, img_b: Image.Image, box: tuple, tolerance: int) -> tuple:
    """Return (diff_pixels, total_pixels) for a region."""
    ra = img_a.crop(box)
    rb = img_b.crop(box)
    if ra.size != rb.size:
        rb = rb.resize(ra.size, Image.LANCZOS)
    total = ra.size[0] * ra.size[1]
    diff = diff_pixels(ra, rb, tolerance)
    return diff, total


def main():
    if len(sys.argv) < 5:
        print("Usage: compare.py <golden.png> <rune.png> <output-dir> <frame-id>")
        sys.exit(2)

    golden_path = Path(sys.argv[1])
    rune_path = Path(sys.argv[2])
    out_dir = Path(sys.argv[3])
    frame_id = sys.argv[4]

    out_dir.mkdir(parents=True, exist_ok=True)

    golden = Image.open(golden_path).convert("RGB")
    rune = Image.open(rune_path).convert("RGB")

    print(f"  Golden: {golden.size[0]}x{golden.size[1]}")
    print(f"  Rune:   {rune.size[0]}x{rune.size[1]}")

    # Resize rune to match golden if needed
    if golden.size != rune.size:
        print(f"  Resizing Rune from {rune.size} to {golden.size}...")
        rune = rune.resize(golden.size, Image.LANCZOS)

    # Overall comparison
    total = golden.size[0] * golden.size[1]
    diff_count = diff_pixels(golden, rune, PIXEL_TOLERANCE)
    diff_pct = 100.0 * diff_count / max(total, 1)

    print(f"  Differing pixels (tolerance={PIXEL_TOLERANCE}): {diff_count} of {total} ({diff_pct:.2f}%)")

    # Side-by-side
    sbs = Image.new("RGB", (golden.size[0] * 2 + 4, golden.size[1]), (128, 128, 128))
    sbs.paste(golden, (0, 0))
    sbs.paste(rune, (golden.size[0] + 4, 0))
    sbs.save(out_dir / f"{frame_id}-side-by-side.png")
    print("  Saved side-by-side")

    # Diff image
    diff_img = make_diff_image(golden, rune, PIXEL_TOLERANCE)
    diff_img.save(out_dir / f"{frame_id}-diff.png")
    print("  Saved diff image")

    # Per-region breakdown
    regions = REGIONS_DW0401 if frame_id == "DW-0401" else {}
    region_results = {}
    for name, box in regions.items():
        # Clamp to image bounds
        x1 = min(box[0], golden.size[0])
        y1 = min(box[1], golden.size[1])
        x2 = min(box[2], golden.size[0])
        y2 = min(box[3], golden.size[1])
        if x2 <= x1 or y2 <= y1:
            continue
        rd, rt = region_diff(golden, rune, (x1, y1, x2, y2), PIXEL_TOLERANCE)
        rpct = 100.0 * rd / max(rt, 1)
        region_results[name] = (rd, rt, rpct)

    # Write summary
    summary = out_dir / f"{frame_id}-compare.md"
    with open(summary, "w") as f:
        f.write(f"# {frame_id} comparison\n\n")
        f.write("| Metric | Value |\n|---|---|\n")
        f.write(f"| Golden | {golden.size[0]}x{golden.size[1]} |\n")
        f.write(f"| Rune | {rune.size[0]}x{rune.size[1]} |\n")
        f.write(f"| Pixel tolerance | {PIXEL_TOLERANCE} |\n")
        f.write(f"| Differing pixels | {diff_count} of {total} |\n")
        f.write(f"| **Overall diff %** | **{diff_pct:.2f}%** |\n")
        f.write(f"| Threshold | {THRESHOLD_PCT}% |\n\n")

        if region_results:
            f.write("## Per-region breakdown\n\n")
            f.write("| Region | Diff pixels | Total | Diff % |\n|---|---|---|---|\n")
            for name in ["caption", "sidebar", "toolbar", "list_header", "rows", "detail_pane", "footer"]:
                if name in region_results:
                    rd, rt, rpct = region_results[name]
                    f.write(f"| {name} | {rd} | {rt} | {rpct:.2f}% |\n")
            f.write("\n")

    with open(summary) as f:
        print(f.read())

    # Pass/fail
    result = "PASS" if diff_pct <= THRESHOLD_PCT else "FAIL"
    print(f"Result: {result} ({diff_pct:.2f}% vs threshold {THRESHOLD_PCT}%)")

    if result == "FAIL":
        sys.exit(1)


if __name__ == "__main__":
    main()
