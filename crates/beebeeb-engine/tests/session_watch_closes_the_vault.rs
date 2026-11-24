//! Task 2535 (CP2 row 4.2): with no sync runner, the account's session watch closes the vault when the server ended
//! the session, and an offline check keeps it open. The sync start is refused (this PC holds the account's own local
//! data and its sync folder is gone), so no tick, heartbeat or paused check runs: only the session watch asks
//! Beebeeb whether it still accepts the session (`GET /api/v1/auth/me`), once per period.
//!
//! The server ends the session (the fake's `end-session` switch: every call with the session token is answered 401).
//! After two refused checks the PC goes offline for one check (the fake hangs up), then comes back. The vault is still
//! open after the offline check, and the next refusal is the third in a row: the session is held as ended and the
//! session watcher closes the vault (the key dropped, the closed mark stored, the stored session kept). The checks are
//! the pins: exactly three refusals at the close, one offline check between the second and the third.
//!
//! The watch runs at the engine's own 30 s period (`app::session_watch::SESSION_WATCH_PERIOD`): an integration test
//! has no way to set it, so this test takes about two and a half minutes of wall clock.
//!
//! Mutants: offline counts as a refusal (`count_api_answer`'s last arm adds one): the vault closes during the offline
//! check. Offline clears the run (that arm clears): the close needs three more refusals and does not come within a
//! period. The engine starts no watch (`session_watch::start` left out of `Engine::open`), or the watch's answer is
//! not counted (`note_watch_answer` left out of `SessionWatch::step`): nothing closes.

mod common;

