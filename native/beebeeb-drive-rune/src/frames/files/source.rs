//! `FilesSource`: what the Files frames ask beyond a folder's listing (`crate::source::DriveSource`):
//! one item's facts for the details panel, and the actions of the file menu.
//!
//! `FilesFixture` holds exactly what the board draws (DW-0401, DW-0406, DW-1104) and records every
//! call, so a test can say which engine call an action made. The engine's implementation is another lane's.

use crate::source::{DriveItem, format_size};
use crate::ui::file_list::format_date;

/// What the details panel shows of one item. `None`: the source does not know, and the row is left out.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Details {
    /// "PDF document"
    pub kind: String,
    /// "1.2 MB", or a folder's "48 items"
    pub size: String,
    pub changed: String,
    pub changed_on: Option<String>,
    pub added: Option<String>,
    pub where_: String,
    pub versions: Option<String>,
    pub shared: String,
    /// The Available offline card's second line: "Online only now · 1.2 MB".
    pub offline_note: String,
    pub kept_offline: bool,
    /// The preview is the file's own picture (a photo), not a page.
    pub picture: bool,
}

/// Why an action did not happen.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ActionError {
    /// Backups and service folders cannot change (DW-0425).
    Protected,
}

/// One call an action made on the source, in the engine's words.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Call {
    Trash(Vec<String>),
    Restore(Vec<String>),
    Duplicate(String),
    KeepOffline(String, bool),
    Sort(super::Sort),
}

/// The counts beside the sidebar's sections.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Counts {
    /// Shares that wait for an answer.
    pub shared_waiting: u32,
    pub trash: u32,
    pub available_offline: u32,
}

pub trait FilesSource: Send + 'static {
    fn counts(&self) -> Counts;
    fn details(&self, item: &DriveItem, folder: &str) -> Details;
    /// A date the list shows in other words than its own ("Just now" for a copy made a moment ago).
    fn changed_label(&self, item: &DriveItem) -> Option<String>;
    fn trash(&mut self, items: &[DriveItem]) -> Result<(), ActionError>;
    fn restore(&mut self, items: &[DriveItem]) -> Result<(), ActionError>;
    /// The copy, as it now stands next to the original (`names`: the folder's names, to find a free one).
    fn duplicate(&mut self, item: &DriveItem, names: &[&str]) -> Result<DriveItem, ActionError>;
    fn keep_offline(&mut self, item: &DriveItem, keep: bool) -> Result<(), ActionError>;
    fn sort(&mut self, sort: super::Sort);
}

#[derive(Default)]
pub struct FilesFixture {
    pub calls: Vec<Call>,
    /// How many items this session moved to Trash and did not take back.
    pub trashed: u32,
    /// Ids of the copies made in this session: the board shows them as changed "Just now" on "This PC".
    copies: Vec<String>,
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

/// "Contract draft.pdf" → "Contract draft copy.pdf"; taken → "Contract draft copy 2.pdf" (DW-0406).
pub fn copy_name(name: &str, taken: &[&str]) -> String {
    let (stem, ext) = match name.rfind('.') {
        Some(dot) if dot > 0 => (&name[..dot], &name[dot..]),
        _ => (name, ""),
    };
    let first = format!("{stem} copy{ext}");
    if !taken.contains(&first.as_str()) {
        return first;
    }
    (2..)
        .map(|n| format!("{stem} copy {n}{ext}"))
        .find(|candidate| !taken.contains(&candidate.as_str()))
        .unwrap_or(first)
}

impl FilesSource for FilesFixture {
    fn counts(&self) -> Counts {
        // The board's sample: 1 share waiting, 12 in Trash, 3 kept offline.
        Counts {
            shared_waiting: 1,
            trash: 12 + self.trashed,
            available_offline: 3,
        }
    }

    fn details(&self, item: &DriveItem, folder: &str) -> Details {
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
        let copy = self.copies.contains(&item.id);
        // The facts the board draws for the rows it selects; for any other row only what the row itself says.
        let (changed_on, added, versions, shared) = if copy {
            (
                Some("This PC"),
                Some("28 Oct 2026"),
                Some("1 version"),
                "Only you".to_string(),
            )
        } else {
            match item.id.as_str() {
                "contract" => (
                    Some("Anna's iPhone"),
                    Some("28 Oct 2026"),
                    Some("3 versions"),
                    "Sam Okafor, pending".to_string(),
                ),
                "harbour" => (
                    Some("Anna's iPhone"),
                    Some("28 Oct 2026"),
                    Some("1 version"),
                    "Anyone with the link".to_string(),
                ),
                _ => (
                    None,
                    None,
                    None,
                    if item.shared_type.is_some() {
                        "Anyone with the link".to_string()
                    } else if item.shared_badges.is_empty() {
                        "Only you".to_string()
                    } else {
                        item.shared_badges
                            .iter()
                            .map(|b| b.initials.as_str())
                            .collect::<Vec<_>>()
                            .join(", ")
                    },
                ),
            }
        };
        Details {
            kind: kind_of(item).to_string(),
            size,
            changed: self
                .changed_label(item)
                .unwrap_or_else(|| format_date(&item.modified_at)),
            changed_on: changed_on.map(str::to_string),
            added: added.map(str::to_string),
            where_: folder.to_string(),
            versions: versions.map(str::to_string),
            shared,
            offline_note,
            kept_offline: kept,
            picture: item.has_thumbnail,
        }
    }

