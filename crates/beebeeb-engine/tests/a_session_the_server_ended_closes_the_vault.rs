//! Task 2521: a session the server ended closes the vault, driven by the running engine itself. Signed in and syncing,
//! the server starts refusing the session (the fake server's `end-session` switch: every call with the session token
//! is answered 401). The sync loop's refused pulls are the refusal run: three in a row and the session watcher closes
//! the vault (the key dropped, the closed mark stored, the stored session kept) and nothing syncs after.
//!
//! The heartbeat is kept off for the whole test (the fake's `heartbeat-off` switch, set before the engine starts, so
//! the device route is refused and no heartbeat runs): the close can only come from the sync ticks. The ticks run at
//! the engine's own 30 s period, so this test takes about two minutes of wall clock.
//!
//! Mutants: the tick clears the run whatever its pull did (`note_result(None)` in the runner's `Ok` arm), or the tick
//! drops why its pull failed (`pull_failure: None`): either way the vault stays open and this test fails.

mod common;

#[cfg(not(target_os = "macos"))]
#[test]
fn refused_sync_ticks_close_the_vault_and_nothing_syncs_after() {
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::{Arc, Mutex};
    use std::time::{Duration, Instant};

    use beebeeb_engine::app::SyncStart;
    use beebeeb_engine::app::lock_api::LockState;
    use beebeeb_engine::keychain::{AuthSecretStore, LockFlag};
    use beebeeb_engine::signin::gate::GateState;
    use beebeeb_engine::signin::start::EngineStartBlocked;
    use beebeeb_engine::test_api::server::{Switches, fixture_handler};

    // The engine's tick period (`runner::tick_loop::TICK_INTERVAL`) and the run that closes (`AUTH_EXPIRED_THRESHOLD`).
    const TICK: Duration = Duration::from_secs(30);
    const RUN: usize = 3;
    // How long nothing may sync after the close: more than one tick.
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
    let count = |lines: &[String], line: &str| lines.iter().filter(|l| l.as_str() == line).count();

    // No heartbeat for the whole run: the device route is refused before the engine asks for it.
    switches.heartbeat_off.store(true, Ordering::SeqCst);
    s.sign_in();
    s.unlock();
    s.engine.watch_session_every(Duration::from_millis(50));
    let folder = s.home.join("Beebeeb Test");
    assert_eq!(s.engine.start_sync(folder), Ok(SyncStart::Started));

    // End the session only after the first pull bootstrapped (the cursor is stored): a refused bootstrap is counted by
    // another path, and this test is about the pulls of a running account.
    assert!(
        common::wait_until(60, || count(&lines_from(0), "GET /api/v1/sync/snapshot -> 200") >= 1),
        "precondition: the first pull bootstrapped: {:?}",
        lines_from(0)
    );
    let ended_at = lines.lock().unwrap().len();
    switches.session_ended.store(true, Ordering::SeqCst);
    let ended = Instant::now();

    // At most four tick periods plus a margin: the third refused tick comes at most three periods after the end.
    let closed = common::wait_until(4 * TICK.as_secs() + 15, || {
        s.engine.lock_state() == LockState::VaultClosed
    });
    let took = ended.elapsed();
    let after_end = lines_from(ended_at);
    let refused_pulls = count(&after_end, "GET /api/v1/sync/ops -> 401");
    let all = lines_from(0);

    // The instrument first: the pulls were refused, and no heartbeat ran that could have closed the vault instead.
    assert!(
        refused_pulls >= RUN,
        "the instrument: at least {RUN} refused pulls after the end, saw {refused_pulls} (closed: {closed}, {took:?}): {after_end:?}"
    );
    assert_eq!(
        count(&all, "POST /api/v1/clients/devices -> 503"),
        1,
        "the heartbeat's device route was refused once: {all:?}"
    );
    assert_eq!(
        all.iter()
            .filter(|l| l.contains("/clients/sessions") || l.contains("/heartbeat"))
            .count(),
        0,
        "no heartbeat session was opened or beat: {all:?}"
    );

    // The close, as core says: no key in memory, the gate locked, the closed mark stored, the stored session kept.
    assert!(
        closed,
        "the vault closed after {refused_pulls} refused pulls ({took:?} after the end; session held as ended: {}): {after_end:?}",
        s.engine.account_session_refused()
    );
    assert!(s.engine.account_session_refused(), "the session is held as ended");
    assert_eq!(s.engine.gate(), GateState::Locked);
    assert_eq!(s.engine.keys_for_engine_start(), Err(EngineStartBlocked::NoSession));
    let store = s.stores.store_for(&s.account_id);
    assert_eq!(
        store.load_lock_flag(LockFlag::VaultClosed).unwrap().as_deref(),
        Some(&b"1"[..]),
        "the closed mark is stored: the next start does not open on the files"
    );
    assert!(
        store.load_session_token().unwrap().is_some(),
        "the stored session stays (no sign-out)"
    );

    // Nothing syncs after the close: no pull, no heartbeat, for more than a tick period.
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
        "closed {took:?} after the end, after {refused_pulls} refused pulls; requests of any kind in the {QUIET:?} after the close: {}",
        after_close.len()
    );
    assert_eq!(s.engine.lock_state(), LockState::VaultClosed, "and it stays closed");
}

#[cfg(target_os = "macos")]
#[test]
fn the_test_build_is_refused_on_macos() {
    common::assert_the_test_build_is_refused();
}
