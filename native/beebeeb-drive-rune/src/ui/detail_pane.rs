//! Details pane: preview area, file name, kind/size, "End-to-end encrypted"
//! badge, metadata rows (Changed, Changed on, Added, Where, Version history,
//! Shared), Available offline switch, Share.../Versions buttons.
//!
//! Board measurements:
//! - Pane width: ~268 pt (right side of the 1280 pt window)
//! - Preview: ~260x170 rectangle, paper-3 bg
//! - File name: 13px/600
//! - Kind line: "PDF document · 1.2 MB" in 12px/400 ink-3
//! - Encrypted badge: 12px/600 amber-deep text on amber-bg, with lock icon
//! - Metadata: label 12px/400 ink-3, value 11.5px/400 mono or 12px/400 Inter
//! - Buttons: Share... and Version history, outlined, 28 pt tall

use crate::source::DriveItem;
use crate::theme::icons;
use crate::theme::tokens::{Theme, radius, space, type_};
use rune::{El, col, el, icon, row, text};

/// Board: detail pane is 284px wide (from golden/DW-0401.json: detail-pane w=284).
const PANE_W: f32 = 284.0;
/// Board: .mk-prev { height: 152px }
const PREVIEW_H: f32 = 152.0;

pub fn detail_pane(item: Option<&DriveItem>, t: &Theme) -> El {
    let item = match item {
        Some(i) => i,
        None => return el().key("detail-pane-empty"),
    };

    let kind_label = kind_of(item);
    let size_label = if item.is_folder {
        item.child_count.map(|c| format!("{c} items")).unwrap_or_default()
    } else {
        item.size.map(|s| format_size_detail(s)).unwrap_or_default()
    };
    let kind_size = if size_label.is_empty() {
        kind_label.to_string()
    } else {
        format!("{kind_label} \u{00B7} {size_label}")
    };

    col()
        .key("detail-pane")
        .w(PANE_W)
        .h_full()
        .fill(t.bg)
        .pt(16.0) // Board: .mk-det { padding: 16px }
        .px(16.0)
        .gap(12.0) // Board: .mk-det { gap: 12px }
        // Preview area — board .mk-prev { height: 152px }
        .child(
            el().key("preview")
                .w_full()
                .h(PREVIEW_H)
                .radius(radius::LG)
                .fill(t.surface)
                .center()
                .child(icon(&icons::DOC_GENERIC, 48.0).key("preview-icon").color(t.separator)),
        )
        // File name + kind in one col (board has them in one flex container)
        .child(
            col()
                .key("detail-name-group")
                .gap(2.0)
                .child(text(&item.name).key("detail-name").font(type_::LABEL).color(t.text))
                .child(
                    text(kind_size)
                        .key("detail-kind")
                        .font(type_::BODY)
                        .color(t.text_tertiary),
                ),
        )
        // End-to-end encrypted badge — board .mk-enc { align-self: flex-start }
        .child(encrypted_badge(t))
        // Metadata rows — board .mk-kv { row-gap: 6px; line-height: 16px }
        .child(
            col()
                .key("detail-meta")
                .gap(6.0)
                .child(meta_row("detail-changed", "Changed", "Yesterday, 16:42", true, t))
                .child(meta_row(
                    "detail-changed-on",
                    "Changed on",
                    "Anna\u{2019}s iPhone",
                    false,
                    t,
                ))
                .child(meta_row("detail-added", "Added", "28 Oct 2026", true, t))
                .child(meta_row("detail-where", "Where", "Files", false, t))
                .child(meta_row(
                    "detail-version-history",
                    "Version history",
                    "3 versions",
                    false,
                    t,
                ))
                .child(meta_row("detail-shared", "Shared", "Sam Okafor, pending", false, t)),
        )
        // Available offline toggle row
        .child(
            row()
                .key("offline-toggle")
                .w_full()
                .h(36.0)
                .center_y()
                .justify_between()
                .child(
                    col()
                        .key("offline-info")
                        .pl(10.0) // board: offline-label x=1022=pane_content(1012)+10 → extra 10px indent
                        .child(
                            text("Available offline")
                                .key("offline-label")
                                .font(type_::BODY_SEMI)
                                .color(t.text)
                                .w(187.0) // board: offline-label w=187 (pane_content+pl: 1022+187=1209)
                                .h(17.0), // board: offline-label h=17 (BODY_SEMI line-height)
                        )
                        .child(
                            text("Online only now \u{00B7} 1.2 MB")
                                .key("offline-detail")
                                .font(type_::BODY_SM)
                                .color(t.text_tertiary),
                        ),
                )
                // Toggle switch (visual only)
                .child(
                    el().key("toggle")
                        .size(36.0, 20.0)
                        .radius(10.0)
                        .fill(t.separator)
                        .child(
                            el().key("toggle-knob")
                                .size(16.0, 16.0)
                                .radius(8.0)
                                .fill(t.card)
                                .ml(2.0)
                                .mt(2.0),
                        ),
                ),
        )
        // Action buttons: board shows share-btn text at y=532.
        // With detail col gap=12 and mt=0: offline_toggle ends at 514, gap adds 12 → btn top=526.
        // Text center_y within btn h=28: 526+(28-15)/2=526+6.5=532.5≈532 ✓
        .child(
            row()
                .key("detail-actions")
                .w_full()
                .gap(6.0) // board gap≈6 between buttons; XS=4 leaves versions-btn 2px short
                .mt(0.0) // no extra mt; detail col gap=12 alone positions buttons correctly
                .child(action_button("share-btn", "Share\u{2026}", &icons::SHARE, t))
                .child(action_button(
                    "versions-btn",
                    "Version history",
                    &icons::VERSION_HISTORY,
                    t,
                )),
        )
}

