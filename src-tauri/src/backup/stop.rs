//! LD-0139 (spec Amendments 12, 12a, 12b and 12c): when a backup turns off, and why, desktop's side of core's
//! `backup_pass_gate` and `backup_stop` (core v0.2.3). Every tick first reads the account's notices
//! (`GET /api/v1/notifications` through the backup client); core decides from them, each backup's `last_pass_at` and
//! this pass's listing whether it runs, turns off, or is skipped because the notices could not be read. A backup
//! that turns off says why in core's words with "Back up again"; a skipped one says "Couldn't check — will retry."
//!
//! Every time handed to core is on the server's clock: `last_pass_at` is a check's `Date`, and the setup time is
//! moved onto it with the check's measured offset.
//!
//! Logs carry categories and reasons, never a name or a path.

use std::sync::atomic::{AtomicBool, Ordering};

use beebeeb_core::backup::{
    BackupPassGate, MSG_BACK_UP_AGAIN, MSG_STOP_CHECK_FAILED, RebuildClock, ReceivedNotice, StopFolder, StopListing,
    backup_pass_gate, backup_stop, backup_stop_words, first_pass_baseline, parse_backup_notice,
};
use beebeeb_types::backup::{BackupStopReason, Category};
use uuid::Uuid;

use crate::state_db::{BackupStop, StateDb};

/// The most notices one check reads: the server's cap for `limit` (it has no cursor and no type filter).
pub(crate) const NOTICE_PAGE: usize = 100;

/// Whether the last notices check failed: the card shows `MSG_STOP_CHECK_FAILED` on every backup that is on until a
/// check succeeds. Set by the runner's tick only; cleared when a scheduler starts (a new session) and when a pass
/// that was due cannot run at all (no check is attempted then).
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
    /// The answer's `Date` (server clock): the new `last_pass_at` of every backup that runs.
    pub(crate) server_now_ms: i64,
    pub(crate) clock: RebuildClock,
    /// `Some(t)` when the answer was a full page: it holds every notice written at or after `t` (its oldest row) and
    /// may have cut older ones. `None`: it holds every notice.
    pub(crate) complete_from_ms: Option<i64>,
}

impl NoticeCheck {
    /// The notices for a backup whose last pass was at `last_pass_at_ms`, or `None` when this check may have cut one
    /// newer than that (a full page whose oldest row is newer): core's gate then skips that backup's pass.
    pub(crate) fn notices_since(&self, last_pass_at_ms: i64) -> Option<&[ReceivedNotice]> {
        match self.complete_from_ms {
            Some(oldest) if last_pass_at_ms < oldest => None,
            _ => Some(&self.notices),
        }
    }
}

/// The check from a `GET /api/v1/notifications` answer. It fails (no pass runs) when the answer has no list, no
/// readable `Date`, a clock that moved during the request, or a notice with no readable time where its time matters
/// (a backup notice, or any row of a full page): each leaves the device unable to say which notices are newer than
/// a pass.
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
    let full = rows.len() >= NOTICE_PAGE;
    let mut notices = Vec::new();
    let mut oldest: Option<i64> = None;
    for row in rows {
        let notice = row["type"]
            .as_str()
            .and_then(|kind| parse_backup_notice(kind, &row["data"]));
        if notice.is_none() && !full {
            continue;
        }
        let created_at_ms = row["created_at"]
            .as_str()
            .and_then(|at| chrono::DateTime::parse_from_rfc3339(at).ok())
            .map(|at| at.timestamp_millis())
            .ok_or("a notice has no readable time")?;
        oldest = Some(oldest.map_or(created_at_ms, |at| at.min(created_at_ms)));
        if let Some(notice) = notice {
            notices.push(ReceivedNotice { notice, created_at_ms });
        }
    }
    Ok(NoticeCheck {
        notices,
        server_now_ms,
        clock,
        complete_from_ms: if full { oldest } else { None },
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
/// `first_pass_baseline` from when it was set up. The old engine's last backup is never passed: the state DB does not
/// hold it (0.8.22's mirror kept no record of its passes). A marks read that fails gives 0 (every notice counts).
pub(crate) fn last_pass_at(db: &StateDb, category: &Category, check: &NoticeCheck) -> i64 {
    let mark = db.backup_pass_mark(&category.key()).unwrap_or_default();
    match mark.last_pass_at_ms {
        Some(at) => at,
        None => first_pass_baseline(mark.setup_at_ms.map(|at| on_server_clock(at, check.clock)), None),
    }
}

/// One backup as the device knows it, for [`decide`]: before its listing (`category_folder_live` `None`, the device
/// folder taken as live) or after it.
#[derive(Debug, Clone, Copy)]
pub(crate) struct ListedBackup<'a> {
    pub(crate) category: &'a Category,
    pub(crate) device_folder_id: Option<Uuid>,
    pub(crate) device_folder_live: bool,
    pub(crate) category_folder_id: Option<Uuid>,
    pub(crate) category_folder_live: Option<bool>,
}

impl ListedBackup<'_> {
    fn listing(&self) -> StopListing {
        StopListing {
            // A device folder id that is not a UUID matches no notice; a folder that is gone still stops (step 3).
            device_folder_id: self.device_folder_id.unwrap_or_else(Uuid::nil),
            device_folder_live: self.device_folder_live,
            folders: vec![StopFolder {
                category: self.category.clone(),
                category_folder_id: self.category_folder_id,
                category_folder_live: self.category_folder_live,
            }],
        }
    }
}

