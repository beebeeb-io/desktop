//! Task 2409: the list behind "Not sent" (`state_db/unsent.rs`) holds exactly the changes the count counts, in the same
//! groups, for every kind of change and every reason; downloads are in neither (task 2224).

use super::*;
use std::collections::BTreeMap;

const NOW: i64 = 1_800_000_000;

/// Every kind the queue holds, with the word it is listed under. `None`: a download, never listed or counted.
const KINDS: [(&str, Option<&str>, Option<&str>); 12] = [
    ("upload_file", None, Some("new_file")),
    (
        "upload_version",
        Some(r#"{"operation":"create_file"}"#),
        Some("new_file"),
    ),
    ("upload_version", Some(r#"{"operation":"update_file"}"#), Some("edit")),
    ("create_folder", None, Some("folder")),
    ("rename_file", None, Some("rename")),
    ("move_file", None, Some("move")),
    ("trash_file", None, Some("delete")),
    ("restore_file", None, Some("restore")),
    ("restore_version", None, Some("restore")),
    // A kind this build does not know reads as an upload (`OperationKind::from_str`): it is counted, so it is listed.
    ("a_kind_from_a_later_build", None, Some("new_file")),
    ("hydrate_file", None, None),
    ("pin_tree", None, None),
];

/// How a row is not sent: its pause reason (stored as is), attempts and retry time, and the group it is counted in.
/// `secret-reason` is a pause reason this build does not know: counted, and listed, as `other`.
const STATES: [(&str, Option<&str>, i64, i64, &str); 13] = [
    ("due", None, 0, 0, "waiting"),
    ("backing-off", None, 3, NOW + 600, "waiting"),
    ("gave-up", None, 5, 0, "gave_up"),
    ("auth", Some("auth"), 0, 0, "auth"),
    ("quota", Some("quota"), 0, 0, "quota"),
    ("permission", Some("permission"), 0, 0, "permission"),
    ("locked", Some("locked"), 0, 0, "locked"),
    ("key-replaced", Some("key_replaced"), 0, 0, "key_replaced"),
    ("found", Some("found_while_paused"), 0, 0, "found_while_paused"),
    ("review", Some("needs_review"), 0, 0, "needs_review"),
    (
        "stored-for-folder",
        Some("waiting_for_folder"),
        0,
        0,
        "waiting_for_folder",
    ),
    ("unknown", Some("secret-reason"), 0, 0, "other"),
    ("paused-and-out-of-attempts", Some("quota"), 5, 0, "quota"),
];

fn db() -> (tempfile::TempDir, StateDb) {
    let dir = tempfile::tempdir().unwrap();
    let db = StateDb::open(dir.path().join("state.db")).unwrap();
    (dir, db)
}

fn sql(db: &StateDb, statement: &str, values: impl rusqlite::Params) {
    db.0.lock().unwrap().execute(statement, values).unwrap();
}

/// One queue row of `kind` in `state`, for a file with a row at `docs/<id>.txt` (so its listed path is that one).
fn queue(
    db: &StateDb,
    id: &str,
    kind: (&str, Option<&str>),
    state: (Option<&str>, i64, i64),
    parent: Option<&str>,
    at: i64,
) {
    sql(
        db,
        "INSERT INTO files (file_id, path) VALUES (?1, ?2)",
        params![format!("file-{id}"), format!("docs/{id}.txt")],
    );
    sql(
        db,
        "INSERT INTO operation_queue (op_id, kind, file_id, parent_id, target_path, metadata_json, attempts, max_attempts,
                                      next_retry_at, paused_reason, created_at)
         VALUES (?1, ?2, ?3, ?4, 'target/name.txt', ?5, ?6, 5, ?7, ?8, ?9)",
        params![id, kind.0, format!("file-{id}"), parent, kind.1, state.1, state.2, state.0, at],
    );
}

/// The whole list, a page of `size` at a time; each page's total must agree.
fn whole_list(db: &StateDb, size: u32) -> (i64, Vec<UnsentChange>) {
    let first = db.unsent_changes(0, size, NOW).unwrap();
    let mut items = first.items.clone();
    while (items.len() as i64) < first.total {
        let page = db.unsent_changes(items.len() as u32, size, NOW).unwrap();
        assert_eq!(page.total, first.total, "every page reads the same total");
        assert!(!page.items.is_empty(), "a page short of the total is not empty");
        items.extend(page.items);
    }
    assert!(
        db.unsent_changes(items.len() as u32, size, NOW)
            .unwrap()
            .items
            .is_empty(),
        "nothing past the total"
    );
    (first.total, items)
}

/// The summary's groups, as `group -> count` with `waiting` and `gave_up` beside the held reasons.
fn summary_groups(db: &StateDb) -> BTreeMap<String, i64> {
    let hold = db.queue_hold_summary().unwrap();
    let mut groups: BTreeMap<String, i64> = hold.held.iter().map(|g| (g.reason.clone(), g.count)).collect();
    for (group, count) in [("waiting", hold.waiting), ("gave_up", hold.gave_up)] {
        if count > 0 {
            groups.insert(group.to_string(), count);
        }
    }
    groups
}

fn list_groups(items: &[UnsentChange]) -> BTreeMap<String, i64> {
    let mut groups = BTreeMap::new();
    for item in items {
        *groups.entry(item.group.clone()).or_insert(0) += 1;
    }
    groups
}

/// The held groups' dates come from the same rows as the list's.
fn list_dates(items: &[UnsentChange], group: &str) -> (i64, i64) {
    let dates: Vec<i64> = items.iter().filter(|i| i.group == group).map(|i| i.queued_at).collect();
    (*dates.iter().min().unwrap(), *dates.iter().max().unwrap())
}

#[test]
fn the_list_holds_exactly_what_not_sent_counts_for_every_kind_and_reason() {
    let (_dir, db) = db();
    let mut at = 1_000;
    let mut expected_kinds: BTreeMap<String, &str> = BTreeMap::new();
    for (k, (kind, meta, word)) in KINDS.iter().enumerate() {
        for (s, (state, reason, attempts, retry, _)) in STATES.iter().enumerate() {
            let id = format!("op-{k:02}-{s:02}-{state}");
            at += 1;
            queue(&db, &id, (kind, *meta), (*reason, *attempts, *retry), None, at);
            if let Some(word) = word {
                expected_kinds.insert(id, word);
            }
        }
    }
    // D7: a change in a folder whose create is held waits for that folder (on macOS, which has no parent-first gate,
    // it is simply waiting). A download in it is still no change of the person's.
    queue(
        &db,
        "held-folder",
        ("create_folder", None),
        (Some("auth"), 0, 0),
        None,
        50,
    );
    queue(
        &db,
        "in-held-folder",
        ("upload_file", None),
        (None, 0, 0),
        Some("file-held-folder"),
        60,
    );
    queue(
        &db,
        "download-in-held-folder",
        ("hydrate_file", None),
        (None, 0, 0),
        Some("file-held-folder"),
        61,
    );
    expected_kinds.insert("held-folder".into(), "folder");
    expected_kinds.insert("in-held-folder".into(), "new_file");
    // Task 2270: two uploads held for review, one still pending (not counted).
    for op in ["fin-a", "fin-b", "fin-pending"] {
        db.put_upload_finalization(&UploadFinalization {
            op_id: op.into(),
            local_file_id: format!("local-{op}"),
            server_file_id: format!("server-{op}"),
            target_path: format!("photos/{op}.jpg"),
            payload_path: format!("proof-{op}"),
            stamped: false,
            review: None,
        })
        .unwrap();
    }
    db.mark_upload_finalization_review("fin-a", "other_file", 700).unwrap();
    db.mark_upload_finalization_review("fin-b", "moved", 800).unwrap();
    expected_kinds.insert("finalization:fin-a".into(), "new_file");
    expected_kinds.insert("finalization:fin-b".into(), "new_file");

    let (total, items) = whole_list(&db, 7);

    let counted = summary_groups(&db);
    assert_eq!(
        total,
        counted.values().sum::<i64>(),
        "the list's total is the count: {counted:?}"
    );
    assert_eq!(items.len() as i64, total, "the list holds its total");
    assert_eq!(
        list_groups(&items),
        counted,
        "each group holds what the summary counts in it"
    );
    let ids: Vec<&str> = items.iter().map(|i| i.id.as_str()).collect();
    let listed: BTreeMap<String, &str> = items.iter().map(|i| (i.id.clone(), i.kind)).collect();
    assert_eq!(
        listed, expected_kinds,
        "every change listed once, under its kind word; no download: {ids:?}"
    );

    let hold = db.queue_hold_summary().unwrap();
    for group in &hold.held {
        assert_eq!(
            list_dates(&items, &group.reason),
            (group.oldest_at, group.newest_at),
            "{}'s dates",
            group.reason
        );
    }
    let in_folder = items.iter().find(|i| i.id == "in-held-folder").unwrap();
    #[cfg(not(target_os = "macos"))]
    assert_eq!(in_folder.group, "waiting_for_folder");
    #[cfg(target_os = "macos")]
    assert_eq!(in_folder.group, "waiting");

    // What needs the person first, then what waits for a folder, then what waits; oldest first within each.
    let rank = |group: &str| match group {
        "waiting" => 2,
        "waiting_for_folder" => 1,
        _ => 0,
    };
    let order: Vec<(i32, i64)> = items.iter().map(|i| (rank(&i.group), i.queued_at)).collect();
    let mut sorted = order.clone();
    sorted.sort();
    assert_eq!(order, sorted);
}

/// Task 2224: the downloads a pin queues are not "Not sent". Fails if the summary counts them again.
#[test]
fn downloads_are_neither_counted_nor_listed() {
    let (_dir, db) = db();
    queue(&db, "upload", ("upload_file", None), (None, 0, 0), None, 1);
    queue(&db, "hydrate", ("hydrate_file", None), (None, 0, 0), None, 2);
    queue(&db, "pin", ("pin_tree", None), (None, 0, 0), None, 3);
    queue(&db, "hydrate-gave-up", ("hydrate_file", None), (None, 5, 0), None, 4);
    queue(&db, "pin-paused", ("pin_tree", None), (Some("auth"), 0, 0), None, 5);

    let hold = db.queue_hold_summary().unwrap();
    assert_eq!((hold.waiting, hold.gave_up, hold.held.len()), (1, 0, 0), "{hold:?}");
    let page = db.unsent_changes(0, 100, NOW).unwrap();
    assert_eq!(page.total, 1);
    assert_eq!(page.items.iter().map(|i| i.id.as_str()).collect::<Vec<_>>(), ["upload"]);
}

/// Each change carries where it is: the file's row first (the path the person sees), else the op's own target, and a
/// backup its source on this computer. A waiting change that failed carries when it is tried again; nothing else does.
#[test]
fn each_change_says_where_it_is_and_when_a_failed_one_is_tried_again() {
    let (_dir, db) = db();
    queue(&db, "with-row", ("upload_version", None), (None, 2, NOW + 90), None, 1);
    sql(
        &db,
        "INSERT INTO operation_queue (op_id, kind, file_id, target_path, created_at) VALUES ('no-row', 'trash_file', 'gone', 'old/plan.docx', 2)",
        [],
    );
    sql(
        &db,
        "INSERT INTO operation_queue (op_id, kind, file_id, target_path, origin, source_path, attempts, max_attempts,
                                      next_retry_at, created_at)
         VALUES ('backup', 'upload_file', 'b1', 'Backups/PC/Pictures/a.jpg', 'backup', 'C:\\Users\\me\\Pictures\\a.jpg',
                 1, 5, ?1, 3)",
        params![NOW - 5],
    );
    queue(&db, "gave-up", ("upload_file", None), (None, 5, NOW + 90), None, 4);

    let items = db.unsent_changes(0, 100, NOW).unwrap().items;
    let by_id: BTreeMap<&str, &UnsentChange> = items.iter().map(|i| (i.id.as_str(), i)).collect();
    let with_row = by_id["with-row"];
    assert_eq!(
        (with_row.path.as_str(), with_row.kind, with_row.retry_at),
        ("docs/with-row.txt", "edit", Some(NOW + 90))
    );
    assert_eq!(
        (by_id["no-row"].path.as_str(), by_id["no-row"].kind),
        ("old/plan.docx", "delete")
    );
    let backup = by_id["backup"];
    assert_eq!(
        (backup.path.as_str(), backup.backup, backup.retry_at),
        ("C:\\Users\\me\\Pictures\\a.jpg", true, None),
        "due again"
    );
    assert!(!with_row.backup);
    assert_eq!(by_id["gave-up"].retry_at, None, "a given-up change is not tried again");
    assert_eq!(with_row.file_id.as_deref(), Some("file-with-row"));
}

/// The read changes nothing, and an empty queue is an empty list.
#[test]
fn the_list_is_a_read() {
    let (_dir, db) = db();
    assert_eq!(
        db.unsent_changes(0, 100, NOW).unwrap(),
        UnsentPage {
            total: 0,
            items: Vec::new()
        }
    );
    queue(&db, "a", ("rename_file", None), (Some("auth"), 0, 0), None, 1);
    let before: i64 =
        db.0.lock()
            .unwrap()
            .query_row(
                "SELECT COUNT(*) FROM operation_queue WHERE paused_reason = 'auth'",
                [],
                |r| r.get(0),
            )
            .unwrap();
    db.unsent_changes(0, 100, NOW).unwrap();
    let after: i64 =
        db.0.lock()
            .unwrap()
            .query_row(
                "SELECT COUNT(*) FROM operation_queue WHERE paused_reason = 'auth'",
                [],
                |r| r.get(0),
            )
            .unwrap();
    assert_eq!((before, after), (1, 1));
}

// ── Try again (Guus, 13:39: "Add in the try again directly if it failed") ─────────────────────────────────────────

/// Every column Try again may write, for one row: a refusal must leave them all as they were.
fn row_state(db: &StateDb, op_id: &str) -> Option<(i64, i64, Option<String>, Option<String>, Option<String>, i64)> {
    db.0.lock()
        .unwrap()
        .query_row(
            "SELECT attempts, next_retry_at, last_error, last_error_class, paused_reason, updated_at
             FROM operation_queue WHERE op_id = ?1",
            params![op_id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?)),
        )
        .optional()
        .unwrap()
}

