//! LD-0139 (spec Amendments 12, 12a, 12b, 12c and 12d): when a backup turns off, and why, desktop's side of core's
//! `backup_pass_gate_v2` (core v0.2.5). Every tick first reads the account's backup notices: every page of
//! `GET /api/v1/notifications?type=files_removed,backup_deleted&since=…` (server LD-0141), from core's `notices_since`
//! of the oldest `last_pass_at` among the tick's backups. Core decides from them, each backup's `last_pass_at`, the
//! notice ids that backup already acknowledged and this pass's listing whether it runs, turns off, or is skipped
//! because the notices could not be read completely. A backup that turns off says why in core's words with "Back up
//! again"; a skipped one says "Couldn't check — will retry." Core's answer replaces the backup's acknowledged set, in
//! the same write as its stop or its new `last_pass_at` (`StateDb::record_backup_stops`, `record_backup_pass`).
//!
//! Every time handed to core is on the server's clock: `last_pass_at` is a check's `Date` (its first page's), and the
//! setup time is moved onto it with the check's measured offset.
//!
//! Logs carry categories and reasons, never a name or a path.

use std::sync::atomic::{AtomicBool, Ordering};

use beebeeb_core::backup::{
    BackupPassGateV2, ListedNotice, MSG_BACK_UP_AGAIN, MSG_STOP_CHECK_FAILED, RebuildClock, StopFolder, StopListing,
    backup_pass_gate_v2, backup_stop_v2, backup_stop_words, first_pass_baseline, notices_since, parse_backup_notice,
};
use beebeeb_types::backup::{BackupStopReason, Category};
use uuid::Uuid;

use crate::state_db::{BackupPassMark, BackupStop, StateDb};

/// Whether the last notices check failed: the card shows `MSG_STOP_CHECK_FAILED` on every backup that is on until a
/// check succeeds. Set by the runner's tick only; cleared when a scheduler starts (a new session) and when a pass
/// that was due cannot run at all (no check is attempted then).
static CHECK_FAILED: AtomicBool = AtomicBool::new(false);

pub fn set_check_failed(failed: bool) {
    CHECK_FAILED.store(failed, Ordering::Relaxed);
}

pub fn check_failed() -> bool {
    CHECK_FAILED.load(Ordering::Relaxed)
}

/// The most pages one notice check reads (500 notices each, server LD-0141): 100,000 backup notices, far more than an
/// account writes (one per removal run, one per step-up delete). A listing that goes on past it fails the check (core's
/// `CheckFailed`), so a server that keeps answering new cursors cannot hold a tick forever.
pub const MAX_NOTICE_PAGES: usize = 200;

/// One complete read of the account's backup notices (every page), the server's clock when it answered the first
/// page, and the device's offset from it, measured on that request.
#[derive(Debug, Clone)]
pub struct NoticeCheck {
    /// Every backup notice from `since_ms` on, with its server id and server-clock time.
    pub notices: Vec<ListedNotice>,
    /// The `since` the check asked from (server clock, inclusive): it holds every notice written at or after it, and
    /// nothing about earlier ones. A backup whose window starts earlier is not covered ([`decide`]).
    pub since_ms: i64,
    /// The first page's `Date` (server clock): the new `last_pass_at` of every backup that runs. Later pages' `Date`s
    /// are not read; a skew between API nodes or the header's one-second resolution is inside `NOTICE_OVERLAP_MS`.
    pub server_now_ms: i64,
    pub clock: RebuildClock,
}

/// The `since` of a tick's notice check, from the marks of the backups the tick gates (read once, the same marks the
/// gate decides with): core's `notices_since` of the oldest `last_pass_at`, so the listing holds every notice any of
/// them can count. A backup with no pass yet (its baseline is the setup time on this device's clock, placed on the
/// server's only by this check) asks from 0: every backup notice of the account. No marks: 0.
pub fn check_since<'a>(marks: impl IntoIterator<Item = &'a BackupPassMark>) -> i64 {
    marks
        .into_iter()
        .map(|mark| mark.last_pass_at_ms.map_or(0, notices_since))
        .min()
        .unwrap_or(0)
}

