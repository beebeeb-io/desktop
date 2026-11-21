//! The edit-open guard (drive-complete 008, by kind since 008b): which kinds of part in a .docx / .xlsx / .pptx the
//! editor's engines still drop on save. A file that holds one is not opened for editing: the helper answers
//! `EditGuarded` with the kinds, and the app opens the file in the viewer and says so.
//!
//! Since desktop-office 139/145 a save keeps charts, OLE objects, embedded workbooks, VML drawings and SmartArt (in a
//! .pptx under the engine's own names, `ppt/media/media1.bin`, `kept*`), the thumbnail, handout masters, and macro
//! projects byte-equal (tested below through the real engines). What still goes: speaker notes (`notesSlide`,
//! `notesMaster`), ActiveX controls and ink. So the guard decides by KIND, never by part name or folder: every entry's
//! content type the OPC way (an `Override` for its part name, else the `Default` for its extension, from
//! `[Content_Types].xml`), and every relationship's type whose target is in the package. A `Default` with no entry of
//! its extension names nothing.
//!
//! A package this cannot read (not a zip, no or an over-long `[Content_Types].xml`) names nothing here: the engine's
//! own open then says what is wrong with it, as before. Relationship files past the read budget name `Other`: the
//! file opens in the viewer rather than risk a save that drops what they point at.

use std::io::Read;

use beebeeb_office_protocol::office::DroppedPart;

/// The most bytes of `[Content_Types].xml` read: real ones are a few KiB; more is not a package we edit.
const MAX_CONTENT_TYPES: u64 = 1 << 20;
/// The most entries looked at: the engines refuse packages far smaller.
const MAX_ENTRIES: usize = 100_000;
/// The most bytes of `.rels` files read in all (real decks: a few KiB per slide).
const MAX_RELS_TOTAL: u64 = 16 << 20;

/// The kinds of part in `package` the editor would drop on save, one of each, in [`DroppedPart`] order. Empty: edit.
pub fn dropped_parts(package: &[u8]) -> Vec<DroppedPart> {
    let Ok(mut zip) = zip::ZipArchive::new(std::io::Cursor::new(package)) else {
        return Vec::new();
    };
    if zip.len() > MAX_ENTRIES {
        return Vec::new();
    }
    let names: Vec<String> = zip.file_names().map(str::to_owned).collect();
    let Some(types) = content_types(&mut zip) else {
        return Vec::new();
    };
    let parts: std::collections::HashSet<String> = names
        .iter()
        .filter(|name| !name.ends_with('/'))
        .map(|name| name.trim_start_matches('/').to_ascii_lowercase())
        .collect();
    let mut found = [false; ORDER.len()];
    for part in &parts {
        if let Some(kind) = type_kind(types.of(part)) {
            found[slot(kind)] = true;
        }
    }
    let mut budget = MAX_RELS_TOTAL;
    for name in &names {
        let part = name.trim_start_matches('/').to_ascii_lowercase();
        if !part.ends_with(".rels") || name.ends_with('/') {
            continue;
        }
        let Some(xml) = read_capped(&mut zip, name, &mut budget) else {
            found[slot(DroppedPart::Other)] = true;
            break;
        };
        for (ty, target) in relationships(&xml) {
            if let Some(kind) = rel_kind(&ty) {
                if resolve(&part, &target).is_some_and(|t| parts.contains(&t)) {
                    found[slot(kind)] = true;
                }
            }
        }
    }
    ORDER.iter().copied().filter(|kind| found[slot(*kind)]).collect()
}

const ORDER: [DroppedPart; 4] = [
    DroppedPart::SpeakerNotes,
    DroppedPart::ActiveX,
    DroppedPart::Ink,
    DroppedPart::Other,
];

fn slot(kind: DroppedPart) -> usize {
    match kind {
        DroppedPart::SpeakerNotes => 0,
        DroppedPart::ActiveX => 1,
        DroppedPart::Ink => 2,
        DroppedPart::Other => 3,
    }
}

