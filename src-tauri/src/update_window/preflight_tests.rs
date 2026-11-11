//! Tests of `preflight`: uploads in flight are waited for and never cut, the wait is bounded, and a refused
//! pre-flight leaves the pause as the person had it. The engine stop itself is `stop_engine_in_slot`'s (its own
//! tests); the Cloud Files disconnect is Windows-only and belongs to the native rehearsal.

use super::preflight::*;
use crate::account::{AccountId, AccountRuntime};
use crate::transfer_progress::{Direction, TransferBoard};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

#[tokio::test(start_paused = true)]
async fn the_wait_for_uploads_is_bounded_and_ends_as_soon_as_none_is_left() {
    let poll = Duration::from_millis(250);
    // None in flight: at once, asked once.
    let asked = AtomicUsize::new(0);
    let none = || {
        asked.fetch_add(1, Ordering::SeqCst);
        0
    };
    assert_eq!(wait_for_uploads(none, Duration::from_secs(120), poll).await, Ok(()));
    assert_eq!(asked.load(Ordering::SeqCst), 1);
    // Two in flight that finish after 3 polls.
    let asked = AtomicUsize::new(0);
    let finishing = || if asked.fetch_add(1, Ordering::SeqCst) < 3 { 2 } else { 0 };
    let started = tokio::time::Instant::now();
    assert_eq!(
        wait_for_uploads(finishing, Duration::from_secs(120), poll).await,
        Ok(())
    );
    assert_eq!(started.elapsed(), Duration::from_millis(750));
    // One that never finishes: the bound, and how many were left.
    let started = tokio::time::Instant::now();
    assert_eq!(wait_for_uploads(|| 1, Duration::from_secs(120), poll).await, Err(1));
    assert_eq!(started.elapsed(), Duration::from_secs(120));
    // A zero bound still looks once.
    assert_eq!(wait_for_uploads(|| 3, Duration::ZERO, poll).await, Err(3));
}

#[test]
fn only_an_upload_that_has_not_failed_counts_as_in_flight() {
    let board = TransferBoard::new();
    assert_eq!(uploads_in_flight(&board), 0);
    let up = board.begin("u1", Direction::Up, 10);
    let _down = board.begin("d1", Direction::Down, 10);
    let _stream = board.begin("s1", Direction::Stream, 10);
    assert_eq!(uploads_in_flight(&board), 1);
    let _up2 = board.begin("u2", Direction::Up, 10);
    assert_eq!(uploads_in_flight(&board), 2);
    drop(up);
    assert_eq!(uploads_in_flight(&board), 1, "a finished upload is no longer in flight");
}

#[tokio::test(start_paused = true)]
async fn an_upload_in_flight_puts_the_update_off_and_leaves_the_pause_as_it_was() {
    let acct = AccountRuntime::new(AccountId::new_v4());
    let board = TransferBoard::new();
    let upload = board.begin("u1", Direction::Up, 10);
    for was_paused in [false, true] {
        acct.sync_paused.store(was_paused, Ordering::SeqCst);
        let refused = quiet_engine(&acct, Some(&board), Duration::from_secs(5))
            .await
            .err()
            .expect("an upload is in flight");
        assert!(refused.starts_with("update_sync_not_quiet: "), "{refused}");
        assert!(refused.contains("still uploading 1 file"), "{refused}");
        assert_eq!(acct.sync_paused.load(Ordering::SeqCst), was_paused);
        assert!(acct.engine.try_lock().is_ok(), "the engine slot is free again");
    }
    // The upload finishes while the pre-flight waits: it goes on, and holds the slot with new work held back.
    acct.sync_paused.store(false, Ordering::SeqCst);
    let finish = tokio::spawn(async move {
        tokio::time::sleep(Duration::from_secs(2)).await;
        drop(upload);
    });
    let quiet = quiet_engine(&acct, Some(&board), Duration::from_secs(5))
        .await
        .expect("the upload finished inside the bound");
    finish.await.unwrap();
    assert!(!quiet.stopped_an_engine, "no engine was running");
    assert!(
        acct.sync_paused.load(Ordering::SeqCst),
        "new work is held back until the installer takes over"
    );
    assert!(acct.engine.try_lock().is_err(), "nothing starts an engine meanwhile");
    // The install did not happen: the pause goes back, the slot is free.
    quiet.resume();
    assert!(!acct.sync_paused.load(Ordering::SeqCst));
    assert!(acct.engine.try_lock().is_ok());
    // No board at all (no popover runtime): nothing to wait for.
    assert!(quiet_engine(&acct, None, Duration::ZERO).await.is_ok());
}