/// One page of a check, read: its backup notices and the next page's cursor (`None`: the listing is complete). It
/// fails when the answer has no list, has no `next_cursor` field (a server without the filtered listing answers its
/// newest rows of every type and may cut one), has a cursor that is neither a non-empty string nor null, or holds a
/// backup notice with no readable time. A row of another type counts for nothing (the filter leaves none).
pub fn read_page(body: &serde_json::Value) -> Result<(Vec<ListedNotice>, Option<String>), String> {
    let rows = body["notifications"]
        .as_array()
        .ok_or("the notices answer has no list")?;
    let next = match body.get("next_cursor") {
        Some(serde_json::Value::Null) => None,
        Some(serde_json::Value::String(next)) if !next.is_empty() => Some(next.clone()),
        Some(_) => return Err("the notices answer has a cursor that is not one".into()),
        None => return Err("the notices answer is not the filtered listing".into()),
    };
    let mut notices = Vec::with_capacity(rows.len());
    let mut without_id = 0usize;
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
            .ok_or("a notice has no readable time")?;
        let id = row["id"].as_str().map(str::to_string);
        without_id += usize::from(id.is_none());
        notices.push(ListedNotice {
            id,
            notice,
            created_at_ms,
        });
    }
    if without_id > 0 {
        // Core never acknowledges a notice without an id: it counts on every pass whose window holds it.
        tracing::warn!(without_id, "backup: notices without an id; they are never acknowledged");
    }
    Ok((notices, next))
}

/// A check from its first page, and the next page's cursor. It fails as [`read_page`] does, and when the answer has no
/// readable `Date` or this device's clock moved during the request: either leaves the device unable to say which
/// notices are newer than a pass.
pub fn first_page(
    body: &serde_json::Value,
    clock: RebuildClock,
    server_now_ms: Option<i64>,
    since_ms: i64,
) -> Result<(NoticeCheck, Option<String>), String> {
    let server_now_ms = server_now_ms.ok_or("the notices answer has no readable Date")?;
    if clock.offset_ms.is_none() {
        return Err("this device's clock moved during the notices check".into());
    }
    let (notices, next) = read_page(body)?;
    Ok((
        NoticeCheck {
            notices,
            since_ms,
            server_now_ms,
            clock,
        },
        next,
    ))
}

/// `device_ms` (this device's clock) on the server's clock, leaning early by the measurement's margin: an earlier
/// `last_pass_at` lets more notices count, never fewer.
fn on_server_clock(device_ms: i64, clock: RebuildClock) -> i64 {
    let margin = i64::try_from(clock.margin_ms).unwrap_or(i64::MAX);
    device_ms
        .saturating_add(clock.offset_ms.unwrap_or(0))
        .saturating_sub(margin)
}

/// A backup's `last_pass_at` for core (server clock), from its marks: its last pass's check, else, at its first pass,
/// core's `first_pass_baseline` from when it was set up, moved onto the server's clock with the check's `clock`. The
/// old engine's last backup is never passed: the state DB does not hold it (0.8.22's mirror kept no record of its
/// passes). Marks that cannot be read are the default: 0 (every notice counts).
pub fn last_pass_at(mark: &BackupPassMark, clock: RebuildClock) -> i64 {
    match mark.last_pass_at_ms {
        Some(at) => at,
        None => first_pass_baseline(mark.setup_at_ms.map(|at| on_server_clock(at, clock)), None),
    }
}

/// The acknowledged set core keeps for a backup after `check`, from its marks: the id of every notice in its window
/// (`backup_stop_v2`'s answer, which does not depend on the listing). For each backup a stop that covers every backup
/// turns off besides the one whose gate decided it: each stores its own set in the stop's write.
pub fn acknowledged_after(check: &NoticeCheck, mark: &BackupPassMark, category: &Category) -> Vec<String> {
    let listing = StopListing {
        device_folder_id: Uuid::nil(),
        device_folder_live: true,
        folders: vec![StopFolder {
            category: category.clone(),
            category_folder_id: None,
            category_folder_live: None,
        }],
    };
    backup_stop_v2(
        &check.notices,
        last_pass_at(mark, check.clock),
        &mark.acknowledged,
        &listing,
    )
    .acknowledged
}