/// What a part of declared content type `declared` is, when the engines still drop it.
fn type_kind(declared: Option<&str>) -> Option<DroppedPart> {
    let ty = declared?.to_ascii_lowercase();
    let ty = ty.split(';').next().unwrap_or("").trim();
    if ty.ends_with("presentationml.notesslide+xml") || ty.ends_with("presentationml.notesmaster+xml") {
        Some(DroppedPart::SpeakerNotes)
    } else if ty.contains("activex") {
        Some(DroppedPart::ActiveX)
    } else if ty == "application/inkml+xml" {
        Some(DroppedPart::Ink)
    } else {
        None
    }
}

/// What a part reached by a relationship of type `ty` is, when the engines still drop it.
fn rel_kind(ty: &str) -> Option<DroppedPart> {
    let last = ty.trim().rsplit('/').next().unwrap_or("").to_ascii_lowercase();
    match last.as_str() {
        "notesslide" | "notesmaster" => Some(DroppedPart::SpeakerNotes),
        "control" | "activexcontrol" | "activexcontrolbinary" => Some(DroppedPart::ActiveX),
        "ink" => Some(DroppedPart::Ink),
        _ => None,
    }
}

/// The part name (lower-case, no leading `/`) a relationship in `rels` (`dir/_rels/name.rels`) points at with
/// `target`: absolute from the package root, else relative to `dir/`. None when it climbs above the root or is a URL.
fn resolve(rels: &str, target: &str) -> Option<String> {
    let target = unescape(target);
    if target.contains("://") {
        return None;
    }
    let mut segs: Vec<&str> = Vec::new();
    let path = if let Some(abs) = target.strip_prefix('/') {
        abs.to_owned()
    } else {
        let dir = rels.rsplit_once("_rels/").map(|(dir, _)| dir).unwrap_or("");
        format!("{dir}{target}")
    };
    for seg in path.split('/') {
        match seg {
            "" | "." => {}
            ".." => {
                segs.pop()?;
            }
            seg => segs.push(seg),
        }
    }
    Some(segs.join("/").to_ascii_lowercase())
}

/// Entry `name` as text, its bytes taken from `budget`; None when it cannot be read or does not fit.
fn read_capped<R: Read + std::io::Seek>(zip: &mut zip::ZipArchive<R>, name: &str, budget: &mut u64) -> Option<String> {
    let entry = zip.by_name(name).ok()?;
    if entry.size() > *budget {
        return None;
    }
    let mut xml = Vec::new();
    entry.take(*budget + 1).read_to_end(&mut xml).ok()?;
    if xml.len() as u64 > *budget {
        return None;
    }
    *budget -= xml.len() as u64;
    Some(String::from_utf8_lossy(&xml).into_owned())
}

/// The `(Type, Target)` of every `Relationship` element of `xml` (any namespace prefix).
fn relationships(xml: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for_each_element(xml, |local, attrs| {
        if local == "Relationship" {
            if let (Some(ty), Some(target)) = (attr(attrs, "Type"), attr(attrs, "Target")) {
                out.push((unescape(&ty), target));
            }
        }
    });
    out
}

/// Calls `f(local name, attributes)` for every start tag of `xml`. Not a full XML reader: the elements read here are
/// empty and carry only attributes.
fn for_each_element(xml: &str, mut f: impl FnMut(&str, &str)) {
    let mut rest = xml;
    while let Some(open) = rest.find('<') {
        rest = &rest[open + 1..];
        let Some(close) = rest.find('>') else { break };
        let tag = &rest[..close];
        rest = &rest[close + 1..];
        let name_end = tag.find(|c: char| c.is_whitespace() || c == '/').unwrap_or(tag.len());
        let name = &tag[..name_end];
        let local = name.rsplit(':').next().unwrap_or(name);
        f(local, &tag[name_end..]);
    }
}

/// `[Content_Types].xml`, read: the overrides by part name and the defaults by extension (both lower-case).
#[derive(Default)]
struct ContentTypes {
    overrides: Vec<(String, String)>,
    defaults: Vec<(String, String)>,
}

impl ContentTypes {
    fn of(&self, part: &str) -> Option<&str> {
        if let Some((_, ty)) = self.overrides.iter().find(|(name, _)| name == part) {
            return Some(ty);
        }
        let file = part.rsplit('/').next().unwrap_or(part);
        let (_, ext) = file.rsplit_once('.')?;
        self.defaults.iter().find(|(e, _)| e == ext).map(|(_, ty)| ty.as_str())
    }
}

