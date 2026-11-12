#!/usr/bin/env python3
"""THROWAWAY (task 2477 editor spike). Feature fixtures: content an editor must not lose on save.

Each file is one of the base fixtures (make-fixtures.py, make-hard-fixtures.py) with parts injected.
Producer of every file written here: hand-written OOXML (this script), standard library only.

  docx: feat-comments (2 comments), feat-tracked (2 insertions + 2 deletions), feat-macro.docm (vbaProject.bin)
  xlsx: feat-comments (2 cell comments), feat-cfdv (conditional format, data validation, defined name),
        feat-pivot (pivot table + cache), feat-macro.xlsm (vbaProject.bin)
  pptx: feat-notes (speaker notes), feat-media (a video part), feat-comments (1 comment), feat-chart (a chart),
        feat-macro.pptm (vbaProject.bin)
  big-30-pages.docx: 30 pages of text, for the keystroke timing
The vbaProject.bin and the .mp4 are NOT real: they are marker bytes behind the right magic number, enough to see
whether a writer keeps the part byte for byte.
"""
import hashlib, sys, zipfile, pathlib

D = pathlib.Path(sys.argv[1] if len(sys.argv) > 1 else "fixtures")
X = '<?xml version="1.0" encoding="UTF-8" standalone="yes"?>\n'
OD = "http://schemas.openxmlformats.org/officeDocument/2006/relationships"
MS = "http://schemas.microsoft.com/office/2006/relationships"
CT = "application/vnd.openxmlformats-officedocument."
W = "http://schemas.openxmlformats.org/wordprocessingml/2006/main"
S = "http://schemas.openxmlformats.org/spreadsheetml/2006/main"
P = "http://schemas.openxmlformats.org/presentationml/2006/main"
A = "http://schemas.openxmlformats.org/drawingml/2006/main"
C = "http://schemas.openxmlformats.org/drawingml/2006/chart"
PKGREL = "http://schemas.openxmlformats.org/package/2006/relationships"
VBA = bytes.fromhex("D0CF11E0A1B11AE1") + b"BB-SPIKE-VBA-MARKER " * 100
MP4 = b"\x00\x00\x00\x18ftypmp42\x00\x00\x00\x00mp42isom" + b"BB-SPIKE-MEDIA-MARKER " * 200


def inject(src, dst, add=None, edit=None):
    """Copy package `src` to `dst`; `edit` maps part -> function(text) -> text; `add` maps part -> text or bytes."""
    add, edit = dict(add or {}), dict(edit or {})
    zin = zipfile.ZipFile(D / src)
    with zipfile.ZipFile(D / dst, "w", zipfile.ZIP_DEFLATED) as z:
        for n in zin.namelist():
            data = zin.read(n)
            if n in edit:
                data = edit.pop(n)(data.decode("utf-8")).encode("utf-8")
            if n in add:
                continue
            info = zipfile.ZipInfo(n, (2026, 11, 12, 0, 0, 0)); info.compress_type = zipfile.ZIP_DEFLATED
            z.writestr(info, data)
        assert not edit, f"{dst}: parts to edit not found: {list(edit)}"
        for n, data in add.items():
            info = zipfile.ZipInfo(n, (2026, 11, 12, 0, 0, 0)); info.compress_type = zipfile.ZIP_DEFLATED
            z.writestr(info, data if isinstance(data, bytes) else (X + data).encode("utf-8"))
    print(dst, (D / dst).stat().st_size, "bytes")


def sub(old, new, count=1):
    def f(t):
        assert old in t, f"anchor not found: {old[:50]}"
        return t.replace(old, new, count)
    return f


def both(*fs):
    def f(t):
        for g in fs:
            t = g(t)
        return t
    return f


def ctype(part, kind, raw=False):
    return sub("</Types>", f'<Override PartName="{part}" ContentType="{kind if raw else CT + kind + "+xml"}"/></Types>')


