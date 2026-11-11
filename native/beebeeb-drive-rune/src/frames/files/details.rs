//! The details panel of the selected item (`.mk-det`), its facts from `FilesSource::details`.
//! Its three controls act on the selection: the Available offline switch, Share… (DW-0407) and
//! Version history (DW-0413).

use super::Intent;
use super::source::Details;
use crate::State;
use crate::source::DriveItem;
use crate::theme::icons;
use crate::theme::tokens::{Theme, radius, type_};
use crate::ui::detail_pane::{action_button, encrypted_badge, meta_row, page_preview, pane, preview, switch};
use crate::ui::{file_list, kit};
use rune::{El, Span, Tx, col, el, row};

/// `ui::detail_pane::switch_card`, with a switch that switches: a card with a title, a second line
/// and the switch. Keys: `<key>-card`, `<key>-label`, `<key>-detail`, `<key>-switch`, `<key>-knob`.
fn offline_card(key: &str, title: &str, sub: &str, on: bool, t: &Theme) -> El {
    row()
        .key(format!("{key}-card"))
        .w_full()
        .min_h(40.0)
        .px(10.0)
        .gap(8.0)
        .center_y()
        .radius(radius::LG)
        .fill(t.card)
        .box_shadow(kit::ring(kit::HAIR, t.separator))
        .child(
            col()
                .key(format!("{key}-text"))
                .grow(1.0)
                .min_w(0.0)
                .gap(1.0)
                .child(kit::label(format!("{key}-label"), title, type_::ITEM_TITLE, t.text).truncate())
                .child(kit::label(format!("{key}-detail"), sub, type_::BODY.lh(15.0), t.text_tertiary).truncate()),
        )
        .child(
            switch(key, title, on, t)
                .description(sub)
                .on_press_tx(|s: &mut State, tx: &mut Tx| {
                    s.toggle_keep_offline(tx);
                }),
        )
}

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
        .child(offline_card(
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
                .child(
                    action_button("share", "Share\u{2026}", &icons::SHARE, t)
                        .on_press(|s: &mut State| s.intent(Intent::Frame("DW-0407"))),
                )
                .child(
                    action_button("versions", "Version history", &icons::VERSION_HISTORY, t)
                        .on_press(|s: &mut State| s.intent(Intent::Frame("DW-0413"))),
                ),
        )
}
