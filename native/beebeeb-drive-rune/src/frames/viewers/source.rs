//! `ViewerSource`: what an open file asks of the engine, and the board's fixture.
//!
//! The viewer never reads a file itself. The source hands it what is already decrypted IN MEMORY
//! (text, pages, pixels) or the reason there is nothing to show; nothing here writes a decrypted
//! byte to disk, and no content goes into a log or an accessibility label beyond what is on screen.
//!
//! The fixture holds exactly the board's samples (DW-1501 to DW-1527): invented data is forbidden
//! where the board shows data.

/// One open file, as the viewer draws it.
#[derive(Clone, Debug, PartialEq)]
pub struct OpenFile {
    pub id: String,
    /// The folders above the file, outermost first ("Files", "Lisbon trip").
    pub path: Vec<String>,
    pub name: String,
    /// The line under the path: "Video · 120 MB".
    pub facts: String,
    pub body: Body,
}

/// What the content area shows.
#[derive(Clone, Debug, PartialEq)]
pub enum Body {
    /// DW-1516: opening takes over about a second. Bytes done and in all.
    Decrypting { done: u64, total: u64 },
    /// DW-1517: the decrypted copy does not fit on this PC.
    TooLarge { size: u64, free: u64 },
    /// DW-1518: decrypting failed. The copy in Beebeeb is untouched.
    Failed,
    /// DW-1512, DW-1527: text or code, decrypted in memory.
    Text(TextDoc),
}

/// Which of the theme's inks a run of text is drawn in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ink {
    /// Keywords, names, bold words.
    Strong,
    Plain,
    /// Comments, Markdown marks.
    Faint,
    /// Strings.
    Green,
    /// Numbers.
    Red,
}

/// A run of one line in one ink.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Run {
    pub text: String,
    pub ink: Ink,
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
}

/// A text file as the viewer draws it: the lines it holds, coloured.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TextDoc {
    pub lines: Vec<Vec<Run>>,
    /// The file is over the editor's limit (2 MB): read-only, the first lines only (DW-1527).
    pub truncated: Option<Truncated>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Truncated {
    /// Lines held (500).
    pub shown: u32,
    /// Lines in the file.
    pub total: u32,
}

impl TextDoc {
    /// The text as it is in the file: what Copy puts on the clipboard.
    pub fn plain(&self) -> String {
        let mut out = String::new();
        for line in &self.lines {
            for run in line {
                out.push_str(&run.text);
            }
            out.push('\n');
        }
        out
    }
}

/// A text with no colours: every line one plain run (what a file without a tokenizer shows).
#[cfg_attr(not(test), allow(dead_code))] // the engine's source uses it
pub fn plain_lines(text: &str) -> Vec<Vec<Run>> {
    text.lines()
        .map(|line| {
            if line.is_empty() {
                Vec::new()
            } else {
                vec![Run {
                    text: line.to_string(),
                    ink: Ink::Plain,
                    bold: false,
                    italic: false,
                    underline: false,
                }]
            }
        })
        .collect()
}

/// Why a file could not be opened at all.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OpenError {
    /// The engine has no such item (deleted elsewhere, or not this account's).
    NotFound,
}

pub trait ViewerSource {
    /// Start opening a file. The answer is what can be shown NOW; a `Decrypting` body is
    /// followed by [`ViewerSource::poll`].
    fn open(&mut self, file_id: &str) -> Result<OpenFile, OpenError>;
    /// Progress of the open file since the last call, if anything changed.
    #[cfg_attr(not(test), allow(dead_code))] // called by `viewers::poll`, which waits for the shell
    fn poll(&mut self, file_id: &str) -> Option<Body>;
    /// Stop decrypting. Nothing is kept half-decrypted.
    fn cancel(&mut self, file_id: &str);
    /// Download and decrypt again after a failure.
    fn retry(&mut self, file_id: &str) -> Result<OpenFile, OpenError>;
}

const MB: u64 = 1_000_000;
const GB: u64 = 1_000_000_000;

/// The ids of the board's sample files.
pub mod board {
    /// DW-1516.
    pub const DECRYPTING: &str = "tram-28.mov";
    /// DW-1517.
    pub const TOO_LARGE: &str = "studio-walkthrough-4k.mov";
    /// DW-1518.
    pub const FAILED: &str = "invoice-0139.pdf";
    /// DW-1512.
    pub const CODE: &str = "export_invoices.py";
    /// DW-1527.
    pub const LOG: &str = "server.log";
}