def cdefault(ext, kind):
    return sub("</Types>", f'<Default Extension="{ext}" ContentType="{kind}"/></Types>')


def rel(i, kind, target, base=OD):
    return f'<Relationship Id="{i}" Type="{base}/{kind}" Target="{target}"/>'


def rels(*items):
    return f'<Relationships xmlns="{PKGREL}">' + "".join(items) + "</Relationships>"


def addrel(item):
    return sub("</Relationships>", item + "</Relationships>")


# ------------------------------------------------------------------ docx
DOCRELS = "word/_rels/document.xml.rels"
when = 'w:author="Reviewer" w:date="2026-11-12T10:00:00Z"'
cpara = "".join(
    f'<w:p><w:commentRangeStart w:id="{i}"/><w:r><w:t>Commented text {i + 1}</w:t></w:r><w:commentRangeEnd w:id="{i}"/>'
    f'<w:r><w:commentReference w:id="{i}"/></w:r></w:p>' for i in (0, 1))
comments = (f'<w:comments xmlns:w="{W}">' + "".join(
    f'<w:comment w:id="{i}" {when} w:initials="R"><w:p><w:r><w:t>COMMENT-{i + 1} body</w:t></w:r></w:p></w:comment>' for i in (0, 1))
    + "</w:comments>")
inject("spike.docx", "feat-comments.docx",
       add={"word/comments.xml": comments, DOCRELS: rels(rel("rId20", "comments", "comments.xml"))},
       edit={"word/document.xml": sub("<w:sectPr>", cpara + "<w:sectPr>"),
             "[Content_Types].xml": ctype("/word/comments.xml", "wordprocessingml.comments")})
tpara = "".join(
    f'<w:p><w:r><w:t xml:space="preserve">Kept text {i} </w:t></w:r><w:ins w:id="{10 + i}" {when}><w:r><w:t>INSERTED-TRACKED-{i}</w:t></w:r></w:ins>'
    f'<w:del w:id="{20 + i}" {when}><w:r><w:delText>DELETED-TRACKED-{i}</w:delText></w:r></w:del></w:p>' for i in (1, 2))
inject("spike.docx", "feat-tracked.docx", edit={"word/document.xml": sub("<w:sectPr>", tpara + "<w:sectPr>")})
inject("spike.docx", "feat-macro.docm",
       add={"word/vbaProject.bin": VBA, DOCRELS: rels(rel("rId30", "vbaProject", "vbaProject.bin", MS))},
       edit={"[Content_Types].xml": both(
           sub(CT + "wordprocessingml.document.main+xml", "application/vnd.ms-word.document.macroEnabled.main+xml"),
           cdefault("bin", "application/vnd.ms-office.vbaProject"))})
# 30 pages for the keystroke timing
line = "The quick brown fox jumps over the lazy dog and keeps running across the page until the line is full. "
para = lambda n: f'<w:p><w:r><w:t xml:space="preserve">Paragraph {n}. {line * 4}</w:t></w:r></w:p>'
big = "".join(f'<w:p><w:r><w:rPr><w:b/><w:sz w:val="36"/></w:rPr><w:t>BIG PAGE {pg} OF 30</w:t></w:r></w:p>'
              + "".join(para(pg * 100 + i) for i in range(1, 9)) + ('<w:p><w:r><w:br w:type="page"/></w:r></w:p>' if pg < 30 else "")
              for pg in range(1, 31))
def bigdoc(t):
    a, b = t.index("<w:body>") + len("<w:body>"), t.index("<w:sectPr>")
    return t[:a] + big + t[b:]
inject("spike.docx", "big-30-pages.docx", edit={"word/document.xml": bigdoc})

