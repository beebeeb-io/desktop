#!/usr/bin/env python3
"""THROWAWAY (task 2477 editor spike). Is the bold edit stored in the saved file itself?
  boldcheck.py <dir with saved files>      Word: the run that holds the inserted marker has <w:b/>.
                                           Excel: cell J1's style points at a bold font."""
import re, sys, zipfile
from pathlib import Path
ok = {"word below the engine": [0, 0], "word through the engine": [0, 0], "excel": [0, 0]}
for p in sorted(Path(sys.argv[1]).rglob("*")):
    kind = ("word below the engine", b"BBEDIT-INSERT") if re.search(r"\.edited\.doc[xm]$", p.name) else \
           ("word through the engine", b"BBEDIT-ENGINE") if re.search(r"\.engine-edited\.doc[xm]$", p.name) else \
           ("excel", None) if re.search(r"\.edited\.xls[xm]$", p.name) else None
    if not kind:
        continue
    z = zipfile.ZipFile(p)
    if kind[1]:
        m = re.search(rb"<w:r>(?:(?!</w:r>).)*" + kind[1], z.read("word/document.xml"), re.S)
        good = bool(m and re.search(rb"<w:b[ /]", m.group(0)))
    else:
        st = z.read("xl/styles.xml"); sheet = z.read("xl/worksheets/sheet1.xml")
        c = re.search(rb'<c r="J1"([^>]*)>', sheet); s = re.search(rb's="(\d+)"', c.group(1)) if c else None
        xf = re.findall(rb"<xf [^>]*?/?>", re.search(rb"<cellXfs[^>]*>(.*?)</cellXfs>", st, re.S).group(1))
        fonts = re.findall(rb"<font>.*?</font>|<font/>", re.search(rb"<fonts[^>]*>(.*?)</fonts>", st, re.S).group(1), re.S)
        good = bool(s) and bool(re.search(rb"<b[ /]", fonts[int(re.search(rb'fontId="(\d+)"', xf[int(s.group(1))]).group(1))]))
    ok[kind[0]][0] += good; ok[kind[0]][1] += 1
    if not good:
        print("NOT bold:", p.name)
for k, (a, b) in ok.items():
    print(f"{k}: bold stored in {a} of {b} saved files")