/// The board's samples. `calls` records what the viewer asked, for tests.
#[derive(Default)]
pub struct FixtureViewerSource {
    pub calls: Vec<String>,
}

impl FixtureViewerSource {
    pub fn new() -> Self {
        Self::default()
    }

    fn file(id: &str) -> Option<OpenFile> {
        let f = |path: &[&str], name: &str, facts: &str, body: Body| OpenFile {
            id: id.to_string(),
            path: path.iter().map(|p| p.to_string()).collect(),
            name: name.to_string(),
            facts: facts.to_string(),
            body,
        };
        Some(match id {
            board::DECRYPTING => f(
                &["Files", "Lisbon trip"],
                "Tram 28.mov",
                "Video \u{00B7} 120 MB",
                Body::Decrypting {
                    done: 46 * MB,
                    total: 120 * MB,
                },
            ),
            board::TOO_LARGE => f(
                &["Files", "Documents"],
                "Studio walkthrough 4K.mov",
                "Video \u{00B7} 18.2 GB",
                Body::TooLarge {
                    size: 18_200 * MB,
                    free: 12_400 * MB,
                },
            ),
            board::FAILED => f(
                &["Files", "Invoices 2026"],
                "Invoice 0139.pdf",
                "PDF \u{00B7} 220 KB",
                Body::Failed,
            ),
            board::CODE => f(
                &["Files", "Documents"],
                "export_invoices.py",
                "Python \u{00B7} 25 lines \u{00B7} UTF-8",
                Body::Text(TextDoc {
                    lines: super::fixture_text::python(),
                    truncated: None,
                }),
            ),
            board::LOG => f(
                &["Files", "Documents"],
                "server.log",
                "Log \u{00B7} 6.4 MB \u{00B7} read-only",
                Body::Text(TextDoc {
                    lines: super::fixture_text::log(),
                    truncated: Some(Truncated {
                        shown: 500,
                        total: 41_206,
                    }),
                }),
            ),
            _ => return None,
        })
    }
}

impl ViewerSource for FixtureViewerSource {
    fn open(&mut self, file_id: &str) -> Result<OpenFile, OpenError> {
        self.calls.push(format!("open {file_id}"));
        Self::file(file_id).ok_or(OpenError::NotFound)
    }

    fn poll(&mut self, _: &str) -> Option<Body> {
        None
    }

    fn cancel(&mut self, file_id: &str) {
        self.calls.push(format!("cancel {file_id}"));
    }

    /// The fixture's second try gets as far as the board's DW-1516 draws.
    fn retry(&mut self, file_id: &str) -> Result<OpenFile, OpenError> {
        self.calls.push(format!("retry {file_id}"));
        let mut file = Self::file(file_id).ok_or(OpenError::NotFound)?;
        file.body = Body::Decrypting {
            done: 0,
            total: 220_000,
        };
        Ok(file)
    }
}

/// "18.2 GB", "120 MB", "220 KB": decimal units, one decimal from a gigabyte up, as the board writes sizes.
pub fn size_words(bytes: u64) -> String {
    if bytes >= GB {
        format!("{:.1} GB", bytes as f64 / GB as f64)
    } else if bytes >= MB {
        format!("{} MB", bytes / MB)
    } else {
        format!("{} KB", bytes / 1000)
    }
}

/// "46 of 120 MB": the part done in the unit of the whole.
pub fn progress_words(done: u64, total: u64) -> String {
    let whole = size_words(total);
    let unit = if total >= GB {
        GB
    } else if total >= MB {
        MB
    } else {
        1000
    };
    let part = if total >= GB {
        format!("{:.1}", done as f64 / unit as f64)
    } else {
        (done / unit).to_string()
    };
    format!("{part} of {whole}")
}

/// "41,206": a count with thousands separators.
pub fn count_words(n: u32) -> String {
    let digits = n.to_string();
    let mut out = String::new();
    for (i, ch) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(ch);
    }
    out
}

/// The whole percent done, as the bar draws it.
pub fn percent(done: u64, total: u64) -> u32 {
    if total == 0 {
        return 0;
    }
    (done.min(total) * 100 / total) as u32
}