/// One backup as the device knows it, for [`decide`]: before its listing (`category_folder_live` `None`, the device
/// folder taken as live) or after it.
#[derive(Debug, Clone, Copy)]
pub struct ListedBackup<'a> {
    pub category: &'a Category,
    pub device_folder_id: Option<Uuid>,
    pub device_folder_live: bool,
    pub category_folder_id: Option<Uuid>,
    pub category_folder_live: Option<bool>,
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

/// What core decides for one backup this pass. `acknowledged` (core's answer) replaces the backup's stored set, in
/// the same write as its new `last_pass_at` (a run) or its stop.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PassDecision {
    Run {
        acknowledged: Vec<String>,
    },
    /// It turns off with `reason`; `every_backup` when the reason covers every backup of this device: a removal of
    /// the account's files, a device folder that is gone, or a notice naming this device's folder (step 2).
    Stop {
        reason: BackupStopReason,
        every_backup: bool,
        acknowledged: Vec<String>,
    },
    /// Core's `CheckFailed` (`check` is `None`: a page could not be read): this pass uploads nothing, turns nothing
    /// off and keeps both `last_pass_at` and the acknowledged set.
    CheckFailed,
}

/// Core's decision for one backup (`backup_pass_gate_v2`: Amendment 12's steps 1-4, 12c's failed check and 12d's
/// overlap margin and acknowledged ids). `check` is `None` when the notices could not be read completely. A check that
/// does not cover this backup's window (it asked from later than `notices_since(last_pass_at_ms)`) holds them only in
/// part, so core gets `None` too: `CheckFailed` (review 2026-11-08; #233's `complete_from_ms` guard, restored).
pub fn decide(
    check: Option<&NoticeCheck>,
    last_pass_at_ms: i64,
    acknowledged: &[String],
    listed: &ListedBackup<'_>,
) -> PassDecision {
    let notices = check
        .filter(|check| notices_since(last_pass_at_ms) >= check.since_ms)
        .map(|check| check.notices.as_slice());
    match backup_pass_gate_v2(notices, last_pass_at_ms, acknowledged, &listed.listing()) {
        BackupPassGateV2::CheckFailed => PassDecision::CheckFailed,
        BackupPassGateV2::Run {
            stopped,
            acknowledged: kept,
        } => {
            let Some(stop) = stopped.into_iter().find(|stop| &stop.category == listed.category) else {
                return PassDecision::Run { acknowledged: kept };
            };
            let every_backup = stop.reason.files_removed()
                || !listed.device_folder_live
                || names_this_device(notices.unwrap_or_default(), last_pass_at_ms, acknowledged, listed);
            PassDecision::Stop {
                reason: stop.reason,
                every_backup,
                acknowledged: kept,
            }
        }
    }
}

/// Whether a notice that counts names this device's folder itself: core asked about the same backup with no category
/// folder and a live device folder, where only such a `backup_deleted` notice stops it as `backup_deleted`.
fn names_this_device(
    notices: &[ListedNotice],
    last_pass_at_ms: i64,
    acknowledged: &[String],
    listed: &ListedBackup<'_>,
) -> bool {
    let probe = ListedBackup {
        device_folder_live: true,
        category_folder_id: None,
        category_folder_live: None,
        ..*listed
    };
    backup_stop_v2(notices, last_pass_at_ms, acknowledged, &probe.listing())
        .stopped
        .first()
        .is_some_and(|stop| stop.reason == BackupStopReason::BackupDeleted)
}

/// LD-0139 (review 2026-11-08): every backup with a stop row has its switch off, persisted. A stop commits its row
/// first and switches the backup off in the settings after (`DesktopBackupHost::stop_backups`); an app killed in
/// between, or a settings write that failed (on Windows, another program holding `desktop.toml`), leaves the switch on.
/// Run when the scheduler starts and before each pass. The stopped categories, or the read's error (the caller then
/// runs no pass: a failed read must never read as "nothing is stopped"). A switch-off that fails is logged and tried
/// again next time; the stop row alone keeps the backup from passing.
pub fn reconcile_stopped_switches(
    identity: &'static crate::identity::EngineIdentity,
    db: &StateDb,
) -> rusqlite::Result<Vec<String>> {
    let stopped: Vec<String> = db.backup_stops()?.into_iter().map(|stop| stop.category).collect();
    if stopped.is_empty() {
        return Ok(stopped);
    }
    let still_on: Vec<String> = match crate::config::DesktopConfig::load_for(identity) {
        Ok(cfg) => stopped
            .iter()
            .filter(|key| cfg.known_folder_enabled(key))
            .cloned()
            .collect(),
        Err(_) => stopped.clone(),
    };
    if !still_on.is_empty() {
        let switched = crate::backup::host::switch_off_in_settings(identity, &still_on);
        tracing::warn!(
            still_on = still_on.len(),
            switched,
            "backup: a stopped backup's switch was on; switched off"
        );
    }
    Ok(stopped)
}

