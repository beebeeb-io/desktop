//! Why a file list is empty, and the storage figure's words. Plain rules, no Tauri types: the window and the
//! native app read the same answers.
//!
//! - An account whose first file list has not been loaded on this PC is NOT an empty folder ([`why_empty`],
//!   [`never_synced`]). "Loaded" is the engine's own mark: the sync cursor it stores once a first snapshot is
//!   applied (`StateDb::get_sync_cursor`: unset means never bootstrapped).
//! - The storage figure is formatted by core's `format_si`, the function the web app prints with
//!   (`storage_format_si` in `beebeeb-wasm`): decimal units, so the two apps show the same words for the same bytes.

use serde::Serialize;

use super::dto::{Listing, StorageSummary, SyncPhase, SyncSummary};

/// Why a listing has no rows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum WhyEmpty {
    /// The folder is loaded and has nothing in it.
    EmptyFolder,
    /// The account's files have not been loaded on this PC yet.
    NeverSynced,
}

/// What keeps the first file list from loading.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum NotLoadedReason {
    Paused,
    Checking,
    Offline,
    Error,
}

/// What the Files area says instead of "Nothing here yet".
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct NeverSynced {
    pub(crate) reason: NotLoadedReason,
    pub(crate) title: &'static str,
    pub(crate) detail: String,
    /// The one action, when there is one: `Resume` while paused.
    pub(crate) action: Option<&'static str>,
}

/// "PC" or "Mac", as the board writes it.
pub(crate) const DEVICE_WORD: &str = if cfg!(target_os = "macos") { "Mac" } else { "PC" };

/// `None` for a listing with rows: the question is only asked of an empty one.
pub(crate) fn why_empty(rows: usize, first_list_loaded: bool) -> Option<WhyEmpty> {
    match (rows, first_list_loaded) {
        (0, true) => Some(WhyEmpty::EmptyFolder),
        (0, false) => Some(WhyEmpty::NeverSynced),
        _ => None,
    }
}

/// The notice for an account whose first file list is not loaded, by sync phase. `None` once it is loaded.
pub(crate) fn never_synced(first_list_loaded: bool, phase: SyncPhase, device: &str) -> Option<NeverSynced> {
    if first_list_loaded {
        return None;
    }
    let not_loaded = format!("Your files have not been loaded on this {device} yet.");
    Some(match phase {
        SyncPhase::Paused => NeverSynced {
            reason: NotLoadedReason::Paused,
            title: "Sync is paused",
            detail: not_loaded,
            action: Some("Resume"),
        },
        SyncPhase::Offline => NeverSynced {
            reason: NotLoadedReason::Offline,
            title: "You’re offline",
            detail: format!("{not_loaded} They load when you’re back online."),
            action: None,
        },
        SyncPhase::Stopped => NeverSynced {
            reason: NotLoadedReason::Error,
            title: "Sync has stopped",
            detail: not_loaded,
            action: None,
        },
        SyncPhase::Checking | SyncPhase::Syncing | SyncPhase::UpToDate | SyncPhase::NeedsDecision => NeverSynced {
            reason: NotLoadedReason::Checking,
            title: "Getting your files ready",
            detail: format!("Your file list is downloading and being decrypted on this {device}."),
            action: None,
        },
    })
}

/// `drive_list`'s answer: the listing, and why it is empty when it is.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ListingAnswer {
    #[serde(flatten)]
    pub(crate) listing: Listing,
    pub(crate) why_empty: Option<WhyEmpty>,
    pub(crate) never_synced: Option<NeverSynced>,
}

impl ListingAnswer {
    pub(crate) fn of(listing: Listing, first_list_loaded: bool, phase: SyncPhase) -> Self {
        let why_empty = why_empty(listing.items.len(), first_list_loaded);
        let never_synced = match why_empty {
            Some(WhyEmpty::NeverSynced) => never_synced(false, phase, DEVICE_WORD),
            _ => None,
        };
        Self {
            listing,
            why_empty,
            never_synced,
        }
    }
}

