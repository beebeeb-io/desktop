//! The board's protected folder (DW-0422 to DW-0425, DW-1110): Backups, what one device backed
//! up, and the folder a service manages. Only what the board draws: a device the board never
//! opens cannot be read here (`BoardDrive` answers `Unreadable`, never invented rows).
//!
//! The words of the protected-folder message are the core's: every Beebeeb app says the same
//! (DW-0425, "The wording comes from the core"). This fixture carries them exactly as the board
//! draws them, and the window shows what the source hands it, never words of its own.

use super::protected::{BackupEntry, BackupKind, FolderNotice, Protection, Words};
use super::source::BACKUPS_ID;
use crate::source::DriveItem;

/// This PC's backup (DW-0423).
pub const LAPTOP_ID: &str = "b-laptop";
/// The folder the mail service manages (DW-0424).
pub const MAIL_ID: &str = "b-mail";

const GB: i64 = 1_000_000_000;

fn folder(id: &str, parent: &str, name: &str, modified: &str) -> DriveItem {
    DriveItem {
        id: id.into(),
        parent_id: Some(parent.into()),
        name: name.into(),
        is_folder: true,
        size: None,
        child_count: None,
        modified_at: modified.into(),
        mime: None,
        has_thumbnail: false,
        shared_type: None,
        shared_badges: Vec::new(),
        pc_status: Some("online_only".into()),
        is_protected: false,
    }
}

/// The rows the board draws in one folder of the protected tree. `None`: not a folder of it, or
/// one the board never opens.
pub fn rows(folder_id: &str) -> Option<Vec<DriveItem>> {
    let sized = |id: &str, name: &str, modified: &str, tenths_gb: i64| DriveItem {
        size: Some(tenths_gb * GB / 10),
        ..folder(id, BACKUPS_ID, name, modified)
    };
    let holding = |id: &str, parent: &str, name: &str, modified: &str, count: i64| DriveItem {
        child_count: Some(count),
        ..folder(id, parent, name, modified)
    };
    match folder_id {
        // DW-0422: one folder per device, then one per service that is on.
        BACKUPS_ID => Some(vec![
            sized("b-iphone", "Anna's iPhone", "2026-11-05T09:12:00Z", 1060),
            sized("b-ipad", "Anna's iPad", "2026-10-19T10:00:00Z", 380),
            sized(LAPTOP_ID, "Anna-Laptop", "2026-11-05T09:30:00Z", 98),
            sized("b-old-iphone", "Old iPhone", "2026-03-03T10:00:00Z", 720),
            sized(MAIL_ID, "Mail Backup", "2026-11-05T08:40:00Z", 62),
        ]),
        // DW-0423: this PC's folders, contacts and calendar.
        LAPTOP_ID => Some(vec![
            holding("b-laptop-desktop", LAPTOP_ID, "Desktop", "2026-11-05T09:30:00Z", 214),
            holding(
                "b-laptop-documents",
                LAPTOP_ID,
                "Documents",
                "2026-11-05T09:30:00Z",
                1820,
            ),
            holding("b-laptop-projects", LAPTOP_ID, "Projects", "2026-11-05T09:28:00Z", 46),
            holding("b-laptop-contacts", LAPTOP_ID, "Contacts", "2026-11-05T06:00:00Z", 1),
            holding("b-laptop-calendar", LAPTOP_ID, "Calendar", "2026-11-05T06:00:00Z", 1),
        ]),
        // DW-0424: one folder per year.
        MAIL_ID => Some(vec![
            holding("b-mail-2026", MAIL_ID, "2026", "2026-11-05T08:40:00Z", 1204),
            holding("b-mail-2025", MAIL_ID, "2025", "2025-12-31T10:00:00Z", 3870),
            holding("b-mail-2024", MAIL_ID, "2024", "2024-12-31T10:00:00Z", 4012),
        ]),
        _ => None,
    }
}

