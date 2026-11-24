//! Task 2521 (review P1-2): a session the server ended closes the vault while sync is paused and no heartbeat runs.
//! A paused tick pulls nothing, and without a heartbeat (its registration failed at the start) nothing else of the
//! engine's own talks to the server, so nothing could count the refusals. The paused tick now asks Beebeeb who is
//! signed in when no heartbeat runs; three refused answers in a row close the vault as a refused pull would.
//!
//! The heartbeat is kept off for the whole test (the fake's `heartbeat-off` switch, set before the engine starts) and
//! the ticks run at the engine's own 30 s period, so this test takes about two minutes of wall clock.

mod common;

#[cfg(not(target_os = "macos"))]
#[test]
fn a_paused_engine_without_a_heartbeat_closes_the_vault_when_the_session_ended() {
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
    // How long nothing may be sent after the close: more than one tick.
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
    assert!(
        common::wait_until(60, || count(&lines_from(0), "GET /api/v1/sync/snapshot -> 200") >= 1),
        "precondition: the first pull bootstrapped: {:?}",
        lines_from(0)
    );

    // The person pauses sync, then the server ends the session.
    assert_eq!(s.engine.pause(), Ok(()));
    let ended_at = lines.lock().unwrap().len();
    switches.session_ended.store(true, Ordering::SeqCst);
    let ended = Instant::now();

    let closed = common::wait_until(4 * TICK.as_secs() + 15, || {
        s.engine.lock_state() == LockState::VaultClosed
    });
    let took = ended.elapsed();
    let after_end = lines_from(ended_at);
    let all = lines_from(0);

    // The instrument first: sync was paused (no pull after the end) and no heartbeat ran.
    assert_eq!(
        after_end
            .iter()
            .filter(|l| l.starts_with("GET /api/v1/sync/ops "))
            .count(),
        0,
        "the instrument: a paused engine pulls nothing: {after_end:?}"
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

    // The close, as core says.
    let checks = count(&after_end, "GET /api/v1/auth/me -> 401");
    assert!(
        closed,
        "the vault closed while paused ({took:?} after the end; {checks} refused checks; session held as ended: {}): {after_end:?}",
        s.engine.account_session_refused()
    );
    assert!(
        checks >= RUN,
        "the paused ticks' checks were the refusals: {checks}: {after_end:?}"
    );
    assert!(s.engine.account_session_refused(), "the session is held as ended");
    assert_eq!(s.engine.gate(), GateState::Locked);
    assert_eq!(s.engine.keys_for_engine_start(), Err(EngineStartBlocked::NoSession));
    let store = s.stores.store_for(&s.account_id);
    assert_eq!(
        store.load_lock_flag(LockFlag::VaultClosed).unwrap().as_deref(),
        Some(&b"1"[..]),
        "the closed mark is stored"
    );
    assert!(
        store.load_session_token().unwrap().is_some(),
        "the stored session stays (no sign-out)"
    );

    // Nothing is sent after the close: no pull, no heartbeat, no check.
    let closed_at = s.fake.requests().len();
    std::thread::sleep(QUIET);
    let after_close = s.routes_from(closed_at);
    assert!(
        after_close.is_empty(),
        "{} requests in the {QUIET:?} after the close: {after_close:?}",
        after_close.len()
    );
    println!("closed {took:?} after the end, after {checks} refused checks; requests after the close: 0");
}

#[cfg(target_os = "macos")]
#[test]
fn the_test_build_is_refused_on_macos() {
    common::assert_the_test_build_is_refused();
}