/// The person turned `key`'s backup on, or chose "Back up again". "Back up again" after a stop starts it over: set
/// up now, no pass yet (Amendment 12c: "Turning backups on is the person's choice, so older notices don't apply"),
/// and its records forgotten, so its first pass rebuilds them from what the server still holds (rule 10) and uploads
/// what the removal took. Switching on a backup that did not stop keeps its `last_pass_at` and records (a notice
/// written while it was off still counts; a file deleted from the backup elsewhere stays deleted) and records the
/// setup time only where none is known (a first switch-on). Either way the acknowledged set is kept (Amendment 12d):
/// a notice this backup already acted on never stops it again, even inside the new baseline's overlap margin.
pub fn restart(db: &StateDb, key: &str) -> rusqlite::Result<()> {
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
pub struct StopView {
    /// Why this backup is off (`backup_stop_words`); `None` while it is on or was switched off by the person.
    pub stop_words: Option<String>,
    /// The action under the words: "Back up again".
    pub stop_action: Option<&'static str>,
    /// "Couldn't check — will retry." on a backup that is on, while the last notices check failed.
    pub check_note: Option<&'static str>,
}

/// The stops the card needs, read once: none (and no state DB opened) while every folder is on.
///
/// Task 2488: `pub` only with the `direct-state-db` feature (the Tauri shell): it opens the state database itself,
/// asking nobody whose it is. The native shell has no such reader.
#[cfg(any(test, feature = "direct-state-db"))]
pub fn stopped_backups(
    identity: &'static crate::identity::EngineIdentity,
    cfg: &crate::config::DesktopConfig,
) -> Vec<BackupStop> {
    if crate::known_folder::KNOWN_FOLDERS
        .iter()
        .all(|kf| cfg.known_folder_enabled(kf.key))
    {
        return Vec::new();
    }
    let db = crate::local_state::state_db_for_config_unbound(identity, cfg)
        .ok()
        .flatten();
    db.and_then(|db| db.backup_stops().ok()).unwrap_or_default()
}

/// The card's view of `key`'s backup.
pub fn stop_view(stops: &[BackupStop], key: &str, folder: &str, enabled: bool) -> StopView {
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
    use beebeeb_core::backup::{BackupRecord, MSG_STOP_ERASE_REQUEST, NOTICE_OVERLAP_MS, RecordRef, RecordState};

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

    fn listed(kind: &str, data: serde_json::Value, created_at_ms: i64) -> ListedNotice {
        let id = uuid::Uuid::new_v4().to_string();
        ListedNotice {
            id: Some(id),
            notice: parse_backup_notice(kind, &data).unwrap(),
            created_at_ms,
        }
    }

    fn check_of(notices: Vec<ListedNotice>) -> NoticeCheck {
        NoticeCheck {
            notices,
            since_ms: 0,
            server_now_ms: 0,
            clock: measured(),
        }
    }

    #[test]
    fn a_page_reads_backup_notices_with_their_ids_and_its_cursor() {
        let body = serde_json::json!({ "next_cursor": "c2", "notifications": [
            { "id": "a", "type": "share_invite", "data": null, "created_at": "not a time" },
            { "id": "b", "type": "files_removed", "data": { "reason": "erase_request" },
              "created_at": "2026-11-07T19:00:00.250Z" },
            { "type": "files_removed", "data": null, "created_at": "2026-11-07T19:00:01Z" },
        ]});
        let (notices, next) = read_page(&body).unwrap();
        let read: Vec<(Option<&str>, BackupStopReason, i64)> = notices
            .iter()
            .map(|n| (n.id.as_deref(), n.notice.reason(), n.created_at_ms))
            .collect();
        assert_eq!(
            read,
            [
                (Some("b"), BackupStopReason::EraseRequest, 1_794_078_000_250),
                (None, BackupStopReason::UnknownRemoval, 1_794_078_001_000)
            ]
        );
        assert_eq!(next.as_deref(), Some("c2"));
        let last = serde_json::json!({ "next_cursor": null, "notifications": [] });
        assert_eq!(
            read_page(&last).unwrap(),
            (Vec::new(), None),
            "a null cursor ends the listing"
        );
    }

    /// A page the device cannot be sure of fails the whole check (core's `CheckFailed`), never a partial listing.
    #[test]
    fn a_page_that_cannot_be_read_fails() {
        let page = |body: serde_json::Value| read_page(&body);
        assert!(page(serde_json::json!({ "error": "x" })).is_err(), "no list");
        assert!(
            page(serde_json::json!({ "notifications": [] })).is_err(),
            "no next_cursor: a server without the filtered listing (its newest rows of every type)"
        );
        assert!(
            page(serde_json::json!({ "notifications": [], "next_cursor": "" })).is_err(),
            "an empty cursor"
        );
        assert!(
            page(serde_json::json!({ "notifications": [], "next_cursor": 5 })).is_err(),
            "a cursor not a string"
        );
        let untimed = serde_json::json!({ "next_cursor": null,
            "notifications": [{ "id": "x", "type": "backup_deleted", "data": {}, "created_at": null }] });
        assert!(page(untimed).is_err(), "a backup notice with no time");
    }

    #[test]
    fn the_first_page_fixes_the_checks_date_and_clock() {
        let rows = [row("files_removed", serde_json::json!({}), 7)];
        let body = serde_json::json!({ "notifications": rows, "next_cursor": "n" });
        assert!(first_page(&body, measured(), None, 0).is_err(), "no Date");
        assert!(
            first_page(&body, RebuildClock::UNKNOWN, Some(1), 0).is_err(),
            "a clock that moved"
        );
        let (check, next) = first_page(&body, measured(), Some(9_000), 4_000).unwrap();
        assert_eq!(
            (check.since_ms, check.server_now_ms, check.clock, check.notices.len()),
            (4_000, 9_000, measured(), 1)
        );
        assert_eq!(next.as_deref(), Some("n"));
    }

    /// The check asks from the oldest window among the gated backups' marks (core's `notices_since`); a backup with
    /// no pass yet asks from 0.
    #[test]
    fn the_check_asks_from_the_oldest_backups_window_and_from_0_before_a_first_pass() {
        let passed = |at: i64| BackupPassMark {
            last_pass_at_ms: Some(at),
            ..BackupPassMark::default()
        };
        let (pictures, music) = (passed(2_000_000_000), passed(1_000_000_000));
        let videos = BackupPassMark {
            setup_at_ms: Some(3_000_000_000),
            ..BackupPassMark::default()
        };
        assert_eq!(check_since([&pictures]), 2_000_000_000 - NOTICE_OVERLAP_MS);
        assert_eq!(check_since([&pictures, &music]), 1_000_000_000 - NOTICE_OVERLAP_MS);
        assert_eq!(
            check_since([&pictures, &music, &videos]),
            0,
            "no pass yet: every notice"
        );
        assert_eq!(check_since([]), 0);
    }

    /// Review (2026-11-08): a check that asked from later than a backup's window holds its notices only in part, so
    /// the backup's pass is core's `CheckFailed`, whatever the partial listing says; at the window's edge it runs.
    #[test]
    fn a_check_that_does_not_cover_the_window_is_a_failed_check() {
        let pictures = Category::Pictures;
        let listing = ListedBackup {
            category: &pictures,
            device_folder_id: None,
            device_folder_live: true,
            category_folder_id: None,
            category_folder_live: Some(true),
        };
        let last = 10_000_000;
        let mut check = check_of(Vec::new());
        check.since_ms = notices_since(last) + 1;
        assert_eq!(decide(Some(&check), last, &[], &listing), PassDecision::CheckFailed);
        check.since_ms = notices_since(last);
        assert_eq!(
            decide(Some(&check), last, &[], &listing),
            PassDecision::Run {
                acknowledged: Vec::new()
            }
        );
    }

    #[test]
    fn the_first_pass_baseline_is_the_setup_on_the_servers_clock_leaning_early() {
        let dir = tempfile::tempdir().unwrap();
        let db = owned_db(&dir);
        let mark = |db: &StateDb| db.backup_pass_mark("pictures").unwrap();
        assert_eq!(last_pass_at(&mark(&db), measured()), 0, "neither known");
        db.note_backup_setup("pictures", 1_000_000).unwrap();
        // Set up at 1,000,000 on a clock 60 s behind the server's, measured within 2.5 s.
        assert_eq!(last_pass_at(&mark(&db), measured()), 1_000_000 + 60_000 - 2_500);
        db.set_backup_last_pass("pictures", 42).unwrap();
        assert_eq!(last_pass_at(&mark(&db), measured()), 42, "a pass's mark wins");
    }

    /// Step 2's scope: a notice naming this device's folder (no category) turns every backup off; one naming a
    /// category folder turns that backup off only; a removal of the account's files turns every backup off. Each
    /// answer carries core's acknowledged set: the ids in the window.
    #[test]
    fn a_stop_says_whether_it_covers_every_backup() {
        let (device, category_folder) = (Uuid::new_v4(), Uuid::new_v4());
        let deleted = |category_folder_id: Option<Uuid>| {
            let mut data = serde_json::json!({ "device_folder_id": device.to_string() });
            if let Some(id) = category_folder_id {
                data["category_folder_id"] = serde_json::json!(id.to_string());
            }
            listed("backup_deleted", data, 10)
        };
        let pictures = Category::Pictures;
        let listing = ListedBackup {
            category: &pictures,
            device_folder_id: Some(device),
            device_folder_live: true,
            category_folder_id: Some(category_folder),
            category_folder_live: Some(true),
        };
        let decision = |notice: Option<ListedNotice>| {
            let ids: Vec<String> = notice.iter().filter_map(|n| n.id.clone()).collect();
            (
                decide(Some(&check_of(notice.into_iter().collect())), 5, &[], &listing),
                ids,
            )
        };
        let stop = |reason, every_backup, acknowledged| PassDecision::Stop {
            reason,
            every_backup,
            acknowledged,
        };
        let backup_deleted = BackupStopReason::BackupDeleted;
        let (got, ids) = decision(Some(deleted(None)));
        assert_eq!(got, stop(backup_deleted, true, ids));
        let (got, ids) = decision(Some(deleted(Some(category_folder))));
        assert_eq!(got, stop(backup_deleted, false, ids));
        let erased = listed("files_removed", serde_json::json!({ "reason": "erase_request" }), 10);
        let (got, ids) = decision(Some(erased));
        assert_eq!(got, stop(BackupStopReason::EraseRequest, true, ids));
        let (got, _) = decision(None);
        assert_eq!(
            got,
            PassDecision::Run {
                acknowledged: Vec::new()
            }
        );
    }

    /// Amendment 12d through core's v2 gate: a notice inside the overlap margin before the last pass counts; one
    /// already acknowledged does not; one older than the margin is out of the window (and its id is dropped from the
    /// set); notices not read completely are core's `CheckFailed`.
    #[test]
    fn the_overlap_margin_and_the_acknowledged_ids_decide() {
        let pictures = Category::Pictures;
        let listing = ListedBackup {
            category: &pictures,
            device_folder_id: None,
            device_folder_live: true,
            category_folder_id: None,
            category_folder_live: Some(true),
        };
        let last = 10_000_000;
        let inside = listed(
            "files_removed",
            serde_json::json!({ "reason": "account_lapsed" }),
            last - 60_000,
        );
        let id = inside.id.clone().unwrap();
        let check = check_of(vec![inside]);
        assert_eq!(
            decide(Some(&check), last, &[], &listing),
            PassDecision::Stop {
                reason: BackupStopReason::AccountLapsed,
                every_backup: true,
                acknowledged: vec![id.clone()]
            },
            "committed during the previous pass: inside the margin, it stops"
        );
        assert_eq!(
            decide(Some(&check), last, std::slice::from_ref(&id), &listing),
            PassDecision::Run {
                acknowledged: vec![id.clone()]
            },
            "acknowledged: it never stops again, and stays acknowledged while in the window"
        );
        let older = listed(
            "files_removed",
            serde_json::json!({ "reason": "account_lapsed" }),
            last - NOTICE_OVERLAP_MS,
        );
        assert_eq!(
            decide(Some(&check_of(vec![older])), last, std::slice::from_ref(&id), &listing),
            PassDecision::Run {
                acknowledged: Vec::new()
            },
            "older than the margin: out of the window, and the set drops what left it"
        );
        assert_eq!(decide(None, last, &[], &listing), PassDecision::CheckFailed);
    }

    /// A stop that covers every backup gives each other stopped backup its own window's ids: EVERY id in the window,
    /// a notice about another device's folder included (core's documented `acknowledged`, which does not depend on
    /// the listing; this pins it, since `acknowledged_after` passes core a stand-in listing).
    #[test]
    fn the_acknowledged_set_after_a_check_is_the_backups_own_window() {
        let last = 10_000_000;
        let (new, old) = (
            listed("files_removed", serde_json::json!({}), last - 1),
            listed("files_removed", serde_json::json!({}), last - NOTICE_OVERLAP_MS - 1),
        );
        let elsewhere = listed(
            "backup_deleted",
            serde_json::json!({ "device_folder_id": Uuid::new_v4().to_string() }),
            last - 2,
        );
        let (new_id, old_id, elsewhere_id) = (
            new.id.clone().unwrap(),
            old.id.clone().unwrap(),
            elsewhere.id.clone().unwrap(),
        );
        let check = check_of(vec![new, old, elsewhere]);
        let mark = BackupPassMark {
            last_pass_at_ms: Some(last),
            setup_at_ms: None,
            acknowledged: vec![old_id.clone()],
        };
        let mut in_window = vec![new_id, elsewhere_id];
        in_window.sort();
        assert_eq!(acknowledged_after(&check, &mark, &Category::Music), in_window);
        let before = BackupPassMark {
            last_pass_at_ms: Some(last - NOTICE_OVERLAP_MS),
            ..mark
        };
        let mut all = [in_window, vec![old_id]].concat();
        all.sort();
        assert_eq!(acknowledged_after(&check, &before, &Category::Music), all);
    }

    /// The acknowledged set is stored with the last pass, read back after a restart (the state DB reopened), and kept
    /// by "Back up again" and by a switch-on; a set that cannot be read is empty.
    #[test]
    fn the_acknowledged_set_survives_a_restart_and_back_up_again() {
        let dir = tempfile::tempdir().unwrap();
        let acknowledged = vec!["n-1".to_string(), "n-2".to_string()];
        {
            let db = owned_db(&dir);
            db.record_backup_pass(&crate::state_db::BackupPassRecord {
                category: "pictures".into(),
                last_pass_at_ms: 77,
                acknowledged: acknowledged.clone(),
            })
            .unwrap();
        }
        let db = StateDb::open(dir.path().join("state.db")).unwrap();
        let mark = db.backup_pass_mark("pictures").unwrap();
        assert_eq!(
            (mark.last_pass_at_ms, &mark.acknowledged),
            (Some(77), &acknowledged),
            "after a restart"
        );
        db.restart_backup_pass("pictures", 5).unwrap();
        let mark = db.backup_pass_mark("pictures").unwrap();
        assert_eq!(
            (mark.last_pass_at_ms, mark.setup_at_ms, &mark.acknowledged),
            (None, Some(5), &acknowledged),
            "Back up again starts over and keeps the set"
        );
        db.note_backup_setup("pictures", 9).unwrap();
        assert_eq!(
            db.backup_pass_mark("pictures").unwrap().acknowledged,
            acknowledged,
            "a switch-on keeps it"
        );
        let conn = rusqlite::Connection::open(dir.path().join("state.db")).unwrap();
        conn.execute("UPDATE backup_pass_marks SET acknowledged_json = 'not json'", [])
            .unwrap();
        assert!(
            db.backup_pass_mark("pictures").unwrap().acknowledged.is_empty(),
            "unreadable: empty"
        );
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
