#!/usr/bin/env python3
"""golden-from-native.py <native-export.json> <golden.json>

A golden for a frame the BOARD DOES NOT DRAW (a design of this app's, ruled by the product owner): the app's
own element export (`--gate-export`), written in the board golden's shape so `scripts/gate.py` reads it.
Its picture is the app's own render of the same scene (`--snap`), saved as `<stem>-board.png`.

What such a golden is: a record of the approved render. It cannot say the app matches a design (it IS the
app); it fails when the frame changes afterwards, which is what it is for. Never use it for a frame the
board draws: that golden comes from the board (`capture-golden.mjs`).

Only what is wholly inside the window is kept (the board's goldens hold what a frame shows): an element
under the fold or with no size has no pixels to compare.
"""
import json
import sys


def main(src, dst):
    with open(src, encoding="utf-8") as f:
        native = json.load(f)
    width, height = int(native["width"]), int(native["height"])
    elements = {}
    for key, e in native["elements"].items():
        if e["w"] <= 0 or e["h"] <= 0:
            continue
        if e["x"] < 0 or e["y"] < 0 or e["x"] + e["w"] > width or e["y"] + e["h"] > height:
            continue
        out = {k: e[k] for k in ("x", "y", "w", "h")}
        if "text" in e:
            out["kind"] = "text"
            for k in ("text", "family", "size", "weight", "color"):
                out[k] = e[k]
            out["tracking"] = e.get("tracking") or 0.0
        elif e.get("icon"):
            out["kind"] = "icon"
            out["icon"] = e["icon"]
            out["filled"] = bool(e.get("filled"))
            if "color" in e:
                out["color"] = e["color"]
            else:
                # Drawn in its own colours (the brand mark): which glyph it is, is its path data.
                out["mark"] = True
        else:
            out["kind"] = "box"
            if "fill" in e:
                out["bg"] = e["fill"]
        elements[key] = out
    golden = {
        "frame": native["frame"],
        "width": width,
        "height": height,
        "theme": native["theme"],
        "source": "the app's own render (no board frame): scripts/golden-from-native.py",
        "elements": elements,
    }
    with open(dst, "w", encoding="utf-8") as f:
        json.dump(golden, f, indent=1, sort_keys=True)
        f.write("\n")
    print(
        f"golden-from-native: {native['frame']} ({native['theme']}): "
        f"{len(elements)} of {len(native['elements'])} elements -> {dst}"
    )


if __name__ == "__main__":
    if len(sys.argv) != 3:
        sys.exit(__doc__)
    main(sys.argv[1], sys.argv[2])
