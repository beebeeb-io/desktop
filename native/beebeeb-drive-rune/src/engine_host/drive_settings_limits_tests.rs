//! This PC's transfer limits and the Bandwidth read on the engine adapter: a limit goes to the engine under the
//! session of the press and is shown as the engine holds it afterwards, a press under an earlier session changes
//! nothing and says so, nothing is read before the pane opens, nothing is handed out while locked, and the traffic
//! is the signed-in account's own.

use super::*;

fn open_bandwidth(fake: &Arc<Fake>) -> EngineDriveSettings<Fake> {
    let mut source = source(fake, Arc::new(FakePlatform::default()));
    source.opened(Pane::Bandwidth);
    settle(&mut source);
    source
}

fn upload(kib: u64) -> TransferLimitsChange {
    TransferLimitsChange {
        upload_kib_per_s: Some(kib),
        ..TransferLimitsChange::default()
    }
}

#[test]
fn bandwidth_is_read_when_its_pane_opens_never_at_launch() {
    let fake = Fake::new();
    *fake.limits.lock().unwrap() = TransferLimits {
        upload_kib_per_s: kib_per_s(Some(8)),
        download_kib_per_s: 0,
        downloaded_copies_bytes: 0,
    };
    let mut source = source(&fake, Arc::new(FakePlatform::default()));
    assert_eq!(
        fake.limit_reads.load(Ordering::SeqCst),
        0,
        "no read for a pane nobody opened"
    );
    assert_eq!(source.bandwidth(), Bandwidth::default(), "and no value before the read");

    source.opened(Pane::Bandwidth);
    settle(&mut source);
    assert_eq!(fake.limit_reads.load(Ordering::SeqCst), 1);
    let pane = source.bandwidth();
    assert_eq!((pane.upload_limit, pane.download_limit), (Some(8), None));
    // What the engine has no answer for stays as the empty pane draws it.
    assert!(pane.devices.is_empty() && pane.now.is_empty() && pane.upload.speed.is_none());
    assert_eq!(pane.test, Default::default());
    assert!(!pane.waiting_metered);
}

#[test]
fn a_speed_limit_goes_to_the_engine_and_the_pane_shows_what_the_engine_holds() {
    let fake = Fake::new();
    let mut source = open_bandwidth(&fake);
    source.set_speed_limit(Direction::Upload, Some(10));
    settle(&mut source);
    assert_eq!(*fake.limit_changes.lock().unwrap(), [upload(1_220)]);
    assert_eq!(source.take_toast(), None);
    assert_eq!(source.bandwidth().upload_limit, Some(10));
    assert_eq!(
        source.bandwidth().download_limit,
        None,
        "the other direction is not touched"
    );

    source.set_speed_limit(Direction::Download, Some(50));
    source.set_speed_limit(Direction::Upload, None);
    settle(&mut source);
    let changes = fake.limit_changes.lock().unwrap().clone();
    assert_eq!(
        changes[1..],
        [
            TransferLimitsChange {
                download_kib_per_s: Some(6_103),
                ..TransferLimitsChange::default()
            },
            upload(0),
        ]
    );
    let pane = source.bandwidth();
    assert_eq!((pane.upload_limit, pane.download_limit), (None, Some(50)));
}

/// The stamp is the session at the press, read on the window's thread: a job that waits behind another while the
/// session moves on is refused by the engine, said in the toast, and the pane shows the limit the file still holds.
#[test]
fn a_limit_pressed_under_an_earlier_session_changes_nothing_and_says_so() {
    let fake = Fake::new();
    let mut source = open_bandwidth(&fake);
    // Hold the worker in a read of the use, so the press waits in the queue.
    fake.in_usage.arm_at(fake.in_usage.passes.load(Ordering::SeqCst) + 1);
    source.reload_insights();
    fake.in_usage.reached();
    source.set_speed_limit(Direction::Upload, Some(10));
    // A lock and an unlock of the same account: the engine's session moved on, the account did not.
    fake.session.fetch_add(1, Ordering::SeqCst);
    fake.in_usage.release();
    settle(&mut source);
    assert!(fake.limit_changes.lock().unwrap().is_empty(), "nothing was written");
    assert_eq!(source.take_toast(), Some(LIMIT_FAILED));
    assert_eq!(
        source.bandwidth().upload_limit,
        None,
        "the pane shows what the file holds"
    );

    // The same press under the session that is current: taken.
    source.set_speed_limit(Direction::Upload, Some(10));
    settle(&mut source);
    assert_eq!(*fake.limit_changes.lock().unwrap(), [upload(1_220)]);
    assert_eq!(source.take_toast(), None);
}

#[test]
fn the_limit_on_downloaded_copies_goes_to_the_engine_and_previews_have_none() {
    let fake = Fake::new();
    let mut source = source(&fake, Arc::new(FakePlatform::default()));
    source.set_limit(Kept::Previews, 5);
    settle(&mut source);
    assert!(
        fake.limit_changes.lock().unwrap().is_empty(),
        "no engine call for previews"
    );
    source.set_limit(Kept::Downloaded, 20);
    settle(&mut source);
    assert_eq!(
        *fake.limit_changes.lock().unwrap(),
        [TransferLimitsChange {
            downloaded_copies_bytes: Some(20 * GB),
            ..TransferLimitsChange::default()
        }]
    );
    assert_eq!(source.take_toast(), None);
}