# ------------------------------------------------------------------ xlsx
S1RELS = "xl/worksheets/_rels/sheet1.xml.rels"
xcomments = (f'<comments xmlns="{S}"><authors><author>Reviewer</author></authors><commentList>'
             + "".join(f'<comment ref="{r}" authorId="0"><text><r><t>CELL-COMMENT-{i} body</t></r></text></comment>' for i, r in ((1, "A2"), (2, "B3")))
             + "</commentList></comments>")
inject("spike.xlsx", "feat-comments.xlsx",
       add={"xl/comments1.xml": xcomments, S1RELS: rels(rel("rId1", "comments", "../comments1.xml"))},
       edit={"[Content_Types].xml": ctype("/xl/comments1.xml", "spreadsheetml.comments")})
cfdv = ('<conditionalFormatting sqref="E3:E8"><cfRule type="cellIs" dxfId="0" priority="1" operator="greaterThan"><formula>1000</formula></cfRule></conditionalFormatting>'
        '<dataValidations count="1"><dataValidation type="list" allowBlank="1" showInputMessage="1" showErrorMessage="1" sqref="C3:C8"><formula1>"A,B,C"</formula1></dataValidation></dataValidations>')
inject("hard.xlsx", "feat-cfdv.xlsx", edit={
    "xl/worksheets/sheet1.xml": sub("<drawing ", cfdv + "<drawing "),
    "xl/styles.xml": sub("</styleSheet>", '<dxfs count="1"><dxf><fill><patternFill><bgColor rgb="FFFFC7CE"/></patternFill></fill></dxf></dxfs></styleSheet>'),
    "xl/workbook.xml": sub("<calcPr", '<definedNames><definedName name="BudgetTotal">Budget!$E$9</definedName></definedNames><calcPr')})
items = [("Hosting", 12, 0), ("Design", 40, 1), ("Support", 25, 0), ("Licences", 6, 2), ("Travel", 3, 1), ("Training", 16, 2)]
pcd = (f'<pivotCacheDefinition xmlns="{S}" xmlns:r="{OD}" r:id="rId1" refreshOnLoad="1" recordCount="6">'
       '<cacheSource type="worksheet"><worksheetSource ref="A2:C8" sheet="Budget"/></cacheSource><cacheFields count="3">'
       '<cacheField name="Item" numFmtId="0"><sharedItems count="6">' + "".join(f'<s v="{n}"/>' for n, _, _ in items) + "</sharedItems></cacheField>"
       '<cacheField name="Qty" numFmtId="0"><sharedItems containsSemiMixedTypes="0" containsString="0" containsNumber="1" containsInteger="1" minValue="3" maxValue="40"/></cacheField>'
       '<cacheField name="Rate code" numFmtId="0"><sharedItems count="3"><s v="A"/><s v="B"/><s v="C"/></sharedItems></cacheField></cacheFields></pivotCacheDefinition>')
pcr = f'<pivotCacheRecords xmlns="{S}" count="6">' + "".join(f'<r><x v="{i}"/><n v="{q}"/><x v="{c}"/></r>' for i, (_, q, c) in enumerate(items)) + "</pivotCacheRecords>"
pt = (f'<pivotTableDefinition xmlns="{S}" name="PivotTable1" cacheId="1" dataCaption="Values" applyNumberFormats="0" applyBorderFormats="0" '
      'applyFontFormats="0" applyPatternFormats="0" applyAlignmentFormats="0" applyWidthHeightFormats="1" updatedVersion="6" minRefreshableVersion="3" '
      'createdVersion="6" useAutoFormatting="1" itemPrintTitles="1" indent="0" outline="1" outlineData="1">'
      '<location ref="D1:E5" firstHeaderRow="1" firstDataRow="1" firstDataCol="1"/><pivotFields count="3"><pivotField showAll="0"/>'
      '<pivotField dataField="1" showAll="0"/><pivotField axis="axisRow" showAll="0"><items count="4"><item x="0"/><item x="1"/><item x="2"/><item t="default"/></items></pivotField></pivotFields>'
      '<rowFields count="1"><field x="2"/></rowFields><rowItems count="4"><i><x/></i><i><x v="1"/></i><i><x v="2"/></i><i t="grand"><x/></i></rowItems>'
      '<colItems count="1"><i/></colItems><dataFields count="1"><dataField name="Sum of Qty" fld="1" baseField="0" baseItem="0"/></dataFields></pivotTableDefinition>')
