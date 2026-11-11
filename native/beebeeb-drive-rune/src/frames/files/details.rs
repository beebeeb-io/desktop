//! The details panel of the selected item (`.mk-det`), its facts from `FilesSource::details`.

use super::source::Details;
use crate::source::DriveItem;
use crate::theme::icons;
use crate::theme::tokens::{Theme, type_};
use crate::ui::detail_pane::{action_button, encrypted_badge, meta_row, page_preview, pane, preview, switch_card};
use crate::ui::{file_list, kit};
use rune::{El, Span, col, el, row};

pub fn details_pane(item: Option<(&DriveItem, Details)>, t: &Theme) -> El {
    let Some((item, d)) = item else {
        return el().key("detail-pane-empty");
    };
    let kind_line = if d.size.is_empty() {
        d.kind.clone()
    } else {
        format!("{} \u{00B7} {}", d.kind, d.size)
    };
    // The size is a figure: JetBrains Mono inside the Inter line.
    let figure = Span {
        start: (kind_line.len() - d.size.len()) as u32,
        end: kind_line.len() as u32,
        mono: true,
        ..Span::default()
    };
    let picture = if d.picture {
        // The file's own picture fills the well (`.mk-prev.mk-ph`).
        file_list::thumbnail(preview(el().key("preview-picture"), t))
    } else {
        preview(page_preview(t), t)
    };

    let mut facts = col().key("detail-meta").role("list").gap(6.0).child(meta_row(
        "detail-changed",
        "Changed",
        &d.changed,
        true,
        t,
    ));
    if let Some(v) = &d.changed_on {
        facts = facts.child(meta_row("detail-changed-on", "Changed on", v, false, t));
    }
    if let Some(v) = &d.added {
        facts = facts.child(meta_row("detail-added", "Added", v, true, t));
    }
    facts = facts.child(meta_row("detail-where", "Where", &d.where_, false, t));
    if let Some(v) = &d.versions {
        facts = facts.child(meta_row("detail-version-history", "Version history", v, true, t));
    }
    facts = facts.child(meta_row("detail-shared", "Shared", &d.shared, false, t));

    pane(t)
        .child(picture)
        .child(
            col()
                .key("detail-name-group")
                .child(kit::label("detail-name", item.name.clone(), type_::HEAD, t.text).truncate())
                .child(
                    kit::label("detail-kind", kind_line, type_::CAPTION, t.text_tertiary)
                        .spans(vec![figure])
                        .mt(2.0),
                ),
        )
        .child(encrypted_badge(t))
        .child(facts)
        .child(switch_card(
            "offline",
            "Available offline",
            &d.offline_note,
            d.kept_offline,
            t,
        ))
        .child(
            row()
                .key("detail-actions")
                .gap(8.0)
                .child(action_button("share", "Share\u{2026}", &icons::SHARE, t))
                .child(action_button("versions", "Version history", &icons::VERSION_HISTORY, t)),
        )
}