#[test]
fn a_locked_vault_hands_out_no_limit_and_takes_no_change() {
    let fake = Fake::new();
    *fake.limits.lock().unwrap() = TransferLimits {
        upload_kib_per_s: kib_per_s(Some(8)),
        ..TransferLimits::default()
    };
    let mut source = open_bandwidth(&fake);
    assert_eq!(source.bandwidth().upload_limit, Some(8));
    fake.view.lock().unwrap().gate = GateState::Locked;
    assert_eq!(source.bandwidth(), Bandwidth::default(), "at once, before any pump");
    source.set_speed_limit(Direction::Upload, Some(1));
    source.set_limit(Kept::Downloaded, 1);
    settle(&mut source);
    assert!(fake.limit_changes.lock().unwrap().is_empty());
    assert_eq!(source.take_toast(), Some(CHANGE_LOCKED));
    assert_eq!(source.bandwidth(), Bandwidth::default());
}

#[test]
fn the_traffic_is_the_signed_in_accounts_own_last_20_hours() {
    let fake = Fake::new();
    let now = chrono::Utc::now().timestamp();
    let db = fake.db();
    db.insert_bandwidth_sample(now - 5 * 60, 30_000_000, 2_000_000, 60)
        .unwrap();
    db.insert_bandwidth_sample(now - 6 * 60, 20_000_000, 0, 60).unwrap();
    db.insert_bandwidth_sample(now - 19 * 3_600 - 55 * 60, 0, 150_000_000, 60)
        .unwrap();
    db.insert_bandwidth_sample(now - 30 * 3_600, 900_000_000, 900_000_000, 60)
        .unwrap();
    let source = open_bandwidth(&fake);
    let history = source.bandwidth().history;
    assert_eq!(history.up_mb.len(), 60);
    assert_eq!(history.down_mb.len(), 60);
    assert_eq!(history.bucket_minutes, 20);
    assert_eq!(history.up_mb[59], 50.0, "the last 20 minutes");
    assert_eq!(history.down_mb[59], 2.0);
    assert_eq!(history.down_mb[0], 150.0, "the first 20 minutes");
    assert_eq!(history.up_total, "50 MB", "nothing older than 20 hours is counted");
    assert_eq!(history.down_total, "152 MB");
    assert_eq!(history.top_mb, 200.0);

    // Another account on this PC: the limits are this PC's, the traffic is not theirs to see.
    *fake.account.lock().unwrap() = Some("other@beebeeb.io".into());
    let mut source = source;
    settle(&mut source);
    assert_eq!(
        source.bandwidth(),
        Bandwidth::default(),
        "nothing held across the account change"
    );
    source.opened(Pane::Bandwidth);
    settle(&mut source);
    assert!(
        source.bandwidth().history.is_empty(),
        "never the first account's traffic"
    );
}

#[test]
fn history_of_puts_each_sample_in_its_20_minutes() {
    use chrono::TimeZone;
    let now = chrono::Local.with_ymd_and_hms(2026, 11, 14, 9, 30, 0).single().unwrap();
    let at = now.timestamp();
    assert_eq!(history_of(&[], now), History::default(), "nothing recorded yet");
    let sample = |ago: i64, up: u64, down: u64| BandwidthSample {
        sampled_at: at - ago,
        up_bytes: up,
        down_bytes: down,
        period_secs: 60,
    };
    let history = history_of(
        &[
            sample(1, 1_000_000, 0),
            sample(20 * 60 + 1, 0, 3_000_000),
            sample(20 * 3_600, 5_000_000, 0),
            sample(20 * 3_600 + 1, 7_000_000, 7_000_000),
            sample(-60, 9_000_000, 9_000_000),
        ],
        now,
    );
    assert_eq!(history.up_mb[59], 1.0);
    assert_eq!(history.down_mb[58], 3.0);
    assert_eq!(history.up_mb[0], 5.0, "the window's first second");
    assert_eq!(history.up_total, "6 MB", "before the window and after now: not counted");
    assert_eq!(history.down_total, "3 MB");
    assert_eq!(history.top_mb, 100.0);
    assert_eq!(history.starts_at, 13 * 60 + 30, "20 hours before 09:30");
    assert_eq!(history.starts_days_ago, 1);
    assert_eq!(history.marks().last().map(String::as_str), Some("Now"));
}

#[test]
fn a_speed_limit_keeps_its_megabits_through_the_engines_unit() {
    assert_eq!(kib_per_s(None), 0);
    assert_eq!(mbps(0), None);
    for limit in 1..=2_000 {
        let stored = kib_per_s(Some(limit));
        assert!(stored > 0, "a limit that was asked for is never no limit");
        assert_eq!(mbps(stored), Some(limit));
    }
    assert_eq!(kib_per_s(Some(10)), 1_220);
}
