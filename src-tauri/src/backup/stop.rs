//! LD-0139 (spec Amendments 12, 12a, 12b and 12c): when a backup turns off, and why, desktop's side of core's
//! `backup_stop`. Every pass first reads the account's notices (`GET /api/v1/notifications` through the backup client);
//! core decides from them, the backup's `last_pass_at` and this pass's listing whether a backup stays on. A backup
//! that turns off says why in core's words with "Back up again"; a check that fails runs no pass at all (Guus: "Skip
//! the pass, retry later") and the card says so.
//!
//! Logs carry categories and reasons, never a name or a path.

use std::sync::atomic::{AtomicBool, Ordering};

use beebeeb_core::backup::{
    MSG_BACK_UP_AGAIN, RebuildClock, ReceivedNotice, StopFolder, StopListing, backup_stop, backup_stop_words,
    parse_backup_notice,
};
use beebeeb_types::backup::{BackupStopReason, Category};
use uuid::Uuid;

use crate::state_db::StateDb;

/// Core's words while the notices could not be checked (spec Amendment 12c).
// TODO(LD-0139 repin to core v0.2.3): core's `MSG_STOP_CHECK_FAILED`.
pub(crate) const MSG_STOP_CHECK_FAILED: &str = "Couldn't check — will retry.";

/// The `last_pass_at` of a backup's first pass under this engine (spec Amendment 12c): when it was set up (a fresh
/// setup), else the old engine's last successful backup (an upgraded device), else 0.
// TODO(LD-0139 repin to core v0.2.3): core's first-pass baseline function decides this.
pub(crate) fn first_pass_baseline(setup_at_ms: Option<i64>, old_engine_last_backup_ms: Option<i64>) -> i64 {
    setup_at_ms.or(old_engine_last_backup_ms).unwrap_or(0)
}

/// Whether the last notices check failed: the card shows [`MSG_STOP_CHECK_FAILED`] on every backup that is on until
/// a check succeeds. In memory: a restart checks again at its first pass.
static CHECK_FAILED: AtomicBool = AtomicBool::new(false);

pub(crate) fn set_check_failed(failed: bool) {
    CHECK_FAILED.store(failed, Ordering::Relaxed);
}

pub(crate) fn check_failed() -> bool {
    CHECK_FAILED.load(Ordering::Relaxed)
}

/// One read of the account's notices: the backup ones, the server's clock when it answered, and the device's offset
/// from it, measured on the same request.
#[derive(Debug, Clone)]
pub(crate) struct NoticeCheck {
    pub(crate) notices: Vec<ReceivedNotice>,
    /// The answer's `Date` (server clock): the new `last_pass_at` of every backup that stays on.
    pub(crate) server_now_ms: i64,
    pub(crate) clock: RebuildClock,
}

/// The check from a `GET /api/v1/notifications` answer. It fails (no pass runs) when the answer has no list, no
/// readable `Date`, a clock that moved during the request, or a backup notice with no readable time: each leaves the
/// device unable to say which notices are newer than its last pass.
pub(crate) fn check_from(
    body: &serde_json::Value,
    clock: RebuildClock,
    server_now_ms: Option<i64>,
) -> Result<NoticeCheck, String> {
    let server_now_ms = server_now_ms.ok_or("the notices answer has no readable Date")?;
    if clock.offset_ms.is_none() {
        return Err("this device's clock moved during the notices check".into());
    }
    let rows = body["notifications"]
        .as_array()
        .ok_or("the notices answer has no list")?;
    let mut notices = Vec::new();
    for row in rows {
        let Some(notice) = row["type"]
            .as_str()
            .and_then(|kind| parse_backup_notice(kind, &row["data"]))
        else {
            continue;
        };
        let created_at_ms = row["created_at"]
            .as_str()
            .and_then(|at| chrono::DateTime::parse_from_rfc3339(at).ok())
            .map(|at| at.timestamp_millis())
            .ok_or("a backup notice has no readable time")?;
        notices.push(ReceivedNotice { notice, created_at_ms });
    }
    Ok(NoticeCheck {
        notices,
        server_now_ms,
        clock,
    })
}

