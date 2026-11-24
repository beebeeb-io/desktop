#!/usr/bin/env python3
"""Writes the Office helper's own test documents, with the standard library only.

  letter-1.docx     one US Letter page (612 x 792 pt): a heading and lines in the default face. The page that shows
                    the one page-pixel rule: at 4/3 (100 % on a 96 DPI screen) it is 816 x 1056 px in single precision,
                    817 x 1057 in double. Every spike fixture is A4, where the two agree.
  system-font.docx  one A4 page: a heading in "Segoe Script", a handwriting face Windows installs and the Office engine
                    does not bundle (its bundled fallback is a sans), and one line in the default face. A handwritten
                    heading in the rendered page can only have come from the system's font folder.
  sheets-3.xlsx     three sheets: "Budget" (bold headers, amounts in #,##0.00, a share in 0.0 %, a SUM with its cached
                    value, the first row frozen, one 8 x 8 picture of 16 colours over D2:F7), "Notes" (text, a merge
                    A3:C3) and "Empty".
  deck-3.pptx       three 16:9 slides (960 x 540 pt) titled "Q3 plan", (no title), "Thanks", each with a filled box.
  latency/grid-2000.xlsx  one sheet "Grid": 2,000 rows x 20 columns (a label, then amounts, shares and counts in
                    three number formats) and one column chart over the first twelve amounts.

Deterministic: fixed timestamps and order, so a re-run gives the same bytes with the same zlib.
Usage: python3 make_fixtures.py [output directory, default: this script's directory]
"""
import pathlib
import struct
import sys
import zipfile
import zlib

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
    """Writes a zip package: each part is (path, XML text without its declaration) or (path, bytes) for a binary."""
    (OUT / name).parent.mkdir(parents=True, exist_ok=True)
    with zipfile.ZipFile(OUT / name, "w", zipfile.ZIP_DEFLATED) as z:
        for path, body in parts:
            info = zipfile.ZipInfo(path, WHEN)
            info.compress_type = zipfile.ZIP_DEFLATED
            info.external_attr = 0o644 << 16
            z.writestr(info, body if isinstance(body, bytes) else X + body)
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


# ---- the spreadsheet and presentation parts (SpreadsheetML, DrawingML, PresentationML)
S = "http://schemas.openxmlformats.org/spreadsheetml/2006/main"
A = "http://schemas.openxmlformats.org/drawingml/2006/main"
XDR = "http://schemas.openxmlformats.org/drawingml/2006/spreadsheetDrawing"
C = "http://schemas.openxmlformats.org/drawingml/2006/chart"
P = "http://schemas.openxmlformats.org/presentationml/2006/main"
EMU = 12700  # per point


def png(w, h, pixel):
    """An RGB PNG, w x h, pixel(x, y) -> (r, g, b)."""
    raw = b"".join(b"\x00" + b"".join(bytes(pixel(x, y)) for x in range(w)) for y in range(h))

    def chunk(tag, data):
        return struct.pack(">I", len(data)) + tag + data + struct.pack(">I", zlib.crc32(tag + data) & 0xFFFFFFFF)

    ihdr = struct.pack(">IIBBBBB", w, h, 8, 2, 0, 0, 0)
    return b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", ihdr) + chunk(b"IDAT", zlib.compress(raw, 9)) + chunk(b"IEND", b"")