fn due_ids(db: &StateDb) -> Vec<String> {
    db.list_due_operations(NOW)
        .unwrap()
        .into_iter()
        .map(|op| op.op_id)
        .collect()
}

fn listed<'a>(items: &'a [UnsentChange], id: &str) -> &'a UnsentChange {
    items
        .iter()
        .find(|i| i.id == id)
        .unwrap_or_else(|| panic!("{id} is listed"))
}

/// A change that stopped after repeated errors is due again, with its attempts back to 0, and is listed as waiting;
/// the count does not move. A second Try again changes nothing.
#[test]
fn try_again_makes_a_given_up_change_due_and_the_count_does_not_move() {
    let (_dir, db) = db();
    queue(&db, "stopped", ("trash_file", None), (None, 5, 0), None, 1);
    sql(
        &db,
        "UPDATE operation_queue SET last_error = 'HTTP 500' WHERE op_id = 'stopped'",
        [],
    );
    let before = db.unsent_changes(0, 100, NOW).unwrap();
    let item = listed(&before.items, "stopped");
    assert_eq!(
        (item.group.as_str(), item.retryable, item.parked),
        ("gave_up", true, false)
    );
    assert!(due_ids(&db).is_empty(), "a given-up change is never due by itself");

    assert_eq!(db.retry_unsent_change("stopped", NOW).unwrap(), RetryOutcome::Retried);

    assert_eq!(due_ids(&db), ["stopped"], "the next queue step sends it");
    assert_eq!(row_state(&db, "stopped"), Some((0, NOW, None, None, None, NOW)));
    let after = db.unsent_changes(0, 100, NOW).unwrap();
    assert_eq!(
        after.total, before.total,
        "Try again moves a change between groups, never out of the count"
    );
    let item = listed(&after.items, "stopped");
    assert_eq!(
        (item.group.as_str(), item.retryable, item.retry_at),
        ("waiting", false, None)
    );
    let hold = db.queue_hold_summary().unwrap();
    assert_eq!((hold.waiting, hold.gave_up), (1, 0));

    assert_eq!(
        db.retry_unsent_change("stopped", NOW + 5).unwrap(),
        RetryOutcome::AlreadyQueued
    );
    assert_eq!(
        row_state(&db, "stopped"),
        Some((0, NOW, None, None, None, NOW)),
        "the second press wrote nothing"
    );
}