#[cfg(not(target_os = "macos"))]
#[test]
fn with_no_runner_the_watch_closes_the_vault_and_an_offline_check_keeps_it_open() {
    use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
    use std::sync::{Arc, Mutex};
    use std::time::{Duration, Instant};

    use beebeeb_engine::app::SyncStartRefused;
    use beebeeb_engine::app::lock_api::LockState;
    use beebeeb_engine::keychain::{AuthSecretStore, LockFlag};
    use beebeeb_engine::signin::gate::GateState;
    use beebeeb_engine::signin::start::EngineStartBlocked;
    use beebeeb_engine::test_api::server::{LineWriter, Switches, fixture_handler};
    use beebeeb_engine::test_api::{Handler, Reply, Request};

    // The watch's period (`app::session_watch::SESSION_WATCH_PERIOD`) and the run that closes (`AUTH_EXPIRED_THRESHOLD`).
    const PERIOD: Duration = Duration::from_secs(30);
    const RUN: usize = 3;
    const CHECK: &str = "GET /api/v1/auth/me";

    let hang_up = Reply::hang_up().status;
    // Every check the fake saw: when, and what it answered (401, or `hang_up` while offline).
    let checks: Arc<Mutex<Vec<(Instant, u16)>>> = Arc::default();
    let offline = Arc::new(AtomicBool::new(false));
    let lines: Arc<Mutex<Vec<String>>> = Arc::default();
    let switches = Switches::default();
    let s = {
        let (checks, offline, lines, switches) = (checks.clone(), offline.clone(), lines.clone(), switches.clone());
        common::setup_with_account(move |account, _, _| {
            let write: LineWriter = Arc::new(move |line: &str| lines.lock().unwrap().push(line.to_string()));
            let served = fixture_handler(account, write, Arc::new(AtomicU64::new(0)), switches);
            let handler: Handler = Arc::new(move |request: &Request| {
                // Offline: every request is hung up before Beebeeb answers (no status, as a dropped link).
                let reply = if offline.load(Ordering::SeqCst) {
                    Reply::hang_up()
                } else {
                    served(request)
                };
                if request.route() == CHECK {
                    checks.lock().unwrap().push((Instant::now(), reply.status));
                }
                reply
            });
            handler
        })
    };
    let answered = |status: u16| checks.lock().unwrap().iter().filter(|(_, s)| *s == status).count();
    let seen = || {
        checks
            .lock()
            .unwrap()
            .iter()
            .map(|(_, status)| *status)
            .collect::<Vec<_>>()
    };

    // The account's own local data, and its sync folder is gone: the start is refused and no runner is made.
    s.state_db_with_one_row(&s.this_account());
    s.sign_in();
    s.unlock();
    s.engine.watch_session_every(Duration::from_millis(50));
    let folder = s.home.join("Beebeeb Test");
    assert!(!folder.exists(), "precondition: the folder is gone");
    let port = common::Port::free();
    assert_eq!(
        s.engine.start_sync_with(folder.clone(), &port),
        Err(SyncStartRefused::FolderMissing)
    );
    assert_eq!(port.registers(), 0, "nothing was registered");
    assert!(!s.engine.is_syncing(), "no runner");
    assert_eq!(s.engine.lock_state(), LockState::Open);
    let started_at = s.fake.requests().len();

    // The server ends the session. The sign-in asked `auth/me` for the profile itself: the log of checks starts here.
    checks.lock().unwrap().clear();
    switches.session_ended.store(true, Ordering::SeqCst);
    let ended = Instant::now();

    // Two refused checks. The first watch step after the sign-in may find the sign-in's own answer within the period
    // and ask nothing, so the second refusal comes at most three periods after the end.
    let two = common::wait_until(4 * PERIOD.as_secs() + 15, || answered(401) >= 2);
    assert!(
        two,
        "two refused checks within four periods of the end ({:?}): {:?}",
        ended.elapsed(),
        seen()
    );
    assert!(!s.engine.account_session_refused(), "two refusals are not yet the run");
    assert_eq!(
        s.engine.lock_state(),
        LockState::Open,
        "two refusals keep the vault open"
    );

    // Offline for one check: the fake hangs up. The vault stays open.
    offline.store(true, Ordering::SeqCst);
    let hung_up = common::wait_until(2 * PERIOD.as_secs() + 15, || answered(hang_up) >= 1);
    assert!(hung_up, "one check while offline: {:?}", seen());
    // The step notes its answer within milliseconds of the hang-up; a second is plenty on a loaded rig.
    std::thread::sleep(Duration::from_secs(1));
    assert!(
        !s.engine.account_session_refused(),
        "an offline check is not a refusal: {:?}",
        seen()
    );
    assert_eq!(
        s.engine.lock_state(),
        LockState::Open,
        "offline: the vault stays open: {:?}",
        seen()
    );
    assert_eq!(s.engine.gate(), GateState::Ready);
    offline.store(false, Ordering::SeqCst);
    let back = Instant::now();

    // Back online: the next check is refused, the third in a row, and the session watcher closes the vault. Within one
    // period plus a margin: had the offline check cleared the run, three more periods would be needed.
    let closed = common::wait_until(PERIOD.as_secs() + 15, || {
        s.engine.lock_state() == LockState::VaultClosed
    });
    let took = back.elapsed();
    let at_close = seen();
    let after_start = s.routes_from(started_at);

    // The instrument first: no runner talked to the server, so only the watch's checks fed the run.
    let runner_calls: Vec<&String> = after_start
        .iter()
        .filter(|route| route.contains("/api/v1/sync/") || route.contains("/clients/"))
        .collect();
    assert!(
        runner_calls.is_empty(),
        "the instrument: no sync or heartbeat call after the refused start: {runner_calls:?}"
    );
    // One check per period, never two in a burst.
    // A hung-up check is retried by the API client within the same check (a burst of hang-ups, a few hundred ms
    // apart): those requests count as the one check they are, at the time of its first request.
    let mut times: Vec<Instant> = Vec::new();
    let mut last_status = None;
    for (at, status) in checks.lock().unwrap().iter() {
        if !(*status == hang_up && last_status == Some(hang_up)) {
            times.push(*at);
        }
        last_status = Some(*status);
    }
    for pair in times.windows(2) {
        let apart = pair[1].duration_since(pair[0]);
        assert!(
            apart >= PERIOD - Duration::from_secs(1),
            "two checks {apart:?} apart, less than one period: {at_close:?}"
        );
    }

    // The pins: exactly three refusals at the close, and the offline check between the second and the third.
    assert!(
        closed,
        "the vault closed within one period of coming back ({took:?}; checks {at_close:?}; session held as ended: {})",
        s.engine.account_session_refused()
    );
    assert_eq!(
        at_close.iter().filter(|status| **status == 401).count(),
        RUN,
        "the close came with the third refusal, not later: the offline check neither counted nor cleared: {at_close:?}"
    );
    let last_two = at_close
        .iter()
        .filter(|status| **status == 401 || **status == hang_up)
        .rev()
        .take(2)
        .copied()
        .collect::<Vec<_>>();
    assert_eq!(
        last_two,
        vec![401, hang_up],
        "the offline check came right before the third refusal: {at_close:?}"
    );

    // The close, as core says: no key in memory, the gate locked, the closed mark stored, the stored session kept.
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
    println!(
        "closed {took:?} after coming back, {:?} after the end; checks {at_close:?}; lines served {}",
        ended.elapsed(),
        lines.lock().unwrap().len()
    );
}

#[cfg(target_os = "macos")]
#[test]
fn the_test_build_is_refused_on_macos() {
    common::assert_the_test_build_is_refused();
}