/// `device_ms` (this device's clock) on the server's clock, leaning early by the measurement's margin: an earlier
/// `last_pass_at` lets more notices count, never fewer.
fn on_server_clock(device_ms: i64, clock: RebuildClock) -> i64 {
    let margin = i64::try_from(clock.margin_ms).unwrap_or(i64::MAX);
    device_ms
        .saturating_add(clock.offset_ms.unwrap_or(0))
        .saturating_sub(margin)
}

/// `category`'s `last_pass_at` for core (server clock): its last pass's check, else, at its first pass, core's
/// baseline from when it was set up. The old engine's last backup is not in the state DB (the 0.8.22 mirror kept no
/// record of its passes), so it is never passed. A marks read that fails gives 0 (every notice counts).
pub(crate) fn last_pass_at(db: &StateDb, category: &Category, check: &NoticeCheck) -> i64 {
    let mark = db.backup_pass_mark(&category.key()).unwrap_or_default();
    match mark.last_pass_at_ms {
        Some(at) => at,
        None => first_pass_baseline(mark.setup_at_ms.map(|at| on_server_clock(at, check.clock)), None),
    }
}

/// One backup as this pass's listing shows it, for [`decide`].
pub(crate) struct ListedBackup<'a> {
    pub(crate) category: &'a Category,
    pub(crate) device_folder_id: Option<Uuid>,
    pub(crate) device_folder_live: bool,
    pub(crate) category_folder_id: Option<Uuid>,
    pub(crate) category_folder_live: Option<bool>,
}

/// Core's decision for one backup (`backup_stop`, Amendment 12's steps 1-4): `Some(reason)` turns it off.
pub(crate) fn decide(check: &NoticeCheck, last_pass_at_ms: i64, listed: &ListedBackup<'_>) -> Option<BackupStopReason> {
    let listing = StopListing {
        // A device folder id that is not a UUID matches no notice; a folder that is gone still stops (step 3).
        device_folder_id: listed.device_folder_id.unwrap_or_else(Uuid::nil),
        device_folder_live: listed.device_folder_live,
        folders: vec![StopFolder {
            category: listed.category.clone(),
            category_folder_id: listed.category_folder_id,
            category_folder_live: listed.category_folder_live,
        }],
    };
    backup_stop(&check.notices, last_pass_at_ms, &listing)
        .into_iter()
        .find(|stopped| &stopped.category == listed.category)
        .map(|stopped| stopped.reason)
}

/// The person turned `key`'s backup on, or chose "Back up again": it starts over with this moment as its setup time
/// (spec Amendment 12c: "Turning backups on is the person's choice, so older notices don't apply"), and its stop is
/// cleared. A backup that had stopped ("Back up again") also forgets its records: its first pass rebuilds them from
/// what the server still holds (rule 10), so a file the removal took is uploaded again and one still there is not.
/// A backup the person switched off and on keeps them (a file deleted from the backup elsewhere stays deleted).
pub(crate) fn restart(db: &StateDb, key: &str) -> rusqlite::Result<()> {
    let stopped = db.backup_stops()?.iter().any(|stop| stop.category == key);
    db.restart_backup_pass(key, chrono::Utc::now().timestamp_millis(), stopped)?;
    db.clear_backup_stop(key)
}

/// What the Backups card shows about stops, in core's words (flattened into `KnownFolderStatus`).
#[derive(Debug, Default, Clone, PartialEq, Eq, serde::Serialize)]
pub(crate) struct StopView {
    /// Why this backup is off (`backup_stop_words`); `None` while it is on or was switched off by the person.
    pub(crate) stop_words: Option<String>,
    /// The action under the words: "Back up again".
    pub(crate) stop_action: Option<&'static str>,
    /// "Couldn't check — will retry." on a backup that is on, while the last notices check failed.
    pub(crate) check_note: Option<&'static str>,
}