fn content_types<R: Read + std::io::Seek>(zip: &mut zip::ZipArchive<R>) -> Option<ContentTypes> {
    let index = zip.index_for_name("[Content_Types].xml")?;
    let entry = zip.by_index(index).ok()?;
    if entry.size() > MAX_CONTENT_TYPES {
        return None;
    }
    let mut xml = Vec::new();
    entry.take(MAX_CONTENT_TYPES + 1).read_to_end(&mut xml).ok()?;
    if xml.len() as u64 > MAX_CONTENT_TYPES {
        return None;
    }
    Some(parse_content_types(&String::from_utf8_lossy(&xml)))
}

/// The `Override` and `Default` elements of `xml` (any namespace prefix).
fn parse_content_types(xml: &str) -> ContentTypes {
    let mut types = ContentTypes::default();
    for_each_element(xml, |local, attrs| match local {
        "Override" => {
            if let (Some(part), Some(ty)) = (attr(attrs, "PartName"), attr(attrs, "ContentType")) {
                let part = unescape(&part).trim_start_matches('/').to_ascii_lowercase();
                types.overrides.push((part, unescape(&ty)));
            }
        }
        "Default" => {
            if let (Some(ext), Some(ty)) = (attr(attrs, "Extension"), attr(attrs, "ContentType")) {
                types.defaults.push((unescape(&ext).to_ascii_lowercase(), unescape(&ty)));
            }
        }
        _ => {}
    });
    types
}

/// The value of attribute `key` in `attrs` (`key="v"` or `key='v'`, spaces around `=` allowed).
fn attr(attrs: &str, key: &str) -> Option<String> {
    let mut rest = attrs;
    loop {
        let at = rest.find(key)?;
        let before_ok = at == 0 || rest[..at].ends_with(|c: char| c.is_whitespace());
        let after = rest[at + key.len()..].trim_start();
        rest = &rest[at + key.len()..];
        if !before_ok {
            continue;
        }
        let Some(after) = after.strip_prefix('=') else { continue };
        let after = after.trim_start();
        let quote = after.chars().next()?;
        if quote != '"' && quote != '\'' {
            continue;
        }
        let body = &after[1..];
        let end = body.find(quote)?;
        return Some(body[..end].to_owned());
    }
}

fn unescape(s: &str) -> String {
    s.replace("&quot;", "\"").replace("&apos;", "'").replace("&lt;", "<").replace("&gt;", ">").replace("&amp;", "&")
}

#[cfg(test)]
mod tests {
    use std::io::Write;

    use super::*;

    const CT_HEAD: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">
<Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/>
<Default Extension="xml" ContentType="application/xml"/>
<Default Extension="vml" ContentType="application/vnd.openxmlformats-officedocument.vmlDrawing"/>
<Default Extension="png" ContentType="image/png"/>
"#;

    fn package(overrides: &str, entries: &[&str]) -> Vec<u8> {
        let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        let opts = zip::write::SimpleFileOptions::default();
        zip.start_file("[Content_Types].xml", opts).unwrap();
        zip.write_all(format!("{CT_HEAD}{overrides}</Types>").as_bytes()).unwrap();
        for name in entries {
            zip.start_file(*name, opts).unwrap();
            zip.write_all(b"<x/>").unwrap();
        }
        zip.finish().unwrap().into_inner()
    }

    const DOC: &str = r#"<Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/>"#;

    #[test]
    fn edit_guard_plain_files_edit() {
        // A document with a picture, and a sheet that declares vml but has no comments: nothing is dropped.
        let docx = package(DOC, &["_rels/.rels", "word/document.xml", "word/media/image1.png"]);
        assert_eq!(dropped_parts(&docx), vec![]);
        let xlsx = package(
            r#"<Override PartName="/xl/workbook.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml"/>"#,
            &["xl/workbook.xml", "xl/worksheets/sheet1.xml"],
        );
        assert_eq!(dropped_parts(&xlsx), vec![]);
    }

