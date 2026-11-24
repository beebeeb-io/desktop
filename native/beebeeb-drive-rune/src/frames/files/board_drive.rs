//! `BoardDrive`: the board's folders as a `crate::source::DriveSource` for the sample window (moved out of `source.rs`
//! whole, as it was; `source` re-exports it).

use super::source::{BACKUPS_ID, SharedEdits, edits_of};
use crate::source::DriveItem;

/// The board's folders: `BoardFixtureSource` holds the root (DW-0401); this honours the folder id.
/// Lisbon trip holds the seven rows of DW-0420, "New folder" nothing (DW-1106). A folder the board
/// never opens (Backups: "4 devices", Documents: "48 items") cannot be read here: it has things in
/// it that the board does not draw, so it shows neither invented rows nor "Nothing here yet".
/// An id that is no folder: `NotFound`.
pub struct BoardDrive {
    root: crate::source::BoardFixtureSource,
    /// What the fixture's actions changed (`FilesFixture::edits`).
    edits: SharedEdits,
}

/// The folder DW-0417 makes and DW-1106 shows.
pub const NEW_FOLDER_ID: &str = "f-new";

impl BoardDrive {
    pub fn new() -> Self {
        BoardDrive {
            root: crate::source::BoardFixtureSource::new(),
            edits: SharedEdits::default(),
        }
    }

    /// The board's folders, showing what this fixture's New folder, Rename and Move change.
    pub fn sharing(edits: &SharedEdits) -> Self {
        BoardDrive {
            root: crate::source::BoardFixtureSource::new(),
            edits: edits.clone(),
        }
    }

    /// The board's rows of one folder, before any change: the root's, Lisbon trip's.
    fn drawn(&self, folder: Option<&str>) -> Option<Vec<DriveItem>> {
        match folder {
            None => crate::source::DriveSource::list(&self.root, None)
                .ok()
                .map(|listing| listing.items),
            Some("f-lisbon") => Some(lisbon_trip("f-lisbon")),
            // Backups, this PC's backup and the mail service's folder (DW-0422 to DW-0424).
            Some(id) => super::protected_fixture::rows(id),
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
        let edits = edits_of(&self.edits);
        // Every item this fixture knows, where the board drew it.
        let mut known: Vec<DriveItem> = root.items.clone();
        known.extend(lisbon_trip("f-lisbon"));
        known.extend(super::protected_fixture::all());
        known.extend(edits.made.iter().cloned());
        for item in &mut known {
            if let Some(name) = edits.names.get(&item.id) {
                item.name = name.clone();
            }
            if let Some(parent) = edits.parents.get(&item.id) {
                item.parent_id = parent.clone();
            }
        }
        let made = |id: &str| edits.made.iter().any(|item| item.id == id);
        let folder = match folder_id {
            None => root.folder.clone(),
            // The folder DW-1106 shows, before anything made it.
            Some(NEW_FOLDER_ID) if !made(NEW_FOLDER_ID) => Crumb {
                id: Some(NEW_FOLDER_ID.to_string()),
                name: "New folder".to_string(),
            },
            Some(id) => known
                .iter()
                .find(|i| i.id == id && i.is_folder)
                .map(|i| Crumb {
                    id: Some(id.to_string()),
                    name: i.name.clone(),
                })
                .ok_or(SourceError::NotFound)?,
        };
        // A folder the board never opens has things in it that the board does not draw: it
        // cannot be read here, whatever was moved into it. A folder made here starts empty.
        let drawn = match folder_id {
            Some(id) if id == NEW_FOLDER_ID || made(id) => true,
            other => self.drawn(other).is_some(),
        };
        if !drawn {
            return Err(SourceError::Unreadable);
        }
        // The board's own order: its rows where it drew them, then what came in.
        let here = |item: &&DriveItem| item.parent_id.as_deref() == folder_id;
        let items: Vec<DriveItem> = known.iter().filter(here).cloned().collect();
        // The way up: every folder above this one.
        let mut path = vec![folder.clone()];
        let mut at = folder_id.map(str::to_string);
        while let Some(id) = at {
            let parent = known.iter().find(|i| i.id == id).and_then(|i| i.parent_id.clone());
            path.push(match &parent {
                None => root.folder.clone(),
                Some(up) => Crumb {
                    id: Some(up.clone()),
                    name: known
                        .iter()
                        .find(|i| &i.id == up)
                        .map(|i| i.name.clone())
                        .unwrap_or_default(),
                },
            });
            at = parent;
        }
        path.reverse();
        // Backups is a section of its own: inside it the path starts at Backups (DW-0423).
        if let Some(at) = path.iter().position(|crumb| crumb.id.as_deref() == Some(BACKUPS_ID)) {
            path.drain(..at);
        }
        Ok(Listing { path, folder, items })
    }

    fn storage(&self) -> Option<crate::source::StorageSummary> {
        self.root.storage()
    }
}
