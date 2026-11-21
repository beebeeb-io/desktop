//! The art for each icon id: Lucide 1.77.0 (ISC; the Feather-derived ones MIT, see THIRD-PARTY-NOTICES.md),
//! re-stroked to the app's `line24!` style. Generated from github.com/lucide-icons/lucide, tag 1.77.0, icons/<name>.svg.
//! Folders are one icon each: the outline, a body at fill-opacity 0.18, and the purpose emblem inside via transform.
//! One tint only: no body names its own colour.

use rune::Icon;

use super::file_icons::IconId;

macro_rules! line24 {
    ($name:expr, $body:expr) => {
        Icon {
            name: $name,
            view: 24.0,
            body: concat!(
                r#"<g fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round">"#,
                $body,
                "</g>"
            ),
        }
    };
}

/// File: Lucide `file`.
pub static FILE: Icon = line24!(
    "file-icon-file",
    r#"<path d="M6 22a2 2 0 0 1-2-2V4a2 2 0 0 1 2-2h8a2.4 2.4 0 0 1 1.704.706l3.588 3.588A2.4 2.4 0 0 1 20 8v12a2 2 0 0 1-2 2z"></path><path d="M14 2v5a1 1 0 0 0 1 1h5"></path>"#
);
/// Pdf: Lucide `file-text`.
pub static PDF: Icon = line24!(
    "file-icon-pdf",
    r#"<path d="M6 22a2 2 0 0 1-2-2V4a2 2 0 0 1 2-2h8a2.4 2.4 0 0 1 1.704.706l3.588 3.588A2.4 2.4 0 0 1 20 8v12a2 2 0 0 1-2 2z"></path><path d="M14 2v5a1 1 0 0 0 1 1h5"></path><path d="M10 9H8"></path><path d="M16 13H8"></path><path d="M16 17H8"></path>"#
);
/// Word: Lucide `file-pen-line`.
pub static WORD: Icon = line24!(
    "file-icon-word",
    r#"<path d="M14.364 13.634a2 2 0 0 0-.506.854l-.837 2.87a.5.5 0 0 0 .62.62l2.87-.837a2 2 0 0 0 .854-.506l4.013-4.009a1 1 0 0 0-3.004-3.004z"></path><path d="M14.487 7.858A1 1 0 0 1 14 7V2"></path><path d="M20 19.645V20a2 2 0 0 1-2 2H6a2 2 0 0 1-2-2V4a2 2 0 0 1 2-2h8a2.4 2.4 0 0 1 1.704.706l2.516 2.516"></path><path d="M8 18h1"></path>"#
);
/// Text: Lucide `file-type`.
pub static TEXT: Icon = line24!(
    "file-icon-text",
    r#"<path d="M6 22a2 2 0 0 1-2-2V4a2 2 0 0 1 2-2h8a2.4 2.4 0 0 1 1.704.706l3.588 3.588A2.4 2.4 0 0 1 20 8v12a2 2 0 0 1-2 2z"></path><path d="M14 2v5a1 1 0 0 0 1 1h5"></path><path d="M11 18h2"></path><path d="M12 12v6"></path><path d="M9 13v-.5a.5.5 0 0 1 .5-.5h5a.5.5 0 0 1 .5.5v.5"></path>"#
);
/// Sheet: Lucide `file-spreadsheet`.
pub static SHEET: Icon = line24!(
    "file-icon-sheet",
    r#"<path d="M6 22a2 2 0 0 1-2-2V4a2 2 0 0 1 2-2h8a2.4 2.4 0 0 1 1.704.706l3.588 3.588A2.4 2.4 0 0 1 20 8v12a2 2 0 0 1-2 2z"></path><path d="M14 2v5a1 1 0 0 0 1 1h5"></path><path d="M8 13h2"></path><path d="M14 13h2"></path><path d="M8 17h2"></path><path d="M14 17h2"></path>"#
);
/// Csv: Lucide `sheet`.
pub static CSV: Icon = line24!(
    "file-icon-csv",
    r#"<rect width="18" height="18" x="3" y="3" rx="2" ry="2"></rect><line x1="3" x2="21" y1="9" y2="9"></line><line x1="3" x2="21" y1="15" y2="15"></line><line x1="9" x2="9" y1="9" y2="21"></line><line x1="15" x2="15" y1="9" y2="21"></line>"#
);
/// Slides: Lucide `presentation`.
pub static SLIDES: Icon = line24!(
    "file-icon-slides",
    r#"<path d="M2 3h20"></path><path d="M21 3v11a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2V3"></path><path d="m7 21 5-5 5 5"></path>"#
);
/// Markdown: Lucide `file-down`.
pub static MARKDOWN: Icon = line24!(
    "file-icon-markdown",
    r#"<path d="M6 22a2 2 0 0 1-2-2V4a2 2 0 0 1 2-2h8a2.4 2.4 0 0 1 1.704.706l3.588 3.588A2.4 2.4 0 0 1 20 8v12a2 2 0 0 1-2 2z"></path><path d="M14 2v5a1 1 0 0 0 1 1h5"></path><path d="M12 18v-6"></path><path d="m9 15 3 3 3-3"></path>"#
);
/// Image: Lucide `file-image`.
pub static IMAGE: Icon = line24!(
    "file-icon-image",
    r#"<path d="M6 22a2 2 0 0 1-2-2V4a2 2 0 0 1 2-2h8a2.4 2.4 0 0 1 1.704.706l3.588 3.588A2.4 2.4 0 0 1 20 8v12a2 2 0 0 1-2 2z"></path><path d="M14 2v5a1 1 0 0 0 1 1h5"></path><circle cx="10" cy="12" r="2"></circle><path d="m20 17-1.296-1.296a2.41 2.41 0 0 0-3.408 0L9 22"></path>"#
);
/// Svg: Lucide `pen-tool`.
pub static SVG: Icon = line24!(
    "file-icon-svg",
    r#"<path d="m18 13-.654-4.736a4 4 0 00-2.756-3.266l-9.163-2.9a2 2 0 00-2.017.493l-.82.819a2 2 0 00-.492 2.017l2.9 9.163a4 4 0 003.266 2.756L13 18"></path><path d="M18.648 12.352a1.205 1.205 0 011.704 0l1.296 1.296a1.205 1.205 0 010 1.704l-6.296 6.296a1.205 1.205 0 01-1.704 0l-1.296-1.296a1.205 1.205 0 010-1.704z"></path><path d="m3 3 6.586 6.586"></path><circle cx="11" cy="11" r="2"></circle>"#
);
/// Video: Lucide `file-play`.
pub static VIDEO: Icon = line24!(
    "file-icon-video",
    r#"<path d="M6 22a2 2 0 0 1-2-2V4a2 2 0 0 1 2-2h8a2.4 2.4 0 0 1 1.704.706l3.588 3.588A2.4 2.4 0 0 1 20 8v12a2 2 0 0 1-2 2z"></path><path d="M14 2v5a1 1 0 0 0 1 1h5"></path><path d="M15.033 13.44a.647.647 0 0 1 0 1.12l-4.065 2.352a.645.645 0 0 1-.968-.56v-4.704a.645.645 0 0 1 .967-.56z"></path>"#
);
/// Audio: Lucide `file-music`.
pub static AUDIO: Icon = line24!(
    "file-icon-audio",
    r#"<path d="M11.65 22H18a2 2 0 0 0 2-2V8a2.4 2.4 0 0 0-.706-1.706l-3.588-3.588A2.4 2.4 0 0 0 14 2H6a2 2 0 0 0-2 2v10.35"></path><path d="M14 2v5a1 1 0 0 0 1 1h5"></path><path d="M8 20v-7l3 1.474"></path><circle cx="6" cy="20" r="2"></circle>"#
);
/// Archive: Lucide `file-archive`.
pub static ARCHIVE: Icon = line24!(
    "file-icon-archive",
    r#"<path d="M13.659 22H18a2 2 0 0 0 2-2V8a2.4 2.4 0 0 0-.706-1.706l-3.588-3.588A2.4 2.4 0 0 0 14 2H6a2 2 0 0 0-2 2v11.5"></path><path d="M14 2v5a1 1 0 0 0 1 1h5"></path><path d="M8 12v-1"></path><path d="M8 18v-2"></path><path d="M8 7V6"></path><circle cx="8" cy="20" r="2"></circle>"#
);
/// DiskImage: Lucide `disc-3`.
pub static DISK_IMAGE: Icon = line24!(
    "file-icon-disk-image",
    r#"<circle cx="12" cy="12" r="10"></circle><path d="M6 12c0-1.7.7-3.2 1.8-4.2"></path><circle cx="12" cy="12" r="2"></circle><path d="M18 12c0 1.7-.7 3.2-1.8 4.2"></path>"#
);
/// Rust: Lucide `file-code`.
pub static RUST: Icon = line24!(
    "file-icon-rust",
    r#"<path d="M6 22a2 2 0 0 1-2-2V4a2 2 0 0 1 2-2h8a2.4 2.4 0 0 1 1.704.706l3.588 3.588A2.4 2.4 0 0 1 20 8v12a2 2 0 0 1-2 2z"></path><path d="M14 2v5a1 1 0 0 0 1 1h5"></path><path d="M10 12.5 8 15l2 2.5"></path><path d="m14 12.5 2 2.5-2 2.5"></path>"#
);
/// JavaScript: Lucide `file-code`.
pub static JAVA_SCRIPT: Icon = line24!(
    "file-icon-java-script",
    r#"<path d="M6 22a2 2 0 0 1-2-2V4a2 2 0 0 1 2-2h8a2.4 2.4 0 0 1 1.704.706l3.588 3.588A2.4 2.4 0 0 1 20 8v12a2 2 0 0 1-2 2z"></path><path d="M14 2v5a1 1 0 0 0 1 1h5"></path><path d="M10 12.5 8 15l2 2.5"></path><path d="m14 12.5 2 2.5-2 2.5"></path>"#
);
/// TypeScript: Lucide `file-code`.
pub static TYPE_SCRIPT: Icon = line24!(
    "file-icon-type-script",
    r#"<path d="M6 22a2 2 0 0 1-2-2V4a2 2 0 0 1 2-2h8a2.4 2.4 0 0 1 1.704.706l3.588 3.588A2.4 2.4 0 0 1 20 8v12a2 2 0 0 1-2 2z"></path><path d="M14 2v5a1 1 0 0 0 1 1h5"></path><path d="M10 12.5 8 15l2 2.5"></path><path d="m14 12.5 2 2.5-2 2.5"></path>"#
);
/// Python: Lucide `file-code`.
pub static PYTHON: Icon = line24!(
    "file-icon-python",
    r#"<path d="M6 22a2 2 0 0 1-2-2V4a2 2 0 0 1 2-2h8a2.4 2.4 0 0 1 1.704.706l3.588 3.588A2.4 2.4 0 0 1 20 8v12a2 2 0 0 1-2 2z"></path><path d="M14 2v5a1 1 0 0 0 1 1h5"></path><path d="M10 12.5 8 15l2 2.5"></path><path d="m14 12.5 2 2.5-2 2.5"></path>"#
);
/// Go: Lucide `file-code`.
pub static GO: Icon = line24!(
    "file-icon-go",
    r#"<path d="M6 22a2 2 0 0 1-2-2V4a2 2 0 0 1 2-2h8a2.4 2.4 0 0 1 1.704.706l3.588 3.588A2.4 2.4 0 0 1 20 8v12a2 2 0 0 1-2 2z"></path><path d="M14 2v5a1 1 0 0 0 1 1h5"></path><path d="M10 12.5 8 15l2 2.5"></path><path d="m14 12.5 2 2.5-2 2.5"></path>"#
);
/// Java: Lucide `file-code`.
pub static JAVA: Icon = line24!(
    "file-icon-java",
    r#"<path d="M6 22a2 2 0 0 1-2-2V4a2 2 0 0 1 2-2h8a2.4 2.4 0 0 1 1.704.706l3.588 3.588A2.4 2.4 0 0 1 20 8v12a2 2 0 0 1-2 2z"></path><path d="M14 2v5a1 1 0 0 0 1 1h5"></path><path d="M10 12.5 8 15l2 2.5"></path><path d="m14 12.5 2 2.5-2 2.5"></path>"#
);
/// Kotlin: Lucide `file-code`.
pub static KOTLIN: Icon = line24!(
    "file-icon-kotlin",
    r#"<path d="M6 22a2 2 0 0 1-2-2V4a2 2 0 0 1 2-2h8a2.4 2.4 0 0 1 1.704.706l3.588 3.588A2.4 2.4 0 0 1 20 8v12a2 2 0 0 1-2 2z"></path><path d="M14 2v5a1 1 0 0 0 1 1h5"></path><path d="M10 12.5 8 15l2 2.5"></path><path d="m14 12.5 2 2.5-2 2.5"></path>"#
);
/// Swift: Lucide `file-code`.
pub static SWIFT: Icon = line24!(
    "file-icon-swift",
    r#"<path d="M6 22a2 2 0 0 1-2-2V4a2 2 0 0 1 2-2h8a2.4 2.4 0 0 1 1.704.706l3.588 3.588A2.4 2.4 0 0 1 20 8v12a2 2 0 0 1-2 2z"></path><path d="M14 2v5a1 1 0 0 0 1 1h5"></path><path d="M10 12.5 8 15l2 2.5"></path><path d="m14 12.5 2 2.5-2 2.5"></path>"#
);
/// C: Lucide `file-code`.
pub static C: Icon = line24!(
    "file-icon-c",
    r#"<path d="M6 22a2 2 0 0 1-2-2V4a2 2 0 0 1 2-2h8a2.4 2.4 0 0 1 1.704.706l3.588 3.588A2.4 2.4 0 0 1 20 8v12a2 2 0 0 1-2 2z"></path><path d="M14 2v5a1 1 0 0 0 1 1h5"></path><path d="M10 12.5 8 15l2 2.5"></path><path d="m14 12.5 2 2.5-2 2.5"></path>"#
);
/// Cpp: Lucide `file-code`.
pub static CPP: Icon = line24!(
    "file-icon-cpp",
    r#"<path d="M6 22a2 2 0 0 1-2-2V4a2 2 0 0 1 2-2h8a2.4 2.4 0 0 1 1.704.706l3.588 3.588A2.4 2.4 0 0 1 20 8v12a2 2 0 0 1-2 2z"></path><path d="M14 2v5a1 1 0 0 0 1 1h5"></path><path d="M10 12.5 8 15l2 2.5"></path><path d="m14 12.5 2 2.5-2 2.5"></path>"#
);
/// CSharp: Lucide `file-code`.
pub static CSHARP: Icon = line24!(
    "file-icon-csharp",
    r#"<path d="M6 22a2 2 0 0 1-2-2V4a2 2 0 0 1 2-2h8a2.4 2.4 0 0 1 1.704.706l3.588 3.588A2.4 2.4 0 0 1 20 8v12a2 2 0 0 1-2 2z"></path><path d="M14 2v5a1 1 0 0 0 1 1h5"></path><path d="M10 12.5 8 15l2 2.5"></path><path d="m14 12.5 2 2.5-2 2.5"></path>"#
);
/// Php: Lucide `file-code`.
pub static PHP: Icon = line24!(
    "file-icon-php",
    r#"<path d="M6 22a2 2 0 0 1-2-2V4a2 2 0 0 1 2-2h8a2.4 2.4 0 0 1 1.704.706l3.588 3.588A2.4 2.4 0 0 1 20 8v12a2 2 0 0 1-2 2z"></path><path d="M14 2v5a1 1 0 0 0 1 1h5"></path><path d="M10 12.5 8 15l2 2.5"></path><path d="m14 12.5 2 2.5-2 2.5"></path>"#
);
/// Ruby: Lucide `file-code`.
pub static RUBY: Icon = line24!(
    "file-icon-ruby",
    r#"<path d="M6 22a2 2 0 0 1-2-2V4a2 2 0 0 1 2-2h8a2.4 2.4 0 0 1 1.704.706l3.588 3.588A2.4 2.4 0 0 1 20 8v12a2 2 0 0 1-2 2z"></path><path d="M14 2v5a1 1 0 0 0 1 1h5"></path><path d="M10 12.5 8 15l2 2.5"></path><path d="m14 12.5 2 2.5-2 2.5"></path>"#
);
/// Html: Lucide `file-code-corner`.
pub static HTML: Icon = line24!(
    "file-icon-html",
    r#"<path d="M4 12.15V4a2 2 0 0 1 2-2h8a2.4 2.4 0 0 1 1.706.706l3.588 3.588A2.4 2.4 0 0 1 20 8v12a2 2 0 0 1-2 2h-3.35"></path><path d="M14 2v5a1 1 0 0 0 1 1h5"></path><path d="m5 16-3 3 3 3"></path><path d="m9 22 3-3-3-3"></path>"#
);
/// Css: Lucide `file-code-corner`.
pub static CSS: Icon = line24!(
    "file-icon-css",
    r#"<path d="M4 12.15V4a2 2 0 0 1 2-2h8a2.4 2.4 0 0 1 1.706.706l3.588 3.588A2.4 2.4 0 0 1 20 8v12a2 2 0 0 1-2 2h-3.35"></path><path d="M14 2v5a1 1 0 0 0 1 1h5"></path><path d="m5 16-3 3 3 3"></path><path d="m9 22 3-3-3-3"></path>"#
);
/// Code: Lucide `file-code`.
pub static CODE: Icon = line24!(
    "file-icon-code",
    r#"<path d="M6 22a2 2 0 0 1-2-2V4a2 2 0 0 1 2-2h8a2.4 2.4 0 0 1 1.704.706l3.588 3.588A2.4 2.4 0 0 1 20 8v12a2 2 0 0 1-2 2z"></path><path d="M14 2v5a1 1 0 0 0 1 1h5"></path><path d="M10 12.5 8 15l2 2.5"></path><path d="m14 12.5 2 2.5-2 2.5"></path>"#
);
/// Shell: Lucide `file-terminal`.
pub static SHELL: Icon = line24!(
    "file-icon-shell",
    r#"<path d="M6 22a2 2 0 0 1-2-2V4a2 2 0 0 1 2-2h8a2.4 2.4 0 0 1 1.704.706l3.588 3.588A2.4 2.4 0 0 1 20 8v12a2 2 0 0 1-2 2z"></path><path d="M14 2v5a1 1 0 0 0 1 1h5"></path><path d="m8 16 2-2-2-2"></path><path d="M12 18h4"></path>"#
);
/// Sql: Lucide `file-database`.
pub static SQL: Icon = line24!(
    "file-icon-sql",
    r#"<path d="M12 12v8a5 2 0 01-10 0v-8"></path><path d="M12 16a5 2 0 01-10 0"></path><path d="M14 2v5a1 1 0 001 1h5"></path><path d="M4 6.3V4a2 2 0 012-2h8a2.4 2.4 0 011.706.706l3.588 3.588A2.4 2.4 0 0120 8v12a2 2 0 01-2 2h-2.43"></path><ellipse cx="7" cy="12" rx="5" ry="2"></ellipse>"#
);
/// Database: Lucide `database`.
pub static DATABASE: Icon = line24!(
    "file-icon-database",
    r#"<ellipse cx="12" cy="5" rx="9" ry="3"></ellipse><path d="M3 5V19A9 3 0 0 0 21 19V5"></path><path d="M3 12A9 3 0 0 0 21 12"></path>"#
);
/// Json: Lucide `file-braces`.
pub static JSON: Icon = line24!(
    "file-icon-json",
    r#"<path d="M6 22a2 2 0 0 1-2-2V4a2 2 0 0 1 2-2h8a2.4 2.4 0 0 1 1.704.706l3.588 3.588A2.4 2.4 0 0 1 20 8v12a2 2 0 0 1-2 2z"></path><path d="M14 2v5a1 1 0 0 0 1 1h5"></path><path d="M10 12a1 1 0 0 0-1 1v1a1 1 0 0 1-1 1 1 1 0 0 1 1 1v1a1 1 0 0 0 1 1"></path><path d="M14 18a1 1 0 0 0 1-1v-1a1 1 0 0 1 1-1 1 1 0 0 1-1-1v-1a1 1 0 0 0-1-1"></path>"#
);
/// Yaml: Lucide `file-braces-corner`.
pub static YAML: Icon = line24!(
    "file-icon-yaml",
    r#"<path d="M14 22h4a2 2 0 0 0 2-2V8a2.4 2.4 0 0 0-.706-1.706l-3.588-3.588A2.4 2.4 0 0 0 14 2H6a2 2 0 0 0-2 2v6"></path><path d="M14 2v5a1 1 0 0 0 1 1h5"></path><path d="M5 14a1 1 0 0 0-1 1v2a1 1 0 0 1-1 1 1 1 0 0 1 1 1v2a1 1 0 0 0 1 1"></path><path d="M9 22a1 1 0 0 0 1-1v-2a1 1 0 0 1 1-1 1 1 0 0 1-1-1v-2a1 1 0 0 0-1-1"></path>"#
);
/// Toml: Lucide `file-sliders`.
pub static TOML: Icon = line24!(
    "file-icon-toml",
    r#"<path d="M6 22a2 2 0 0 1-2-2V4a2 2 0 0 1 2-2h8a2.4 2.4 0 0 1 1.704.706l3.588 3.588A2.4 2.4 0 0 1 20 8v12a2 2 0 0 1-2 2z"></path><path d="M14 2v5a1 1 0 0 0 1 1h5"></path><path d="M8 12h8"></path><path d="M10 11v2"></path><path d="M8 17h8"></path><path d="M14 16v2"></path>"#
);
/// Xml: Lucide `file-code-corner`.
pub static XML: Icon = line24!(
    "file-icon-xml",
    r#"<path d="M4 12.15V4a2 2 0 0 1 2-2h8a2.4 2.4 0 0 1 1.706.706l3.588 3.588A2.4 2.4 0 0 1 20 8v12a2 2 0 0 1-2 2h-3.35"></path><path d="M14 2v5a1 1 0 0 0 1 1h5"></path><path d="m5 16-3 3 3 3"></path><path d="m9 22 3-3-3-3"></path>"#
);
/// Config: Lucide `file-cog`.
pub static CONFIG: Icon = line24!(
    "file-icon-config",
    r#"<path d="M15 8a1 1 0 0 1-1-1V2a2.4 2.4 0 0 1 1.704.706l3.588 3.588A2.4 2.4 0 0 1 20 8z"></path><path d="M20 8v12a2 2 0 0 1-2 2h-4.182"></path><path d="m3.305 19.53.923-.382"></path><path d="M4 10.592V4a2 2 0 0 1 2-2h8"></path><path d="m4.228 16.852-.924-.383"></path><path d="m5.852 15.228-.383-.923"></path><path d="m5.852 20.772-.383.924"></path><path d="m8.148 15.228.383-.923"></path><path d="m8.53 21.696-.382-.924"></path><path d="m9.773 16.852.922-.383"></path><path d="m9.773 19.148.922.383"></path><circle cx="7" cy="18" r="3"></circle>"#
);
/// Lock: Lucide `file-lock`.
pub static LOCK: Icon = line24!(
    "file-icon-lock",
    r#"<path d="M4 9.8V4a2 2 0 0 1 2-2h8a2.4 2.4 0 0 1 1.706.706l3.588 3.588A2.4 2.4 0 0 1 20 8v12a2 2 0 0 1-2 2h-3"></path><path d="M14 2v5a1 1 0 0 0 1 1h5"></path><path d="M9 17v-2a2 2 0 0 0-4 0v2"></path><rect width="8" height="5" x="3" y="17" rx="1"></rect>"#
);
/// Design: Lucide `palette`.
pub static DESIGN: Icon = line24!(
    "file-icon-design",
    r#"<path d="M12 22a1 1 0 0 1 0-20 10 9 0 0 1 10 9 5 5 0 0 1-5 5h-2.25a1.75 1.75 0 0 0-1.4 2.8l.3.4a1.75 1.75 0 0 1-1.4 2.8z"></path><circle cx="13.5" cy="6.5" r=".5" fill="currentColor"></circle><circle cx="17.5" cy="10.5" r=".5" fill="currentColor"></circle><circle cx="6.5" cy="12.5" r=".5" fill="currentColor"></circle><circle cx="8.5" cy="7.5" r=".5" fill="currentColor"></circle>"#
);
/// Model3d: Lucide `box`.
pub static MODEL_3D: Icon = line24!(
    "file-icon-model-3d",
    r#"<path d="M21 8a2 2 0 0 0-1-1.73l-7-4a2 2 0 0 0-2 0l-7 4A2 2 0 0 0 3 8v8a2 2 0 0 0 1 1.73l7 4a2 2 0 0 0 2 0l7-4A2 2 0 0 0 21 16Z"></path><path d="m3.3 7 8.7 5 8.7-5"></path><path d="M12 22V12"></path>"#
);
/// Font: Lucide `file-type-corner`.
pub static FONT: Icon = line24!(
    "file-icon-font",
    r#"<path d="M12 22h6a2 2 0 0 0 2-2V8a2.4 2.4 0 0 0-.706-1.706l-3.588-3.588A2.4 2.4 0 0 0 14 2H6a2 2 0 0 0-2 2v6"></path><path d="M14 2v5a1 1 0 0 0 1 1h5"></path><path d="M3 16v-1.5a.5.5 0 0 1 .5-.5h7a.5.5 0 0 1 .5.5V16"></path><path d="M6 22h2"></path><path d="M7 14v8"></path>"#
);
/// Ebook: Lucide `book`.
pub static EBOOK: Icon = line24!(
    "file-icon-ebook",
    r#"<path d="M4 19.5v-15A2.5 2.5 0 0 1 6.5 2H19a1 1 0 0 1 1 1v18a1 1 0 0 1-1 1H6.5a1 1 0 0 1 0-5H20"></path>"#
);
/// Executable: Lucide `file-box`.
pub static EXECUTABLE: Icon = line24!(
    "file-icon-executable",
    r#"<path d="M14 2v5a1 1 0 001 1h5"></path><path d="M14.692 22H18a2 2 0 002-2V8a2.4 2.4 0 00-.706-1.706l-3.588-3.588A2.4 2.4 0 0014 2H6a2 2 0 00-2 2v3.804"></path><path d="M2.264 13.752 7 16.5l4.737-2.748"></path><path d="M2.995 13.014A2 2 0 002 14.744v3.516a2 2 0 00.996 1.73l3 1.74a2 2 0 002.008 0l3-1.74A2 2 0 0012 18.26v-3.517a2 2 0 00-.995-1.73l-3-1.742a2 2 0 00-1.892-.064z"></path><path d="M7 16.5V22"></path>"#
);
/// Key: Lucide `key-round`.
pub static KEY: Icon = line24!(
    "file-icon-key",
    r#"<path d="M2.586 17.414A2 2 0 0 0 2 18.828V21a1 1 0 0 0 1 1h3a1 1 0 0 0 1-1v-1a1 1 0 0 1 1-1h1a1 1 0 0 0 1-1v-1a1 1 0 0 1 1-1h.172a2 2 0 0 0 1.414-.586l.814-.814a6.5 6.5 0 1 0-4-4z"></path><circle cx="16.5" cy="7.5" r=".5" fill="currentColor"></circle>"#
);
/// Log: Lucide `scroll-text`.
pub static LOG: Icon = line24!(
    "file-icon-log",
    r#"<path d="M15 12h-5"></path><path d="M15 8h-5"></path><path d="M19 17V5a2 2 0 0 0-2-2H4"></path><path d="M8 21h12a2 2 0 0 0 2-2v-1a1 1 0 0 0-1-1H11a1 1 0 0 0-1 1v1a2 2 0 1 1-4 0V5a2 2 0 1 0-4 0v2a1 1 0 0 0 1 1h3"></path>"#
);
/// Calendar: Lucide `calendar`.
pub static CALENDAR: Icon = line24!(
    "file-icon-calendar",
    r#"<path d="M8 2v3"></path><path d="M16 2v3"></path><rect x="3" y="3" width="18" height="18" rx="2"></rect><path d="M3 9h18"></path>"#
);
/// Contact: Lucide `contact`.
pub static CONTACT: Icon = line24!(
    "file-icon-contact",
    r#"<path d="M16 2v2"></path><path d="M7 21v-2a2 2 0 012-2h6a2 2 0 012 2v2"></path><path d="M8 2v2"></path><circle cx="12" cy="10" r="3"></circle><rect x="3" y="3" width="18" height="18" rx="2"></rect>"#
);
/// Email: Lucide `mail`.
pub static EMAIL: Icon = line24!(
    "file-icon-email",
    r#"<path d="m22 7-8.991 5.727a2 2 0 0 1-2.009 0L2 7"></path><rect x="2" y="4" width="20" height="16" rx="2"></rect>"#
);
/// Docker: Lucide `container`.
pub static DOCKER: Icon = line24!(
    "file-icon-docker",
    r#"<path d="M22 7.7c0-.6-.4-1.2-.8-1.5l-6.3-3.9a1.72 1.72 0 0 0-1.7 0l-10.3 6c-.5.2-.9.8-.9 1.4v6.6c0 .5.4 1.2.8 1.5l6.3 3.9a1.72 1.72 0 0 0 1.7 0l10.3-6c.5-.3.9-1 .9-1.5Z"></path><path d="M10 21.9V14L2.1 9.1"></path><path d="m10 14 11.9-6.9"></path><path d="M14 19.8v-8.1"></path><path d="M18 17.5V9.4"></path>"#
);
/// Git: Lucide `git-branch`.
pub static GIT: Icon = line24!(
    "file-icon-git",
    r#"<path d="M15 6a9 9 0 0 0-9 9V3"></path><circle cx="18" cy="6" r="3"></circle><circle cx="6" cy="18" r="3"></circle>"#
);
/// Readme: Lucide `book-open`.
pub static README: Icon = line24!(
    "file-icon-readme",
    r#"<path d="M12 5v16"></path><path d="M20.001 19A2 2 0 0022 17V5a2 2 0 00-1.999-2L16 3.002A5 5 0 0012 5a5 5 0 00-4-2H4a2 2 0 00-2 2v12a2 2 0 001.999 2H8a5 5 0 014 2 5 5 0 014-2z"></path>"#
);
/// License: Lucide `file-badge`.
pub static LICENSE: Icon = line24!(
    "file-icon-license",
    r#"<path d="M13 22h5a2 2 0 0 0 2-2V8a2.4 2.4 0 0 0-.706-1.706l-3.588-3.588A2.4 2.4 0 0 0 14 2H6a2 2 0 0 0-2 2v3.3"></path><path d="M14 2v5a1 1 0 0 0 1 1h5"></path><path d="m7.69 16.479 1.29 4.88a.5.5 0 0 1-.698.591l-1.843-.849a1 1 0 0 0-.879.001l-1.846.85a.5.5 0 0 1-.692-.593l1.29-4.88"></path><circle cx="6" cy="14" r="3"></circle>"#
);
/// Env: Lucide `file-key`.
pub static ENV: Icon = line24!(
    "file-icon-env",
    r#"<path d="M14 2v5a1 1 0 0 0 1 1h5"></path><path d="M4 12v6"></path><path d="M4 14h2"></path><path d="M9.65 22H18a2 2 0 0 0 2-2V8a2.4 2.4 0 0 0-.706-1.706l-3.588-3.588A2.4 2.4 0 0 0 14 2H6a2 2 0 0 0-2 2v4"></path><circle cx="4" cy="20" r="2"></circle>"#
);
/// Folder: Lucide `folder`.
pub static FOLDER: Icon = line24!(
    "file-icon-folder",
    r#"<path d="M20 20a2 2 0 0 0 2-2V8a2 2 0 0 0-2-2h-7.9a2 2 0 0 1-1.69-.9L9.6 3.9A2 2 0 0 0 7.93 3H4a2 2 0 0 0-2 2v13a2 2 0 0 0 2 2Z" fill="currentColor" fill-opacity="0.18" stroke="none"></path><path d="M20 20a2 2 0 0 0 2-2V8a2 2 0 0 0-2-2h-7.9a2 2 0 0 1-1.69-.9L9.6 3.9A2 2 0 0 0 7.93 3H4a2 2 0 0 0-2 2v13a2 2 0 0 0 2 2Z"></path>"#
);
/// FolderPhotos: Lucide `folder + image`.
pub static FOLDER_PHOTOS: Icon = line24!(
    "file-icon-folder-photos",
    r#"<path d="M20 20a2 2 0 0 0 2-2V8a2 2 0 0 0-2-2h-7.9a2 2 0 0 1-1.69-.9L9.6 3.9A2 2 0 0 0 7.93 3H4a2 2 0 0 0-2 2v13a2 2 0 0 0 2 2Z" fill="currentColor" fill-opacity="0.18" stroke="none"></path><path d="M20 20a2 2 0 0 0 2-2V8a2 2 0 0 0-2-2h-7.9a2 2 0 0 1-1.69-.9L9.6 3.9A2 2 0 0 0 7.93 3H4a2 2 0 0 0-2 2v13a2 2 0 0 0 2 2Z"></path><g transform="translate(6 7) scale(0.5)" stroke-width="3.2"><rect width="18" height="18" x="3" y="3" rx="2" ry="2"></rect><circle cx="9" cy="9" r="2"></circle><path d="m21 15-3.086-3.086a2 2 0 0 0-2.828 0L6 21"></path></g>"#
);
/// FolderScreenshots: Lucide `folder + scan`.
pub static FOLDER_SCREENSHOTS: Icon = line24!(
    "file-icon-folder-screenshots",
    r#"<path d="M20 20a2 2 0 0 0 2-2V8a2 2 0 0 0-2-2h-7.9a2 2 0 0 1-1.69-.9L9.6 3.9A2 2 0 0 0 7.93 3H4a2 2 0 0 0-2 2v13a2 2 0 0 0 2 2Z" fill="currentColor" fill-opacity="0.18" stroke="none"></path><path d="M20 20a2 2 0 0 0 2-2V8a2 2 0 0 0-2-2h-7.9a2 2 0 0 1-1.69-.9L9.6 3.9A2 2 0 0 0 7.93 3H4a2 2 0 0 0-2 2v13a2 2 0 0 0 2 2Z"></path><g transform="translate(6 7) scale(0.5)" stroke-width="3.2"><path d="M3 7V5a2 2 0 0 1 2-2h2"></path><path d="M17 3h2a2 2 0 0 1 2 2v2"></path><path d="M21 17v2a2 2 0 0 1-2 2h-2"></path><path d="M7 21H5a2 2 0 0 1-2-2v-2"></path></g>"#
);
/// FolderVideos: Lucide `folder + play`.
pub static FOLDER_VIDEOS: Icon = line24!(
    "file-icon-folder-videos",
    r#"<path d="M20 20a2 2 0 0 0 2-2V8a2 2 0 0 0-2-2h-7.9a2 2 0 0 1-1.69-.9L9.6 3.9A2 2 0 0 0 7.93 3H4a2 2 0 0 0-2 2v13a2 2 0 0 0 2 2Z" fill="currentColor" fill-opacity="0.18" stroke="none"></path><path d="M20 20a2 2 0 0 0 2-2V8a2 2 0 0 0-2-2h-7.9a2 2 0 0 1-1.69-.9L9.6 3.9A2 2 0 0 0 7.93 3H4a2 2 0 0 0-2 2v13a2 2 0 0 0 2 2Z"></path><g transform="translate(6 7) scale(0.5)" stroke-width="3.2"><path d="M5 5a2 2 0 0 1 3.008-1.728l11.997 6.998a2 2 0 0 1 .003 3.458l-12 7A2 2 0 0 1 5 19z"></path></g>"#
);
/// FolderMusic: Lucide `folder + headphones`.
pub static FOLDER_MUSIC: Icon = line24!(
    "file-icon-folder-music",
    r#"<path d="M20 20a2 2 0 0 0 2-2V8a2 2 0 0 0-2-2h-7.9a2 2 0 0 1-1.69-.9L9.6 3.9A2 2 0 0 0 7.93 3H4a2 2 0 0 0-2 2v13a2 2 0 0 0 2 2Z" fill="currentColor" fill-opacity="0.18" stroke="none"></path><path d="M20 20a2 2 0 0 0 2-2V8a2 2 0 0 0-2-2h-7.9a2 2 0 0 1-1.69-.9L9.6 3.9A2 2 0 0 0 7.93 3H4a2 2 0 0 0-2 2v13a2 2 0 0 0 2 2Z"></path><g transform="translate(6 7) scale(0.5)" stroke-width="3.2"><path d="M3 14h3a2 2 0 0 1 2 2v3a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-7a9 9 0 0 1 18 0v7a2 2 0 0 1-2 2h-1a2 2 0 0 1-2-2v-3a2 2 0 0 1 2-2h3"></path></g>"#
);
/// FolderDocuments: Lucide `folder + file-text`.
pub static FOLDER_DOCUMENTS: Icon = line24!(
    "file-icon-folder-documents",
    r#"<path d="M20 20a2 2 0 0 0 2-2V8a2 2 0 0 0-2-2h-7.9a2 2 0 0 1-1.69-.9L9.6 3.9A2 2 0 0 0 7.93 3H4a2 2 0 0 0-2 2v13a2 2 0 0 0 2 2Z" fill="currentColor" fill-opacity="0.18" stroke="none"></path><path d="M20 20a2 2 0 0 0 2-2V8a2 2 0 0 0-2-2h-7.9a2 2 0 0 1-1.69-.9L9.6 3.9A2 2 0 0 0 7.93 3H4a2 2 0 0 0-2 2v13a2 2 0 0 0 2 2Z"></path><g transform="translate(6 7) scale(0.5)" stroke-width="3.2"><path d="M6 22a2 2 0 0 1-2-2V4a2 2 0 0 1 2-2h8a2.4 2.4 0 0 1 1.704.706l3.588 3.588A2.4 2.4 0 0 1 20 8v12a2 2 0 0 1-2 2z"></path><path d="M14 2v5a1 1 0 0 0 1 1h5"></path><path d="M10 9H8"></path><path d="M16 13H8"></path><path d="M16 17H8"></path></g>"#
);
/// FolderDownloads: Lucide `folder-down`.
pub static FOLDER_DOWNLOADS: Icon = line24!(
    "file-icon-folder-downloads",
    r#"<path d="M20 20a2 2 0 0 0 2-2V8a2 2 0 0 0-2-2h-7.9a2 2 0 0 1-1.69-.9L9.6 3.9A2 2 0 0 0 7.93 3H4a2 2 0 0 0-2 2v13a2 2 0 0 0 2 2Z" fill="currentColor" fill-opacity="0.18" stroke="none"></path><path d="M20 20a2 2 0 0 0 2-2V8a2 2 0 0 0-2-2h-7.9a2 2 0 0 1-1.69-.9L9.6 3.9A2 2 0 0 0 7.93 3H4a2 2 0 0 0-2 2v13a2 2 0 0 0 2 2Z"></path><path d="M12 10v6"></path><path d="m15 13-3 3-3-3"></path>"#
);
/// FolderDesktop: Lucide `folder + monitor`.
pub static FOLDER_DESKTOP: Icon = line24!(
    "file-icon-folder-desktop",
    r#"<path d="M20 20a2 2 0 0 0 2-2V8a2 2 0 0 0-2-2h-7.9a2 2 0 0 1-1.69-.9L9.6 3.9A2 2 0 0 0 7.93 3H4a2 2 0 0 0-2 2v13a2 2 0 0 0 2 2Z" fill="currentColor" fill-opacity="0.18" stroke="none"></path><path d="M20 20a2 2 0 0 0 2-2V8a2 2 0 0 0-2-2h-7.9a2 2 0 0 1-1.69-.9L9.6 3.9A2 2 0 0 0 7.93 3H4a2 2 0 0 0-2 2v13a2 2 0 0 0 2 2Z"></path><g transform="translate(6 7) scale(0.5)" stroke-width="3.2"><rect width="20" height="14" x="2" y="3" rx="2"></rect><line x1="8" x2="16" y1="21" y2="21"></line><line x1="12" x2="12" y1="17" y2="21"></line></g>"#
);
/// FolderBackups: Lucide `folder-sync`.
pub static FOLDER_BACKUPS: Icon = line24!(
    "file-icon-folder-backups",
    r#"<path d="M9 20H4a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h3.9a2 2 0 0 1 1.69.9l.81 1.2a2 2 0 0 0 1.67.9H20a2 2 0 0 1 2 2v.5" fill="currentColor" fill-opacity="0.18" stroke="none"></path><path d="M9 20H4a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h3.9a2 2 0 0 1 1.69.9l.81 1.2a2 2 0 0 0 1.67.9H20a2 2 0 0 1 2 2v.5"></path><path d="M12 10v4h4"></path><path d="m12 14 1.535-1.605a5 5 0 0 1 8 1.5"></path><path d="M22 22v-4h-4"></path><path d="m22 18-1.535 1.605a5 5 0 0 1-8-1.5"></path>"#
);
/// FolderGit: Lucide `folder-git-2`.
pub static FOLDER_GIT: Icon = line24!(
    "file-icon-folder-git",
    r#"<path d="M9 20H4a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h3.9a2 2 0 0 1 1.69.9l.81 1.2a2 2 0 0 0 1.67.9H20a2 2 0 0 1 2 2v5" fill="currentColor" fill-opacity="0.18" stroke="none"></path><path d="M18 19a5 5 0 0 1-5-5v8"></path><path d="M9 20H4a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h3.9a2 2 0 0 1 1.69.9l.81 1.2a2 2 0 0 0 1.67.9H20a2 2 0 0 1 2 2v5"></path><circle cx="13" cy="12" r="2"></circle><circle cx="20" cy="19" r="2"></circle>"#
);
/// FolderNodeModules: Lucide `folder + hexagon`.
pub static FOLDER_NODE_MODULES: Icon = line24!(
    "file-icon-folder-node-modules",
    r#"<path d="M20 20a2 2 0 0 0 2-2V8a2 2 0 0 0-2-2h-7.9a2 2 0 0 1-1.69-.9L9.6 3.9A2 2 0 0 0 7.93 3H4a2 2 0 0 0-2 2v13a2 2 0 0 0 2 2Z" fill="currentColor" fill-opacity="0.18" stroke="none"></path><path d="M20 20a2 2 0 0 0 2-2V8a2 2 0 0 0-2-2h-7.9a2 2 0 0 1-1.69-.9L9.6 3.9A2 2 0 0 0 7.93 3H4a2 2 0 0 0-2 2v13a2 2 0 0 0 2 2Z"></path><g transform="translate(6 7) scale(0.5)" stroke-width="3.2"><path d="M21 16V8a2 2 0 0 0-1-1.73l-7-4a2 2 0 0 0-2 0l-7 4A2 2 0 0 0 3 8v8a2 2 0 0 0 1 1.73l7 4a2 2 0 0 0 2 0l7-4A2 2 0 0 0 21 16z"></path></g>"#
);
/// FolderSrc: Lucide `folder-code`.
pub static FOLDER_SRC: Icon = line24!(
    "file-icon-folder-src",
    r#"<path d="M20 20a2 2 0 0 0 2-2V8a2 2 0 0 0-2-2h-7.9a2 2 0 0 1-1.69-.9L9.6 3.9A2 2 0 0 0 7.93 3H4a2 2 0 0 0-2 2v13a2 2 0 0 0 2 2z" fill="currentColor" fill-opacity="0.18" stroke="none"></path><path d="M10 10.5 8 13l2 2.5"></path><path d="m14 10.5 2 2.5-2 2.5"></path><path d="M20 20a2 2 0 0 0 2-2V8a2 2 0 0 0-2-2h-7.9a2 2 0 0 1-1.69-.9L9.6 3.9A2 2 0 0 0 7.93 3H4a2 2 0 0 0-2 2v13a2 2 0 0 0 2 2z"></path>"#
);
/// FolderTests: Lucide `folder + flask-conical`.
pub static FOLDER_TESTS: Icon = line24!(
    "file-icon-folder-tests",
    r#"<path d="M20 20a2 2 0 0 0 2-2V8a2 2 0 0 0-2-2h-7.9a2 2 0 0 1-1.69-.9L9.6 3.9A2 2 0 0 0 7.93 3H4a2 2 0 0 0-2 2v13a2 2 0 0 0 2 2Z" fill="currentColor" fill-opacity="0.18" stroke="none"></path><path d="M20 20a2 2 0 0 0 2-2V8a2 2 0 0 0-2-2h-7.9a2 2 0 0 1-1.69-.9L9.6 3.9A2 2 0 0 0 7.93 3H4a2 2 0 0 0-2 2v13a2 2 0 0 0 2 2Z"></path><g transform="translate(6 7) scale(0.5)" stroke-width="3.2"><path d="M14 2v6a2 2 0 0 0 .245.96l5.51 10.08A2 2 0 0 1 18 22H6a2 2 0 0 1-1.755-2.96l5.51-10.08A2 2 0 0 0 10 8V2"></path><path d="M6.453 15h11.094"></path><path d="M8.5 2h7"></path></g>"#
);
/// FolderDocs: Lucide `folder + book-open`.
pub static FOLDER_DOCS: Icon = line24!(
    "file-icon-folder-docs",
    r#"<path d="M20 20a2 2 0 0 0 2-2V8a2 2 0 0 0-2-2h-7.9a2 2 0 0 1-1.69-.9L9.6 3.9A2 2 0 0 0 7.93 3H4a2 2 0 0 0-2 2v13a2 2 0 0 0 2 2Z" fill="currentColor" fill-opacity="0.18" stroke="none"></path><path d="M20 20a2 2 0 0 0 2-2V8a2 2 0 0 0-2-2h-7.9a2 2 0 0 1-1.69-.9L9.6 3.9A2 2 0 0 0 7.93 3H4a2 2 0 0 0-2 2v13a2 2 0 0 0 2 2Z"></path><g transform="translate(6 7) scale(0.5)" stroke-width="3.2"><path d="M12 5v16"></path><path d="M20.001 19A2 2 0 0022 17V5a2 2 0 00-1.999-2L16 3.002A5 5 0 0012 5a5 5 0 00-4-2H4a2 2 0 00-2 2v12a2 2 0 001.999 2H8a5 5 0 014 2 5 5 0 014-2z"></path></g>"#
);
/// FolderBuild: Lucide `folder + box`.
pub static FOLDER_BUILD: Icon = line24!(
    "file-icon-folder-build",
    r#"<path d="M20 20a2 2 0 0 0 2-2V8a2 2 0 0 0-2-2h-7.9a2 2 0 0 1-1.69-.9L9.6 3.9A2 2 0 0 0 7.93 3H4a2 2 0 0 0-2 2v13a2 2 0 0 0 2 2Z" fill="currentColor" fill-opacity="0.18" stroke="none"></path><path d="M20 20a2 2 0 0 0 2-2V8a2 2 0 0 0-2-2h-7.9a2 2 0 0 1-1.69-.9L9.6 3.9A2 2 0 0 0 7.93 3H4a2 2 0 0 0-2 2v13a2 2 0 0 0 2 2Z"></path><g transform="translate(6 7) scale(0.5)" stroke-width="3.2"><path d="M21 8a2 2 0 0 0-1-1.73l-7-4a2 2 0 0 0-2 0l-7 4A2 2 0 0 0 3 8v8a2 2 0 0 0 1 1.73l7 4a2 2 0 0 0 2 0l7-4A2 2 0 0 0 21 16Z"></path><path d="m3.3 7 8.7 5 8.7-5"></path><path d="M12 22V12"></path></g>"#
);
/// FolderPublic: Lucide `folder + globe`.
pub static FOLDER_PUBLIC: Icon = line24!(
    "file-icon-folder-public",
    r#"<path d="M20 20a2 2 0 0 0 2-2V8a2 2 0 0 0-2-2h-7.9a2 2 0 0 1-1.69-.9L9.6 3.9A2 2 0 0 0 7.93 3H4a2 2 0 0 0-2 2v13a2 2 0 0 0 2 2Z" fill="currentColor" fill-opacity="0.18" stroke="none"></path><path d="M20 20a2 2 0 0 0 2-2V8a2 2 0 0 0-2-2h-7.9a2 2 0 0 1-1.69-.9L9.6 3.9A2 2 0 0 0 7.93 3H4a2 2 0 0 0-2 2v13a2 2 0 0 0 2 2Z"></path><g transform="translate(6 7) scale(0.5)" stroke-width="3.2"><circle cx="12" cy="12" r="10"></circle><path d="M12 2a14.5 14.5 0 0 0 0 20 14.5 14.5 0 0 0 0-20"></path><path d="M2 12h20"></path></g>"#
);
/// FolderAssets: Lucide `folder + shapes`.
pub static FOLDER_ASSETS: Icon = line24!(
    "file-icon-folder-assets",
    r#"<path d="M20 20a2 2 0 0 0 2-2V8a2 2 0 0 0-2-2h-7.9a2 2 0 0 1-1.69-.9L9.6 3.9A2 2 0 0 0 7.93 3H4a2 2 0 0 0-2 2v13a2 2 0 0 0 2 2Z" fill="currentColor" fill-opacity="0.18" stroke="none"></path><path d="M20 20a2 2 0 0 0 2-2V8a2 2 0 0 0-2-2h-7.9a2 2 0 0 1-1.69-.9L9.6 3.9A2 2 0 0 0 7.93 3H4a2 2 0 0 0-2 2v13a2 2 0 0 0 2 2Z"></path><g transform="translate(6 7) scale(0.5)" stroke-width="3.2"><path d="M8.3 10a.7.7 0 0 1-.626-1.079L11.4 3a.7.7 0 0 1 1.198-.043L16.3 8.9a.7.7 0 0 1-.572 1.1Z"></path><rect x="3" y="14" width="7" height="7" rx="1"></rect><circle cx="17.5" cy="17.5" r="3.5"></circle></g>"#
);
/// FolderScripts: Lucide `folder + terminal`.
pub static FOLDER_SCRIPTS: Icon = line24!(
    "file-icon-folder-scripts",
    r#"<path d="M20 20a2 2 0 0 0 2-2V8a2 2 0 0 0-2-2h-7.9a2 2 0 0 1-1.69-.9L9.6 3.9A2 2 0 0 0 7.93 3H4a2 2 0 0 0-2 2v13a2 2 0 0 0 2 2Z" fill="currentColor" fill-opacity="0.18" stroke="none"></path><path d="M20 20a2 2 0 0 0 2-2V8a2 2 0 0 0-2-2h-7.9a2 2 0 0 1-1.69-.9L9.6 3.9A2 2 0 0 0 7.93 3H4a2 2 0 0 0-2 2v13a2 2 0 0 0 2 2Z"></path><g transform="translate(6 7) scale(0.5)" stroke-width="3.2"><path d="M12 19h8"></path><path d="m4 17 6-6-6-6"></path></g>"#
);
/// FolderConfig: Lucide `folder-cog`.
pub static FOLDER_CONFIG: Icon = line24!(
    "file-icon-folder-config",
    r#"<path d="M10.3 20H4a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h3.98a2 2 0 0 1 1.69.9l.66 1.2A2 2 0 0 0 12 6h8a2 2 0 0 1 2 2v3.3" fill="currentColor" fill-opacity="0.18" stroke="none"></path><path d="M10.3 20H4a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h3.98a2 2 0 0 1 1.69.9l.66 1.2A2 2 0 0 0 12 6h8a2 2 0 0 1 2 2v3.3"></path><path d="m14.305 19.53.923-.382"></path><path d="m15.228 16.852-.923-.383"></path><path d="m16.852 15.228-.383-.923"></path><path d="m16.852 20.772-.383.924"></path><path d="m19.148 15.228.383-.923"></path><path d="m19.53 21.696-.382-.924"></path><path d="m20.772 16.852.924-.383"></path><path d="m20.772 19.148.924.383"></path><circle cx="18" cy="18" r="3"></circle>"#
);
/// FolderGithub: Lucide `folder + workflow`.
pub static FOLDER_GITHUB: Icon = line24!(
    "file-icon-folder-github",
    r#"<path d="M20 20a2 2 0 0 0 2-2V8a2 2 0 0 0-2-2h-7.9a2 2 0 0 1-1.69-.9L9.6 3.9A2 2 0 0 0 7.93 3H4a2 2 0 0 0-2 2v13a2 2 0 0 0 2 2Z" fill="currentColor" fill-opacity="0.18" stroke="none"></path><path d="M20 20a2 2 0 0 0 2-2V8a2 2 0 0 0-2-2h-7.9a2 2 0 0 1-1.69-.9L9.6 3.9A2 2 0 0 0 7.93 3H4a2 2 0 0 0-2 2v13a2 2 0 0 0 2 2Z"></path><g transform="translate(6 7) scale(0.5)" stroke-width="3.2"><rect width="8" height="8" x="3" y="3" rx="2"></rect><path d="M7 11v4a2 2 0 0 0 2 2h4"></path><rect width="8" height="8" x="13" y="13" rx="2"></rect></g>"#
);
/// FolderVscode: Lucide `folder + braces`.
pub static FOLDER_VSCODE: Icon = line24!(
    "file-icon-folder-vscode",
    r#"<path d="M20 20a2 2 0 0 0 2-2V8a2 2 0 0 0-2-2h-7.9a2 2 0 0 1-1.69-.9L9.6 3.9A2 2 0 0 0 7.93 3H4a2 2 0 0 0-2 2v13a2 2 0 0 0 2 2Z" fill="currentColor" fill-opacity="0.18" stroke="none"></path><path d="M20 20a2 2 0 0 0 2-2V8a2 2 0 0 0-2-2h-7.9a2 2 0 0 1-1.69-.9L9.6 3.9A2 2 0 0 0 7.93 3H4a2 2 0 0 0-2 2v13a2 2 0 0 0 2 2Z"></path><g transform="translate(6 7) scale(0.5)" stroke-width="3.2"><path d="M8 3H7a2 2 0 0 0-2 2v5a2 2 0 0 1-2 2 2 2 0 0 1 2 2v5c0 1.1.9 2 2 2h1"></path><path d="M16 21h1a2 2 0 0 0 2-2v-5c0-1.1.9-2 2-2a2 2 0 0 1-2-2V5a2 2 0 0 0-2-2h-1"></path></g>"#
);
/// FolderLib: Lucide `folder + library`.
pub static FOLDER_LIB: Icon = line24!(
    "file-icon-folder-lib",
    r#"<path d="M20 20a2 2 0 0 0 2-2V8a2 2 0 0 0-2-2h-7.9a2 2 0 0 1-1.69-.9L9.6 3.9A2 2 0 0 0 7.93 3H4a2 2 0 0 0-2 2v13a2 2 0 0 0 2 2Z" fill="currentColor" fill-opacity="0.18" stroke="none"></path><path d="M20 20a2 2 0 0 0 2-2V8a2 2 0 0 0-2-2h-7.9a2 2 0 0 1-1.69-.9L9.6 3.9A2 2 0 0 0 7.93 3H4a2 2 0 0 0-2 2v13a2 2 0 0 0 2 2Z"></path><g transform="translate(6 7) scale(0.5)" stroke-width="3.2"><path d="m16 6 4 14"></path><path d="M12 6v14"></path><path d="M8 8v12"></path><path d="M4 4v16"></path></g>"#
);
/// FolderArchive: Lucide `folder-archive`.
pub static FOLDER_ARCHIVE: Icon = line24!(
    "file-icon-folder-archive",
    r#"<path d="M20.9 19.8A2 2 0 0 0 22 18V8a2 2 0 0 0-2-2h-7.9a2 2 0 0 1-1.69-.9L9.6 3.9A2 2 0 0 0 7.93 3H4a2 2 0 0 0-2 2v13a2 2 0 0 0 2 2h5.1" fill="currentColor" fill-opacity="0.18" stroke="none"></path><circle cx="15" cy="19" r="2"></circle><path d="M20.9 19.8A2 2 0 0 0 22 18V8a2 2 0 0 0-2-2h-7.9a2 2 0 0 1-1.69-.9L9.6 3.9A2 2 0 0 0 7.93 3H4a2 2 0 0 0-2 2v13a2 2 0 0 0 2 2h5.1"></path><path d="M15 11v-1"></path><path d="M15 17v-2"></path>"#
);
/// FolderShared: Lucide `folder + users`.
pub static FOLDER_SHARED: Icon = line24!(
    "file-icon-folder-shared",
    r#"<path d="M20 20a2 2 0 0 0 2-2V8a2 2 0 0 0-2-2h-7.9a2 2 0 0 1-1.69-.9L9.6 3.9A2 2 0 0 0 7.93 3H4a2 2 0 0 0-2 2v13a2 2 0 0 0 2 2Z" fill="currentColor" fill-opacity="0.18" stroke="none"></path><path d="M20 20a2 2 0 0 0 2-2V8a2 2 0 0 0-2-2h-7.9a2 2 0 0 1-1.69-.9L9.6 3.9A2 2 0 0 0 7.93 3H4a2 2 0 0 0-2 2v13a2 2 0 0 0 2 2Z"></path><g transform="translate(6 7) scale(0.5)" stroke-width="3.2"><path d="M16 21v-2a4 4 0 0 0-4-4H6a4 4 0 0 0-4 4v2"></path><path d="M16 3.128a4 4 0 0 1 0 7.744"></path><path d="M22 21v-2a4 4 0 0 0-3-3.87"></path><circle cx="9" cy="7" r="4"></circle></g>"#
);
/// FolderTemplates: Lucide `folder + layout-template`.
pub static FOLDER_TEMPLATES: Icon = line24!(
    "file-icon-folder-templates",
    r#"<path d="M20 20a2 2 0 0 0 2-2V8a2 2 0 0 0-2-2h-7.9a2 2 0 0 1-1.69-.9L9.6 3.9A2 2 0 0 0 7.93 3H4a2 2 0 0 0-2 2v13a2 2 0 0 0 2 2Z" fill="currentColor" fill-opacity="0.18" stroke="none"></path><path d="M20 20a2 2 0 0 0 2-2V8a2 2 0 0 0-2-2h-7.9a2 2 0 0 1-1.69-.9L9.6 3.9A2 2 0 0 0 7.93 3H4a2 2 0 0 0-2 2v13a2 2 0 0 0 2 2Z"></path><g transform="translate(6 7) scale(0.5)" stroke-width="3.2"><rect width="18" height="7" x="3" y="3" rx="1"></rect><rect width="9" height="7" x="3" y="14" rx="1"></rect><rect width="5" height="7" x="16" y="14" rx="1"></rect></g>"#
);
/// FolderProjects: Lucide `folder-kanban`.
pub static FOLDER_PROJECTS: Icon = line24!(
    "file-icon-folder-projects",
    r#"<path d="M4 20h16a2 2 0 0 0 2-2V8a2 2 0 0 0-2-2h-7.93a2 2 0 0 1-1.66-.9l-.82-1.2A2 2 0 0 0 7.93 3H4a2 2 0 0 0-2 2v13c0 1.1.9 2 2 2Z" fill="currentColor" fill-opacity="0.18" stroke="none"></path><path d="M4 20h16a2 2 0 0 0 2-2V8a2 2 0 0 0-2-2h-7.93a2 2 0 0 1-1.66-.9l-.82-1.2A2 2 0 0 0 7.93 3H4a2 2 0 0 0-2 2v13c0 1.1.9 2 2 2Z"></path><path d="M8 10v4"></path><path d="M12 10v2"></path><path d="M16 10v6"></path>"#
);

