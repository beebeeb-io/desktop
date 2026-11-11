//! Details panel (`.mk-det`): a preview, the name, kind and size, the encryption mark, the
//! facts, the Available offline switch and the two actions.

use crate::source::DriveItem;
use crate::theme::icons;
use crate::theme::tokens::{Theme, radius, type_};
use rune::{Color, El, Icon, Span, col, el, row};

use super::kit;

const PANE_W: f32 = 284.0;
const PREVIEW_H: f32 = 152.0;
const KV_LABEL_W: f32 = 92.0;

/// The panel's shell: 284 pt of ground with a hairline at its leading edge.
pub fn pane(t: &Theme) -> El {
    col()
        .key("detail-pane")
        .role("complementary")
        .label("Details")
        .w(PANE_W)
        .h_full()
        .fill(t.surface)
        .border_sides([0.0, 0.0, 0.0, kit::HAIR], t.separator)
        .border_layout()
        .padding(16.0)
        .gap(12.0)
}

pub fn detail_pane(item: Option<&DriveItem>, t: &Theme) -> El {
    let Some(item) = item else {
        return el().key("detail-pane-empty");
    };

    let kind = kind_of(item);
    let size = if item.is_folder {
        item.child_count.map(|c| format!("{c} items")).unwrap_or_default()
    } else {
        item.size.map(crate::source::format_size).unwrap_or_default()
    };
    // "PDF document · 1.2 MB": the figure is mono, as every size is.
    let kind_line = if size.is_empty() {
        kind.to_string()
    } else {
        format!("{kind} \u{00B7} {size}")
    };
    let mono_from = kind_line.len() - size.len();
    let figure = Span {
        start: mono_from as u32,
        end: kind_line.len() as u32,
        mono: true,
        ..Span::default()
    };

    pane(t)
        .child(preview(page_preview(t), t))
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
        // The board's sample facts for the selected file.
        .child(
            col()
                .key("detail-meta")
                .role("list")
                .gap(6.0)
                .child(meta_row("detail-changed", "Changed", "Yesterday, 16:42", true, t))
                .child(meta_row("detail-changed-on", "Changed on", "Anna's iPhone", false, t))
                .child(meta_row("detail-added", "Added", "28 Oct 2026", true, t))
                .child(meta_row("detail-where", "Where", "Files", false, t))
                .child(meta_row(
                    "detail-version-history",
                    "Version history",
                    "3 versions",
                    true,
                    t,
                ))
                .child(meta_row("detail-shared", "Shared", "Sam Okafor, pending", false, t)),
        )
        .child(switch_card(
            "offline",
            "Available offline",
            &format!("Online only now \u{00B7} {size}"),
            false,
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

/// The preview well (`.mk-prev`) around whatever stands for the file.
pub fn preview(content: El, t: &Theme) -> El {
    el().key("preview")
        .role("image")
        .label("Preview")
        .w_full()
        .h(PREVIEW_H)
        .radius(radius::LG)
        .clip_rounded(radius::LG)
        .fill(t.well)
        .center()
        .child(content)
}

/// A document's first page, drawn (`.mk-page`): white in both themes, a heading and lines.
pub fn page_preview(t: &Theme) -> El {
    let widths = [1.0, 1.0, 0.8, 1.0, 0.65, 1.0, 0.9];
    let inner = 96.0 - 2.0 * 10.0;
    let mut page = col()
        .key("preview-page")
        .size(96.0, 124.0)
        .radius(radius::SM)
        .fill(t.white)
        .box_shadow(kit::drop(2.0, 10.0, t.shadow_page))
        .py(12.0)
        .px(10.0)
        .gap(5.0)
        .child(
            el().key("preview-line-0")
                .h(6.0)
                .w(inner * 0.6)
                .radius(2.0)
                .fill(Color::BLACK.alpha(0.30))
                .mb(3.0),
        );
    for (i, w) in widths.iter().enumerate() {
        page = page.child(
            el().key(format!("preview-line-{}", i + 1))
                .h(3.0)
                .w(inner * w)
                .radius(2.0)
                .fill(t.shadow_page),
        );
    }
    page
}

/// The encryption mark (`.mk-enc`): one of the two Amber things in the window.
pub fn encrypted_badge(t: &Theme) -> El {
    row()
        .key("encrypted-badge")
        .self_start()
        .h(24.0)
        .pl(7.0)
        .pr(9.0)
        .gap(6.0)
        .center_y()
        .radius(radius::CONTROL)
        .fill(t.accent_wash)
        .child(kit::glyph("lock-icon", &icons::LOCK, 13.0, t.accent_deep))
        .child(kit::label(
            "encrypted-text",
            "End-to-end encrypted",
            type_::BODY_SEMI.lh(16.0),
            t.accent_deep,
        ))
}

/// One fact (`.mk-kv`): a 92 pt label and its value; figures (`mono`) in JetBrains Mono.
/// Keys: `<key>` and `<key>-value`.
pub fn meta_row(key: &'static str, label: &'static str, value: &str, mono: bool, t: &Theme) -> El {
    row()
        .key(format!("meta-row-{key}"))
        .role("listitem")
        .label(format!("{label}: {value}"))
        .w_full()
        .gap(8.0)
        .child(kit::label(key, label, type_::BODY, t.text_tertiary).w(KV_LABEL_W))
        .child(
            kit::label(
                format!("{key}-value"),
                value,
                if mono { type_::META_MONO } else { type_::BODY },
                t.text,
            )
            .grow(1.0)
            .min_w(0.0)
            .truncate(),
        )
}

/// A switch (the kit's Switch): 36 x 20, off is a recessed track, on is ink.
/// Keys: `<key>-switch`, `<key>-knob`.
pub fn switch(key: &str, name: &str, on: bool, t: &Theme) -> El {
    row()
        .key(format!("{key}-switch"))
        .role("switch")
        .label(name)
        .checked(on)
        .size(36.0, 20.0)
        .radius(10.0)
        .fill(if on { t.text } else { t.paper3 })
        .border(1.0, if on { t.text } else { t.separator_strong })
        .border_layout()
        .center_y()
        .focusable(true)
        .child(
            el().key(format!("{key}-knob"))
                .size(14.0, 14.0)
                .radius(7.0)
                .fill(t.white)
                .box_shadow(kit::drop(1.0, 3.0, t.shadow))
                .ml(if on { 18.0 } else { 2.0 }),
        )
}

/// A card with one switch row (`.kit-list-body` > `.kit-item`): a title, a second line and
/// the switch. Keys: `<key>-card`, `<key>-label`, `<key>-detail`, `<key>-switch`.
pub fn switch_card(key: &str, title: &str, sub: &str, on: bool, t: &Theme) -> El {
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
        .child(switch(key, title, on, t))
}

/// A small outlined button (`.kit-btn.kit-btn-sm`). Keys: `<key>-box`, `<key>-icon`, `<key>-btn`.
pub fn action_button(key: &str, label: &str, ic: &'static Icon, t: &Theme) -> El {
    row()
        .key(format!("{key}-box"))
        .role("button")
        .label(label)
        .h(22.0)
        .px(12.0)
        .gap(7.0)
        .center()
        .radius(radius::MD)
        .fill(t.paper)
        .border(1.0, t.card_border)
        .hover(t.hover)
        .press(t.press)
        .focusable(true)
        .child(kit::glyph(format!("{key}-icon"), ic, 14.0, t.text))
        .child(kit::label(format!("{key}-btn"), label, type_::BUTTON_SM, t.text))
}

fn kind_of(item: &DriveItem) -> &'static str {
    if item.is_folder {
        return "Folder";
    }
    match item.mime.as_deref() {
        Some("application/pdf") => "PDF document",
        Some(m) if m.contains("spreadsheet") => "Spreadsheet",
        Some(m) if m.contains("presentation") || m.contains("keynote") => "Presentation",
        Some(m) if m.starts_with("image/heic") || m.starts_with("image/heif") => "HEIC image",
        Some(m) if m.starts_with("image/") => "Image",
        Some(m) if m.starts_with("video/") => "Video",
        Some(m) if m.starts_with("audio/") => "Audio",
        Some(m) if m.contains("document") => "Document",
        _ => "File",
    }
}
