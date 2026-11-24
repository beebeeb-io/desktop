#!/usr/bin/env python3
"""THROWAWAY (task 2477 editor spike). Did every cell and every slide text survive an unedited save?

  celldiff.py <dir with originals> <dir with saved files>

Workbooks: every cell of every sheet (by sheet name), its resolved value (shared string, inline string or number)
and its formula, original against `name.resaved.ext`. Decks: the text of every slide, in order. Word: the body text.
Standard library only. `--mutant` drops one cell from the saved side first, to show the comparison can fail.
"""
import html, re, sys, zipfile
from pathlib import Path


def parts(p):
    with zipfile.ZipFile(p) as z:
        return {i.filename: z.read(i).decode("utf-8", "replace") for i in z.infolist() if i.filename.endswith((".xml", ".rels"))}


def txt(x):
    return html.unescape("".join(re.findall(r"<(?:\w+:)?t(?: [^>]*)?>([^<]*)</", x)))


def workbook(p):
    z = parts(p)
    shared = [txt(si) for si in re.findall(r"<si>(.*?)</si>", z.get("xl/sharedStrings.xml", ""), re.S)]
    rels = dict(re.findall(r'<Relationship [^>]*?Id="([^"]+)"[^>]*?Target="([^"]+)"', z.get("xl/_rels/workbook.xml.rels", "")))
    rels.update({b: a for a, b in re.findall(r'<Relationship [^>]*?Target="([^"]+)"[^>]*?Id="([^"]+)"', z.get("xl/_rels/workbook.xml.rels", ""))})
    cells = {}
    for name, rid in re.findall(r'<sheet [^>]*?name="([^"]+)"[^>]*?r:id="([^"]+)"', z["xl/workbook.xml"]):
        target = rels[rid].lstrip("/")
        x = z[target if target.startswith("xl/") else "xl/" + target]
        for attrs, body in re.findall(r"<c ([^>]*?)(?:/>|>(.*?)</c>)", x, re.S):
            ref = re.search(r'r="([A-Z]+\d+)"', attrs).group(1)
            t = (re.search(r'\bt="(\w+)"', attrs) or [None, "n"])[1]
            v = re.search(r"<v>([^<]*)</v>", body or "")
            f = re.search(r"<f[^>]*>([^<]*)</f>", body or "")
            if t == "s" and v:
                val = shared[int(v.group(1))]
            elif t == "inlineStr":
                val = txt(body or "")
            else:
                val = html.unescape(v.group(1)) if v else ""
                try:
                    val = repr(round(float(val), 9))
                except ValueError:
                    pass
            if val != "" or f:
                cells[(html.unescape(name), ref)] = (val, html.unescape(f.group(1)) if f else None)
    return cells


def deck(p):
    z = parts(p)
    n = lambda k: int(re.search(r"(\d+)\.xml$", k).group(1))
    return [sorted(re.findall(r"<a:t>([^<]*)</a:t>", z[k])) for k in sorted((k for k in z if re.match(r"ppt/slides/slide\d+\.xml$", k)), key=n)]


def word(p):
    return txt(parts(p)["word/document.xml"])


def main():
    src, out = Path(sys.argv[1]), Path(sys.argv[2])
    mutant = "--mutant" in sys.argv
    tot = bad = 0
    for o in sorted(src.iterdir()):
        s = out / f"{o.stem}.resaved{o.suffix}"
        if not s.exists():
            continue
        kind = o.suffix[1:4]
        if kind == "xls":
            a, b = workbook(o), workbook(s)
            if mutant and b:
                b.pop(sorted(b)[0])
            lost = sorted(k for k in a if k not in b); new = sorted(k for k in b if k not in a)
            fx = lambda f: (f or "").replace("_xlfn.", "")
            # a formula the original stores WITHOUT a cached result, and the save stores with the computed one, is not a loss
            filled = sorted(k for k in a if k in b and a[k] != b[k] and a[k][0] == "" and fx(a[k][1]) == fx(b[k][1]) and a[k][1])
            diff = sorted(k for k in a if k in b and a[k] != b[k] and k not in filled)
            ok = not lost and not diff
            print(f"{o.name}: cells {len(a)} -> {len(b)}; lost {len(lost)}; value or formula changed {len(diff)}; formula results filled in {len(filled)}; new {len(new)}: {'SAME' if ok else 'DIFFERENT'}")
            for k in (lost + diff)[:6]:
                print(f"    {k[0]}!{k[1]}: {a[k]} -> {b.get(k)}")
        elif kind == "ppt":
            a, b = deck(o), deck(s)
            ok = a == b
            print(f"{o.name}: slides {len(a)} -> {len(b)}; text runs {sum(map(len, a))} -> {sum(map(len, b))}: {'SAME' if ok else 'DIFFERENT'}")
            if not ok:
                for i, (x, y) in enumerate(zip(a, b)):
                    if x != y:
                        print(f"    slide {i + 1}: only in the original {[t for t in x if t not in y][:5]}; only in the save {[t for t in y if t not in x][:5]}")
        else:
            a, b = word(o), word(s)
            ok = a == b
            print(f"{o.name}: body characters {len(a)} -> {len(b)}: {'SAME' if ok else 'DIFFERENT'}")
        tot += 1; bad += not ok
    print(f"files compared: {tot}; different: {bad}")


if __name__ == "__main__":
    main()
