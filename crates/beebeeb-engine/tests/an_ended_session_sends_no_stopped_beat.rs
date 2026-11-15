//! Task 2521 (review P3-1): with the heartbeat running, a session the server ended closes the vault, and the close
//! sends nothing more with the ended token: the engine stop's usual last "stopped" beat is left out when the session
//! is held as ended, and no beat or pull follows the close.
//!
//! The heartbeat beats every 20 s and the ticks run every 30 s, both at the engine's own periods, so this test takes
//! about a minute and a half of wall clock.

mod common;

#[cfg(not(target_os = "macos"))]
#[test]
fn the_close_of_an_ended_session_sends_no_stopped_beat_and_nothing_after() {
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::{Arc, Mutex};
    use std::time::{Duration, Instant};

    use beebeeb_engine::app::SyncStart;
    use beebeeb_engine::app::lock_api::LockState;
    use beebeeb_engine::test_api::server::{Switches, fixture_handler};

    const TICK: Duration = Duration::from_secs(30);
    const QUIET: Duration = Duration::from_secs(35);

    let lines: Arc<Mutex<Vec<String>>> = Arc::default();
    let switches = Switches::default();
    let s = {
        let (lines, switches) = (lines.clone(), switches.clone());
        common::setup_with_account(move |account, _, _| {
            let write: beebeeb_engine::test_api::server::LineWriter =
                Arc::new(move |line: &str| lines.lock().unwrap().push(line.to_string()));
            fixture_handler(account, write, Arc::new(AtomicU64::new(0)), switches)
        })
    };
    let lines_from = |from: usize| -> Vec<String> { lines.lock().unwrap()[from..].to_vec() };
    let beats = |lines: &[String], status: u16| {
        lines
            .iter()
            .filter(|l| {
                l.starts_with("POST /api/v1/clients/sessions/") && l.ends_with(&format!("/heartbeat -> {status}"))
            })
            .count()
    };
    // The heartbeat requests whose body reports `stopped` (the stop's last beat).
    let stopped_beats = |s: &common::Setup| {
        s.fake
            .requests()
            .iter()
            .filter(|r| r.route().ends_with("/heartbeat") && r.json()["status"] == "stopped")
            .count()
    };

    s.sign_in();
    s.unlock();
    s.engine.watch_session_every(Duration::from_millis(50));
    let folder = s.home.join("Beebeeb Test");
    assert_eq!(s.engine.start_sync(folder), Ok(SyncStart::Started));
    assert!(
        common::wait_until(60, || {
            let all = lines_from(0);
            all.iter().any(|l| l == "GET /api/v1/sync/snapshot -> 200") && beats(&all, 200) >= 1
        }),
        "precondition: the first pull bootstrapped and the heartbeat beat: {:?}",
        lines_from(0)
    );
    assert_eq!(stopped_beats(&s), 0, "precondition: no stop yet");

    let ended_at = lines.lock().unwrap().len();
    switches.session_ended.store(true, Ordering::SeqCst);
    let ended = Instant::now();
    let closed = common::wait_until(4 * TICK.as_secs() + 15, || {
        s.engine.lock_state() == LockState::VaultClosed
    });
    let took = ended.elapsed();
    let after_end = lines_from(ended_at);

    // The instrument: the heartbeat ran and was refused (it fed the run with the ticks).
    assert!(
        beats(&after_end, 401) >= 1,
        "the instrument: the heartbeat was refused after the end: {after_end:?}"
    );
    assert!(
        closed,
        "the vault closed ({took:?} after the end; session held as ended: {}): {after_end:?}",
        s.engine.account_session_refused()
    );

    // The close sent no "stopped" beat with the ended token.
    let stopped = stopped_beats(&s);
    assert_eq!(
        stopped, 0,
        "the close sent {stopped} \"stopped\" beat(s) with a token the server refuses: {after_end:?}"
    );

    // Nothing beats or pulls after the close.
    let closed_at = s.fake.requests().len();
    std::thread::sleep(QUIET);
    let after_close = s.routes_from(closed_at);
    let syncing: Vec<&String> = after_close
        .iter()
        .filter(|route| route.contains("/api/v1/sync/") || route.contains("/heartbeat"))
        .collect();
    assert!(
        syncing.is_empty(),
        "{} sync or heartbeat requests in the {QUIET:?} after the close: {syncing:?}",
        syncing.len()
    );
    println!(
        "closed {took:?} after the end; refused beats {}; stopped beats 0; requests of any kind after the close: {}",
        beats(&after_end, 401),
        after_close.len()
    );
}

#[cfg(target_os = "macos")]
#[test]
fn the_test_build_is_refused_on_macos() {
    common::assert_the_test_build_is_refused();
}
