//! A text or Markdown file that is not in the drive, shown in the viewer: the licence notice beside the exe
//! (`frames::open_file`). The path is the id. Read whole (up to [`MAX_BYTES`]), as text, or as formatted Markdown for
//! `.md`. Nothing is written, nothing is decrypted, and nothing leaves this PC.

use std::path::{Path, PathBuf};

use super::source::{Body, Details, MdDoc, OpenError, OpenFile, TextDoc, ViewerSource, plain_lines};

/// The most it reads: the shipped notice is about 1.3 MB.
pub const MAX_BYTES: u64 = 4 * 1024 * 1024;

pub struct LocalFile;

impl LocalFile {
    fn read(path: &Path) -> Result<OpenFile, OpenError> {
        let meta = std::fs::metadata(path).map_err(|_| OpenError::NotFound)?;
        if !meta.is_file() || meta.len() > MAX_BYTES {
            return Err(OpenError::NotNow);
        }
        let text =
            String::from_utf8(std::fs::read(path).map_err(|_| OpenError::NotNow)?).map_err(|_| OpenError::NotNow)?;
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let markdown = path
            .extension()
            .and_then(|e| e.to_str())
            .is_some_and(|e| matches!(e.to_ascii_lowercase().as_str(), "md" | "markdown"));
        let size = format!("{} KB", meta.len().div_ceil(1024));
        let body = if markdown {
            Body::Markdown(MdDoc { source: text })
        } else {
            Body::Text(TextDoc {
                lines: plain_lines(&text),
                truncated: None,
            })
        };
        let kind = if markdown { "Markdown" } else { "Text" };
        Ok(OpenFile {
            id: path.to_string_lossy().into_owned(),
            path: Vec::new(),
            name,
            facts: format!("{kind} \u{00B7} {size}"),
            version: 0,
            body,
            details: Details {
                kind: kind.to_string(),
                size,
                facts: Vec::new(),
                offline: false,
            },
            newer: None,
        })
    }
}

impl ViewerSource for LocalFile {
    fn open(&mut self, file_id: &str) -> Result<OpenFile, OpenError> {
        Self::read(&PathBuf::from(file_id))
    }
    fn poll(&mut self, _: &str) -> Option<Body> {
        None
    }
    fn cancel(&mut self, _: &str) {}
    fn retry(&mut self, file_id: &str) -> Result<OpenFile, OpenError> {
        self.open(file_id)
    }
    fn can_save(&self) -> bool {
        false
    }
    fn saves_copies(&self) -> bool {
        false
    }
    fn previews(&self, name: &str) -> bool {
        crate::frames::open_file::local_has_preview(Path::new(name))
    }
}

#[cfg(test)]
mod tests {
    use super::super::markdown::{Block, parse};

    /// The notice the installer ships is formatted Markdown: headings, lists, links and code blocks, not raw text.
    #[test]
    fn the_shipped_licence_notice_renders_as_markdown_blocks() {
        let notice = include_str!("../../../../../packaging/licences/THIRD-PARTY-LICENSES.md");
        let blocks = parse(notice);
        let count = |f: fn(&Block) -> bool| blocks.iter().filter(|b| f(b)).count();
        assert!(
            matches!(blocks.first(), Some(Block::Heading(1, _))),
            "starts with its title"
        );
        assert!(count(|b| matches!(b, Block::Heading(2, _))) >= 6, "its six sections");
        assert!(count(|b| matches!(b, Block::List(_))) > 100, "lists of crates");
        assert!(
            count(|b| matches!(b, Block::Code(_))) > 200,
            "licence texts as code blocks"
        );
    }
}
