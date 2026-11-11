//! The view models the `drive_*` commands return: the Rust side of `DriveSource`'s types
//! (`packages/shared/src/drive-ui/source.ts`). Field names serialize in camelCase to match.
//!
//! Desktop v2 spec 2.1: nothing here carries key material, a wrapped key, a session token, a URL,
//! a nonce or a ciphertext. `tests::no_dto_has_a_secret_shaped_field` serializes every DTO and
//! refuses a field named like one.

use serde::Serialize;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum LocalState {
    OnlineOnly,
    Downloaded,
    AvailableOffline,
    Syncing,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Person {
    pub name: String,
    pub initials: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum SharedState {
    None,
    #[allow(dead_code)] // links are listed by B05's share commands; the local mirror does not know them
    Link,
    People {
        people: Vec<Person>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DriveItem {
    pub id: String,
    pub parent_id: Option<String>,
    pub name: String,
    pub is_folder: bool,
    pub size: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub child_count: Option<i64>,
    pub modified_at: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mime: Option<String>,
    pub shared: SharedState,
    pub local: Option<LocalState>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub read_only: Option<bool>,
    pub has_thumbnail: bool,
    /// The row's place in the Backups tree (LD-0113), for the native window. Not part of the JSON the web view
    /// reads. Filled by `model::children_of`; `Outside` from every other constructor.
    #[serde(skip)]
    pub backup: crate::backup_actions::BackupMark,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Crumb {
    pub id: Option<String>,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Listing {
    pub folder: Crumb,
    pub path: Vec<Crumb>,
    pub items: Vec<DriveItem>,
    pub as_of: String,
    pub stale: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ItemDetails {
    pub item: DriveItem,
    pub path: Vec<Crumb>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub changed_on: Option<String>,
    pub version_count: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VersionEntry {
    pub id: String,
    pub created_at: String,
    pub size: i64,
    pub current: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TrashEntry {
    pub item: DriveItem,
    pub trashed_at: String,
    pub original_path: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StorageSummary {
    pub used_bytes: i64,
    pub quota_bytes: i64,
    pub files_bytes: i64,
    pub photos_bytes: i64,
    pub backups_bytes: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum SyncPhase {
    Checking,
    UpToDate,
    Syncing,
    Paused,
    Offline,
    NeedsDecision,
    Stopped,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncSummary {
    pub phase: SyncPhase,
    pub on_device: u32,
    pub available_offline: u32,
    pub online_only: u32,
    pub pending: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum ShareDirection {
    ByMe,
    WithMe,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum ShareRole {
    Viewer,
    Editor,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SharePerson {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub email: Option<String>,
    pub role: ShareRole,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ShareEntry {
    pub item: DriveItem,
    pub direction: ShareDirection,
    pub people: Vec<SharePerson>,
    pub has_link: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PhotoEntry {
    pub item: DriveItem,
    pub taken_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PhotoPage {
    pub entries: Vec<PhotoEntry>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UndoTicket {
    /// An opaque handle into this process's undo table; it names no file and grants nothing on its own.
    pub undo_token: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct UploadStarted {
    pub started: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TextDocument {
    pub text: String,
    pub version_id: String,
}

/// `drive://event`, the `DriveEvent` union.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum DriveEvent {
    #[serde(rename_all = "camelCase")]
    Changed {
        folder_id: Option<String>,
    },
    #[allow(dead_code)] // part of the contract; the poller sends `changed` and `sync`
    Item {
        item: DriveItem,
    },
    #[allow(dead_code)]
    Removed {
        id: String,
    },
    Sync {
        sync: SyncSummary,
    },
    #[allow(dead_code)]
    Storage {
        storage: StorageSummary,
    },
}