/// `drive_sync_summary`'s answer: the summary, and the notice while the first file list is not loaded.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SyncAnswer {
    #[serde(flatten)]
    pub(crate) summary: SyncSummary,
    pub(crate) never_synced: Option<NeverSynced>,
}

impl SyncAnswer {
    pub(crate) fn of(summary: SyncSummary, first_list_loaded: bool) -> Self {
        let never_synced = never_synced(first_list_loaded, summary.phase, DEVICE_WORD);
        Self { summary, never_synced }
    }
}

/// Shown for a figure that is not a size.
pub(crate) const UNKNOWN_SIZE: &str = "—";

/// One storage figure, in the web app's words: decimal units (1 GB = 1,000,000,000 B), whole below a terabyte
/// (truncated, never rounded up), one decimal from it. Never a binary value under a decimal label.
pub(crate) fn storage_label(bytes: i64) -> String {
    if bytes < 0 {
        return UNKNOWN_SIZE.to_string();
    }
    beebeeb_types::format::format_si(bytes)
}

/// The words under the meter: `229 GB of 1.1 TB`. A plan without a limit shows the used figure alone.
pub(crate) fn storage_line(used_bytes: i64, quota_bytes: i64) -> String {
    if quota_bytes <= 0 {
        return storage_label(used_bytes);
    }
    format!("{} of {}", storage_label(used_bytes), storage_label(quota_bytes))
}

/// `drive_storage`'s answer: the account's bytes and their words.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct StorageAnswer {
    #[serde(flatten)]
    pub(crate) summary: StorageSummary,
    pub(crate) used_label: String,
    pub(crate) quota_label: String,
    pub(crate) line: String,
}