pub fn art(id: IconId) -> &'static Icon {
    match id {
        IconId::File => &FILE,
        IconId::Pdf => &PDF,
        IconId::Word => &WORD,
        IconId::Text => &TEXT,
        IconId::Sheet => &SHEET,
        IconId::Csv => &CSV,
        IconId::Slides => &SLIDES,
        IconId::Markdown => &MARKDOWN,
        IconId::Image => &IMAGE,
        IconId::Svg => &SVG,
        IconId::Video => &VIDEO,
        IconId::Audio => &AUDIO,
        IconId::Archive => &ARCHIVE,
        IconId::DiskImage => &DISK_IMAGE,
        IconId::Rust => &RUST,
        IconId::JavaScript => &JAVA_SCRIPT,
        IconId::TypeScript => &TYPE_SCRIPT,
        IconId::Python => &PYTHON,
        IconId::Go => &GO,
        IconId::Java => &JAVA,
        IconId::Kotlin => &KOTLIN,
        IconId::Swift => &SWIFT,
        IconId::C => &C,
        IconId::Cpp => &CPP,
        IconId::CSharp => &CSHARP,
        IconId::Php => &PHP,
        IconId::Ruby => &RUBY,
        IconId::Html => &HTML,
        IconId::Css => &CSS,
        IconId::Code => &CODE,
        IconId::Shell => &SHELL,
        IconId::Sql => &SQL,
        IconId::Database => &DATABASE,
        IconId::Json => &JSON,
        IconId::Yaml => &YAML,
        IconId::Toml => &TOML,
        IconId::Xml => &XML,
        IconId::Config => &CONFIG,
        IconId::Lock => &LOCK,
        IconId::Design => &DESIGN,
        IconId::Model3d => &MODEL_3D,
        IconId::Font => &FONT,
        IconId::Ebook => &EBOOK,
        IconId::Executable => &EXECUTABLE,
        IconId::Key => &KEY,
        IconId::Log => &LOG,
        IconId::Calendar => &CALENDAR,
        IconId::Contact => &CONTACT,
        IconId::Email => &EMAIL,
        IconId::Docker => &DOCKER,
        IconId::Git => &GIT,
        IconId::Readme => &README,
        IconId::License => &LICENSE,
        IconId::Env => &ENV,
        IconId::Folder => &FOLDER,
        IconId::FolderPhotos => &FOLDER_PHOTOS,
        IconId::FolderScreenshots => &FOLDER_SCREENSHOTS,
        IconId::FolderVideos => &FOLDER_VIDEOS,
        IconId::FolderMusic => &FOLDER_MUSIC,
        IconId::FolderDocuments => &FOLDER_DOCUMENTS,
        IconId::FolderDownloads => &FOLDER_DOWNLOADS,
        IconId::FolderDesktop => &FOLDER_DESKTOP,
        IconId::FolderBackups => &FOLDER_BACKUPS,
        IconId::FolderGit => &FOLDER_GIT,
        IconId::FolderNodeModules => &FOLDER_NODE_MODULES,
        IconId::FolderSrc => &FOLDER_SRC,
        IconId::FolderTests => &FOLDER_TESTS,
        IconId::FolderDocs => &FOLDER_DOCS,
        IconId::FolderBuild => &FOLDER_BUILD,
        IconId::FolderPublic => &FOLDER_PUBLIC,
        IconId::FolderAssets => &FOLDER_ASSETS,
        IconId::FolderScripts => &FOLDER_SCRIPTS,
        IconId::FolderConfig => &FOLDER_CONFIG,
        IconId::FolderGithub => &FOLDER_GITHUB,
        IconId::FolderVscode => &FOLDER_VSCODE,
        IconId::FolderLib => &FOLDER_LIB,
        IconId::FolderArchive => &FOLDER_ARCHIVE,
        IconId::FolderShared => &FOLDER_SHARED,
        IconId::FolderTemplates => &FOLDER_TEMPLATES,
        IconId::FolderProjects => &FOLDER_PROJECTS,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_icon_art_is_complete_and_single_tint() {
        let mut names = std::collections::HashSet::new();
        for &id in IconId::ALL {
            let icon = art(id);
            assert!(names.insert(icon.name), "{id:?}: name {} is not unique", icon.name);
            assert_eq!(icon.view, 24.0, "{id:?}");
            assert!(icon.body.len() > 120, "{id:?}: empty body");
            assert!(!icon.body.contains("fill=\"#"), "{id:?}: names a fill colour");
            assert!(!icon.body.contains("stroke=\"#"), "{id:?}: names a stroke colour");
        }
    }
}