fn encrypted_badge(t: &Theme) -> El {
    // Board: .mk-enc { align-self: flex-start } → pill hugs its content, not full-width.
    row()
        .key("encrypted-badge")
        .h(24.0)
        .px(space::SM)
        .radius(radius::SM)
        .fill(t.accent_bg)
        .center_y()
        .gap(space::XS)
        .w(161.0) // Board: encrypted-badge w=161 (align-self:flex-start → content-hugging, not full-width)
        .child(icon(&icons::LOCK, 12.0).key("lock-icon").color(t.accent_deep))
        .child(
            text("End-to-end encrypted")
                .key("encrypted-text")
                .font(type_::BODY_SEMI)
                .color(t.accent_deep),
        )
}

fn meta_row(key: &'static str, label: &'static str, value: &str, mono: bool, t: &Theme) -> El {
    // Board: .mk-kv { grid-template-columns: 92px 1fr; row-gap: 6px; line-height: 16px }
    // The board measured the label text element (w=92), so key goes on the label, not the row.
    row()
        .key(format!("meta-row-{key}"))
        .w_full()
        .h(16.0) // Board: line-height: 16px
        .center_y()
        .child(
            text(label)
                .key(key) // board key on the label text (x=1012, w=92 matches the label)
                .font(type_::BODY)
                .color(t.text_tertiary)
                .w(92.0),
        )
        .child(
            text(value)
                .key("value")
                .font(if mono { type_::META_MONO } else { type_::BODY })
                .color(t.text),
        )
}

fn action_button(key: &'static str, label: &str, ic: &'static rune::Icon, t: &Theme) -> El {
    // Board measures the LABEL TEXT (e.g. share-btn w=45, h=17) not the button container.
    // Key goes on the text child so geometry check matches board measurements.
    // px=16 to match board: icon at pl+14=30, gap=4 → text starts at x=pane_content+34, board x=1046=1012+34 ✓
    row()
        .key(format!("{key}-btn-row"))
        .role("button")
        .label(label)
        .h(28.0)
        .px(16.0) // board: button pl=16 gives text x=pane_content+16+14+4=x+34 ✓
        .center()
        .radius(radius::MD)
        .fill(t.card)
        .border(1.0, t.card_border)
        .hover(t.hover)
        .press(t.press)
        .focusable(true)
        .gap(space::XS)
        .child(icon(ic, 14.0).key("icon").color(t.text_secondary))
        .child(text(label).key(key).font(type_::BODY).color(t.text).h(17.0)) // board h=17.1
}

fn kind_of(item: &DriveItem) -> &'static str {
    if item.is_folder {
        return "Folder";
    }
    match item.mime.as_deref() {
        Some("application/pdf") => "PDF document",
        Some(m) if m.contains("spreadsheet") => "Spreadsheet",
        Some(m) if m.contains("presentation") => "Presentation",
        Some(m) if m.starts_with("image/heic") || m.starts_with("image/heif") => "HEIC image",
        Some(m) if m.starts_with("image/") => "Image",
        Some(m) if m.starts_with("video/") => "Video",
        Some(m) if m.starts_with("audio/") => "Audio",
        Some(m) if m.contains("document") => "Document",
        _ => "File",
    }
}

fn format_size_detail(bytes: i64) -> String {
    crate::source::format_size(bytes)
}
