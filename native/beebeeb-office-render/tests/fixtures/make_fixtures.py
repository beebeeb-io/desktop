#!/usr/bin/env python3
"""Writes the Office helper's own test documents, with the standard library only.

  letter-1.docx     one US Letter page (612 x 792 pt): a heading and lines in the default face. The page that shows
                    the one page-pixel rule: at 4/3 (100 % on a 96 DPI screen) it is 816 x 1056 px in single precision,
                    817 x 1057 in double. Every spike fixture is A4, where the two agree.
  system-font.docx  one A4 page: a heading in "Segoe Script", a handwriting face Windows installs and the Office engine
                    does not bundle (its bundled fallback is a sans), and one line in the default face. A handwritten
                    heading in the rendered page can only have come from the system's font folder.

Deterministic: fixed timestamps and order, so a re-run gives the same bytes with the same zlib.
Usage: python3 make_fixtures.py [output directory, default: this script's directory]
"""
import pathlib
import sys
import zipfile

OUT = pathlib.Path(sys.argv[1]) if len(sys.argv) > 1 else pathlib.Path(__file__).resolve().parent
OUT.mkdir(parents=True, exist_ok=True)
X = '<?xml version="1.0" encoding="UTF-8" standalone="yes"?>\n'
PKG = "http://schemas.openxmlformats.org/package/2006"
OD = "http://schemas.openxmlformats.org/officeDocument/2006"
W = "http://schemas.openxmlformats.org/wordprocessingml/2006/main"
R = OD + "/relationships"
CT = "application/vnd.openxmlformats-officedocument."
WHEN = (2026, 11, 15, 0, 0, 0)


def write(name, parts):
    with zipfile.ZipFile(OUT / name, "w", zipfile.ZIP_DEFLATED) as z:
        for path, body in parts:
            info = zipfile.ZipInfo(path, WHEN)
            info.compress_type = zipfile.ZIP_DEFLATED
            info.external_attr = 0o644 << 16
            z.writestr(info, X + body)
    print(name, (OUT / name).stat().st_size, "bytes")


def package(body, sect):
    return [
        ("[Content_Types].xml",
         f'<Types xmlns="{PKG}/content-types">'
         f'<Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/>'
         f'<Default Extension="xml" ContentType="application/xml"/>'
         f'<Override PartName="/word/document.xml" ContentType="{CT}wordprocessingml.document.main+xml"/></Types>'),
        ("_rels/.rels",
         f'<Relationships xmlns="{PKG}/relationships">'
         f'<Relationship Id="rId1" Type="{R}/officeDocument" Target="word/document.xml"/></Relationships>'),
        ("word/document.xml", f'<w:document xmlns:w="{W}"><w:body>{body}{sect}</w:body></w:document>'),
    ]


def para(text, size=24, bold=False, face=None):
    fonts = f'<w:rFonts w:ascii="{face}" w:hAnsi="{face}" w:cs="{face}"/>' if face else ""
    b = "<w:b/>" if bold else ""
    return (f'<w:p><w:r><w:rPr>{fonts}{b}<w:sz w:val="{size}"/></w:rPr>'
            f'<w:t xml:space="preserve">{text}</w:t></w:r></w:p>')


MARGINS = '<w:pgMar w:top="1440" w:right="1440" w:bottom="1440" w:left="1440" w:header="708" w:footer="708" w:gutter="0"/>'
# Twips: 1/20 pt. Letter 8.5 x 11 in = 12240 x 15840 (612 x 792 pt); A4 = 11906 x 16838 (595.3 x 841.9 pt).
LETTER = f'<w:sectPr><w:pgSz w:w="12240" w:h="15840"/>{MARGINS}</w:sectPr>'
A4 = f'<w:sectPr><w:pgSz w:w="11906" w:h="16838"/>{MARGINS}</w:sectPr>'

# ---- letter-1.docx: one Letter page
body = para("ONE LETTER PAGE", 72, True)
body += para("A US Letter page for the Office helper's page-pixel test.")
for i in range(1, 11):
    body += para(f"Line {i}: the quick brown fox jumps over the lazy dog.")
write("letter-1.docx", package(body, LETTER))

# ---- system-font.docx: one A4 page, a heading in a face only the system folder has
body = para("Handwritten heading", 56, False, "Segoe Script")
body += para("This line is in the default face.")
write("system-font.docx", package(body, A4))
