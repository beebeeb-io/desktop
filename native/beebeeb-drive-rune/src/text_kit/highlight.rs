//! Syntax highlight spans for a whole document.

use rune_kit::syntax::{self, SyntaxTheme};

/// The highlight language for a file name, if any.
pub fn language_for(file_name: &str) -> Option<&'static str> {
    syntax::lang_for_path(file_name)
}

/// Spans for the whole document, relative to `text`. Empty when the file has no language.
pub fn spans(file_name: &str, text: &str, dark: bool) -> Vec<rune::Span> {
    let Some(lang) = language_for(file_name) else {
        return Vec::new();
    };
    let found = syntax::highlight(Some(lang), text);
    let mut out = SyntaxTheme::neutral(dark).spans(&found, 0);
    out.retain(|s| (s.end as usize) <= text.len() && s.start < s.end);
    out
}

#[cfg(test)]
#[path = "highlight_tests.rs"]
mod tests;
