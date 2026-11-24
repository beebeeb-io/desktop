#!/usr/bin/env python3
"""THROWAWAY (task 2477 editor spike). Compares OOXML packages: an original with what an engine saved.

  pkgdiff.py <dir with originals> <dir with saved files> <producer label> [--json out.json]

For every original `name.ext` it looks for `name.resaved.ext` (opened and saved, no edit), `name.edited.ext`
(three edits, then saved) and `name.engine-edited.ext`. Per pair it reports:
  * package parts lost / added / changed (bytes differ) / identical;
  * per FEATURE the amount in the original and in the saved file: preserved, reduced or dropped.
A feature only counts for a file when the ORIGINAL has it (amount > 0). Standard library only.
"""
import hashlib, json, re, sys, zipfile
from pathlib import Path

MARKERS = ["COMMENT-1 body", "COMMENT-2 body", "INSERTED-TRACKED", "DELETED-TRACKED", "CELL-COMMENT", "SPEAKER-NOTES-ONE",
           "SLIDE-COMMENT-1", "CHART-TITLE-MARKER"]


def load(path):
    with zipfile.ZipFile(path) as z:
        return {i.filename: z.read(i) for i in z.infolist() if not i.is_dir()}


def n(parts, name_re, pattern=None):
    """Parts whose name matches, or occurrences of `pattern` inside them."""
    hits = [v for k, v in parts.items() if re.search(name_re, k)]
    return len(hits) if pattern is None else sum(len(re.findall(pattern, v)) for v in hits)


def text_chars(parts):
    total = 0
    for k, v in parts.items():
        if k.endswith(".xml") and not k.startswith("docProps/"):
            for m in re.finditer(rb"<(?:w:t|a:t|t|w:delText)(?: [^>]*)?>([^<]*)</", v):
                total += len(m.group(1).decode("utf-8", "replace"))
    return total


FEATURES = {
    # all three
    "text characters": text_chars,
    "macro project (vbaProject.bin)": lambda p: n(p, r"vbaProject\.bin$"),
    "embedded media and images (parts under media/)": lambda p: n(p, r"/media/"),
    "embedded objects (parts under embeddings/)": lambda p: n(p, r"/embeddings/"),
    "chart parts": lambda p: n(p, r"/charts/chart[^/]*\.xml$"),
    "custom XML parts": lambda p: n(p, r"^customXml/item\d+\.xml$"),
    "hyperlinks": lambda p: n(p, r"(document|sheet\d+|slide\d+)\.xml$", rb"<(?:w:hyperlink|hyperlink|a:hlinkClick)[ >/]"),
    # word
    "comments (word)": lambda p: n(p, r"^word/comments[^/]*\.xml$", rb"<w:comment "),
    "comment anchors (word)": lambda p: n(p, r"^word/document\.xml$", rb"<w:commentReference "),
    "tracked insertions": lambda p: n(p, r"^word/document\.xml$", rb"<w:ins "),
    "tracked deletions": lambda p: n(p, r"^word/document\.xml$", rb"<w:del "),
    "tables (word)": lambda p: n(p, r"^word/document\.xml$", rb"<w:tbl>"),
    "drawings and pictures (word)": lambda p: n(p, r"^word/document\.xml$", rb"<w:drawing>|<w:pict>"),
    "headers and footers (parts)": lambda p: n(p, r"^word/(header|footer)\d*\.xml$"),
    "footnotes and endnotes": lambda p: n(p, r"^word/document\.xml$", rb"<w:(?:footnote|endnote)Reference "),
    "fields (word)": lambda p: n(p, r"^word/document\.xml$", rb"<w:fldSimple |<w:instrText"),
    "content controls (word)": lambda p: n(p, r"^word/document\.xml$", rb"<w:sdt>"),
    "numbering definitions": lambda p: n(p, r"^word/numbering\.xml$", rb"<w:num "),
    "styles (word)": lambda p: n(p, r"^word/styles\.xml$", rb"<w:style "),
    # sheets
    "formulas": lambda p: n(p, r"^xl/worksheets/sheet\d+\.xml$", rb"<f[ >]"),
    "cell comments": lambda p: n(p, r"^xl/comments\d*\.xml$", rb"<comment "),
    "conditional formatting": lambda p: n(p, r"^xl/worksheets/sheet\d+\.xml$", rb"<conditionalFormatting "),
    "data validation": lambda p: n(p, r"^xl/worksheets/sheet\d+\.xml$", rb"<dataValidation "),
    "pivot tables (parts)": lambda p: n(p, r"^xl/pivotTables/[^/]+\.xml$"),
    "pivot caches (parts)": lambda p: n(p, r"^xl/pivotCache/pivotCacheDefinition\d+\.xml$"),
    "merged cells": lambda p: n(p, r"^xl/worksheets/sheet\d+\.xml$", rb"<mergeCell "),
    "defined names": lambda p: n(p, r"^xl/workbook\.xml$", rb"<definedName "),
    "tables (sheet, parts)": lambda p: n(p, r"^xl/tables/[^/]+\.xml$"),
    "sheets": lambda p: n(p, r"^xl/workbook\.xml$", rb"<sheet "),
    "frozen panes": lambda p: n(p, r"^xl/worksheets/sheet\d+\.xml$", rb"<pane "),
    "number formats": lambda p: n(p, r"^xl/styles\.xml$", rb"<numFmt "),
    # slides
    "slides": lambda p: n(p, r"^ppt/slides/slide\d+\.xml$"),
    "slide layouts (parts)": lambda p: n(p, r"^ppt/slideLayouts/slideLayout\d+\.xml$"),
    "slide masters (parts)": lambda p: n(p, r"^ppt/slideMasters/slideMaster\d+\.xml$"),
    "speaker notes (parts)": lambda p: n(p, r"^ppt/notesSlides/notesSlide\d+\.xml$"),
    "slide comments (parts)": lambda p: n(p, r"^ppt/comments/[^/]+\.xml$"),
    "shapes": lambda p: n(p, r"^ppt/slides/slide\d+\.xml$", rb"<p:(?:sp|cxnSp)>"),
    "pictures (slides)": lambda p: n(p, r"^ppt/slides/slide\d+\.xml$", rb"<p:pic>"),
    "graphic frames (tables, charts)": lambda p: n(p, r"^ppt/slides/slide\d+\.xml$", rb"<p:graphicFrame>"),
    "transitions": lambda p: n(p, r"^ppt/slides/slide\d+\.xml$", rb"<p:transition[ >/]"),
    "animations": lambda p: n(p, r"^ppt/slides/slide\d+\.xml$", rb"<p:timing>"),
}
for mk in MARKERS:
    FEATURES[f"marker text {mk!r}"] = (lambda p, mk=mk: sum(v.count(mk.encode()) for k, v in p.items() if k.endswith((".xml", ".vml"))))