impl StorageAnswer {
    pub(crate) fn of(summary: StorageSummary) -> Self {
        Self {
            used_label: storage_label(summary.used_bytes),
            quota_label: storage_label(summary.quota_bytes),
            line: storage_line(summary.used_bytes, summary.quota_bytes),
            summary,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::drive_v2::dto::Crumb;

    fn empty_listing() -> Listing {
        let root = Crumb {
            id: None,
            name: "Files".to_string(),
        };
        Listing {
            folder: root.clone(),
            path: vec![root],
            items: Vec::new(),
            as_of: "2026-11-11T11:30:00Z".to_string(),
            stale: false,
        }
    }

    const PHASES: [SyncPhase; 7] = [
        SyncPhase::Checking,
        SyncPhase::UpToDate,
        SyncPhase::Syncing,
        SyncPhase::Paused,
        SyncPhase::Offline,
        SyncPhase::NeedsDecision,
        SyncPhase::Stopped,
    ];

    #[test]
    fn a_never_synced_account_is_not_an_empty_folder() {
        assert_eq!(why_empty(0, false), Some(WhyEmpty::NeverSynced));
        assert_eq!(why_empty(0, true), Some(WhyEmpty::EmptyFolder));
        assert_eq!(why_empty(3, false), None);
        assert_eq!(why_empty(3, true), None);
    }

    /// The measured case: sync paused, nothing ever loaded. The window says so, with Resume.
    #[test]
    fn paused_before_the_first_list_says_so_and_offers_resume() {
        let answer = ListingAnswer::of(empty_listing(), false, SyncPhase::Paused);
        assert_eq!(answer.why_empty, Some(WhyEmpty::NeverSynced));
        assert_eq!(
            answer.never_synced,
            Some(NeverSynced {
                reason: NotLoadedReason::Paused,
                title: "Sync is paused",
                detail: format!("Your files have not been loaded on this {DEVICE_WORD} yet."),
                action: Some("Resume"),
            })
        );
        let json = serde_json::to_value(&answer).expect("json");
        assert_eq!(json["whyEmpty"], "never-synced");
        assert_eq!(json["neverSynced"]["reason"], "paused");
        assert_eq!(json["neverSynced"]["action"], "Resume");
        // The listing's own fields stay where the window reads them.
        assert_eq!(json["items"], serde_json::json!([]));
        assert_eq!(json["folder"]["name"], "Files");
    }

    #[test]
    fn every_phase_has_a_reason_before_the_first_list_and_none_after() {
        let reasons: Vec<NotLoadedReason> = PHASES
            .iter()
            .map(|phase| never_synced(false, *phase, "PC").expect("a notice").reason)
            .collect();
        assert_eq!(
            reasons,
            [
                NotLoadedReason::Checking,
                NotLoadedReason::Checking,
                NotLoadedReason::Checking,
                NotLoadedReason::Paused,
                NotLoadedReason::Offline,
                NotLoadedReason::Checking,
                NotLoadedReason::Error,
            ]
        );
        for phase in PHASES {
            assert_eq!(never_synced(true, phase, "PC"), None);
            let notice = never_synced(false, phase, "Mac").expect("a notice");
            assert!(notice.detail.contains("this Mac"), "{}", notice.detail);
            // Only a paused sync has something to press.
            assert_eq!(notice.action.is_some(), phase == SyncPhase::Paused);
        }
    }

    #[test]
    fn a_loaded_empty_folder_keeps_the_empty_folder_answer() {
        for phase in PHASES {
            let answer = ListingAnswer::of(empty_listing(), true, phase);
            assert_eq!(answer.why_empty, Some(WhyEmpty::EmptyFolder));
            assert_eq!(answer.never_synced, None);
        }
    }

    #[test]
    fn the_summary_carries_the_notice_until_the_first_list_is_loaded() {
        let summary = SyncSummary {
            phase: SyncPhase::Paused,
            on_device: 0,
            available_offline: 0,
            online_only: 0,
            pending: 0,
        };
        let before = SyncAnswer::of(summary.clone(), false);
        assert_eq!(
            before.never_synced.as_ref().map(|n| n.reason),
            Some(NotLoadedReason::Paused)
        );
        let json = serde_json::to_value(&before).expect("json");
        assert_eq!(json["phase"], "paused");
        assert_eq!(json["neverSynced"]["title"], "Sync is paused");
        assert_eq!(SyncAnswer::of(summary, true).never_synced, None);
    }

    /// The web app's words for the same bytes (`formatStorageSI` → core's `format_si`).
    #[test]
    fn storage_figures_are_the_web_apps_words() {
        let table: [(i64, &str); 12] = [
            (229_903_237_485, "229 GB"),
            (235_335_866_544, "235 GB"),
            (1_100_000_000_000, "1.1 TB"),
            (1_000_000_000_000, "1.0 TB"),
            (999_999_999_999, "999 GB"),
            (0, "0 B"),
            (999, "999 B"),
            (1_000, "1 KB"),
            (1_500_000, "1 MB"),
            (5_368_709_120, "5 GB"),
            (2_000_000_000_000, "2.0 TB"),
            (-1, "—"),
        ];
        for (bytes, words) in table {
            assert_eq!(storage_label(bytes), words, "{bytes}");
        }
        // 229.9e9 bytes is 214.1 GiB: the binary value must never appear under "GB".
        assert!(!storage_label(229_903_237_485).starts_with("214"));
        assert!(!storage_label(229_903_237_485).starts_with("215"));
    }

    #[test]
    fn the_storage_answer_carries_the_line() {
        let answer = StorageAnswer::of(StorageSummary {
            used_bytes: 229_903_237_485,
            quota_bytes: 1_100_000_000_000,
            files_bytes: 0,
            photos_bytes: 0,
            backups_bytes: 0,
        });
        assert_eq!(answer.line, "229 GB of 1.1 TB");
        let json = serde_json::to_value(&answer).expect("json");
        assert_eq!(json["usedBytes"], 229_903_237_485_i64);
        assert_eq!(json["usedLabel"], "229 GB");
        assert_eq!(json["quotaLabel"], "1.1 TB");
        assert_eq!(json["line"], "229 GB of 1.1 TB");
        assert_eq!(storage_line(235_335_866_544, 0), "235 GB");
        assert_eq!(storage_line(235_335_866_544, -1), "235 GB");
    }
}
