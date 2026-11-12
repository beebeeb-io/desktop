#!/usr/bin/env python3
"""THROWAWAY (task 2477 spike). Writes the four fixtures with the standard library only.

  spike.docx   3 pages  ("PAGE n OF 3", explicit page breaks)
  spike.xlsx   2 sheets ("SHEET n OF 2" plus a small table)
  spike.pptx   3 slides ("SLIDE n OF 3")
  remote.docx  1 page with an image LINKED to http://127.0.0.1:47477/remote.png
               (the spike program listens there and counts the fetch attempts)
Deterministic: fixed timestamps, so a re-run gives the same bytes.
"""
import sys, zipfile, pathlib

OUT = pathlib.Path(sys.argv[1] if len(sys.argv) > 1 else "fixtures")
OUT.mkdir(parents=True, exist_ok=True)
X = '<?xml version="1.0" encoding="UTF-8" standalone="yes"?>\n'
PKG = "http://schemas.openxmlformats.org/package/2006"
OD = "http://schemas.openxmlformats.org/officeDocument/2006"
W = "http://schemas.openxmlformats.org/wordprocessingml/2006/main"
S = "http://schemas.openxmlformats.org/spreadsheetml/2006/main"
P = "http://schemas.openxmlformats.org/presentationml/2006/main"
A = "http://schemas.openxmlformats.org/drawingml/2006/main"
R = OD + "/relationships"
CT = "application/vnd.openxmlformats-officedocument."


def write(name, parts):
    with zipfile.ZipFile(OUT / name, "w", zipfile.ZIP_DEFLATED) as z:
        for path, body in parts:
            info = zipfile.ZipInfo(path, (2026, 11, 12, 0, 0, 0))
            info.compress_type = zipfile.ZIP_DEFLATED
            z.writestr(info, X + body)
    print(name, (OUT / name).stat().st_size, "bytes")


def types(overrides, extra=""):
    o = "".join(f'<Override PartName="{p}" ContentType="{CT}{t}+xml"/>' for p, t in overrides)
    return (f'<Types xmlns="{PKG}/content-types">'
            f'<Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/>'
            f'<Default Extension="xml" ContentType="application/xml"/>{extra}{o}</Types>')


def rels(items):
    r = "".join(f'<Relationship Id="{i}" Type="{R}/{t}" Target="{tg}"{" TargetMode=\"External\"" if ext else ""}/>'
                for i, t, tg, ext in items)
    return f'<Relationships xmlns="{PKG}/relationships">{r}</Relationships>'


def para(text, size=28, bold=False):
    b = "<w:b/>" if bold else ""
    return (f'<w:p><w:r><w:rPr>{b}<w:sz w:val="{size}"/></w:rPr>'
            f'<w:t xml:space="preserve">{text}</w:t></w:r></w:p>')


BREAK = '<w:p><w:r><w:br w:type="page"/></w:r></w:p>'
SECT = ('<w:sectPr><w:pgSz w:w="11906" w:h="16838"/>'
        '<w:pgMar w:top="1440" w:right="1440" w:bottom="1440" w:left="1440" w:header="708" w:footer="708" w:gutter="0"/></w:sectPr>')

# ---- spike.docx: 3 pages
body = ""
for n in (1, 2, 3):
    body += para(f"PAGE {n} OF 3", 72, True)
    body += para("Spike fixture for the native Office viewer (task 2477).")
    for i in range(1, 9):
        body += para(f"Page {n}, line {i}: the quick brown fox jumps over the lazy dog.", 24)
    if n < 3:
        body += BREAK
write("spike.docx", [
    ("[Content_Types].xml", types([("/word/document.xml", "wordprocessingml.document.main")])),
    ("_rels/.rels", rels([("rId1", "officeDocument", "word/document.xml", False)])),
    ("word/document.xml", f'<w:document xmlns:w="{W}"><w:body>{body}{SECT}</w:body></w:document>'),
])

# ---- remote.docx: one page, one LINKED (not embedded) image
WP = "http://schemas.openxmlformats.org/drawingml/2006/wordprocessingDrawing"
PIC = "http://schemas.openxmlformats.org/drawingml/2006/picture"
drawing = (
    f'<w:p><w:r><w:drawing><wp:inline xmlns:wp="{WP}" distT="0" distB="0" distL="0" distR="0">'
    f'<wp:extent cx="1905000" cy="1905000"/><wp:docPr id="1" name="Remote"/>'
    f'<a:graphic xmlns:a="{A}"><a:graphicData uri="{PIC}">'
    f'<pic:pic xmlns:pic="{PIC}"><pic:nvPicPr><pic:cNvPr id="1" name="remote.png"/><pic:cNvPicPr/></pic:nvPicPr>'
    f'<pic:blipFill><a:blip xmlns:r="{R}" r:link="rId9"/><a:stretch><a:fillRect/></a:stretch></pic:blipFill>'
    f'<pic:spPr><a:xfrm><a:off x="0" y="0"/><a:ext cx="1905000" cy="1905000"/></a:xfrm>'
    f'<a:prstGeom prst="rect"><a:avLst/></a:prstGeom></pic:spPr></pic:pic>'
    f'</a:graphicData></a:graphic></wp:inline></w:drawing></w:r></w:p>')