/// A change that waits out a backoff after an error is due at once.
#[test]
fn try_again_sends_a_backing_off_change_at_once() {
    let (_dir, db) = db();
    queue(&db, "backing-off", ("upload_file", None), (None, 3, NOW + 600), None, 1);
    let item = listed(&db.unsent_changes(0, 100, NOW).unwrap().items, "backing-off").clone();
    assert_eq!((item.retry_at, item.retryable), (Some(NOW + 600), true));
    assert!(due_ids(&db).is_empty(), "not due before its retry time");

    assert_eq!(
        db.retry_unsent_change("backing-off", NOW).unwrap(),
        RetryOutcome::Retried
    );

    assert_eq!(due_ids(&db), ["backing-off"]);
    assert_eq!(row_state(&db, "backing-off").map(|s| (s.0, s.1)), Some((0, NOW)));
}

/// A held change keeps its own action, a parked one would only park again, and an unknown id, a download, a change
/// being sent and one already due are no failure to retry: each is refused or left as it is, and nothing changes.
#[test]
fn try_again_refuses_what_did_not_fail_and_changes_nothing() {
    let (_dir, db) = db();
    queue(
        &db,
        "held",
        ("upload_file", None),
        (Some("quota"), 3, NOW + 600),
        None,
        1,
    );
    queue(&db, "parked", ("upload_version", None), (None, 5, 0), None, 2);
    sql(
        &db,
        "UPDATE operation_queue SET last_error_class = 'stale_base' WHERE op_id = 'parked'",
        [],
    );
    // Spec D8: any `last_error_class` is a park (`held_queue::types`, the reconcile pass), not only the upload park
    // reasons: here a pause class kept after its pause was lifted (2409 self-review).
    queue(&db, "parked-other-class", ("rename", None), (None, 5, 0), None, 2);
    sql(
        &db,
        "UPDATE operation_queue SET last_error_class = 'auth' WHERE op_id = 'parked-other-class'",
        [],
    );
    queue(&db, "download", ("hydrate_file", None), (None, 5, 0), None, 3);
    queue(&db, "due", ("upload_file", None), (None, 0, 0), None, 4);
    queue(
        &db,
        "due-after-a-failure",
        ("upload_file", None),
        (None, 2, NOW - 1),
        None,
        5,
    );
    queue(&db, "sending", ("upload_file", None), (None, 2, NOW + 600), None, 6);
    sql(
        &db,
        "UPDATE operation_queue SET claim_id = 'claim', claimed_at = 1 WHERE op_id = 'sending'",
        [],
    );
    let items = db.unsent_changes(0, 100, NOW).unwrap().items;
    for id in ["held", "parked", "parked-other-class"] {
        assert!(!listed(&items, id).retryable, "{id} has no Try again");
    }
    assert!(listed(&items, "parked").parked && listed(&items, "parked-other-class").parked);
    let ids = [
        "held",
        "parked",
        "parked-other-class",
        "download",
        "due",
        "due-after-a-failure",
        "sending",
        "never-queued",
    ];
    let before: Vec<_> = ids.iter().map(|id| row_state(&db, id)).collect();

    let outcomes: Vec<_> = ids.iter().map(|id| db.retry_unsent_change(id, NOW).unwrap()).collect();

    use RetryOutcome::*;
    assert_eq!(
        outcomes,
        [
            Held,
            Parked,
            Parked,
            Unknown,
            AlreadyQueued,
            AlreadyQueued,
            AlreadyQueued,
            Unknown
        ]
    );
    assert_eq!(
        ids.iter().map(|id| row_state(&db, id)).collect::<Vec<_>>(),
        before,
        "nothing changed"
    );
}

/// A Finder upload's give-up presented its file as `error` (§9.1); Try again presents it `uploading` again, as a
/// rebase does. Another file's row is not touched.
#[test]
fn try_again_on_a_finder_upload_presents_its_file_uploading_again() {
    let (_dir, db) = db();
    queue(&db, "finder", ("upload_version", None), (None, 5, 0), None, 1);
    queue(&db, "other", ("upload_version", None), (None, 5, 0), None, 2);
    sql(&db, "UPDATE operation_queue SET write_id = 'w-' || op_id", []);
    sql(&db, "UPDATE files SET status = 'error'", []);
    let status = |file: &str| -> String {
        db.0.lock()
            .unwrap()
            .query_row("SELECT status FROM files WHERE file_id = ?1", params![file], |r| {
                r.get(0)
            })
            .unwrap()
    };

    assert_eq!(db.retry_unsent_change("finder", NOW).unwrap(), RetryOutcome::Retried);

    assert_eq!(
        (status("file-finder").as_str(), status("file-other").as_str()),
        ("uploading", "error")
    );
}
