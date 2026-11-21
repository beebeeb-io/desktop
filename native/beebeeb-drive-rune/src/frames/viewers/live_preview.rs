//! The live preview beside the editor: which files have one, and the pane that shows it.

use rune::{El, col};

use super::super::Ui;
use super::{live_html, live_json, live_md, live_svg, live_table};
use crate::theme::tokens::Theme;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PreviewKind {
    Markdown,
    Table,
    Json,
    Svg,
    Html,
}

/// The preview a file of this name has, if any.
pub fn kind_of(name: &str) -> Option<PreviewKind> {
    let ext = name.rsplit_once('.')?.1.to_ascii_lowercase();
    Some(match ext.as_str() {
        "md" | "markdown" => PreviewKind::Markdown,
        "csv" | "tsv" => PreviewKind::Table,
        "json" => PreviewKind::Json,
        "svg" => PreviewKind::Svg,
        "html" | "htm" => PreviewKind::Html,
        _ => return None,
    })
}

/// The pane: `source` as its kind shows it.
pub fn pane(kind: PreviewKind, source: &str, ui: &Ui, t: &Theme) -> El {
    let view = match kind {
        PreviewKind::Markdown => live_md::view(source, ui, t),
        PreviewKind::Table => live_table::view(source, ui, t),
        PreviewKind::Json => live_json::view(source, ui, t),
        PreviewKind::Svg => live_svg::view(source, ui, t),
        PreviewKind::Html => live_html::view(source, ui, t),
    };
    col()
        .key("live-preview")
        .role("region")
        .label("Preview")
        .w_full()
        .h_full()
        .min_w(0.0)
        .fill(t.bg)
        .border(1.0, t.separator)
        .border_layout()
        .child(view)
}
