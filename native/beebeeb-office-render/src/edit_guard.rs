//! The edit-open guard (drive-complete 008): which parts of a .docx / .xlsx / .pptx the editor's engines would drop
//! on save. Until they survive (139), a file that holds one is not opened for editing: the helper answers
//! `EditGuarded` with the kinds, and the app opens the file in the viewer and says so.
//!
//! The package's own declaration decides: every entry of the zip gets its content type the OPC way (an `Override`
//! for its part name, else the `Default` for its extension, from `[Content_Types].xml`), and the types the engines
//! drop are named. A `Default` with no entry of its extension names nothing (Excel declares `vml` in files without
//! comments). Anything under an `embeddings/` folder is an embedded object whatever it declares.
//!
//! A package this cannot read (not a zip, no or an over-long `[Content_Types].xml`) names nothing here: the engine's
//! own open then says what is wrong with it, as before.

use std::io::Read;

use beebeeb_office_protocol::office::DroppedPart;

/// The most bytes of `[Content_Types].xml` read: real ones are a few KiB; more is not a package we edit.
const MAX_CONTENT_TYPES: u64 = 1 << 20;
/// The most entries looked at: the engines refuse packages far smaller.
const MAX_ENTRIES: usize = 100_000;

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
    let mut found = [false; 5];
    for name in &names {
        if name.ends_with('/') {
            continue;
        }
        let part = name.trim_start_matches('/').to_ascii_lowercase();
        let declared = types.of(&part);
        if let Some(kind) = part_kind(&part, declared) {
            found[slot(kind)] = true;
        }
    }
    ORDER.iter().copied().filter(|kind| found[slot(*kind)]).collect()
}

const ORDER: [DroppedPart; 5] =
    [DroppedPart::Chart, DroppedPart::Ole, DroppedPart::Vml, DroppedPart::SmartArt, DroppedPart::Other];

fn slot(kind: DroppedPart) -> usize {
    match kind {
        DroppedPart::Chart => 0,
        DroppedPart::Ole => 1,
        DroppedPart::Vml => 2,
        DroppedPart::SmartArt => 3,
        DroppedPart::Other => 4,
    }
}

/// What a part (lower-case name, no leading `/`) of declared type `declared` is, when the engines drop it.
fn part_kind(part: &str, declared: Option<&str>) -> Option<DroppedPart> {
    if part.contains("/embeddings/") || part.starts_with("embeddings/") {
        return Some(DroppedPart::Ole);
    }
    let ty = declared?.to_ascii_lowercase();
    let ty = ty.split(';').next().unwrap_or("").trim();
    if ty.ends_with("drawingml.chart+xml") || ty.ends_with("ms-office.chartex+xml") {
        Some(DroppedPart::Chart)
    } else if ty.ends_with("officedocument.oleobject") || ty.contains("activex") {
        Some(DroppedPart::Ole)
    } else if ty.ends_with("officedocument.vmldrawing") {
        Some(DroppedPart::Vml)
    } else if ty.contains("drawingml.diagram") {
        Some(DroppedPart::SmartArt)
    } else if ty == "application/inkml+xml" || ty.ends_with("ms-office.vbaproject") {
        Some(DroppedPart::Other)
    } else {
        None
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

/// The `Override` and `Default` elements of `xml` (any namespace prefix). Not a full XML reader: these two elements
/// are empty and carry only attributes.
fn parse_content_types(xml: &str) -> ContentTypes {
    let mut types = ContentTypes::default();
    let mut rest = xml;
    while let Some(open) = rest.find('<') {
        rest = &rest[open + 1..];
        let Some(close) = rest.find('>') else { break };
        let tag = &rest[..close];
        rest = &rest[close + 1..];
        let name_end = tag.find(|c: char| c.is_whitespace() || c == '/').unwrap_or(tag.len());
        let name = &tag[..name_end];
        let local = name.rsplit(':').next().unwrap_or(name);
        let attrs = &tag[name_end..];
        match local {
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
        }
    }
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

    #[test]
    fn edit_guard_names_each_dropped_kind_once_in_order() {
        let overrides = format!(
            "{DOC}{}{}{}{}",
            r#"<Override PartName="/word/charts/chart1.xml" ContentType="application/vnd.openxmlformats-officedocument.drawingml.chart+xml"/>"#,
            r#"<Override PartName="/word/charts/chart2.xml" ContentType="application/vnd.openxmlformats-officedocument.drawingml.chart+xml"/>"#,
            r#"<Override PartName="/word/diagrams/data1.xml" ContentType="application/vnd.openxmlformats-officedocument.drawingml.diagramData+xml"/>"#,
            r#"<Default Extension="bin" ContentType="application/vnd.openxmlformats-officedocument.oleObject"/>"#,
        );
        let docx = package(
            &overrides,
            &[
                "word/document.xml",
                "word/charts/chart1.xml",
                "word/charts/chart2.xml",
                "word/diagrams/data1.xml",
                "word/media/oleObject1.bin",
                "word/drawings/vmlDrawing1.vml",
            ],
        );
        assert_eq!(
            dropped_parts(&docx),
            vec![DroppedPart::Chart, DroppedPart::Ole, DroppedPart::Vml, DroppedPart::SmartArt]
        );
    }

    #[test]
    fn edit_guard_sees_comments_vml_embedded_packages_and_chartex() {
        let xlsx = package("", &["xl/workbook.xml", "xl/drawings/vmlDrawing1.vml"]);
        assert_eq!(dropped_parts(&xlsx), vec![DroppedPart::Vml]);
        // An embedded workbook declares a spreadsheet type, not an OLE one: its folder says what it is.
        let pptx = package("", &["ppt/presentation.xml", "ppt/embeddings/Microsoft_Excel_Worksheet.xlsx"]);
        assert_eq!(dropped_parts(&pptx), vec![DroppedPart::Ole]);
        let chartex = package(
            r#"<ct:Override PartName='/xl/charts/chartEx1.xml' ContentType='application/vnd.ms-office.chartex+xml'/>"#,
            &["xl/workbook.xml", "xl/charts/chartEx1.xml"],
        );
        assert_eq!(dropped_parts(&chartex), vec![DroppedPart::Chart]);
        // Part names compare without case, as OPC says.
        let upper = package(
            r#"<Override PartName="/WORD/CHARTS/CHART1.XML" ContentType="application/vnd.openxmlformats-officedocument.drawingml.chart+xml"/>"#,
            &["word/charts/chart1.xml"],
        );
        assert_eq!(dropped_parts(&upper), vec![DroppedPart::Chart]);
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