    fn changed_label(&self, item: &DriveItem) -> Option<String> {
        self.copies.contains(&item.id).then(|| "Just now".to_string())
    }

    fn trash(&mut self, items: &[DriveItem]) -> Result<(), ActionError> {
        if items.iter().any(|i| i.is_protected) {
            return Err(ActionError::Protected);
        }
        self.calls
            .push(Call::Trash(items.iter().map(|i| i.id.clone()).collect()));
        self.trashed += items.len() as u32;
        Ok(())
    }

    fn restore(&mut self, items: &[DriveItem]) -> Result<(), ActionError> {
        self.calls
            .push(Call::Restore(items.iter().map(|i| i.id.clone()).collect()));
        self.trashed = self.trashed.saturating_sub(items.len() as u32);
        Ok(())
    }

    fn duplicate(&mut self, item: &DriveItem, names: &[&str]) -> Result<DriveItem, ActionError> {
        if item.is_protected {
            return Err(ActionError::Protected);
        }
        self.calls.push(Call::Duplicate(item.id.clone()));
        let id = format!("{}-copy-{}", item.id, self.copies.len() + 1);
        self.copies.push(id.clone());
        Ok(DriveItem {
            id,
            name: copy_name(&item.name, names),
            // A new file: on this PC, its own versions start now, and it is not shared (DW-0406).
            shared_type: None,
            shared_badges: Vec::new(),
            pc_status: Some("on_this_pc".into()),
            ..item.clone()
        })
    }

    fn keep_offline(&mut self, item: &DriveItem, keep: bool) -> Result<(), ActionError> {
        self.calls.push(Call::KeepOffline(item.id.clone(), keep));
        Ok(())
    }

    fn sort(&mut self, sort: super::Sort) {
        self.calls.push(Call::Sort(sort));
    }
}

/// The board's folders: `BoardFixtureSource` holds the root (DW-0401); this honours the folder id.
/// Lisbon trip holds the seven rows of DW-0420, "New folder" nothing (DW-1106); a folder the board
/// never opens lists nothing rather than invented rows. An id that is no folder: `NotFound`.
pub struct BoardDrive {
    root: crate::source::BoardFixtureSource,
}

/// The folder DW-0417 makes and DW-1106 shows.
pub const NEW_FOLDER_ID: &str = "f-new";

impl BoardDrive {
    pub fn new() -> Self {
        BoardDrive {
            root: crate::source::BoardFixtureSource::new(),
        }
    }
}

impl Default for BoardDrive {
    fn default() -> Self {
        Self::new()
    }
}

fn lisbon_trip(parent: &str) -> Vec<DriveItem> {
    let item = |id: &str, name: &str, modified: &str| DriveItem {
        id: id.into(),
        parent_id: Some(parent.into()),
        name: name.into(),
        is_folder: false,
        size: None,
        child_count: None,
        modified_at: modified.into(),
        mime: None,
        has_thumbnail: false,
        shared_type: None,
        shared_badges: Vec::new(),
        pc_status: Some("online_only".into()),
        is_protected: false,
    };
    let folder = |id: &str, name: &str, modified: &str, count: i64| DriveItem {
        is_folder: true,
        child_count: Some(count),
        ..item(id, name, modified)
    };
    let file = |id: &str, name: &str, modified: &str, size: i64, mime: &str| DriveItem {
        size: Some(size),
        mime: Some(mime.into()),
        has_thumbnail: mime.starts_with("image/") || mime.starts_with("video/"),
        ..item(id, name, modified)
    };
    const MB: f64 = 1024.0 * 1024.0;
    let mb = |n: f64| (n * MB) as i64;
    vec![
        folder("f-day1", "Day 1", "2026-10-09T10:00:00Z", 48),
        folder("f-day2", "Day 2", "2026-10-10T10:00:00Z", 61),
        DriveItem {
            pc_status: Some("on_this_pc".into()),
            ..file(
                "tickets",
                "Tickets.pdf",
                "2026-10-08T10:00:00Z",
                220 * 1024,
                "application/pdf",
            )
        },
        file("alfama", "Alfama.heic", "2026-10-09T10:00:00Z", mb(3.4), "image/heic"),
        file("belem", "Belem.heic", "2026-10-09T10:00:00Z", mb(4.1), "image/heic"),
        file(
            "tram28",
            "Tram 28.mov",
            "2026-10-10T10:00:00Z",
            mb(82.0),
            "video/quicktime",
        ),
        file("sunset", "Sunset.heic", "2026-10-10T10:00:00Z", mb(3.8), "image/heic"),
    ]
}

impl crate::source::DriveSource for BoardDrive {
    fn list(&self, folder_id: Option<&str>) -> Result<crate::source::Listing, crate::source::SourceError> {
        use crate::source::{Crumb, Listing, SourceError};
        let root = self.root.list(None)?;
        let Some(id) = folder_id else { return Ok(root) };
        let name = if id == NEW_FOLDER_ID {
            "New folder".to_string()
        } else {
            root.items
                .iter()
                .find(|i| i.id == id && i.is_folder)
                .map(|i| i.name.clone())
                .ok_or(SourceError::NotFound)?
        };
        let folder = Crumb {
            id: Some(id.to_string()),
            name,
        };
        Ok(Listing {
            path: vec![root.folder.clone(), folder.clone()],
            folder,
            items: if id == "f-lisbon" { lisbon_trip(id) } else { Vec::new() },
        })
    }

    fn storage(&self) -> Option<crate::source::StorageSummary> {
        self.root.storage()
    }
}