def compare(orig, saved):
    a, b = load(orig), load(saved)
    lost = sorted(set(a) - set(b)); added = sorted(set(b) - set(a))
    changed = sorted(k for k in set(a) & set(b) if a[k] != b[k])
    feats = {}
    for name, f in FEATURES.items():
        x = f(a)
        if x > 0:
            y = f(b)
            feats[name] = {"original": x, "saved": y, "result": "preserved" if y >= x else ("dropped" if y == 0 else "reduced")}
    same_bytes = {}
    for k in a:
        if re.search(r"vbaProject\.bin$|/media/|/embeddings/", k):
            sha = hashlib.sha256(a[k]).hexdigest()
            same_bytes[k] = any(hashlib.sha256(v).hexdigest() == sha for v in b.values())
    return {"parts_original": len(a), "parts_saved": len(b), "lost": lost, "added": added, "changed": len(changed),
            "identical": len(set(a) & set(b)) - len(changed), "features": feats, "binary_parts_with_the_same_bytes": same_bytes}


def main():
    src, out, producer = Path(sys.argv[1]), Path(sys.argv[2]), sys.argv[3]
    rows = []
    for o in sorted(src.iterdir()):
        if o.suffix.lower() not in (".docx", ".docm", ".xlsx", ".xlsm", ".pptx", ".pptm"):
            continue
        for kind in ("resaved", "edited", "engine-edited"):
            s = out / f"{o.stem}.{kind}{o.suffix}"
            if not s.exists():
                if kind != "engine-edited":
                    rows.append({"file": o.name, "producer": producer, "save": kind, "error": "NO SAVED FILE"})
                continue
            try:
                rows.append({"file": o.name, "producer": producer, "save": kind, **compare(o, s)})
            except Exception as e:  # a saved file that is not a readable package is a finding, not a crash
                rows.append({"file": o.name, "producer": producer, "save": kind, "error": f"{type(e).__name__}: {e}"})
    if "--json" in sys.argv:
        Path(sys.argv[sys.argv.index("--json") + 1]).write_text(json.dumps(rows, indent=1))
    print(f"## {producer}: {len(rows)} comparisons\n")
    print("| file | save | parts orig → saved | lost | added | changed | identical | features preserved / reduced / dropped |")
    print("|---|---|---|---|---|---|---|---|")
    for r in rows:
        if "error" in r:
            print(f"| {r['file']} | {r['save']} | {r['error']} | | | | | |"); continue
        c = [sum(1 for f in r["features"].values() if f["result"] == k) for k in ("preserved", "reduced", "dropped")]
        print(f"| {r['file']} | {r['save']} | {r['parts_original']} → {r['parts_saved']} | {len(r['lost'])} | {len(r['added'])} | {r['changed']} | {r['identical']} | {c[0]} / {c[1]} / {c[2]} |")
    print("\n### Not preserved (every row where a feature of the original is reduced or dropped)\n")
    print("| file | save | feature | original | saved | result |"); print("|---|---|---|---|---|---|")
    bad = 0
    for r in rows:
        for name, f in r.get("features", {}).items():
            if f["result"] != "preserved":
                bad += 1; print(f"| {r['file']} | {r['save']} | {name} | {f['original']} | {f['saved']} | {f['result']} |")
        for k, same in r.get("binary_parts_with_the_same_bytes", {}).items():
            if not same:
                bad += 1; print(f"| {r['file']} | {r['save']} | bytes of `{k}` | present | no part with the same bytes | dropped or rewritten |")
    print(f"\nrows not preserved: {bad}\n")
    print("### Per feature (files whose original has it; preserved / reduced / dropped), the unedited save\n")
    print("| feature | files | preserved | reduced | dropped |"); print("|---|---|---|---|---|")
    agg = {}
    for r in rows:
        if r["save"] == "resaved":
            for name, f in r.get("features", {}).items():
                agg.setdefault(name, {"preserved": 0, "reduced": 0, "dropped": 0})[f["result"]] += 1
    for name in FEATURES:
        if name in agg:
            a = agg[name]; print(f"| {name} | {sum(a.values())} | {a['preserved']} | {a['reduced']} | {a['dropped']} |")
    print("\n### Parts lost, by name\n")
    for r in rows:
        if r.get("lost"):
            print(f"- {r['file']} ({r['save']}): {', '.join(r['lost'])}")


if __name__ == "__main__":
    main()