/// The card's view of `key`'s backup. A stop row a development build wrote before LD-0139 (`device_deleted`,
/// `category_deleted`) reads as `unknown`: a folder gone with no notice.
pub(crate) fn stop_view(db: Option<&StateDb>, key: &str, folder: &str, enabled: bool) -> StopView {
    view_of(db, key, folder, enabled, check_failed())
}

fn view_of(db: Option<&StateDb>, key: &str, folder: &str, enabled: bool, check_failed: bool) -> StopView {
    if enabled {
        return StopView {
            check_note: check_failed.then_some(MSG_STOP_CHECK_FAILED),
            ..StopView::default()
        };
    }
    let stop = db
        .and_then(|db| db.backup_stops().ok())
        .and_then(|stops| stops.into_iter().find(|s| s.category == key));
    let Some(stop) = stop else {
        return StopView::default();
    };
    let reason = BackupStopReason::parse(&stop.reason).unwrap_or(BackupStopReason::Unknown);
    let device = crate::runner::device_name().unwrap_or_else(|| "This device".to_string());
    StopView {
        stop_words: Some(backup_stop_words(reason, &device, folder)),
        stop_action: Some(MSG_BACK_UP_AGAIN),
        check_note: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use beebeeb_core::backup::MSG_STOP_ERASE_REQUEST;

    fn measured() -> RebuildClock {
        RebuildClock {
            offset_ms: Some(60_000),
            margin_ms: 2_500,
        }
    }

    #[test]
    fn the_check_reads_backup_notices_and_skips_other_rows() {
        let body = serde_json::json!({ "notifications": [
            { "type": "share_invite", "data": null, "created_at": "not a time" },
            { "type": "files_removed", "data": { "reason": "erase_request" }, "created_at": "2026-11-07T19:00:00.250Z" },
            { "type": "files_removed", "data": null, "created_at": "2026-11-07T19:00:01Z" },
        ]});
        let check = check_from(&body, measured(), Some(1)).unwrap();
        let reasons: Vec<(BackupStopReason, i64)> = check
            .notices
            .iter()
            .map(|n| (n.notice.reason(), n.created_at_ms))
            .collect();
        assert_eq!(
            reasons,
            [
                (BackupStopReason::EraseRequest, 1_794_078_000_250),
                (BackupStopReason::UnknownRemoval, 1_794_078_001_000)
            ]
        );
        assert_eq!(check.server_now_ms, 1);
    }

    #[test]
    fn a_check_that_cannot_place_its_notices_fails() {
        let ok = serde_json::json!({ "notifications": [] });
        assert!(check_from(&ok, measured(), None).is_err(), "no Date");
        assert!(
            check_from(&ok, RebuildClock::UNKNOWN, Some(1)).is_err(),
            "a clock that moved"
        );
        assert!(
            check_from(&serde_json::json!({ "error": "x" }), measured(), Some(1)).is_err(),
            "no list"
        );
        let untimed =
            serde_json::json!({ "notifications": [{ "type": "backup_deleted", "data": {}, "created_at": null }] });
        assert!(
            check_from(&untimed, measured(), Some(1)).is_err(),
            "a backup notice with no time"
        );
    }

    #[test]
    fn the_first_pass_takes_its_baseline_from_the_setup_on_the_servers_clock_leaning_early() {
        assert_eq!(first_pass_baseline(Some(5), Some(3)), 5);
        assert_eq!(first_pass_baseline(None, Some(3)), 3);
        assert_eq!(first_pass_baseline(None, None), 0);
        // Set up at 1_000_000 on a clock 60 s behind the server's, measured within 2.5 s.
        assert_eq!(on_server_clock(1_000_000, measured()), 1_000_000 + 60_000 - 2_500);
    }

    /// "Back up again" after a stop forgets the backup's records (its first pass rebuilds them); switching a backup
    /// off and on keeps them. Either way it starts over: no pass yet, set up now.
    #[test]
    fn back_up_again_forgets_the_records_and_a_switch_on_keeps_them() {
        use beebeeb_core::backup::{BackupRecord, RecordRef, RecordState};
        let dir = tempfile::tempdir().unwrap();
        let db = StateDb::open(dir.path().join("state.db")).unwrap();
        db.set_owner(&crate::account_binding::Identity::new(
            Some("u-a"),
            Some("a@beebeeb.io"),
        ))
        .unwrap();
        let record = BackupRecord {
            key: "a.jpg".into(),
            size: 5,
            modified_ms: 1,
            remote_id: Uuid::new_v4(),
            uploaded_at_ms: 2,
            state: RecordState::BackedUp,
        };
        let expected = RecordRef {
            key: record.key.clone(),
            remote_id: record.remote_id,
        };
        for key in ["pictures", "music"] {
            assert!(db.put_backup_record_cas(key, &expected, &record).unwrap());
            db.set_backup_last_pass(key, 7).unwrap();
        }
        db.record_backup_stop(&crate::state_db::BackupStop {
            category: "pictures".into(),
            reason: "erase_request".into(),
            at_ms: 1,
        })
        .unwrap();

        restart(&db, "pictures").unwrap();
        restart(&db, "music").unwrap();
        assert!(
            db.backup_records("pictures").unwrap().is_empty(),
            "Back up again rebuilds its records"
        );
        assert_eq!(db.backup_records("music").unwrap().len(), 1, "a switch-on keeps them");
        assert!(db.backup_stops().unwrap().is_empty());
        for key in ["pictures", "music"] {
            let mark = db.backup_pass_mark(key).unwrap();
            assert_eq!(mark.last_pass_at_ms, None, "{key} starts over");
            assert!(
                mark.setup_at_ms.is_some_and(|at| at > 1_700_000_000_000),
                "{key} is set up now"
            );
        }
    }

    #[test]
    fn a_stop_says_why_in_cores_words_and_offers_back_up_again() {
        let dir = tempfile::tempdir().unwrap();
        let db = StateDb::open(dir.path().join("state.db")).unwrap();
        db.set_owner(&crate::account_binding::Identity::new(
            Some("u-a"),
            Some("a@beebeeb.io"),
        ))
        .unwrap();
        db.record_backup_stop(&crate::state_db::BackupStop {
            category: "pictures".into(),
            reason: "erase_request".into(),
            at_ms: 1,
        })
        .unwrap();
        db.record_backup_stop(&crate::state_db::BackupStop {
            category: "music".into(),
            reason: "category_deleted".into(),
            at_ms: 1,
        })
        .unwrap();
        let view = view_of(Some(&db), "pictures", "Pictures", false, false);
        assert_eq!(view.stop_words.as_deref(), Some(MSG_STOP_ERASE_REQUEST));
        assert_eq!(view.stop_action, Some("Back up again"));
        let legacy = view_of(Some(&db), "music", "Music", false, false).stop_words.unwrap();
        assert!(
            legacy.ends_with("'s backup of Music was deleted, so it is off."),
            "{legacy}"
        );
        assert_eq!(
            view_of(Some(&db), "documents", "Documents", false, false),
            StopView::default()
        );
        // On again: no stop words, whatever the old row says; a failed check says so.
        assert_eq!(
            view_of(Some(&db), "pictures", "Pictures", true, false),
            StopView::default()
        );
        let waiting = view_of(Some(&db), "pictures", "Pictures", true, true);
        assert_eq!(
            waiting,
            StopView {
                check_note: Some(MSG_STOP_CHECK_FAILED),
                ..StopView::default()
            }
        );
    }
}