write("remote.docx", [
    ("[Content_Types].xml", types([("/word/document.xml", "wordprocessingml.document.main")])),
    ("_rels/.rels", rels([("rId1", "officeDocument", "word/document.xml", False)])),
    ("word/_rels/document.xml.rels", rels([("rId9", "image", "http://127.0.0.1:47477/remote.png", True)])),
    ("word/document.xml",
     f'<w:document xmlns:w="{W}"><w:body>{para("REMOTE IMAGE BELOW", 56, True)}{drawing}'
     f'{para("If a picture shows above, the library fetched it.")}{SECT}</w:body></w:document>'),
])


# ---- spike.xlsx: 2 sheets
def cell(ref, v):
    if isinstance(v, str):
        return f'<c r="{ref}" t="inlineStr"><is><t>{v}</t></is></c>'
    return f'<c r="{ref}"><v>{v}</v></c>'


def sheet(n):
    rows = [[f"SHEET {n} OF 2", "", ""], ["Item", "Qty", "Price"]]
    rows += [[f"Row {i} on sheet {n}", i * n, round(i * 12.5 * n, 2)] for i in range(1, 13)]
    out = ""
    for ri, row in enumerate(rows, 1):
        out += f'<row r="{ri}">' + "".join(cell("ABC"[ci] + str(ri), v) for ci, v in enumerate(row) if v != "") + "</row>"
    out += '<row r="15"><c r="A15" t="inlineStr"><is><t>Total</t></is></c><c r="C15"><f>SUM(C3:C14)</f></c></row>'
    return (f'<worksheet xmlns="{S}"><cols><col min="1" max="1" width="26" customWidth="1"/></cols>'
            f'<sheetData>{out}</sheetData></worksheet>')


write("spike.xlsx", [
    ("[Content_Types].xml", types([("/xl/workbook.xml", "spreadsheetml.sheet.main"),
                                   ("/xl/worksheets/sheet1.xml", "spreadsheetml.worksheet"),
                                   ("/xl/worksheets/sheet2.xml", "spreadsheetml.worksheet")])),
    ("_rels/.rels", rels([("rId1", "officeDocument", "xl/workbook.xml", False)])),
    ("xl/workbook.xml", f'<workbook xmlns="{S}" xmlns:r="{R}"><sheets>'
                        f'<sheet name="First" sheetId="1" r:id="rId1"/><sheet name="Second" sheetId="2" r:id="rId2"/>'
                        f'</sheets></workbook>'),
    ("xl/_rels/workbook.xml.rels", rels([("rId1", "worksheet", "worksheets/sheet1.xml", False),
                                         ("rId2", "worksheet", "worksheets/sheet2.xml", False)])),
    ("xl/worksheets/sheet1.xml", sheet(1)),
    ("xl/worksheets/sheet2.xml", sheet(2)),
])

# ---- spike.pptx: 3 slides
NS = f'xmlns:a="{A}" xmlns:r="{R}" xmlns:p="{P}"'
TREE0 = ('<p:nvGrpSpPr><p:cNvPr id="1" name=""/><p:cNvGrpSpPr/><p:nvPr/></p:nvGrpSpPr>'
         '<p:grpSpPr><a:xfrm><a:off x="0" y="0"/><a:ext cx="0" cy="0"/><a:chOff x="0" y="0"/><a:chExt cx="0" cy="0"/></a:xfrm></p:grpSpPr>')
CLR = ('<p:clrMap bg1="lt1" tx1="dk1" bg2="lt2" tx2="dk2" accent1="accent1" accent2="accent2" accent3="accent3" '
       'accent4="accent4" accent5="accent5" accent6="accent6" hlink="hlink" folHlink="folHlink"/>')


def box(i, y, h, text, size, bold=False):
    return (f'<p:sp><p:nvSpPr><p:cNvPr id="{i}" name="Box {i}"/><p:cNvSpPr txBox="1"/><p:nvPr/></p:nvSpPr>'
            f'<p:spPr><a:xfrm><a:off x="685800" y="{y}"/><a:ext cx="10820400" cy="{h}"/></a:xfrm>'
            f'<a:prstGeom prst="rect"><a:avLst/></a:prstGeom></p:spPr>'
            f'<p:txBody><a:bodyPr wrap="square"/><a:p><a:r><a:rPr lang="en-GB" sz="{size}" b="{1 if bold else 0}"/>'
            f'<a:t>{text}</a:t></a:r></a:p></p:txBody></p:sp>')


def slide(n):
    fill = ["DDE8F5", "E6F2DD", "F5E6DD"][n - 1]
    bg = f'<p:bg><p:bgPr><a:solidFill><a:srgbClr val="{fill}"/></a:solidFill><a:effectLst/></p:bgPr></p:bg>'
    return (f'<p:sld {NS}><p:cSld>{bg}<p:spTree>{TREE0}'
            f'{box(2, 914400, 1524000, f"SLIDE {n} OF 3", 6000, True)}'
            f'{box(3, 2895600, 1524000, f"Spike fixture, slide {n}: three points and a coloured background.", 2400)}'
            f'</p:spTree></p:cSld><p:clrMapOvr><a:masterClrMapping/></p:clrMapOvr></p:sld>')