inject("hard.xlsx", "feat-pivot.xlsx",
       add={"xl/pivotCache/pivotCacheDefinition1.xml": pcd, "xl/pivotCache/pivotCacheRecords1.xml": pcr, "xl/pivotTables/pivotTable1.xml": pt,
            "xl/pivotCache/_rels/pivotCacheDefinition1.xml.rels": rels(rel("rId1", "pivotCacheRecords", "pivotCacheRecords1.xml")),
            "xl/pivotTables/_rels/pivotTable1.xml.rels": rels(rel("rId1", "pivotCacheDefinition", "../pivotCache/pivotCacheDefinition1.xml")),
            "xl/worksheets/_rels/sheet3.xml.rels": rels(rel("rId1", "pivotTable", "../pivotTables/pivotTable1.xml"))},
       edit={"xl/workbook.xml": sub("</workbook>", '<pivotCaches><pivotCache cacheId="1" r:id="rId9"/></pivotCaches></workbook>'),
             "xl/_rels/workbook.xml.rels": addrel(rel("rId9", "pivotCacheDefinition", "pivotCache/pivotCacheDefinition1.xml")),
             "[Content_Types].xml": both(ctype("/xl/pivotCache/pivotCacheDefinition1.xml", "spreadsheetml.pivotCacheDefinition"),
                                         ctype("/xl/pivotCache/pivotCacheRecords1.xml", "spreadsheetml.pivotCacheRecords"),
                                         ctype("/xl/pivotTables/pivotTable1.xml", "spreadsheetml.pivotTable"))})
inject("spike.xlsx", "feat-macro.xlsm", add={"xl/vbaProject.bin": VBA},
       edit={"xl/_rels/workbook.xml.rels": addrel(rel("rId30", "vbaProject", "vbaProject.bin", MS)),
             "[Content_Types].xml": both(sub(CT + "spreadsheetml.sheet.main+xml", "application/vnd.ms-excel.sheet.macroEnabled.main+xml"),
                                         cdefault("bin", "application/vnd.ms-office.vbaProject"))})

# ------------------------------------------------------------------ pptx
NS = f'xmlns:a="{A}" xmlns:r="{OD}" xmlns:p="{P}"'
TREE0 = ('<p:nvGrpSpPr><p:cNvPr id="1" name=""/><p:cNvGrpSpPr/><p:nvPr/></p:nvGrpSpPr>'
         '<p:grpSpPr><a:xfrm><a:off x="0" y="0"/><a:ext cx="0" cy="0"/><a:chOff x="0" y="0"/><a:chExt cx="0" cy="0"/></a:xfrm></p:grpSpPr>')
CLR = ('<p:clrMap bg1="lt1" tx1="dk1" bg2="lt2" tx2="dk2" accent1="accent1" accent2="accent2" accent3="accent3" '
       'accent4="accent4" accent5="accent5" accent6="accent6" hlink="hlink" folHlink="folHlink"/>')
PRELS, S1 = "ppt/_rels/presentation.xml.rels", "ppt/slides/_rels/slide1.xml.rels"
nbody = ('<p:sp><p:nvSpPr><p:cNvPr id="3" name="Notes"/><p:cNvSpPr><a:spLocks noGrp="1"/></p:cNvSpPr><p:nvPr><p:ph type="body" idx="1"/></p:nvPr></p:nvSpPr>'
         '<p:spPr><a:xfrm><a:off x="685800" y="4400550"/><a:ext cx="5486400" cy="3600450"/></a:xfrm></p:spPr>'
         '<p:txBody><a:bodyPr/><a:lstStyle/><a:p><a:r><a:rPr lang="en-GB"/><a:t>{}</a:t></a:r></a:p></p:txBody></p:sp>')