    /// 008b: charts, SmartArt, OLE, VML comments, embedded packages and chartex are kept (139): docx and xlsx edit.
    #[test]
    fn edit_guard_kept_kinds_in_docx_and_xlsx_edit() {
        let overrides = format!(
            "{DOC}{}{}{}",
            r#"<Override PartName="/word/charts/chart1.xml" ContentType="application/vnd.openxmlformats-officedocument.drawingml.chart+xml"/>"#,
            r#"<Override PartName="/word/diagrams/data1.xml" ContentType="application/vnd.openxmlformats-officedocument.drawingml.diagramData+xml"/>"#,
            r#"<Default Extension="bin" ContentType="application/vnd.openxmlformats-officedocument.oleObject"/>"#,
        );
        let docx = package(
            &overrides,
            &[
                "word/document.xml",
                "word/charts/chart1.xml",
                "word/diagrams/data1.xml",
                "word/embeddings/oleObject1.bin",
                "word/drawings/vmlDrawing1.vml",
            ],
        );
        assert_eq!(dropped_parts(&docx), vec![]);
        let xlsx = package(
            r#"<ct:Override PartName='/xl/charts/chartEx1.xml' ContentType='application/vnd.ms-office.chartex+xml'/>"#,
            &["xl/workbook.xml", "xl/drawings/vmlDrawing1.vml", "xl/charts/chartEx1.xml", "xl/embeddings/Book1.xlsx"],
        );
        assert_eq!(dropped_parts(&xlsx), vec![]);
    }

    /// What still drops besides notes, ActiveX controls and ink, is named by kind, by type whatever its name.
    #[test]
    fn edit_guard_activex_and_ink_are_guarded() {
        let overrides = format!(
            "{DOC}{}{}",
            r#"<Override PartName="/WORD/ODD/A1.XML" ContentType="application/vnd.ms-office.activeX+xml"/>"#,
            r#"<Override PartName="/word/ink/ink1.xml" ContentType="application/inkml+xml"/>"#,
        );
        let docx = package(&overrides, &["word/document.xml", "word/odd/a1.xml", "word/ink/ink1.xml"]);
        assert_eq!(dropped_parts(&docx), vec![DroppedPart::ActiveX, DroppedPart::Ink]);
    }

    const PRES: &str = r#"<Override PartName="/ppt/presentation.xml" ContentType="application/vnd.openxmlformats-officedocument.presentationml.presentation.main+xml"/><Override PartName="/ppt/slides/slide1.xml" ContentType="application/vnd.openxmlformats-officedocument.presentationml.slide+xml"/>"#;

    /// A zip of `(name, bytes)` entries, `[Content_Types].xml` first.
    fn package_with(overrides: &str, entries: &[(&str, &str)]) -> Vec<u8> {
        let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        let opts = zip::write::SimpleFileOptions::default();
        zip.start_file("[Content_Types].xml", opts).unwrap();
        zip.write_all(format!("{CT_HEAD}{overrides}</Types>").as_bytes()).unwrap();
        for (name, body) in entries {
            zip.start_file(*name, opts).unwrap();
            zip.write_all(body.as_bytes()).unwrap();
        }
        zip.finish().unwrap().into_inner()
    }

