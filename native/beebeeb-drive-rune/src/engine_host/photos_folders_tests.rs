//! Slice B1's rules: the backup line's words, its session rule, and that nothing is invented. Every name is invented.

use super::*;

use beebeeb_engine::account::SessionGeneration;

fn stamped(turn: u64, account: &str) -> Stamp {
    (SessionGeneration::for_test(turn), Some(account.to_string()))
}

fn at(day: NaiveDate, hour: u32, minute: u32) -> i64 {
    let naive = day.and_hms_opt(hour, minute, 0).unwrap();
    Local.from_local_datetime(&naive).single().unwrap().timestamp_millis()
}

#[test]
fn backup_status_last_is_the_devices_last_backup() {
    let today = Local::now().date_naive();
    assert_eq!(
        last_backup_words(Some(at(today, 9, 12)), today).as_deref(),
        Some("last backup 09:12"),
        "today: the time alone"
    );
    let yesterday = today.pred_opt().unwrap();
    assert_eq!(
        last_backup_words(Some(at(yesterday, 18, 5)), today).as_deref(),
        Some("last backup Yesterday 18:05"),
        "an earlier day names the day"
    );
}

#[test]
fn backup_status_now_is_never_invented() {
    let today = Local::now().date_naive();
    // No device named a pass time: no line at all.
    assert_eq!(last_backup_words(None, today), None);
    // The held line carries an empty `now`, whatever it was read from.
    let mut held = Held::default();
    let asked = stamped(1, "first");
    held.follow(&asked);
    assert!(held.take(&asked, last_backup_words(Some(at(today, 9, 12)), today)));
    let status = held.last.clone().map(|last| BackupStatus {
        now: String::new(),
        last,
    });
    assert_eq!(status.as_ref().map(|status| status.now.as_str()), Some(""));
    assert_eq!(status.map(|status| status.last).as_deref(), Some("last backup 09:12"));
}

#[test]
fn photo_folders_late_answer_dropped() {
    let mut held = Held::default();
    let before = stamped(1, "first");
    held.follow(&before);
    held.reading = true;
    // The session moved on, then another account: the held state is for the new stamp and empty.
    let after = stamped(2, "second");
    held.follow(&after);
    assert!(
        !held.take(&before, Some("last backup 09:12".into())),
        "a late answer is dropped"
    );
    assert_eq!(held.last, None);
    assert!(held.at.is_none());
    // The same session under another account is another stamp too.
    assert!(!held.take(&stamped(2, "first"), Some("last backup 09:12".into())));
    // The answer for the current stamp is kept.
    assert!(held.take(&after, Some("last backup 10:00".into())));
    assert_eq!(held.last.as_deref(), Some("last backup 10:00"));
}