# 16 distinct colours, one per 2 x 2 block of an 8 x 8 picture.
PALETTE = [(r, g, b) for r in (20, 230) for g in (40, 200) for b in (60, 120, 180, 250)]
assert len(set(PALETTE)) == 16
PICTURE = png(8, 8, lambda x, y: PALETTE[(y // 2) * 4 + x // 2])


def col_name(c):
    name = ""
    c += 1
    while c:
        c, rem = divmod(c - 1, 26)
        name = chr(65 + rem) + name
    return name


def ref(r, c):
    return f"{col_name(c)}{r + 1}"


STYLES = (
    f'<styleSheet xmlns="{S}">'
    '<numFmts count="2"><numFmt numFmtId="164" formatCode="#,##0.00"/><numFmt numFmtId="165" formatCode="0.0%"/></numFmts>'
    '<fonts count="2"><font><sz val="11"/><name val="Calibri"/></font><font><b/><sz val="11"/><name val="Calibri"/></font></fonts>'
    '<fills count="2"><fill><patternFill patternType="none"/></fill><fill><patternFill patternType="gray125"/></fill></fills>'
    '<borders count="1"><border><left/><right/><top/><bottom/><diagonal/></border></borders>'
    '<cellStyleXfs count="1"><xf numFmtId="0" fontId="0" fillId="0" borderId="0"/></cellStyleXfs>'
    '<cellXfs count="4">'
    '<xf numFmtId="0" fontId="0" fillId="0" borderId="0" xfId="0"/>'
    '<xf numFmtId="0" fontId="1" fillId="0" borderId="0" xfId="0" applyFont="1"/>'
    '<xf numFmtId="164" fontId="0" fillId="0" borderId="0" xfId="0" applyNumberFormat="1"/>'
    '<xf numFmtId="165" fontId="0" fillId="0" borderId="0" xfId="0" applyNumberFormat="1"/>'
    "</cellXfs>"
    '<cellStyles count="1"><cellStyle name="Normal" xfId="0" builtinId="0"/></cellStyles>'
    "</styleSheet>"
)
BOLD, AMOUNT, SHARE = 1, 2, 3


class Strings:
    def __init__(self):
        self.items = []

    def __call__(self, text):
        if text not in self.items:
            self.items.append(text)
        return self.items.index(text)

    def part(self):
        body = "".join(f"<si><t>{t}</t></si>" for t in self.items)
        return f'<sst xmlns="{S}" count="{len(self.items)}" uniqueCount="{len(self.items)}">{body}</sst>'


def text_cell(strings, r, c, text, style=0):
    s_attr = f' s="{style}"' if style else ""
    return f'<c r="{ref(r, c)}" t="s"{s_attr}><v>{strings(text)}</v></c>'


def num_cell(r, c, value, style=0, formula=None):
    s_attr = f' s="{style}"' if style else ""
    f = f"<f>{formula}</f>" if formula else ""
    return f'<c r="{ref(r, c)}"{s_attr}>{f}<v>{value}</v></c>'


def worksheet(rows, extra="", before="", drawing=False):
    """rows: {row index: [cell xml, ...]}; before: sheetViews etc.; extra: after sheetData (mergeCells)."""
    data = "".join(f'<row r="{r + 1}">{"".join(cells)}</row>' for r, cells in sorted(rows.items()))
    draw = '<drawing r:id="rId1"/>' if drawing else ""
    return (f'<worksheet xmlns="{S}" xmlns:r="{R}">{before}<sheetFormatPr defaultRowHeight="15"/>'
            f"<sheetData>{data}</sheetData>{extra}{draw}</worksheet>")


def anchor_xml(frm, to, body):
    (fc, fco, fr, fro), (tc, tco, tr, tro) = frm, to
    return ("<xdr:twoCellAnchor>"
            f"<xdr:from><xdr:col>{fc}</xdr:col><xdr:colOff>{fco}</xdr:colOff><xdr:row>{fr}</xdr:row><xdr:rowOff>{fro}</xdr:rowOff></xdr:from>"
            f"<xdr:to><xdr:col>{tc}</xdr:col><xdr:colOff>{tco}</xdr:colOff><xdr:row>{tr}</xdr:row><xdr:rowOff>{tro}</xdr:rowOff></xdr:to>"
            f"{body}<xdr:clientData/></xdr:twoCellAnchor>")


def drawing(anchors):
    return f'<xdr:wsDr xmlns:xdr="{XDR}" xmlns:a="{A}" xmlns:r="{R}">{"".join(anchors)}</xdr:wsDr>'


def rels(items):
    body = "".join(f'<Relationship Id="rId{i + 1}" Type="{t}" Target="{target}"/>' for i, (t, target) in enumerate(items))
    return f'<Relationships xmlns="{PKG}/relationships">{body}</Relationships>'


def workbook_parts(sheets, overrides, defaults=""):
    """sheets: [(name, worksheet xml)]; the rest of the package around them."""
    n = len(sheets)
    names = "".join(f'<sheet name="{name}" sheetId="{i + 1}" r:id="rId{i + 1}"/>' for i, (name, _) in enumerate(sheets))
    wb_rels = [(f"{R}/worksheet", f"worksheets/sheet{i + 1}.xml") for i in range(n)]
    wb_rels += [(f"{R}/styles", "styles.xml"), (f"{R}/sharedStrings", "sharedStrings.xml")]
    sheet_types = "".join(
        f'<Override PartName="/xl/worksheets/sheet{i + 1}.xml" ContentType="{CT}spreadsheetml.worksheet+xml"/>'
        for i in range(n))
    types = (f'<Types xmlns="{PKG}/content-types">'
             f'<Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/>'
             f'<Default Extension="xml" ContentType="application/xml"/>{defaults}'
             f'<Override PartName="/xl/workbook.xml" ContentType="{CT}spreadsheetml.sheet.main+xml"/>{sheet_types}'
             f'<Override PartName="/xl/styles.xml" ContentType="{CT}spreadsheetml.styles+xml"/>'
             f'<Override PartName="/xl/sharedStrings.xml" ContentType="{CT}spreadsheetml.sharedStrings+xml"/>'
             f"{overrides}</Types>")
    parts = [
        ("[Content_Types].xml", types),
        ("_rels/.rels", rels([(f"{R}/officeDocument", "xl/workbook.xml")])),
        ("xl/workbook.xml", f'<workbook xmlns="{S}" xmlns:r="{R}"><sheets>{names}</sheets></workbook>'),
        ("xl/_rels/workbook.xml.rels", rels(wb_rels)),
        ("xl/styles.xml", STYLES),
    ]
    parts += [(f"xl/worksheets/sheet{i + 1}.xml", body) for i, (_, body) in enumerate(sheets)]
    return parts


FROZEN_FIRST_ROW = ('<sheetViews><sheetView workbookViewId="0">'
                    '<pane ySplit="1" topLeftCell="A2" activePane="bottomLeft" state="frozen"/></sheetView></sheetViews>')

# ---- sheets-3.xlsx
st = Strings()
budget = {
    0: [text_cell(st, 0, 0, "Item", BOLD), text_cell(st, 0, 1, "Amount", BOLD), text_cell(st, 0, 2, "Share", BOLD)],
    1: [text_cell(st, 1, 0, "Rent"), num_cell(1, 1, "1234.5", AMOUNT), num_cell(1, 2, "0.125", SHARE)],
    2: [text_cell(st, 2, 0, "Costs"), num_cell(2, 1, "2765.25", AMOUNT)],
    3: [text_cell(st, 3, 0, "Fees"), num_cell(3, 1, "362", AMOUNT)],
    4: [text_cell(st, 4, 0, "Total", BOLD), num_cell(4, 1, "4361.75", AMOUNT, "SUM(B2:B4)")],
}
notes = {0: [text_cell(st, 0, 0, "Plain text")], 2: [text_cell(st, 2, 0, "Merged")]}
# The picture: 120 x 90 pt from D2 (col 3, row 1), its far corner at col 5 + 24 pt and on the top edge of row 7 (F8):
# over 48 pt columns and 15 pt rows it covers D2:F7.
pic = ('<xdr:pic><xdr:nvPicPr><xdr:cNvPr id="2" name="Picture 1"/><xdr:cNvPicPr/></xdr:nvPicPr>'
       '<xdr:blipFill><a:blip r:embed="rId1"/><a:stretch><a:fillRect/></a:stretch></xdr:blipFill>'
       f'<xdr:spPr><a:xfrm><a:off x="{144 * EMU}" y="{15 * EMU}"/><a:ext cx="{120 * EMU}" cy="{90 * EMU}"/></a:xfrm>'
       '<a:prstGeom prst="rect"><a:avLst/></a:prstGeom></xdr:spPr></xdr:pic>')
parts = workbook_parts(
    [("Budget", worksheet(budget, before=FROZEN_FIRST_ROW, drawing=True)),
     ("Notes", worksheet(notes, extra='<mergeCells count="1"><mergeCell ref="A3:C3"/></mergeCells>')),
     ("Empty", worksheet({}))],
    f'<Override PartName="/xl/drawings/drawing1.xml" ContentType="{CT}drawing+xml"/>',
    '<Default Extension="png" ContentType="image/png"/>',
)
parts += [
    ("xl/sharedStrings.xml", st.part()),
    ("xl/worksheets/_rels/sheet1.xml.rels", rels([(f"{R}/drawing", "../drawings/drawing1.xml")])),
    ("xl/drawings/drawing1.xml", drawing([anchor_xml((3, 0, 1, 0), (5, 24 * EMU, 7, 0), pic)])),
    ("xl/drawings/_rels/drawing1.xml.rels", rels([(f"{R}/image", "../media/image1.png")])),
    ("xl/media/image1.png", PICTURE),
]
write("sheets-3.xlsx", parts)

# ---- latency/grid-2000.xlsx
GRID_ROWS, GRID_COLS = 2000, 20
st = Strings()
grid = {0: [text_cell(st, 0, c, "Label" if c == 0 else f"Column {c}", BOLD) for c in range(GRID_COLS)]}
for r in range(1, GRID_ROWS):
    cells = [text_cell(st, r, 0, f"Row {r}")]
    for c in range(1, GRID_COLS):
        kind = c % 3
        if kind == 1:
            cells.append(num_cell(r, c, f"{(r * 7919 + c * 104729) % 1000000 / 100:.2f}", AMOUNT))
        elif kind == 2:
            cells.append(num_cell(r, c, f"{(r * 31 + c * 17) % 1000 / 1000:.3f}", SHARE))
        else:
            cells.append(num_cell(r, c, str((r * 13 + c) % 5000)))
    grid[r] = cells
values = "".join(
    f'<c:pt idx="{i}"><c:v>{(i + 1) * 7919 % 1000000 / 100:.2f}</c:v></c:pt>' for i in range(12))
chart = (f'<c:chartSpace xmlns:c="{C}" xmlns:a="{A}" xmlns:r="{R}"><c:chart><c:autoTitleDeleted val="1"/>'
         '<c:plotArea><c:layout/><c:barChart><c:barDir val="col"/><c:grouping val="clustered"/><c:varyColors val="0"/>'
         '<c:ser><c:idx val="0"/><c:order val="0"/>'
         '<c:spPr><a:solidFill><a:srgbClr val="3A7D44"/></a:solidFill></c:spPr>'
         f'<c:val><c:numRef><c:f>Grid!$B$2:$B$13</c:f><c:numCache><c:formatCode>General</c:formatCode>'
         f'<c:ptCount val="12"/>{values}</c:numCache></c:numRef></c:val></c:ser>'
         '<c:axId val="1"/><c:axId val="2"/></c:barChart>'
         '<c:catAx><c:axId val="1"/><c:scaling><c:orientation val="minMax"/></c:scaling><c:delete val="0"/>'
         '<c:axPos val="b"/><c:crossAx val="2"/></c:catAx>'
         '<c:valAx><c:axId val="2"/><c:scaling><c:orientation val="minMax"/></c:scaling><c:delete val="0"/>'
         '<c:axPos val="l"/><c:crossAx val="1"/></c:valAx></c:plotArea><c:plotVisOnly val="1"/></c:chart></c:chartSpace>')
frame = ('<xdr:graphicFrame macro=""><xdr:nvGraphicFramePr><xdr:cNvPr id="2" name="Chart 1"/><xdr:cNvGraphicFramePr/>'
         '</xdr:nvGraphicFramePr><xdr:xfrm><a:off x="0" y="0"/><a:ext cx="0" cy="0"/></xdr:xfrm>'
         f'<a:graphic><a:graphicData uri="{C}"><c:chart xmlns:c="{C}" r:id="rId1"/></a:graphicData></a:graphic>'
         "</xdr:graphicFrame>")
parts = workbook_parts(
    [("Grid", worksheet(grid, before=FROZEN_FIRST_ROW, drawing=True))],
    f'<Override PartName="/xl/drawings/drawing1.xml" ContentType="{CT}drawing+xml"/>'
    f'<Override PartName="/xl/charts/chart1.xml" ContentType="{CT}drawingml.chart+xml"/>',
)
parts += [
    ("xl/sharedStrings.xml", st.part()),
    ("xl/worksheets/_rels/sheet1.xml.rels", rels([(f"{R}/drawing", "../drawings/drawing1.xml")])),
    ("xl/drawings/drawing1.xml", drawing([anchor_xml((21, 0, 1, 0), (28, 0, 16, 0), frame)])),
    ("xl/drawings/_rels/drawing1.xml.rels", rels([(f"{R}/chart", "../charts/chart1.xml")])),
    ("xl/charts/chart1.xml", chart),
]
write("latency/grid-2000.xlsx", parts)

# ---- deck-3.pptx
NS_P = f'xmlns:a="{A}" xmlns:r="{R}" xmlns:p="{P}"'
GROUP = ('<p:nvGrpSpPr><p:cNvPr id="1" name=""/><p:cNvGrpSpPr/><p:nvPr/></p:nvGrpSpPr>'
         '<p:grpSpPr><a:xfrm><a:off x="0" y="0"/><a:ext cx="0" cy="0"/><a:chOff x="0" y="0"/><a:chExt cx="0" cy="0"/></a:xfrm></p:grpSpPr>')
W_EMU, H_EMU = 960 * EMU, 540 * EMU


def title_shape(text):
    run = f'<a:r><a:rPr lang="en-US" sz="4400"/><a:t>{text}</a:t></a:r>' if text else ""
    return ('<p:sp><p:nvSpPr><p:cNvPr id="2" name="Title 1"/><p:cNvSpPr><a:spLocks noGrp="1"/></p:cNvSpPr>'
            '<p:nvPr><p:ph type="title"/></p:nvPr></p:nvSpPr>'
            f'<p:spPr><a:xfrm><a:off x="{48 * EMU}" y="{36 * EMU}"/><a:ext cx="{864 * EMU}" cy="{96 * EMU}"/></a:xfrm></p:spPr>'
            f'<p:txBody><a:bodyPr/><a:lstStyle/><a:p>{run}</a:p></p:txBody></p:sp>')


def box_shape(label):
    return ('<p:sp><p:nvSpPr><p:cNvPr id="3" name="Box 2"/><p:cNvSpPr/><p:nvPr/></p:nvSpPr>'
            f'<p:spPr><a:xfrm><a:off x="{72 * EMU}" y="{216 * EMU}"/><a:ext cx="{816 * EMU}" cy="{252 * EMU}"/></a:xfrm>'
            '<a:prstGeom prst="rect"><a:avLst/></a:prstGeom><a:solidFill><a:srgbClr val="3A7D44"/></a:solidFill></p:spPr>'
            '<p:txBody><a:bodyPr anchor="ctr"/><a:lstStyle/><a:p><a:pPr algn="ctr"/>'
            f'<a:r><a:rPr lang="en-US" sz="3200"><a:solidFill><a:srgbClr val="FFFFFF"/></a:solidFill></a:rPr><a:t>{label}</a:t></a:r>'
            "</a:p></p:txBody></p:sp>")


THEME = (
    f'<a:theme xmlns:a="{A}" name="Fixture"><a:themeElements>'
    '<a:clrScheme name="Fixture">'
    '<a:dk1><a:srgbClr val="000000"/></a:dk1><a:lt1><a:srgbClr val="FFFFFF"/></a:lt1>'
    '<a:dk2><a:srgbClr val="1F2937"/></a:dk2><a:lt2><a:srgbClr val="E5E7EB"/></a:lt2>'
    '<a:accent1><a:srgbClr val="3A7D44"/></a:accent1><a:accent2><a:srgbClr val="7D3A44"/></a:accent2>'
    '<a:accent3><a:srgbClr val="3A447D"/></a:accent3><a:accent4><a:srgbClr val="7D7D3A"/></a:accent4>'
    '<a:accent5><a:srgbClr val="3A7D7D"/></a:accent5><a:accent6><a:srgbClr val="7D3A7D"/></a:accent6>'
    '<a:hlink><a:srgbClr val="0000EE"/></a:hlink><a:folHlink><a:srgbClr val="551A8B"/></a:folHlink></a:clrScheme>'
    '<a:fontScheme name="Fixture"><a:majorFont><a:latin typeface="Calibri"/><a:ea typeface=""/><a:cs typeface=""/></a:majorFont>'
    '<a:minorFont><a:latin typeface="Calibri"/><a:ea typeface=""/><a:cs typeface=""/></a:minorFont></a:fontScheme>'
    '<a:fmtScheme name="Fixture">'
    '<a:fillStyleLst>' + '<a:solidFill><a:schemeClr val="phClr"/></a:solidFill>' * 3 + '</a:fillStyleLst>'
    '<a:lnStyleLst>' + '<a:ln w="9525"><a:solidFill><a:schemeClr val="phClr"/></a:solidFill></a:ln>' * 3 + '</a:lnStyleLst>'
    '<a:effectStyleLst>' + '<a:effectStyle><a:effectLst/></a:effectStyle>' * 3 + '</a:effectStyleLst>'
    '<a:bgFillStyleLst>' + '<a:solidFill><a:schemeClr val="phClr"/></a:solidFill>' * 3 + '</a:bgFillStyleLst>'
    "</a:fmtScheme></a:themeElements></a:theme>"
)
TITLES = ["Q3 plan", "", "Thanks"]
slide_ids = "".join(f'<p:sldId id="{256 + i}" r:id="rId{i + 2}"/>' for i in range(len(TITLES)))
pres_rels = [(f"{R}/slideMaster", "slideMasters/slideMaster1.xml")]
pres_rels += [(f"{R}/slide", f"slides/slide{i + 1}.xml") for i in range(len(TITLES))]
pres_rels += [(f"{R}/theme", "theme/theme1.xml")]
slide_types = "".join(
    f'<Override PartName="/ppt/slides/slide{i + 1}.xml" ContentType="{CT}presentationml.slide+xml"/>'
    for i in range(len(TITLES)))
parts = [
    ("[Content_Types].xml",
     f'<Types xmlns="{PKG}/content-types">'
     f'<Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/>'
     f'<Default Extension="xml" ContentType="application/xml"/>'
     f'<Override PartName="/ppt/presentation.xml" ContentType="{CT}presentationml.presentation.main+xml"/>{slide_types}'
     f'<Override PartName="/ppt/slideLayouts/slideLayout1.xml" ContentType="{CT}presentationml.slideLayout+xml"/>'
     f'<Override PartName="/ppt/slideMasters/slideMaster1.xml" ContentType="{CT}presentationml.slideMaster+xml"/>'
     f'<Override PartName="/ppt/theme/theme1.xml" ContentType="{CT}theme+xml"/></Types>'),
    ("_rels/.rels", rels([(f"{R}/officeDocument", "ppt/presentation.xml")])),
    ("ppt/presentation.xml",
     f'<p:presentation {NS_P}><p:sldMasterIdLst><p:sldMasterId id="2147483648" r:id="rId1"/></p:sldMasterIdLst>'
     f'<p:sldIdLst>{slide_ids}</p:sldIdLst><p:sldSz cx="{W_EMU}" cy="{H_EMU}"/><p:notesSz cx="{H_EMU}" cy="{W_EMU}"/>'
     "</p:presentation>"),
    ("ppt/_rels/presentation.xml.rels", rels(pres_rels)),
    ("ppt/slideMasters/slideMaster1.xml",
     f'<p:sldMaster {NS_P}><p:cSld><p:bg><p:bgPr><a:solidFill><a:srgbClr val="FFFFFF"/></a:solidFill><a:effectLst/>'
     f'</p:bgPr></p:bg><p:spTree>{GROUP}{title_shape("")}</p:spTree></p:cSld>'
     '<p:clrMap bg1="lt1" tx1="dk1" bg2="lt2" tx2="dk2" accent1="accent1" accent2="accent2" accent3="accent3" '
     'accent4="accent4" accent5="accent5" accent6="accent6" hlink="hlink" folHlink="folHlink"/>'
     '<p:sldLayoutIdLst><p:sldLayoutId id="2147483649" r:id="rId1"/></p:sldLayoutIdLst>'
     '<p:txStyles><p:titleStyle><a:lvl1pPr><a:defRPr sz="4400"/></a:lvl1pPr></p:titleStyle>'
     '<p:bodyStyle><a:lvl1pPr><a:defRPr sz="2800"/></a:lvl1pPr></p:bodyStyle>'
     '<p:otherStyle><a:lvl1pPr><a:defRPr sz="1800"/></a:lvl1pPr></p:otherStyle></p:txStyles></p:sldMaster>'),
    ("ppt/slideMasters/_rels/slideMaster1.xml.rels",
     rels([(f"{R}/slideLayout", "../slideLayouts/slideLayout1.xml"), (f"{R}/theme", "../theme/theme1.xml")])),
    ("ppt/slideLayouts/slideLayout1.xml",
     f'<p:sldLayout {NS_P} type="title" preserve="1"><p:cSld name="Title"><p:spTree>{GROUP}{title_shape("")}'
     "</p:spTree></p:cSld><p:clrMapOvr><a:masterClrMapping/></p:clrMapOvr></p:sldLayout>"),
    ("ppt/slideLayouts/_rels/slideLayout1.xml.rels", rels([(f"{R}/slideMaster", "../slideMasters/slideMaster1.xml")])),
    ("ppt/theme/theme1.xml", THEME),
]
for i, title in enumerate(TITLES):
    shapes = (title_shape(title) if title else "") + box_shape(f"Slide {i + 1}")
    parts.append((f"ppt/slides/slide{i + 1}.xml",
                  f'<p:sld {NS_P}><p:cSld><p:spTree>{GROUP}{shapes}</p:spTree></p:cSld>'
                  "<p:clrMapOvr><a:masterClrMapping/></p:clrMapOvr></p:sld>"))
    parts.append((f"ppt/slides/_rels/slide{i + 1}.xml.rels",
                  rels([(f"{R}/slideLayout", "../slideLayouts/slideLayout1.xml")])))
write("deck-3.pptx", parts)
