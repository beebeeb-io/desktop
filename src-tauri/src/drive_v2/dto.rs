//! The view models the `drive_*` commands return: the Rust side of `DriveSource`'s types
//! (`packages/shared/src/drive-ui/source.ts`). Field names serialize in camelCase to match.
//!
//! Desktop v2 spec 2.1: nothing here carries key material, a wrapped key, a session token, a URL,
//! a nonce or a ciphertext. `tests::no_dto_has_a_secret_shaped_field` serializes every DTO and
//! refuses a field named like one.

use serde::Serialize;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum LocalState {
    OnlineOnly,
    Downloaded,
    AvailableOffline,
    Syncing,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct Person {
    pub(crate) name: String,
    pub(crate) initials: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub(crate) enum SharedState {
    None,
    #[allow(dead_code)] // links are listed by B05's share commands; the local mirror does not know them
    Link,
    People {
        people: Vec<Person>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct DriveItem {
    pub(crate) id: String,
    pub(crate) parent_id: Option<String>,
    pub(crate) name: String,
    pub(crate) is_folder: bool,
    pub(crate) size: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) child_count: Option<i64>,
    pub(crate) modified_at: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) mime: Option<String>,
    pub(crate) shared: SharedState,
    pub(crate) local: Option<LocalState>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) read_only: Option<bool>,
    pub(crate) has_thumbnail: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct Crumb {
    pub(crate) id: Option<String>,
    pub(crate) name: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Listing {
    pub(crate) folder: Crumb,
    pub(crate) path: Vec<Crumb>,
    pub(crate) items: Vec<DriveItem>,
    pub(crate) as_of: String,
    pub(crate) stale: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ItemDetails {
    pub(crate) item: DriveItem,
    pub(crate) path: Vec<Crumb>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) changed_on: Option<String>,
    pub(crate) version_count: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct VersionEntry {
    pub(crate) id: String,
    pub(crate) created_at: String,
    pub(crate) size: i64,
    pub(crate) current: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct TrashEntry {
    pub(crate) item: DriveItem,
    pub(crate) trashed_at: String,
    pub(crate) original_path: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct StorageSummary {
    pub(crate) used_bytes: i64,
    pub(crate) quota_bytes: i64,
    pub(crate) files_bytes: i64,
    pub(crate) photos_bytes: i64,
    pub(crate) backups_bytes: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum SyncPhase {
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
pub(crate) struct SyncSummary {
    pub(crate) phase: SyncPhase,
    pub(crate) on_device: u32,
    pub(crate) available_offline: u32,
    pub(crate) online_only: u32,
    pub(crate) pending: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum ShareDirection {
    ByMe,
    WithMe,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum ShareRole {
    Viewer,
    Editor,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct SharePerson {
    pub(crate) name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) email: Option<String>,
    pub(crate) role: ShareRole,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ShareEntry {
    pub(crate) item: DriveItem,
    pub(crate) direction: ShareDirection,
    pub(crate) people: Vec<SharePerson>,
    pub(crate) has_link: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PhotoEntry {
    pub(crate) item: DriveItem,
    pub(crate) taken_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct PhotoPage {
    pub(crate) entries: Vec<PhotoEntry>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) next: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct UndoTicket {
    /// An opaque handle into this process's undo table; it names no file and grants nothing on its own.
    pub(crate) undo_token: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct UploadStarted {
    pub(crate) started: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct TextDocument {
    pub(crate) text: String,
    pub(crate) version_id: String,
}

/// `drive://event`, the `DriveEvent` union.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub(crate) enum DriveEvent {
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