/// What core decides for one backup this pass.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PassDecision {
    Run,
    /// It turns off with `reason`; `every_backup` when the reason covers every backup of this device: a removal of
    /// the account's files, a device folder that is gone, or a notice naming this device's folder (step 2).
    Stop {
        reason: BackupStopReason,
        every_backup: bool,
    },
    /// Core's `CheckFailed`: this pass uploads nothing, turns nothing off and keeps `last_pass_at`.
    CheckFailed,
}

/// Core's decision for one backup (`backup_pass_gate`, Amendment 12's steps 1-4 and 12c's failed check).
pub(crate) fn decide(check: &NoticeCheck, last_pass_at_ms: i64, listed: &ListedBackup<'_>) -> PassDecision {
    let notices = check.notices_since(last_pass_at_ms);
    match backup_pass_gate(notices, last_pass_at_ms, &listed.listing()) {
        BackupPassGate::CheckFailed => PassDecision::CheckFailed,
        BackupPassGate::Run { stopped } => {
            let Some(stop) = stopped.into_iter().find(|stop| &stop.category == listed.category) else {
                return PassDecision::Run;
            };
            let every_backup = stop.reason.files_removed()
                || !listed.device_folder_live
                || names_this_device(notices.unwrap_or_default(), last_pass_at_ms, listed);
            PassDecision::Stop {
                reason: stop.reason,
                every_backup,
            }
        }
    }
}

/// Whether a fresh `backup_deleted` notice names this device's folder itself: core asked about the same backup with
/// no category folder and a live device folder, where only such a notice stops it as `backup_deleted`.
fn names_this_device(notices: &[ReceivedNotice], last_pass_at_ms: i64, listed: &ListedBackup<'_>) -> bool {
    let probe = ListedBackup {
        device_folder_live: true,
        category_folder_id: None,
        category_folder_live: None,
        ..*listed
    };
    backup_stop(notices, last_pass_at_ms, &probe.listing())
        .first()
        .is_some_and(|stop| stop.reason == BackupStopReason::BackupDeleted)
}

