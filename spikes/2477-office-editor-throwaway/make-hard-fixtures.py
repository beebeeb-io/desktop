#!/usr/bin/env python3
"""THROWAWAY (task 2477 spikes). One harder, real-world-style file per type, standard library only.

  hard.docx  2 pages: heading styles, mixed run formatting, a bullet list (numbering.xml), a 4x4 table
             with a shaded header row and borders, an EMBEDDED png, a header and a footer with a PAGE field
  hard.xlsx  3 sheets: styles (bold, fill, currency / percent formats), a merged title, a frozen header,
             formulas WITHOUT cached values (SUM, AVERAGE, IF, VLOOKUP, ROUND, a cross-sheet reference),
             and a bar chart
  hard.pptx  4 slides on a real master: a master footer band, 2 layouts, placeholders that take their
             position from the layout, two bullet levels, a table, an embedded picture, a gradient shape
"""
import struct, sys, zipfile, zlib, pathlib

OUT = pathlib.Path(sys.argv[1] if len(sys.argv) > 1 else "fixtures")
X = '<?xml version="1.0" encoding="UTF-8" standalone="yes"?>\n'
PKG = "http://schemas.openxmlformats.org/package/2006"
OD = "http://schemas.openxmlformats.org/officeDocument/2006"
R = OD + "/relationships"
A = "http://schemas.openxmlformats.org/drawingml/2006/main"
CT = "application/vnd.openxmlformats-officedocument."