/// Every item of the protected tree the board draws.
pub fn all() -> Vec<DriveItem> {
    [BACKUPS_ID, LAPTOP_ID, MAIL_ID]
        .into_iter()
        .filter_map(rows)
        .flatten()
        .collect()
}

/// Is this folder Backups or a folder inside it?
pub fn holds(folder: Option<&str>) -> bool {
    folder.is_some_and(|id| id == BACKUPS_ID || all().iter().any(|item| item.id == id))
}

/// Which top folder of Backups an item or folder of the tree belongs to (its own id for a
/// device's or a service's folder).
fn top_of(id: &str) -> Option<String> {
    let known = all();
    let mut at = known.iter().find(|item| item.id == id)?;
    while at.parent_id.as_deref() != Some(BACKUPS_ID) {
        let parent = at.parent_id.clone()?;
        at = known.iter().find(|item| item.id == parent)?;
    }
    Some(at.id.clone())
}

/// What a row of the tree is, beyond a folder (DW-0422's icons and its Status column).
pub fn entry(item: &DriveItem) -> Option<BackupEntry> {
    if item.id == "b-laptop-projects" {
        // A folder Anna chose, beside the PC's known folders.
        return Some(BackupEntry {
            kind: BackupKind::Folder,
            status: None,
            managed: false,
            note: Some("Chosen folder".into()),
        });
    }
    let (kind, status, managed) = match item.id.as_str() {
        "b-iphone" => (BackupKind::Phone, Some("12,418 photos"), false),
        "b-ipad" => (BackupKind::Tablet, Some("4,120 photos"), false),
        LAPTOP_ID => (BackupKind::Laptop, Some("This PC"), false),
        "b-old-iphone" => (BackupKind::Phone, Some("Not signed in"), false),
        MAIL_ID => (BackupKind::Mail, Some("Managed by Beebeeb"), true),
        "b-laptop-contacts" => (BackupKind::Contacts, None, false),
        "b-laptop-calendar" => (BackupKind::Calendar, None, false),
        _ => return None,
    };
    Some(BackupEntry {
        kind,
        status: status.map(str::to_string),
        managed,
        note: None,
    })
}

const TITLE: &str = "Beebeeb manages this folder";
/// DW-1110: the service's own folder.
const OF_THE_FOLDER: &str = "Mail Backup keeps its files here, so it can't be moved, renamed or deleted. You can turn the service on or off in Settings.";
/// DW-0425: something inside it.
const OF_WHAT_IS_INSIDE: &str =
    "Mail Backup keeps its files here, so they can't be moved, renamed or deleted. Turn it on or off in Settings.";

/// Why `item` (or, with no item, anything in `folder`) cannot change.
pub fn protection(folder: Option<&str>, item: Option<&DriveItem>) -> Option<Protection> {
    let words = |body: &str| {
        Protection::Managed(Words {
            title: TITLE.into(),
            body: body.into(),
        })
    };
    match item {
        // The Backups row in Files keeps the rule it had: protected by its own flag, no words drawn.
        Some(item) if item.id == BACKUPS_ID => None,
        Some(item) => match top_of(&item.id)?.as_str() {
            MAIL_ID if item.id == MAIL_ID => Some(words(OF_THE_FOLDER)),
            MAIL_ID => Some(words(OF_WHAT_IS_INSIDE)),
            device => Some(Protection::DeviceBackup(device.to_string())),
        },
        None => match folder? {
            BACKUPS_ID => Some(Protection::BackupsRoot),
            id => match top_of(id)?.as_str() {
                MAIL_ID => Some(words(OF_WHAT_IS_INSIDE)),
                device => Some(Protection::DeviceBackup(device.to_string())),
            },
        },
    }
}

/// DW-0424: the note over a service's folder.
pub fn notice(folder: Option<&str>) -> Option<FolderNotice> {
    (top_of(folder?)?.as_str() == MAIL_ID).then(|| FolderNotice {
        words: "Mail Backup manages this folder. You can open and download its files.".into(),
        action: "Settings".into(),
    })
}