    /// 008b: the editor keeps charts, OLE objects, embedded workbooks, VML and SmartArt (139): a deck with them edits.
    #[test]
    fn edit_guard_pptx_with_kept_kinds_edits() {
        let overrides = format!(
            "{PRES}{}{}{}{}",
            r#"<Override PartName="/ppt/charts/chart1.xml" ContentType="application/vnd.openxmlformats-officedocument.drawingml.chart+xml"/>"#,
            r#"<Override PartName="/ppt/diagrams/data1.xml" ContentType="application/vnd.openxmlformats-officedocument.drawingml.diagramData+xml"/>"#,
            r#"<Default Extension="bin" ContentType="application/vnd.openxmlformats-officedocument.oleObject"/>"#,
            r#"<Default Extension="xlsx" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sheet"/>"#,
        );
        let pptx = package(
            &overrides,
            &[
                "ppt/presentation.xml",
                "ppt/slides/slide1.xml",
                "ppt/charts/chart1.xml",
                "ppt/diagrams/data1.xml",
                "ppt/embeddings/oleObject1.bin",
                "ppt/embeddings/Microsoft_Excel_Worksheet.xlsx",
                "ppt/drawings/vmlDrawing1.vml",
            ],
        );
        assert_eq!(dropped_parts(&pptx), vec![]);
        // A deck the engine saved: the OLE object is kept by content under the engine's name.
        let saved = package(
            &format!("{PRES}{}", r#"<Default Extension="bin" ContentType="application/vnd.openxmlformats-officedocument.oleObject"/>"#),
            &["ppt/presentation.xml", "ppt/slides/slide1.xml", "ppt/media/media1.bin", "ppt/kept1.xlsx"],
        );
        assert_eq!(dropped_parts(&saved), vec![]);
    }

    /// 008b: speaker notes are still dropped on save: a deck with them stays view-only, the notice names them.
    #[test]
    fn edit_guard_pptx_with_notes_is_guarded() {
        let overrides = format!(
            "{PRES}{}{}",
            r#"<Override PartName="/ppt/notesSlides/notesSlide1.xml" ContentType="application/vnd.openxmlformats-officedocument.presentationml.notesSlide+xml"/>"#,
            r#"<Override PartName="/ppt/notesMasters/notesMaster1.xml" ContentType="application/vnd.openxmlformats-officedocument.presentationml.notesMaster+xml"/>"#,
        );
        let pptx = package(&overrides, &["ppt/presentation.xml", "ppt/slides/slide1.xml", "ppt/notesSlides/notesSlide1.xml", "ppt/notesMasters/notesMaster1.xml"]);
        assert_eq!(dropped_parts(&pptx), vec![DroppedPart::SpeakerNotes]);
        // A chart beside the notes does not change the answer.
        let both = package(
            &format!("{overrides}{}", r#"<Override PartName="/ppt/charts/chart1.xml" ContentType="application/vnd.openxmlformats-officedocument.drawingml.chart+xml"/>"#),
            &["ppt/presentation.xml", "ppt/notesSlides/notesSlide1.xml", "ppt/charts/chart1.xml"],
        );
        assert_eq!(dropped_parts(&both), vec![DroppedPart::SpeakerNotes]);
        // Notes under an odd name, declared only by the relationship that points at them: still guarded.
        let by_rel = package_with(
            PRES,
            &[
                ("ppt/presentation.xml", "<p/>"),
                ("ppt/slides/slide1.xml", "<p/>"),
                (
                    "ppt/slides/_rels/slide1.xml.rels",
                    r#"<?xml version="1.0"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId2" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/notesSlide" Target="../odd/n1.xml"/></Relationships>"#,
                ),
                ("ppt/odd/n1.xml", "<p/>"),
            ],
        );
        assert_eq!(dropped_parts(&by_rel), vec![DroppedPart::SpeakerNotes]);
        // A relationship whose target is not in the package names nothing.
        let dangling = package_with(
            PRES,
            &[(
                "ppt/slides/_rels/slide1.xml.rels",
                r#"<Relationships><Relationship Id="rId2" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/notesSlide" Target="../notesSlides/missing.xml"/></Relationships>"#,
            )],
        );
        assert_eq!(dropped_parts(&dangling), vec![]);
    }

    /// `src` with each `(name, bytes)` put in: an existing entry replaced, a new one added at the end.
    fn repack(src: &[u8], put: &[(&str, Vec<u8>)]) -> Vec<u8> {
        let mut from = zip::ZipArchive::new(std::io::Cursor::new(src)).unwrap();
        let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        let opts = zip::write::SimpleFileOptions::default();
        for i in 0..from.len() {
            let mut entry = from.by_index(i).unwrap();
            let name = entry.name().to_owned();
            let mut body = Vec::new();
            entry.read_to_end(&mut body).unwrap();
            let body = put.iter().find(|(n, _)| *n == name).map(|(_, b)| b.clone()).unwrap_or(body);
            zip.start_file(name, opts).unwrap();
            zip.write_all(&body).unwrap();
        }
        for (name, body) in put {
            if from.index_for_name(name).is_none() {
                zip.start_file(*name, opts).unwrap();
                zip.write_all(body).unwrap();
            }
        }
        zip.finish().unwrap().into_inner()
    }

    fn entry(package: &[u8], name: &str) -> Option<Vec<u8>> {
        let mut zip = zip::ZipArchive::new(std::io::Cursor::new(package)).ok()?;
        let mut e = zip.by_name(name).ok()?;
        let mut body = Vec::new();
        e.read_to_end(&mut body).ok()?;
        Some(body)
    }

    fn text(package: &[u8], name: &str) -> String {
        String::from_utf8(entry(package, name).unwrap()).unwrap()
    }

    /// Opens `package` in the real editor, applies `op`, saves: the saved bytes.
    fn edit_and_save(kind: beebeeb_office_protocol::office::EditKind, package: &[u8], op: beebeeb_office_protocol::office::EditOp) -> Vec<u8> {
        use crate::engine::Engine;
        let mut doc = crate::engine::Office.open_edit(kind, package).unwrap_or_else(|e| panic!("open_edit: {e:?}"));
        doc.apply(op).unwrap_or_else(|e| panic!("apply: {e:?}"));
        doc.save().unwrap_or_else(|e| panic!("save: {e:?}")).0
    }

    /// Bytes no engine writes itself: kept only if carried.
    fn macro_bytes() -> Vec<u8> {
        let mut b = vec![0xD0, 0xCF, 0x11, 0xE0, 0xA1, 0xB1, 0x1A, 0xE1];
        b.extend((0..4096u32).map(|i| (i.wrapping_mul(2654435761) >> 13) as u8));
        b
    }

    const VBA_REL: &str = "http://schemas.microsoft.com/office/2006/relationships/vbaProject";
    const VBA_TYPES: &str = r#"<Default Extension="bin" ContentType="application/vnd.ms-office.vbaProject"/>"#;

    /// 008b: a macro project is not guarded because a save keeps it byte-equal (desktop-office `carry`, or GridCraft's
    /// own for .xlsm; a project that cannot be carried makes the save answer `Unsupported`): never lost silently.
    #[test]
    fn edit_guard_xlsm_macros_kept_or_guarded() {
        use beebeeb_office_protocol::office::{EditKind, EditOp};
        let base = include_bytes!("../tests/fixtures/sheets-3.xlsx");
        let types = text(base, "[Content_Types].xml")
            .replace("spreadsheetml.sheet.main+xml", "PLACEHOLDER")
            .replace("application/vnd.openxmlformats-officedocument.PLACEHOLDER", "application/vnd.ms-excel.sheet.macroEnabled.main+xml")
            .replace("</Types>", &format!("{VBA_TYPES}</Types>"));
        let rels = text(base, "xl/_rels/workbook.xml.rels")
            .replace("</Relationships>", &format!(r#"<Relationship Id="rId99" Type="{VBA_REL}" Target="vbaProject.bin"/></Relationships>"#));
        let xlsm = repack(base, &[("[Content_Types].xml", types.into_bytes()), ("xl/_rels/workbook.xml.rels", rels.into_bytes()), ("xl/vbaProject.bin", macro_bytes())]);
        assert_eq!(dropped_parts(&xlsm), vec![], "a macro project alone does not guard: the save keeps it");
        let saved = edit_and_save(EditKind::Xlsx, &xlsm, EditOp::SetCell { sheet: 1, row: 4, col: 0, text: "x".into() });
        assert_eq!(entry(&saved, "xl/vbaProject.bin"), Some(macro_bytes()), "the macro project is kept byte-equal");
    }

    #[test]
    fn edit_guard_docm_macros_kept_or_guarded() {
        use beebeeb_office_protocol::office::{EditKind, EditOp};
        let base = include_bytes!("../tests/fixtures/letter-1.docx");
        let types = text(base, "[Content_Types].xml")
            .replace("application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml", "application/vnd.ms-word.document.macroEnabled.main+xml")
            .replace("</Types>", &format!("{VBA_TYPES}</Types>"));
        let rels = format!(r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId99" Type="{VBA_REL}" Target="vbaProject.bin"/></Relationships>"#);
        let docm = repack(base, &[("[Content_Types].xml", types.into_bytes()), ("word/_rels/document.xml.rels", rels.into_bytes()), ("word/vbaProject.bin", macro_bytes())]);
        assert_eq!(dropped_parts(&docm), vec![], "a macro project alone does not guard: the save keeps it");
        let saved = edit_and_save(EditKind::Docx, &docm, EditOp::InsertText("x".into()));
        assert_eq!(entry(&saved, "word/vbaProject.bin"), Some(macro_bytes()), "the macro project is kept byte-equal");
    }

    /// 008b: a deck's handout master is not guarded because a save keeps it: the part, its relationship and the
    /// presentation's reference to it.
    #[test]
    fn edit_guard_pptx_handout_master_is_kept() {
        use beebeeb_office_protocol::office::{EditKind, EditOp};
        let pptx = handout_deck();
        assert_eq!(dropped_parts(&pptx), vec![]);
        let saved = edit_and_save(EditKind::Pptx, &pptx, EditOp::NewSlide { layout: None });
        assert!(entry(&saved, "ppt/handoutMasters/handoutMaster1.xml").is_some(), "the handout master is kept");
        assert!(text(&saved, "ppt/_rels/presentation.xml.rels").contains("relationships/handoutMaster\""));
        assert!(text(&saved, "ppt/presentation.xml").contains("<p:handoutMasterId "));
    }

    fn handout_deck() -> Vec<u8> {
        let base = include_bytes!("../tests/fixtures/deck-3.pptx");
        let types = text(base, "[Content_Types].xml").replace(
            "</Types>",
            r#"<Override PartName="/ppt/handoutMasters/handoutMaster1.xml" ContentType="application/vnd.openxmlformats-officedocument.presentationml.handoutMaster+xml"/><Override PartName="/ppt/theme/theme2.xml" ContentType="application/vnd.openxmlformats-officedocument.theme+xml"/></Types>"#,
        );
        let rels = text(base, "ppt/_rels/presentation.xml.rels").replace(
            "</Relationships>",
            r#"<Relationship Id="rId99" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/handoutMaster" Target="handoutMasters/handoutMaster1.xml"/></Relationships>"#,
        );
        let pres = text(base, "ppt/presentation.xml").replace(
            "<p:sldIdLst>",
            r#"<p:handoutMasterIdLst><p:handoutMasterId r:id="rId99"/></p:handoutMasterIdLst><p:sldIdLst>"#,
        );
        let master = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><p:handoutMaster xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships" xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main"><p:cSld><p:spTree><p:nvGrpSpPr><p:cNvPr id="1" name=""/><p:cNvGrpSpPr/><p:nvPr/></p:nvGrpSpPr><p:grpSpPr/></p:spTree></p:cSld><p:clrMap bg1="lt1" tx1="dk1" bg2="lt2" tx2="dk2" accent1="accent1" accent2="accent2" accent3="accent3" accent4="accent4" accent5="accent5" accent6="accent6" hlink="hlink" folHlink="folHlink"/></p:handoutMaster>"#;
        let master_rels = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/theme" Target="../theme/theme2.xml"/></Relationships>"#;
        repack(
            base,
            &[
                ("[Content_Types].xml", types.into_bytes()),
                ("ppt/_rels/presentation.xml.rels", rels.into_bytes()),
                ("ppt/presentation.xml", pres.into_bytes()),
                ("ppt/handoutMasters/handoutMaster1.xml", master.as_bytes().to_vec()),
                ("ppt/handoutMasters/_rels/handoutMaster1.xml.rels", master_rels.as_bytes().to_vec()),
                ("ppt/theme/theme2.xml", entry(base, "ppt/theme/theme1.xml").unwrap()),
            ],
        )
    }

    #[test]
    fn edit_guard_unreadable_package_names_nothing() {
        assert_eq!(dropped_parts(b"not a zip"), vec![]);
        assert_eq!(dropped_parts(&[]), vec![]);
        let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        zip.start_file("word/charts/chart1.xml", zip::write::SimpleFileOptions::default()).unwrap();
        let no_types = zip.finish().unwrap().into_inner();
        assert_eq!(dropped_parts(&no_types), vec![]);
    }
}
