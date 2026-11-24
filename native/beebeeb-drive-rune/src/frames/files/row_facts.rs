//! What a row says of its item when the source knows no more: the details panel's facts and the kind's name.

use super::source::Details;
use crate::source::{DriveItem, format_size};
use crate::ui::file_list::format_date;

/// What a row itself says about the item, for a source that knows no more.
pub fn details_of_row(item: &DriveItem, folder: &str) -> Details {
    let size = if item.is_folder {
        item.child_count.map(|c| format!("{c} items")).unwrap_or_default()
    } else {
        item.size.map(format_size).unwrap_or_default()
    };
    let kept = item.pc_status.as_deref() == Some("available_offline");
    let place = match item.pc_status.as_deref() {
        Some("online_only") => "Online only now",
        Some("on_this_pc") => "Downloaded",
        Some("available_offline") => "Kept on this PC",
        _ => "",
    };
    let offline_note = match (place.is_empty(), size.is_empty()) {
        (false, false) => format!("{place} \u{00B7} {size}"),
        (false, true) => place.to_string(),
        (true, _) => size.clone(),
    };
    let shared = if item.shared_type.is_some() {
        "Anyone with the link".to_string()
    } else if item.shared_badges.is_empty() {
        "Only you".to_string()
    } else {
        item.shared_badges
            .iter()
            .map(|b| b.initials.as_str())
            .collect::<Vec<_>>()
            .join(", ")
    };
    Details {
        kind: kind_of(item).to_string(),
        size,
        changed: format_date(&item.modified_at),
        changed_on: None,
        added: None,
        where_: folder.to_string(),
        versions: None,
        shared,
        offline_note,
        kept_offline: kept,
        picture: item.has_thumbnail,
    }
}

pub fn kind_of(item: &DriveItem) -> &'static str {
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