def clr(tag, v):
    return f'<a:{tag}><a:srgbClr val="{v}"/></a:{tag}>'


FILL3 = '<a:solidFill><a:schemeClr val="phClr"/></a:solidFill>' * 3
LN3 = '<a:ln w="9525"><a:solidFill><a:schemeClr val="phClr"/></a:solidFill></a:ln>' * 3
EFF3 = '<a:effectStyle><a:effectLst/></a:effectStyle>' * 3
theme = (f'<a:theme xmlns:a="{A}" name="Spike"><a:themeElements><a:clrScheme name="Spike">'
         + clr("dk1", "000000") + clr("lt1", "FFFFFF") + clr("dk2", "1F2937") + clr("lt2", "F3F4F6")
         + "".join(clr(f"accent{i}", v) for i, v in enumerate(["4472C4", "ED7D31", "A5A5A5", "FFC000", "5B9BD5", "70AD47"], 1))
         + clr("hlink", "0563C1") + clr("folHlink", "954F72")
         + f'</a:clrScheme><a:fontScheme name="Spike"><a:majorFont>'
           f'<a:latin typeface="Arial"/><a:ea typeface=""/><a:cs typeface=""/></a:majorFont>'
           f'<a:minorFont><a:latin typeface="Arial"/><a:ea typeface=""/><a:cs typeface=""/></a:minorFont></a:fontScheme>'
           f'<a:fmtScheme name="Spike"><a:fillStyleLst>{FILL3}</a:fillStyleLst><a:lnStyleLst>{LN3}</a:lnStyleLst>'
           f'<a:effectStyleLst>{EFF3}</a:effectStyleLst><a:bgFillStyleLst>{FILL3}</a:bgFillStyleLst></a:fmtScheme>'
           f'</a:themeElements></a:theme>')
master = (f'<p:sldMaster {NS}><p:cSld><p:bg><p:bgRef idx="1001"><a:schemeClr val="bg1"/></p:bgRef></p:bg>'
          f'<p:spTree>{TREE0}</p:spTree></p:cSld>{CLR}'
          f'<p:sldLayoutIdLst><p:sldLayoutId id="2147483649" r:id="rId1"/></p:sldLayoutIdLst></p:sldMaster>')
layout = (f'<p:sldLayout {NS} type="blank"><p:cSld name="Blank"><p:spTree>{TREE0}</p:spTree></p:cSld>'
          f'<p:clrMapOvr><a:masterClrMapping/></p:clrMapOvr></p:sldLayout>')
pres = (f'<p:presentation {NS}><p:sldMasterIdLst><p:sldMasterId id="2147483648" r:id="rId1"/></p:sldMasterIdLst>'
        f'<p:sldIdLst>' + "".join(f'<p:sldId id="{255 + n}" r:id="rId{n + 1}"/>' for n in (1, 2, 3))
        + '</p:sldIdLst><p:sldSz cx="12192000" cy="6858000"/><p:notesSz cx="6858000" cy="9144000"/></p:presentation>')
parts = [
    ("[Content_Types].xml", types([("/ppt/presentation.xml", "presentationml.presentation.main"),
                                   ("/ppt/slideMasters/slideMaster1.xml", "presentationml.slideMaster"),
                                   ("/ppt/slideLayouts/slideLayout1.xml", "presentationml.slideLayout"),
                                   ("/ppt/theme/theme1.xml", "theme")]
                                  + [(f"/ppt/slides/slide{n}.xml", "presentationml.slide") for n in (1, 2, 3)])),
    ("_rels/.rels", rels([("rId1", "officeDocument", "ppt/presentation.xml", False)])),
    ("ppt/presentation.xml", pres),
    ("ppt/_rels/presentation.xml.rels",
     rels([("rId1", "slideMaster", "slideMasters/slideMaster1.xml", False)]
          + [(f"rId{n + 1}", "slide", f"slides/slide{n}.xml", False) for n in (1, 2, 3)]
          + [("rId5", "theme", "theme/theme1.xml", False)])),
    ("ppt/slideMasters/slideMaster1.xml", master),
    ("ppt/slideMasters/_rels/slideMaster1.xml.rels",
     rels([("rId1", "slideLayout", "../slideLayouts/slideLayout1.xml", False), ("rId2", "theme", "../theme/theme1.xml", False)])),
    ("ppt/slideLayouts/slideLayout1.xml", layout),
    ("ppt/slideLayouts/_rels/slideLayout1.xml.rels", rels([("rId1", "slideMaster", "../slideMasters/slideMaster1.xml", False)])),
    ("ppt/theme/theme1.xml", theme),
]
for n in (1, 2, 3):
    parts.append((f"ppt/slides/slide{n}.xml", slide(n)))
    parts.append((f"ppt/slides/_rels/slide{n}.xml.rels", rels([("rId1", "slideLayout", "../slideLayouts/slideLayout1.xml", False)])))
write("spike.pptx", parts)
