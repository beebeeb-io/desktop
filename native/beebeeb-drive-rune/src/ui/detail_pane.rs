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

const PANE_W: f32 = 268.0;
const PREVIEW_H: f32 = 170.0;

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
        .pt(space::LG)
        .px(space::LG)
        .gap(space::SM)
        // Preview area
        .child(
            el().key("preview")
                .w_full()
                .h(PREVIEW_H)
                .radius(radius::LG)
                .fill(t.surface)
                .center()
                .child(icon(&icons::DOC_GENERIC, 48.0).key("preview-icon").color(t.separator)),
        )
        // File name
        .child(
            text(&item.name)
                .key("detail-name")
                .font(type_::LABEL)
                .color(t.text)
                .mt(space::SM),
        )
        // Kind + size
        .child(
            text(kind_size)
                .key("detail-kind")
                .font(type_::BODY)
                .color(t.text_tertiary),
        )
        // End-to-end encrypted badge
        .child(encrypted_badge(t))
        // Separator
        .child(el().key("detail-sep-1").w_full().h(1.0).fill(t.separator).mt(space::XS))
        // Metadata rows
        .child(meta_row("Changed", "Yesterday, 16:42", true, t))
        .child(meta_row("Changed on", "Anna\u{2019}s iPhone", false, t))
        .child(meta_row("Added", "28 Oct 2026", true, t))
        .child(meta_row("Where", "Files", false, t))
        .child(meta_row("Version history", "3 versions", false, t))
        .child(meta_row("Shared", "Sam Okafor, pending", false, t))
        // Separator
        .child(el().key("detail-sep-2").w_full().h(1.0).fill(t.separator).mt(space::XS))
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
                        .child(
                            text("Available offline")
                                .key("offline-label")
                                .font(type_::BODY_SEMI)
                                .color(t.text),
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
        .child(el().key("detail-spacer").grow(1.0))
        // Action buttons
        .child(
            row()
                .key("detail-actions")
                .w_full()
                .gap(space::SM)
                .pb(space::LG)
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
    row()
        .key("encrypted-badge")
        .h(24.0)
        .px(space::SM)
        .radius(radius::SM)
        .fill(t.accent_bg)
        .center_y()
        .gap(space::XS)
        .child(icon(&icons::LOCK, 12.0).key("lock-icon").color(t.accent_deep))
        .child(
            text("End-to-end encrypted")
                .key("encrypted-text")
                .font(type_::BODY_SEMI)
                .color(t.accent_deep),
        )
}

fn meta_row(label: &'static str, value: &str, mono: bool, t: &Theme) -> El {
    row()
        .key(label)
        .w_full()
        .h(22.0)
        .center_y()
        .child(
            text(label)
                .key("label")
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
    row()
        .key(key)
        .role("button")
        .label(label)
        .h(28.0)
        .px(space::MD)
        .center()
        .radius(radius::MD)
        .fill(t.card)
        .border(1.0, t.card_border)
        .hover(t.hover)
        .press(t.press)
        .focusable(true)
        .gap(space::XS)
        .child(icon(ic, 14.0).key("icon").color(t.text_secondary))
        .child(text(label).key("label").font(type_::BODY).color(t.text))
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
    const KB: f64 = 1024.0;
    const MB: f64 = 1024.0 * 1024.0;
    const GB: f64 = 1024.0 * 1024.0 * 1024.0;
    let b = bytes as f64;
    if b >= GB {
        format!("{:.1} GB", b / GB)
    } else if b >= MB {
        format!("{:.1} MB", b / MB)
    } else if b >= KB {
        format!("{} KB", (b / KB).round() as i64)
    } else {
        format!("{bytes} B")
    }
}