def png(w=240, h=160):
    rows = b""
    for y in range(h):
        rows += b"\x00" + b"".join(bytes((40 + x * 200 // w, 90 + y * 150 // h, 200 - x * 120 // w)) if (x // 40 + y // 40) % 2
                                    else bytes((250, 190, 40)) for x in range(w))
    def chunk(t, d):
        return struct.pack(">I", len(d)) + t + d + struct.pack(">I", zlib.crc32(t + d))
    return b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack(">IIBBBBB", w, h, 8, 2, 0, 0, 0)) + chunk(b"IDAT", zlib.compress(rows, 9)) + chunk(b"IEND", b"")


def write(name, parts):
    with zipfile.ZipFile(OUT / name, "w", zipfile.ZIP_DEFLATED) as z:
        for path, body in parts:
            info = zipfile.ZipInfo(path, (2026, 11, 12, 0, 0, 0))
            info.compress_type = zipfile.ZIP_DEFLATED
            z.writestr(info, body if isinstance(body, bytes) else X + body)
    print(name, (OUT / name).stat().st_size, "bytes")


def types(overrides):
    o = "".join(f'<Override PartName="{p}" ContentType="{CT}{t}+xml"/>' for p, t in overrides)
    return (f'<Types xmlns="{PKG}/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/>'
            f'<Default Extension="xml" ContentType="application/xml"/><Default Extension="png" ContentType="image/png"/>{o}</Types>')


def rels(items):
    return (f'<Relationships xmlns="{PKG}/relationships">'
            + "".join(f'<Relationship Id="{i}" Type="{R}/{t}" Target="{tg}"/>' for i, t, tg in items) + "</Relationships>")


# ------------------------------------------------------------------ hard.docx
W = "http://schemas.openxmlformats.org/wordprocessingml/2006/main"
WP = "http://schemas.openxmlformats.org/drawingml/2006/wordprocessingDrawing"
PIC = "http://schemas.openxmlformats.org/drawingml/2006/picture"
WNS = f'xmlns:w="{W}" xmlns:r="{R}"'


def run(t, props=""):
    return f'<w:r><w:rPr>{props}</w:rPr><w:t xml:space="preserve">{t}</w:t></w:r>'


def p(inner, ppr=""):
    return f"<w:p><w:pPr>{ppr}</w:pPr>{inner}</w:p>"


def tc(t, head=False, right=False):
    shade = '<w:shd w:val="clear" w:color="auto" w:fill="1F3A5F"/>' if head else ""
    props = '<w:b/><w:color w:val="FFFFFF"/>' if head else ""
    jc = '<w:jc w:val="right"/>' if right else ""
    return f'<w:tc><w:tcPr><w:tcW w:w="2250" w:type="dxa"/>{shade}</w:tcPr>{p(run(t, props), jc)}</w:tc>'


BORD = "".join(f'<w:{s} w:val="single" w:sz="6" w:space="0" w:color="444444"/>' for s in ("top", "left", "bottom", "right", "insideH", "insideV"))
rows = [("Service", "Hours", "Rate", "Amount")] + [(f"Design sprint {i}", str(8 * i), "EUR 95.00", f"EUR {760 * i}.00") for i in (1, 2, 3)]
table = (f'<w:tbl><w:tblPr><w:tblW w:w="9000" w:type="dxa"/><w:tblBorders>{BORD}</w:tblBorders></w:tblPr>'
         '<w:tblGrid>' + '<w:gridCol w:w="2250"/>' * 4 + '</w:tblGrid>'
         + "".join("<w:tr>" + "".join(tc(c, ri == 0, ci > 0 and ri > 0) for ci, c in enumerate(r)) + "</w:tr>" for ri, r in enumerate(rows))
         + "</w:tbl>")
image = (f'<w:p><w:r><w:drawing><wp:inline xmlns:wp="{WP}" distT="0" distB="0" distL="0" distR="0"><wp:extent cx="2286000" cy="1524000"/>'
         f'<wp:docPr id="1" name="Picture 1"/><a:graphic xmlns:a="{A}"><a:graphicData uri="{PIC}"><pic:pic xmlns:pic="{PIC}">'
         f'<pic:nvPicPr><pic:cNvPr id="1" name="image1.png"/><pic:cNvPicPr/></pic:nvPicPr><pic:blipFill><a:blip r:embed="rId4"/>'
         f'<a:stretch><a:fillRect/></a:stretch></pic:blipFill><pic:spPr><a:xfrm><a:off x="0" y="0"/><a:ext cx="2286000" cy="1524000"/></a:xfrm>'
         f'<a:prstGeom prst="rect"><a:avLst/></a:prstGeom></pic:spPr></pic:pic></a:graphicData></a:graphic></wp:inline></w:drawing></w:r></w:p>')
LOREM = ("The supplier delivers the services described in this agreement with reasonable skill and care, "
         "within the agreed time, and keeps the client informed of anything that may delay them. ") * 4
bul = '<w:numPr><w:ilvl w:val="0"/><w:numId w:val="1"/></w:numPr>'
body = (p(run("Service agreement (hard fixture)"), '<w:pStyle w:val="Heading1"/>')
        + p(run("This paragraph mixes ") + run("bold", "<w:b/>") + run(", ") + run("italic", "<w:i/>") + run(", ")
            + run("underlined", '<w:u w:val="single"/>') + run(" and ") + run("red text", '<w:color w:val="C00000"/>') + run(" in one line."))
        + p(run("1. Scope of work"), '<w:pStyle w:val="Heading2"/>')
        + "".join(p(run(t), bul) for t in ("First bullet: discovery", "Second bullet: design", "Third bullet: delivery"))
        + table + p(run("Figure 1: an embedded picture (checker pattern)."), '<w:jc w:val="center"/>') + image
        + '<w:p><w:r><w:br w:type="page"/></w:r></w:p>'
        + p(run("2. Terms (PAGE 2 OF 2)"), '<w:pStyle w:val="Heading2"/>') + p(run(LOREM), '<w:jc w:val="both"/>')
        + p(run("Right-aligned closing line."), '<w:jc w:val="right"/>')
        + '<w:sectPr><w:headerReference w:type="default" r:id="rId5"/><w:footerReference w:type="default" r:id="rId6"/>'
          '<w:pgSz w:w="11906" w:h="16838"/><w:pgMar w:top="1440" w:right="1440" w:bottom="1440" w:left="1440" w:header="708" w:footer="708" w:gutter="0"/></w:sectPr>')
styles = (f'<w:styles {WNS}><w:docDefaults><w:rPrDefault><w:rPr><w:rFonts w:ascii="Arial" w:hAnsi="Arial"/><w:sz w:val="22"/></w:rPr></w:rPrDefault>'
          '<w:pPrDefault><w:pPr><w:spacing w:after="160" w:line="276" w:lineRule="auto"/></w:pPr></w:pPrDefault></w:docDefaults>'
          '<w:style w:type="paragraph" w:default="1" w:styleId="Normal"><w:name w:val="Normal"/></w:style>'
          '<w:style w:type="paragraph" w:styleId="Heading1"><w:name w:val="heading 1"/><w:basedOn w:val="Normal"/><w:pPr><w:spacing w:before="240" w:after="120"/></w:pPr>'
          '<w:rPr><w:b/><w:color w:val="1F3A5F"/><w:sz w:val="44"/></w:rPr></w:style>'
          '<w:style w:type="paragraph" w:styleId="Heading2"><w:name w:val="heading 2"/><w:basedOn w:val="Normal"/><w:pPr><w:spacing w:before="200" w:after="80"/></w:pPr>'
          '<w:rPr><w:b/><w:color w:val="2E74B5"/><w:sz w:val="30"/></w:rPr></w:style></w:styles>')
numbering = (f'<w:numbering {WNS}><w:abstractNum w:abstractNumId="0"><w:lvl w:ilvl="0"><w:start w:val="1"/><w:numFmt w:val="bullet"/>'
             '<w:lvlText w:val="&#8226;"/><w:lvlJc w:val="left"/><w:pPr><w:ind w:left="720" w:hanging="360"/></w:pPr></w:lvl></w:abstractNum>'
             '<w:num w:numId="1"><w:abstractNumId w:val="0"/></w:num></w:numbering>')
header = f'<w:hdr {WNS}>{p(run("HEADER: Northwind / Service agreement", "<w:color w:val=\"808080\"/>"), "<w:jc w:val=\"right\"/>")}</w:hdr>'
footer = (f'<w:ftr {WNS}><w:p><w:pPr><w:jc w:val="center"/></w:pPr><w:r><w:t xml:space="preserve">FOOTER page </w:t></w:r>'
          '<w:r><w:fldChar w:fldCharType="begin"/></w:r><w:r><w:instrText xml:space="preserve"> PAGE </w:instrText></w:r>'
          '<w:r><w:fldChar w:fldCharType="separate"/></w:r><w:r><w:t>1</w:t></w:r><w:r><w:fldChar w:fldCharType="end"/></w:r></w:p></w:ftr>')
write("hard.docx", [
    ("[Content_Types].xml", types([("/word/document.xml", "wordprocessingml.document.main"), ("/word/styles.xml", "wordprocessingml.styles"),
                                   ("/word/numbering.xml", "wordprocessingml.numbering"), ("/word/header1.xml", "wordprocessingml.header"),
                                   ("/word/footer1.xml", "wordprocessingml.footer")])),
    ("_rels/.rels", rels([("rId1", "officeDocument", "word/document.xml")])),
    ("word/_rels/document.xml.rels", rels([("rId2", "styles", "styles.xml"), ("rId3", "numbering", "numbering.xml"),
                                           ("rId4", "image", "media/image1.png"), ("rId5", "header", "header1.xml"), ("rId6", "footer", "footer1.xml")])),
    ("word/document.xml", f'<w:document {WNS} xmlns:a="{A}"><w:body>{body}</w:body></w:document>'),
    ("word/styles.xml", styles), ("word/numbering.xml", numbering), ("word/header1.xml", header), ("word/footer1.xml", footer),
    ("word/media/image1.png", png()),
])

# ------------------------------------------------------------------ hard.xlsx
S = "http://schemas.openxmlformats.org/spreadsheetml/2006/main"
C = "http://schemas.openxmlformats.org/drawingml/2006/chart"
XDR = "http://schemas.openxmlformats.org/drawingml/2006/spreadsheetDrawing"
# cellXfs: 0 plain, 1 header (bold white on blue), 2 currency, 3 percent, 4 bold, 5 title (bold 16)
xstyles = (f'<styleSheet xmlns="{S}"><numFmts count="1"><numFmt numFmtId="164" formatCode="&quot;EUR&quot; #,##0.00"/></numFmts>'
           '<fonts count="4"><font><sz val="11"/><name val="Arial"/></font><font><b/><sz val="11"/><color rgb="FFFFFFFF"/><name val="Arial"/></font>'
           '<font><b/><sz val="11"/><name val="Arial"/></font><font><b/><sz val="16"/><name val="Arial"/></font></fonts>'
           '<fills count="3"><fill><patternFill patternType="none"/></fill><fill><patternFill patternType="gray125"/></fill>'
           '<fill><patternFill patternType="solid"><fgColor rgb="FF1F3A5F"/><bgColor indexed="64"/></patternFill></fill></fills>'
           '<borders count="2"><border><left/><right/><top/><bottom/><diagonal/></border>'
           '<border><left style="thin"><color auto="1"/></left><right style="thin"><color auto="1"/></right><top style="thin"><color auto="1"/></top>'
           '<bottom style="thin"><color auto="1"/></bottom><diagonal/></border></borders>'
           '<cellStyleXfs count="1"><xf numFmtId="0" fontId="0" fillId="0" borderId="0"/></cellStyleXfs>'
           '<cellXfs count="6"><xf numFmtId="0" fontId="0" fillId="0" borderId="0" xfId="0"/>'
           '<xf numFmtId="0" fontId="1" fillId="2" borderId="1" xfId="0" applyFont="1" applyFill="1" applyBorder="1"/>'
           '<xf numFmtId="164" fontId="0" fillId="0" borderId="1" xfId="0" applyNumberFormat="1" applyBorder="1"/>'
           '<xf numFmtId="10" fontId="0" fillId="0" borderId="1" xfId="0" applyNumberFormat="1" applyBorder="1"/>'
           '<xf numFmtId="0" fontId="2" fillId="0" borderId="1" xfId="0" applyFont="1" applyBorder="1"/>'
           '<xf numFmtId="0" fontId="3" fillId="0" borderId="0" xfId="0" applyFont="1"><alignment horizontal="center"/></xf></cellXfs></styleSheet>')


def c(ref, v=None, s=0, f=None):
    if f is not None:
        return f'<c r="{ref}" s="{s}"><f>{f}</f></c>'
    if isinstance(v, str):
        return f'<c r="{ref}" s="{s}" t="inlineStr"><is><t>{v}</t></is></c>'
    return f'<c r="{ref}" s="{s}"><v>{v}</v></c>'


items = [("Hosting", 12, "A"), ("Design", 40, "B"), ("Support", 25, "A"), ("Licences", 6, "C"), ("Travel", 3, "B"), ("Training", 16, "C")]
r = ['<row r="1" ht="24" customHeight="1">' + c("A1", "Budget 2026 (hard fixture)", 5) + "</row>",
     '<row r="2">' + "".join(c(col + "2", t, 1) for col, t in zip("ABCDE", ("Item", "Qty", "Rate code", "Unit price", "Total"))) + "</row>"]
for i, (name, qty, code) in enumerate(items, 3):
    r.append(f'<row r="{i}">' + c(f"A{i}", name, 4) + c(f"B{i}", qty) + c(f"C{i}", code)
             + c(f"D{i}", s=2, f=f"VLOOKUP(C{i},Rates!$A$2:$B$4,2,FALSE)") + c(f"E{i}", s=2, f=f"ROUND(B{i}*D{i},2)") + "</row>")
r.append('<row r="9">' + c("A9", "Total", 4) + c("B9", f="SUM(B3:B8)") + c("E9", s=2, f="SUM(E3:E8)") + "</row>")
r.append('<row r="10">' + c("A10", "Average", 4) + c("E10", s=2, f="AVERAGE(E3:E8)") + "</row>")
r.append('<row r="11">' + c("A11", "Over budget?", 4) + c("E11", f='IF(E9&gt;8000,"YES over 8000","no")') + "</row>")
sheet1 = (f'<worksheet xmlns="{S}" xmlns:r="{R}"><sheetViews><sheetView workbookViewId="0"><pane ySplit="2" topLeftCell="A3" activePane="bottomLeft" state="frozen"/></sheetView></sheetViews>'
          '<cols><col min="1" max="1" width="22" customWidth="1"/><col min="2" max="5" width="14" customWidth="1"/></cols>'
          f'<sheetData>{"".join(r)}</sheetData><mergeCells count="1"><mergeCell ref="A1:E1"/></mergeCells><drawing r:id="rId1"/></worksheet>')
sheet2 = (f'<worksheet xmlns="{S}"><sheetData><row r="1">{c("A1", "Code", 1)}{c("B1", "Rate", 1)}</row>'
          + "".join(f'<row r="{i}">{c(f"A{i}", k)}{c(f"B{i}", v, 2)}</row>' for i, (k, v) in enumerate((("A", 49.5), ("B", 95), ("C", 120.25)), 2))
          + "</sheetData></worksheet>")
sheet3 = (f'<worksheet xmlns="{S}"><sheetData><row r="1">{c("A1", "Summary", 5)}</row>'
          f'<row r="2">{c("A2", "Grand total", 4)}{c("B2", s=2, f="Budget!E9")}</row>'
          f'<row r="3">{c("A3", "Design share", 4)}{c("B3", s=3, f="Budget!E4/Budget!E9")}</row>'
          f'<row r="4">{c("A4", "Largest line", 4)}{c("B4", s=2, f="MAX(Budget!E3:E8)")}</row></sheetData></worksheet>')
drawing = (f'<xdr:wsDr xmlns:xdr="{XDR}" xmlns:a="{A}"><xdr:twoCellAnchor><xdr:from><xdr:col>6</xdr:col><xdr:colOff>0</xdr:colOff><xdr:row>1</xdr:row><xdr:rowOff>0</xdr:rowOff></xdr:from>'
           '<xdr:to><xdr:col>12</xdr:col><xdr:colOff>0</xdr:colOff><xdr:row>16</xdr:row><xdr:rowOff>0</xdr:rowOff></xdr:to>'
           '<xdr:graphicFrame macro=""><xdr:nvGraphicFramePr><xdr:cNvPr id="2" name="Chart 1"/><xdr:cNvGraphicFramePr/></xdr:nvGraphicFramePr>'
           '<xdr:xfrm><a:off x="0" y="0"/><a:ext cx="0" cy="0"/></xdr:xfrm><a:graphic><a:graphicData uri="' + C + '">'
           f'<c:chart xmlns:c="{C}" xmlns:r="{R}" r:id="rId1"/></a:graphicData></a:graphic></xdr:graphicFrame><xdr:clientData/></xdr:twoCellAnchor></xdr:wsDr>')
chart = (f'<c:chartSpace xmlns:c="{C}" xmlns:a="{A}"><c:chart><c:title><c:tx><c:rich><a:bodyPr/><a:p><a:r><a:t>Total per item (chart)</a:t></a:r></a:p></c:rich></c:tx><c:overlay val="0"/></c:title>'
         '<c:autoTitleDeleted val="0"/><c:plotArea><c:layout/><c:barChart><c:barDir val="col"/><c:grouping val="clustered"/><c:varyColors val="0"/>'
         '<c:ser><c:idx val="0"/><c:order val="0"/><c:tx><c:strRef><c:f>Budget!$E$2</c:f></c:strRef></c:tx>'
         '<c:cat><c:strRef><c:f>Budget!$A$3:$A$8</c:f></c:strRef></c:cat><c:val><c:numRef><c:f>Budget!$E$3:$E$8</c:f></c:numRef></c:val></c:ser>'
         '<c:gapWidth val="150"/><c:axId val="111"/><c:axId val="222"/></c:barChart>'
         '<c:catAx><c:axId val="111"/><c:scaling><c:orientation val="minMax"/></c:scaling><c:delete val="0"/><c:axPos val="b"/><c:crossAx val="222"/></c:catAx>'
         '<c:valAx><c:axId val="222"/><c:scaling><c:orientation val="minMax"/></c:scaling><c:delete val="0"/><c:axPos val="l"/><c:majorGridlines/><c:crossAx val="111"/></c:valAx>'
         '</c:plotArea><c:legend><c:legendPos val="r"/><c:overlay val="0"/></c:legend><c:plotVisOnly val="1"/></c:chart></c:chartSpace>')
write("hard.xlsx", [
    ("[Content_Types].xml", types([("/xl/workbook.xml", "spreadsheetml.sheet.main"), ("/xl/styles.xml", "spreadsheetml.styles"),
                                   ("/xl/worksheets/sheet1.xml", "spreadsheetml.worksheet"), ("/xl/worksheets/sheet2.xml", "spreadsheetml.worksheet"),
                                   ("/xl/worksheets/sheet3.xml", "spreadsheetml.worksheet"), ("/xl/drawings/drawing1.xml", "drawing"),
                                   ("/xl/charts/chart1.xml", "drawingml.chart")])),
    ("_rels/.rels", rels([("rId1", "officeDocument", "xl/workbook.xml")])),
    ("xl/workbook.xml", f'<workbook xmlns="{S}" xmlns:r="{R}"><sheets><sheet name="Budget" sheetId="1" r:id="rId1"/>'
                        '<sheet name="Rates" sheetId="2" r:id="rId2"/><sheet name="Summary" sheetId="3" r:id="rId3"/></sheets><calcPr fullCalcOnLoad="1"/></workbook>'),
    ("xl/_rels/workbook.xml.rels", rels([("rId1", "worksheet", "worksheets/sheet1.xml"), ("rId2", "worksheet", "worksheets/sheet2.xml"),
                                         ("rId3", "worksheet", "worksheets/sheet3.xml"), ("rId4", "styles", "styles.xml")])),
    ("xl/styles.xml", xstyles), ("xl/worksheets/sheet1.xml", sheet1), ("xl/worksheets/sheet2.xml", sheet2), ("xl/worksheets/sheet3.xml", sheet3),
    ("xl/worksheets/_rels/sheet1.xml.rels", rels([("rId1", "drawing", "../drawings/drawing1.xml")])),
    ("xl/drawings/drawing1.xml", drawing), ("xl/drawings/_rels/drawing1.xml.rels", rels([("rId1", "chart", "../charts/chart1.xml")])),
    ("xl/charts/chart1.xml", chart),
])

# ------------------------------------------------------------------ hard.pptx
P = "http://schemas.openxmlformats.org/presentationml/2006/main"
NS = f'xmlns:a="{A}" xmlns:r="{R}" xmlns:p="{P}"'
TREE0 = ('<p:nvGrpSpPr><p:cNvPr id="1" name=""/><p:cNvGrpSpPr/><p:nvPr/></p:nvGrpSpPr>'
         '<p:grpSpPr><a:xfrm><a:off x="0" y="0"/><a:ext cx="0" cy="0"/><a:chOff x="0" y="0"/><a:chExt cx="0" cy="0"/></a:xfrm></p:grpSpPr>')
CLR = ('<p:clrMap bg1="lt1" tx1="dk1" bg2="lt2" tx2="dk2" accent1="accent1" accent2="accent2" accent3="accent3" '
       'accent4="accent4" accent5="accent5" accent6="accent6" hlink="hlink" folHlink="folHlink"/>')
OVR = "<p:clrMapOvr><a:masterClrMapping/></p:clrMapOvr>"


def ph(i, kind, idx=None, xfrm=None, text=None, name="ph"):
    """A placeholder shape. xfrm None = position inherited from the layout / master."""
    at = f' type="{kind}"' if kind else ""
    ix = f' idx="{idx}"' if idx is not None else ""
    sp = f'<p:spPr><a:xfrm><a:off x="{xfrm[0]}" y="{xfrm[1]}"/><a:ext cx="{xfrm[2]}" cy="{xfrm[3]}"/></a:xfrm></p:spPr>' if xfrm else "<p:spPr/>"
    paras = "".join(f'<a:p><a:pPr lvl="{lvl}"/><a:r><a:rPr lang="en-GB"/><a:t>{t}</a:t></a:r></a:p>' for lvl, t in (text or [(0, "")]))
    return (f'<p:sp><p:nvSpPr><p:cNvPr id="{i}" name="{name} {i}"/><p:cNvSpPr><a:spLocks noGrp="1"/></p:cNvSpPr><p:nvPr><p:ph{at}{ix}/></p:nvPr></p:nvSpPr>'
            f'{sp}<p:txBody><a:bodyPr/><a:lstStyle/>{paras}</p:txBody></p:sp>')


def lvl(n, sz, indent, bullet=True):
    b = '<a:buFont typeface="Arial"/><a:buChar char="&#8226;"/>' if bullet else "<a:buNone/>"
    return f'<a:lvl{n}pPr marL="{indent}" indent="{-342900 if bullet else 0}" algn="l">{b}<a:defRPr sz="{sz}"><a:solidFill><a:schemeClr val="tx1"/></a:solidFill><a:latin typeface="Arial"/></a:defRPr></a:lvl{n}pPr>'


band = ('<p:sp><p:nvSpPr><p:cNvPr id="9" name="Master band"/><p:cNvSpPr/><p:nvPr/></p:nvSpPr><p:spPr><a:xfrm><a:off x="0" y="6400800"/><a:ext cx="12192000" cy="457200"/></a:xfrm>'
        '<a:prstGeom prst="rect"><a:avLst/></a:prstGeom><a:solidFill><a:srgbClr val="1F3A5F"/></a:solidFill></p:spPr>'
        '<p:txBody><a:bodyPr anchor="ctr"/><a:p><a:pPr algn="r"/><a:r><a:rPr lang="en-GB" sz="1400"><a:solidFill><a:srgbClr val="FFFFFF"/></a:solidFill></a:rPr>'
        '<a:t>MASTER FOOTER BAND (from the slide master)</a:t></a:r></a:p></p:txBody></p:sp>')
master = (f'<p:sldMaster {NS}><p:cSld><p:bg><p:bgPr><a:solidFill><a:srgbClr val="F7F4EC"/></a:solidFill><a:effectLst/></p:bgPr></p:bg><p:spTree>{TREE0}'
          + ph(2, "title", None, (838200, 365125, 10515600, 1325563), [(0, "Master title")], "Title")
          + ph(3, "body", 1, (838200, 1825625, 10515600, 4351338), [(0, "Master text"), (1, "Second level")], "Body") + band
          + f'</p:spTree></p:cSld>{CLR}<p:sldLayoutIdLst><p:sldLayoutId id="2147483649" r:id="rId1"/><p:sldLayoutId id="2147483650" r:id="rId2"/></p:sldLayoutIdLst>'
          '<p:txStyles><p:titleStyle><a:lvl1pPr algn="l"><a:defRPr sz="4000" b="1"><a:solidFill><a:srgbClr val="1F3A5F"/></a:solidFill><a:latin typeface="Arial"/></a:defRPr></a:lvl1pPr></p:titleStyle>'
          f'<p:bodyStyle>{lvl(1, 2800, 342900)}{lvl(2, 2200, 800100)}</p:bodyStyle><p:otherStyle>{lvl(1, 1800, 0, False)}</p:otherStyle></p:txStyles></p:sldMaster>')
layout1 = (f'<p:sldLayout {NS} type="title"><p:cSld name="Title Slide"><p:spTree>{TREE0}'
           + ph(2, "ctrTitle", None, (1524000, 1600200, 9144000, 1800000), [(0, "Layout title")], "Title")
           + ph(3, "subTitle", 1, (1524000, 3600000, 9144000, 1200000), [(0, "Layout subtitle")], "Subtitle")
           + f'</p:spTree></p:cSld>{OVR}</p:sldLayout>')
layout2 = (f'<p:sldLayout {NS} type="obj"><p:cSld name="Title and Content"><p:spTree>{TREE0}'
           + ph(2, "title", None, None, [(0, "Layout title")], "Title") + ph(3, None, 1, None, [(0, "Layout text")], "Content")
           + f'</p:spTree></p:cSld>{OVR}</p:sldLayout>')


def sld(shapes):
    return f'<p:sld {NS}><p:cSld><p:spTree>{TREE0}{shapes}</p:spTree></p:cSld>{OVR}</p:sld>'


def cell(t, head=False):
    fill = '<a:solidFill><a:srgbClr val="1F3A5F"/></a:solidFill>' if head else '<a:solidFill><a:srgbClr val="FFFFFF"/></a:solidFill>'
    col = '<a:solidFill><a:srgbClr val="FFFFFF"/></a:solidFill>' if head else ""
    return f'<a:tc><a:txBody><a:bodyPr/><a:p><a:r><a:rPr lang="en-GB" sz="1800" b="{1 if head else 0}">{col}</a:rPr><a:t>{t}</a:t></a:r></a:p></a:txBody><a:tcPr>{fill}</a:tcPr></a:tc>'


trows = [("Quarter", "Photos", "Videos")] + [(f"Q{i}", str(120 * i), str(14 * i)) for i in (1, 2, 3)]
tbl = ('<p:graphicFrame><p:nvGraphicFramePr><p:cNvPr id="4" name="Table 4"/><p:cNvGraphicFramePr><a:graphicFrameLocks noGrp="1"/></p:cNvGraphicFramePr><p:nvPr/></p:nvGraphicFramePr>'
       '<p:xfrm><a:off x="838200" y="1825625"/><a:ext cx="9000000" cy="2000000"/></p:xfrm><a:graphic><a:graphicData uri="http://schemas.openxmlformats.org/drawingml/2006/table">'
       '<a:tbl><a:tblPr firstRow="1" bandRow="1"/><a:tblGrid>' + '<a:gridCol w="3000000"/>' * 3 + '</a:tblGrid>'
       + "".join('<a:tr h="500000">' + "".join(cell(t, ri == 0) for t in row) + "</a:tr>" for ri, row in enumerate(trows))
       + "</a:tbl></a:graphicData></a:graphic></p:graphicFrame>")
pic = ('<p:pic><p:nvPicPr><p:cNvPr id="5" name="Picture 5"/><p:cNvPicPr/><p:nvPr/></p:nvPicPr><p:blipFill><a:blip r:embed="rId2"/><a:stretch><a:fillRect/></a:stretch></p:blipFill>'
       '<p:spPr><a:xfrm><a:off x="838200" y="1825625"/><a:ext cx="4572000" cy="3048000"/></a:xfrm><a:prstGeom prst="rect"><a:avLst/></a:prstGeom></p:spPr></p:pic>')
grad = ('<p:sp><p:nvSpPr><p:cNvPr id="6" name="Gradient 6"/><p:cNvSpPr/><p:nvPr/></p:nvSpPr><p:spPr><a:xfrm><a:off x="6096000" y="1825625"/><a:ext cx="5000000" cy="3048000"/></a:xfrm>'
        '<a:prstGeom prst="roundRect"><a:avLst/></a:prstGeom><a:gradFill><a:gsLst><a:gs pos="0"><a:srgbClr val="F5B301"/></a:gs><a:gs pos="100000"><a:srgbClr val="C00000"/></a:gs></a:gsLst><a:lin ang="5400000"/></a:gradFill>'
        '<a:ln w="38100"><a:solidFill><a:srgbClr val="1F3A5F"/></a:solidFill></a:ln></p:spPr>'
        '<p:txBody><a:bodyPr anchor="ctr"/><a:p><a:pPr algn="ctr"/><a:r><a:rPr lang="en-GB" sz="2400" b="1"><a:solidFill><a:srgbClr val="FFFFFF"/></a:solidFill></a:rPr>'
        '<a:t>Rounded, gradient, outlined</a:t></a:r></a:p></p:txBody></p:sp>')
slides = [
    sld(ph(2, "ctrTitle", None, None, [(0, "Lisbon trip recap (hard fixture)")], "Title") + ph(3, "subTitle", 1, None, [(0, "SLIDE 1 OF 4: title layout, positions inherited")], "Subtitle")),
    sld(ph(2, "title", None, None, [(0, "SLIDE 2 OF 4: what we shot")], "Title")
        + ph(3, None, 1, None, [(0, "First point, level one"), (1, "A detail at level two"), (1, "Another detail at level two"), (0, "Second point, level one")], "Content")),
    sld(ph(2, "title", None, None, [(0, "SLIDE 3 OF 4: a table")], "Title") + tbl),
    sld(ph(2, "title", None, None, [(0, "SLIDE 4 OF 4: picture and shape")], "Title") + pic + grad),
]


def clr(tag, v):
    return f'<a:{tag}><a:srgbClr val="{v}"/></a:{tag}>'


FILL3 = '<a:solidFill><a:schemeClr val="phClr"/></a:solidFill>' * 3
theme = (f'<a:theme xmlns:a="{A}" name="Hard"><a:themeElements><a:clrScheme name="Hard">' + clr("dk1", "000000") + clr("lt1", "FFFFFF") + clr("dk2", "1F2937") + clr("lt2", "F3F4F6")
         + "".join(clr(f"accent{i}", v) for i, v in enumerate(["4472C4", "ED7D31", "A5A5A5", "FFC000", "5B9BD5", "70AD47"], 1)) + clr("hlink", "0563C1") + clr("folHlink", "954F72")
         + '</a:clrScheme><a:fontScheme name="Hard"><a:majorFont><a:latin typeface="Arial"/><a:ea typeface=""/><a:cs typeface=""/></a:majorFont>'
           '<a:minorFont><a:latin typeface="Arial"/><a:ea typeface=""/><a:cs typeface=""/></a:minorFont></a:fontScheme>'
           f'<a:fmtScheme name="Hard"><a:fillStyleLst>{FILL3}</a:fillStyleLst><a:lnStyleLst>' + '<a:ln w="9525"><a:solidFill><a:schemeClr val="phClr"/></a:solidFill></a:ln>' * 3
         + '</a:lnStyleLst><a:effectStyleLst>' + '<a:effectStyle><a:effectLst/></a:effectStyle>' * 3 + f'</a:effectStyleLst><a:bgFillStyleLst>{FILL3}</a:bgFillStyleLst></a:fmtScheme></a:themeElements></a:theme>')
pres = (f'<p:presentation {NS}><p:sldMasterIdLst><p:sldMasterId id="2147483648" r:id="rId1"/></p:sldMasterIdLst><p:sldIdLst>'
        + "".join(f'<p:sldId id="{255 + n}" r:id="rId{n + 1}"/>' for n in range(1, 5)) + '</p:sldIdLst><p:sldSz cx="12192000" cy="6858000"/><p:notesSz cx="6858000" cy="9144000"/></p:presentation>')
SL = "../slideLayouts/slideLayout"
parts = [
    ("[Content_Types].xml", types([("/ppt/presentation.xml", "presentationml.presentation.main"), ("/ppt/slideMasters/slideMaster1.xml", "presentationml.slideMaster"),
                                   ("/ppt/slideLayouts/slideLayout1.xml", "presentationml.slideLayout"), ("/ppt/slideLayouts/slideLayout2.xml", "presentationml.slideLayout"),
                                   ("/ppt/theme/theme1.xml", "theme")] + [(f"/ppt/slides/slide{n}.xml", "presentationml.slide") for n in range(1, 5)])),
    ("_rels/.rels", rels([("rId1", "officeDocument", "ppt/presentation.xml")])),
    ("ppt/presentation.xml", pres),
    ("ppt/_rels/presentation.xml.rels", rels([("rId1", "slideMaster", "slideMasters/slideMaster1.xml")] + [(f"rId{n + 1}", "slide", f"slides/slide{n}.xml") for n in range(1, 5)]
                                              + [("rId6", "theme", "theme/theme1.xml")])),
    ("ppt/slideMasters/slideMaster1.xml", master),
    ("ppt/slideMasters/_rels/slideMaster1.xml.rels", rels([("rId1", "slideLayout", SL + "1.xml"), ("rId2", "slideLayout", SL + "2.xml"), ("rId3", "theme", "../theme/theme1.xml")])),
    ("ppt/slideLayouts/slideLayout1.xml", layout1), ("ppt/slideLayouts/slideLayout2.xml", layout2),
    ("ppt/slideLayouts/_rels/slideLayout1.xml.rels", rels([("rId1", "slideMaster", "../slideMasters/slideMaster1.xml")])),
    ("ppt/slideLayouts/_rels/slideLayout2.xml.rels", rels([("rId1", "slideMaster", "../slideMasters/slideMaster1.xml")])),
    ("ppt/theme/theme1.xml", theme), ("ppt/media/image1.png", png()),
]
for n, s in enumerate(slides, 1):
    parts.append((f"ppt/slides/slide{n}.xml", s))
    extra = [("rId2", "image", "../media/image1.png")] if n == 4 else []
    parts.append((f"ppt/slides/_rels/slide{n}.xml.rels", rels([("rId1", "slideLayout", SL + ("1.xml" if n == 1 else "2.xml"))] + extra)))
write("hard.pptx", parts)