/// The person turned `key`'s backup on, or chose "Back up again". "Back up again" after a stop starts it over: set
/// up now, no pass yet (Amendment 12c: "Turning backups on is the person's choice, so older notices don't apply"),
/// and its records forgotten, so its first pass rebuilds them from what the server still holds (rule 10) and uploads
/// what the removal took. Switching on a backup that did not stop keeps its `last_pass_at` and records (a notice
/// written while it was off still counts; a file deleted from the backup elsewhere stays deleted) and records the
/// setup time only where none is known (a first switch-on).
pub(crate) fn restart(db: &StateDb, key: &str) -> rusqlite::Result<()> {
    let stopped = db.backup_stops()?.iter().any(|stop| stop.category == key);
    let now = chrono::Utc::now().timestamp_millis();
    if stopped {
        db.restart_backup_pass(key, now)?;
    } else {
        db.note_backup_setup(key, now)?;
    }
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

/// The stops the card needs, read once: none (and no state DB opened) while every folder is on.
pub(crate) fn stopped_backups(cfg: &crate::config::DesktopConfig) -> Vec<BackupStop> {
    if crate::known_folder::KNOWN_FOLDERS
        .iter()
        .all(|kf| cfg.known_folder_enabled(kf.key))
    {
        return Vec::new();
    }
    let db = crate::local_state_purge::state_db_for_config(cfg).ok().flatten();
    db.and_then(|db| db.backup_stops().ok()).unwrap_or_default()
}

/// The card's view of `key`'s backup.
pub(crate) fn stop_view(stops: &[BackupStop], key: &str, folder: &str, enabled: bool) -> StopView {
    view_of(stops, key, folder, enabled, check_failed())
}

/// A stop row a development build wrote before LD-0139 (`device_deleted`, `category_deleted`) reads as `unknown`: a
/// folder gone with no notice.
fn view_of(stops: &[BackupStop], key: &str, folder: &str, enabled: bool, check_failed: bool) -> StopView {
    if enabled {
        return StopView {
            check_note: check_failed.then_some(MSG_STOP_CHECK_FAILED),
            ..StopView::default()
        };
    }
    let Some(stop) = stops.iter().find(|stop| stop.category == key) else {
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
    use beebeeb_core::backup::{BackupRecord, MSG_STOP_ERASE_REQUEST, RecordRef, RecordState};
    use beebeeb_types::backup::BackupNotice;

    fn measured() -> RebuildClock {
        RebuildClock {
            offset_ms: Some(60_000),
            margin_ms: 2_500,
        }
    }

    fn row(kind: &str, data: serde_json::Value, created_at_ms: i64) -> serde_json::Value {
        let at = chrono::DateTime::from_timestamp_millis(created_at_ms).unwrap();
        serde_json::json!({ "type": kind, "data": data, "created_at": at.to_rfc3339() })
    }

    fn owned_db(dir: &tempfile::TempDir) -> StateDb {
        let db = StateDb::open(dir.path().join("state.db")).unwrap();
        db.set_owner(&crate::account_binding::Identity::new(
            Some("u-a"),
            Some("a@beebeeb.io"),
        ))
        .unwrap();
        db
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
        assert_eq!((check.server_now_ms, check.complete_from_ms), (1, None));
        assert_eq!(
            check.notices_since(i64::MIN).map(<[_]>::len),
            Some(2),
            "a page that is not full holds them all"
        );
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
        let mut full: Vec<serde_json::Value> = (0..NOTICE_PAGE as i64)
            .map(|i| row("share_invite", serde_json::json!({}), 1_000 + i))
            .collect();
        full[3]["created_at"] = serde_json::Value::Null;
        assert!(
            check_from(&serde_json::json!({ "notifications": full }), measured(), Some(1)).is_err(),
            "a full page with an untimed row"
        );
    }

    /// The server's newest 100 rows may cut older ones: a backup whose last pass is older than the page's oldest row
    /// gets no notices (core's gate skips its pass); one whose last pass is inside the page gets them.
    #[test]
    fn a_full_page_holds_the_notices_only_from_its_oldest_row() {
        let mut rows: Vec<serde_json::Value> = (0..NOTICE_PAGE as i64 - 1)
            .map(|i| row("share_invite", serde_json::json!({}), 5_000 + i))
            .collect();
        rows.push(row(
            "files_removed",
            serde_json::json!({ "reason": "account_lapsed" }),
            4_000,
        ));
        let check = check_from(&serde_json::json!({ "notifications": rows }), measured(), Some(9_000)).unwrap();
        assert_eq!(check.complete_from_ms, Some(4_000));
        assert!(
            check.notices_since(3_999).is_none(),
            "a notice newer than 3,999 may have been cut"
        );
        assert_eq!(check.notices_since(4_000).map(<[_]>::len), Some(1));
        let pictures = Category::Pictures;
        let listed = ListedBackup {
            category: &pictures,
            device_folder_id: None,
            device_folder_live: true,
            category_folder_id: None,
            category_folder_live: Some(true),
        };
        assert_eq!(decide(&check, 3_999, &listed), PassDecision::CheckFailed);
    }

    #[test]
    fn the_first_pass_baseline_is_the_setup_on_the_servers_clock_leaning_early() {
        let dir = tempfile::tempdir().unwrap();
        let db = owned_db(&dir);
        let check = NoticeCheck {
            notices: Vec::new(),
            server_now_ms: 0,
            clock: measured(),
            complete_from_ms: None,
        };
        assert_eq!(last_pass_at(&db, &Category::Pictures, &check), 0, "neither known");
        db.note_backup_setup("pictures", 1_000_000).unwrap();
        // Set up at 1,000,000 on a clock 60 s behind the server's, measured within 2.5 s.
        assert_eq!(
            last_pass_at(&db, &Category::Pictures, &check),
            1_000_000 + 60_000 - 2_500
        );
        db.set_backup_last_pass("pictures", 42).unwrap();
        assert_eq!(last_pass_at(&db, &Category::Pictures, &check), 42, "a pass's mark wins");
    }

    /// Step 2's scope: a notice naming this device's folder (no category) turns every backup off; one naming a
    /// category folder turns that backup off only; a removal of the account's files turns every backup off.
    #[test]
    fn a_stop_says_whether_it_covers_every_backup() {
        let (device, category_folder) = (Uuid::new_v4(), Uuid::new_v4());
        let notice = |notice| ReceivedNotice {
            notice,
            created_at_ms: 10,
        };
        let deleted = |category_folder_id: Option<Uuid>| {
            notice(BackupNotice::BackupDeleted {
                device_folder_id: device.to_string(),
                category_folder_id: category_folder_id.map(|id| id.to_string()),
            })
        };
        let pictures = Category::Pictures;
        let listed = ListedBackup {
            category: &pictures,
            device_folder_id: Some(device),
            device_folder_live: true,
            category_folder_id: Some(category_folder),
            category_folder_live: Some(true),
        };
        let decision = |notices: Vec<ReceivedNotice>| {
            let check = NoticeCheck {
                notices,
                server_now_ms: 20,
                clock: measured(),
                complete_from_ms: None,
            };
            decide(&check, 5, &listed)
        };
        let backup_deleted = BackupStopReason::BackupDeleted;
        assert_eq!(
            decision(vec![deleted(None)]),
            PassDecision::Stop {
                reason: backup_deleted,
                every_backup: true
            }
        );
        assert_eq!(
            decision(vec![deleted(Some(category_folder))]),
            PassDecision::Stop {
                reason: backup_deleted,
                every_backup: false
            }
        );
        let erased = notice(BackupNotice::FilesRemoved {
            reason: BackupStopReason::EraseRequest,
        });
        assert_eq!(
            decision(vec![erased]),
            PassDecision::Stop {
                reason: BackupStopReason::EraseRequest,
                every_backup: true
            }
        );
        assert_eq!(decision(Vec::new()), PassDecision::Run);
    }

    /// "Back up again" after a stop starts over and forgets the records; switching on a backup that did not stop
    /// keeps its last pass and records, and records a setup time only where none is known.
    #[test]
    fn back_up_again_starts_over_and_a_switch_on_keeps_the_last_pass() {
        let dir = tempfile::tempdir().unwrap();
        let db = owned_db(&dir);
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
        db.record_backup_stop(&BackupStop {
            category: "pictures".into(),
            reason: "erase_request".into(),
            at_ms: 1,
        })
        .unwrap();

        restart(&db, "pictures").unwrap();
        restart(&db, "music").unwrap();
        restart(&db, "videos").unwrap();
        assert!(db.backup_stops().unwrap().is_empty());
        assert!(
            db.backup_records("pictures").unwrap().is_empty(),
            "Back up again rebuilds its records"
        );
        let pictures = db.backup_pass_mark("pictures").unwrap();
        assert_eq!(pictures.last_pass_at_ms, None, "Back up again starts over");
        assert!(
            pictures.setup_at_ms.is_some_and(|at| at > 1_700_000_000_000),
            "set up now"
        );
        assert_eq!(
            db.backup_records("music").unwrap().len(),
            1,
            "a switch-on keeps the records"
        );
        assert_eq!(
            db.backup_pass_mark("music").unwrap().last_pass_at_ms,
            Some(7),
            "and the last pass"
        );
        let videos = db.backup_pass_mark("videos").unwrap();
        assert!(
            videos.setup_at_ms.is_some() && videos.last_pass_at_ms.is_none(),
            "a first switch-on is a setup"
        );
        db.note_backup_setup("videos", 1).unwrap();
        assert_eq!(
            db.backup_pass_mark("videos").unwrap().setup_at_ms,
            videos.setup_at_ms,
            "the first setup stays"
        );
    }

    #[test]
    fn a_stop_says_why_in_cores_words_and_offers_back_up_again() {
        let stops = [
            BackupStop {
                category: "pictures".into(),
                reason: "erase_request".into(),
                at_ms: 1,
            },
            BackupStop {
                category: "music".into(),
                reason: "category_deleted".into(),
                at_ms: 1,
            },
        ];
        let view = view_of(&stops, "pictures", "Pictures", false, false);
        assert_eq!(view.stop_words.as_deref(), Some(MSG_STOP_ERASE_REQUEST));
        assert_eq!(view.stop_action, Some("Back up again"));
        let legacy = view_of(&stops, "music", "Music", false, false).stop_words.unwrap();
        assert!(
            legacy.ends_with("'s backup of Music was deleted, so it is off."),
            "{legacy}"
        );
        assert_eq!(
            view_of(&stops, "documents", "Documents", false, false),
            StopView::default()
        );
        // On again: no stop words, whatever the old row says; a failed check says so in core's words.
        assert_eq!(
            view_of(&stops, "pictures", "Pictures", true, false),
            StopView::default()
        );
        let waiting = view_of(&stops, "pictures", "Pictures", true, true);
        assert_eq!(
            waiting,
            StopView {
                check_note: Some(MSG_STOP_CHECK_FAILED),
                ..StopView::default()
            }
        );
        assert_eq!(MSG_STOP_CHECK_FAILED, "Couldn't check \u{2014} will retry.");
    }
}
