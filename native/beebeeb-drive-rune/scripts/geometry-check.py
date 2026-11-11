#!/usr/bin/env python3
"""Geometry check: compare board element positions against Rune's.

Usage: geometry-check.py <board-geometry.json> <rune-geometry.json>

Matches elements by key, reports mismatches sorted by worst offender.
Exit code: 0 if all pass, 1 if any fail.
"""
import json
import math
import sys

POS_TOLERANCE = 2.0    # px tolerance for x, y, w, h
FONT_SIZE_TOL = 1.0    # px tolerance for font size
COLOUR_DE_TOL = 0.15   # rgb ΔE tolerance

def parse_rgb(s):
    """Parse 'rgb(r, g, b)' to normalised (r, g, b)."""
    if not s or not s.startswith('rgb('):
        return None
    parts = s[4:-1].split(',')
    return tuple(float(p.strip()) / 255.0 for p in parts[:3])

def colour_de(a, b):
    if a is None or b is None:
        return 0.0
    return math.sqrt(sum((x - y) ** 2 for x, y in zip(a, b)))

def main():
    if len(sys.argv) < 3:
        print("Usage: geometry-check.py <board.json> <rune.json>")
        sys.exit(2)

    with open(sys.argv[1]) as f:
        board_data = json.load(f)
    with open(sys.argv[2]) as f:
        rune_data = json.load(f)

    board_els = board_data.get('elements', board_data)
    rune_els = rune_data.get('elements', rune_data)

    # Only check board elements with non-zero size
    check_keys = {k for k, v in board_els.items() if v.get('w', 0) > 0 and v.get('h', 0) > 0}

    failures = []
    checked = 0

    # Missing in Rune
    for k in sorted(check_keys - set(rune_els.keys())):
        b = board_els[k]
        failures.append({
            'key': k, 'type': 'MISSING',
            'detail': f"board ({b['x']:.0f},{b['y']:.0f} {b['w']:.0f}x{b['h']:.0f})",
            'severity': 100,
        })

    # Compare matching keys
    for k in sorted(check_keys & set(rune_els.keys())):
        b, r = board_els[k], rune_els[k]
        checked += 1
        issues = []

        dx = abs(b.get('x',0) - r.get('x',0))
        dy = abs(b.get('y',0) - r.get('y',0))
        dw = abs(b.get('w',0) - r.get('w',0))
        dh = abs(b.get('h',0) - r.get('h',0))
        max_d = max(dx, dy, dw, dh)
        if max_d > POS_TOLERANCE:
            issues.append(f"pos Δx={dx:.1f} Δy={dy:.1f} Δw={dw:.1f} Δh={dh:.1f}")

        bf, rf = b.get('fontFamily',''), r.get('fontFamily','')
        if bf and rf and bf != rf:
            issues.append(f"font {bf} vs {rf}")

        bs, rs = b.get('fontSize',0), r.get('fontSize',0)
        if bs and rs and abs(bs - rs) > FONT_SIZE_TOL:
            issues.append(f"size {bs} vs {rs}")

        bw, rw = b.get('fontWeight',0), r.get('fontWeight',0)
        if bw and rw and bw != rw:
            issues.append(f"weight {bw} vs {rw}")

        bc, rc = parse_rgb(b.get('color','')), parse_rgb(r.get('color',''))
        if bc and rc and colour_de(bc, rc) > COLOUR_DE_TOL:
            issues.append(f"color ΔE={colour_de(bc,rc):.3f}")

        if issues:
            failures.append({
                'key': k, 'type': 'MISMATCH',
                'detail': '; '.join(issues), 'severity': max_d,
            })

    failures.sort(key=lambda f: -f['severity'])

    total = len(check_keys)
    failing = len(failures)

    print(f"{'Key':<35} {'Type':<10} {'Detail'}")
    print('-' * 100)
    for f in failures[:60]:
        print(f"{f['key']:<35} {f['type']:<10} {f['detail']}")
    if len(failures) > 60:
        print(f"  ... and {len(failures)-60} more")

    print()
    print(f"geometry: {total} checked, {failing} failing")
    sys.exit(1 if failing > 0 else 0)

if __name__ == '__main__':
    main()