inject("spike.pptx", "feat-notes.pptx",
       add={"ppt/notesMasters/notesMaster1.xml": f'<p:notesMaster {NS}><p:cSld><p:spTree>{TREE0}{nbody.format("Notes master text")}</p:spTree></p:cSld>{CLR}</p:notesMaster>',
            "ppt/notesMasters/_rels/notesMaster1.xml.rels": rels(rel("rId1", "theme", "../theme/theme1.xml")),
            "ppt/notesSlides/notesSlide1.xml": f'<p:notes {NS}><p:cSld><p:spTree>{TREE0}{nbody.format("SPEAKER-NOTES-ONE for slide 1")}</p:spTree></p:cSld><p:clrMapOvr><a:masterClrMapping/></p:clrMapOvr></p:notes>',
            "ppt/notesSlides/_rels/notesSlide1.xml.rels": rels(rel("rId1", "notesMaster", "../notesMasters/notesMaster1.xml"), rel("rId2", "slide", "../slides/slide1.xml"))},
       edit={S1: addrel(rel("rId7", "notesSlide", "../notesSlides/notesSlide1.xml")),
             PRELS: addrel(rel("rId8", "notesMaster", "notesMasters/notesMaster1.xml")),
             "ppt/presentation.xml": sub("</p:sldMasterIdLst>", '</p:sldMasterIdLst><p:notesMasterIdLst><p:notesMasterId r:id="rId8"/></p:notesMasterIdLst>'),
             "[Content_Types].xml": both(ctype("/ppt/notesMasters/notesMaster1.xml", "presentationml.notesMaster"),
                                         ctype("/ppt/notesSlides/notesSlide1.xml", "presentationml.notesSlide"))})
S4 = "ppt/slides/_rels/slide4.xml.rels"
video = ('<p:pic><p:nvPicPr><p:cNvPr id="20" name="Video 20"><a:hlinkClick r:id="" action="ppaction://media"/></p:cNvPr><p:cNvPicPr><a:picLocks noChangeAspect="1"/></p:cNvPicPr>'
         '<p:nvPr><a:videoFile r:link="rId21"/><p:extLst><p:ext uri="{DAA4B4D4-6D71-4841-9C94-3DE7FCFB9230}">'
         '<p14:media xmlns:p14="http://schemas.microsoft.com/office/powerpoint/2010/main" r:embed="rId22"/></p:ext></p:extLst></p:nvPr></p:nvPicPr>'
         '<p:blipFill><a:blip r:embed="rId2"/><a:stretch><a:fillRect/></a:stretch></p:blipFill>'
         '<p:spPr><a:xfrm><a:off x="838200" y="5029200"/><a:ext cx="1828800" cy="1219200"/></a:xfrm><a:prstGeom prst="rect"><a:avLst/></a:prstGeom></p:spPr></p:pic>')
inject("hard.pptx", "feat-media.pptx", add={"ppt/media/media1.mp4": MP4},
       edit={"ppt/slides/slide4.xml": sub("</p:spTree>", video + "</p:spTree>"),
             S4: both(addrel(rel("rId21", "video", "../media/media1.mp4")), addrel(rel("rId22", "media", "../media/media1.mp4", "http://schemas.microsoft.com/office/2007/relationships"))),
             "[Content_Types].xml": cdefault("mp4", "video/mp4")})
inject("spike.pptx", "feat-comments.pptx",
       add={"ppt/commentAuthors.xml": f'<p:cmAuthorLst {NS}><p:cmAuthor id="0" name="Reviewer" initials="R" lastIdx="1" clrIdx="0"/></p:cmAuthorLst>',
            "ppt/comments/comment1.xml": f'<p:cmLst {NS}><p:cm authorId="0" dt="2026-11-12T10:00:00.000" idx="1"><p:pos x="100" y="100"/><p:text>SLIDE-COMMENT-1 body</p:text></p:cm></p:cmLst>'},
       edit={S1: addrel(rel("rId7", "comments", "../comments/comment1.xml")),
             PRELS: addrel(rel("rId8", "commentAuthors", "commentAuthors.xml")),
             "[Content_Types].xml": both(ctype("/ppt/commentAuthors.xml", "presentationml.commentAuthors"), ctype("/ppt/comments/comment1.xml", "presentationml.comments"))})
lit = lambda tag, vals, kind: (f"<c:{tag}><c:{kind}><c:ptCount val=\"{len(vals)}\"/>" + "".join(f'<c:pt idx="{i}"><c:v>{v}</c:v></c:pt>' for i, v in enumerate(vals)) + f"</c:{kind}></c:{tag}>")
chart = (f'<c:chartSpace xmlns:c="{C}" xmlns:a="{A}"><c:chart><c:title><c:tx><c:rich><a:bodyPr/><a:p><a:r><a:t>CHART-TITLE-MARKER</a:t></a:r></a:p></c:rich></c:tx><c:overlay val="0"/></c:title>'
         '<c:autoTitleDeleted val="0"/><c:plotArea><c:layout/><c:barChart><c:barDir val="col"/><c:grouping val="clustered"/><c:varyColors val="0"/><c:ser><c:idx val="0"/><c:order val="0"/>'
         + lit("cat", ["Q1", "Q2", "Q3"], "strLit") + lit("val", [120, 240, 360], "numLit") +
         '</c:ser><c:gapWidth val="150"/><c:axId val="111"/><c:axId val="222"/></c:barChart>'
         '<c:catAx><c:axId val="111"/><c:scaling><c:orientation val="minMax"/></c:scaling><c:delete val="0"/><c:axPos val="b"/><c:crossAx val="222"/></c:catAx>'
         '<c:valAx><c:axId val="222"/><c:scaling><c:orientation val="minMax"/></c:scaling><c:delete val="0"/><c:axPos val="l"/><c:crossAx val="111"/></c:valAx>'
         '</c:plotArea><c:plotVisOnly val="1"/></c:chart></c:chartSpace>')
frame = ('<p:graphicFrame><p:nvGraphicFramePr><p:cNvPr id="30" name="Chart 30"/><p:cNvGraphicFramePr/><p:nvPr/></p:nvGraphicFramePr>'
         '<p:xfrm><a:off x="685800" y="4267200"/><a:ext cx="5486400" cy="2286000"/></p:xfrm><a:graphic><a:graphicData uri="' + C + '">'
         f'<c:chart xmlns:c="{C}" r:id="rId7"/></a:graphicData></a:graphic></p:graphicFrame>')
inject("spike.pptx", "feat-chart.pptx", add={"ppt/charts/chart1.xml": chart},
       edit={"ppt/slides/slide1.xml": sub("</p:spTree>", frame + "</p:spTree>"), S1: addrel(rel("rId7", "chart", "../charts/chart1.xml")),
             "[Content_Types].xml": ctype("/ppt/charts/chart1.xml", "drawingml.chart")})
inject("spike.pptx", "feat-macro.pptm", add={"ppt/vbaProject.bin": VBA},
       edit={PRELS: addrel(rel("rId30", "vbaProject", "vbaProject.bin", MS)),
             "[Content_Types].xml": both(sub(CT + "presentationml.presentation.main+xml", "application/vnd.ms-powerpoint.presentation.macroEnabled.main+xml"),
                                         cdefault("bin", "application/vnd.ms-office.vbaProject"))})
print("vba sha256", hashlib.sha256(VBA).hexdigest()[:16], "mp4 sha256", hashlib.sha256(MP4).hexdigest()[:16])
