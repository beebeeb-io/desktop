//! Sign-in and security on the real engine against the parent module's fake: what the page is told
//! (`AccountSource::security_facts`), for which session and account, and when its count is read. Every name
//! is invented.

use super::*;

/// Everything this source never reads stays "not known", whatever else is held.
fn the_rest_is_not_known(facts: &SecurityFacts) {
    assert!(facts.app_lock.is_none() && facts.password_changed.is_none() && facts.backup_codes.is_none());
    assert!(facts.passkeys.is_none() && facts.activity.is_none() && facts.factors.is_none());
    assert!(!facts.can_sign_out);
}

#[test]
fn the_security_page_says_two_factor_and_the_devices_of_this_account_and_nothing_else() {
    let server = server();
    server.two_factor.lock().unwrap()[0] = Some(true);
    let mut source = Source::new(signed_in(&server));
    // Before anything is read: nothing is known, and nothing is a guess.
    let before = source.security_facts();
    assert_eq!((before.two_factor, before.sessions), (None, None));
    the_rest_is_not_known(&before);
    assert_eq!(source.pane_state(Pane::Security), PaneState::Loading);
    opened(&mut source, Pane::Security);
    assert_eq!(source.pane_state(Pane::Security), PaneState::Ready);
    let facts = source.security_facts();
    assert_eq!(facts.two_factor, Some(true));
    assert_eq!(facts.sessions, Some(3), "one per session of the list");
    the_rest_is_not_known(&facts);
    // Off is said only when the profile says off.
    server.two_factor.lock().unwrap()[0] = Some(false);
    source.reload(Pane::Security);
    settle(&mut source);
    assert_eq!(source.security_facts().two_factor, Some(false));
}

#[test]
fn a_profile_without_the_two_factor_mark_is_not_known_never_off() {
    let server = server();
    let mut source = Source::new(signed_in(&server));
    opened(&mut source, Pane::Security);
    assert_eq!(source.pane_state(Pane::Security), PaneState::Ready);
    assert_eq!(source.security_facts().two_factor, None);
}

#[test]
fn a_sessions_list_that_fails_leaves_the_device_count_not_known_and_the_page_up() {
    let server = server();
    server.two_factor.lock().unwrap()[0] = Some(true);
    let mut source = Source::new(signed_in(&server));
    opened(&mut source, Pane::Security);
    assert_eq!(source.security_facts().sessions, Some(3));
    // The list is read again and fails: the count is not known (never 0, never the old one), the page stays.
    server.read_mode.store(SERVER_ERROR, Ordering::SeqCst);
    source.reload(Pane::Sessions);
    settle(&mut source);
    let facts = source.security_facts();
    assert_eq!(facts.sessions, None, "never 0 for a list that was not read");
    assert_eq!(facts.two_factor, Some(true), "what the page's own read said stands");
    assert_eq!(source.pane_state(Pane::Security), PaneState::Ready);
    // Reading the page again reads the list again too.
    server.read_mode.store(ANSWER, Ordering::SeqCst);
    source.reload(Pane::Security);
    settle(&mut source);
    assert_eq!(source.security_facts().sessions, Some(3));
}

#[test]
fn signed_out_the_security_page_holds_nothing_before_any_poll() {
    let server = server();
    server.two_factor.lock().unwrap()[0] = Some(true);
    let engine = signed_in(&server);
    let mut source = Source::new(engine.clone());
    opened(&mut source, Pane::Security);
    assert_eq!(source.security_facts().two_factor, Some(true));
    engine.sign_out_keeping_local_data().expect("signs out");
    // No poll ran: the accessor itself refuses what another session read.
    let facts = source.security_facts();
    assert_eq!((facts.two_factor, facts.sessions), (None, None));
    assert_eq!(source.pane_state(Pane::Security), PaneState::SignedOut);
}

#[test]
fn another_account_never_sees_the_first_ones_two_factor_or_device_count() {
    let server = server();
    *server.two_factor.lock().unwrap() = [Some(true), Some(false)];
    let engine = signed_in(&server);
    let mut source = Source::new(engine.clone());
    opened(&mut source, Pane::Security);
    assert_eq!(source.security_facts().two_factor, Some(true));
    assert_eq!(source.security_facts().sessions, Some(3));
    // The first account's answers are slow: they land after the second account signed in.
    server.read_delay_ms.store(400, Ordering::SeqCst);
    source.reload(Pane::Security);
    source.reload(Pane::Sessions);
    engine.sign_out_keeping_local_data().expect("signs out");
    sign_in(&engine, &server.people[1]);
    let facts = source.security_facts();
    assert_eq!(
        (facts.two_factor, facts.sessions),
        (None, None),
        "before the poll: nothing of the first"
    );
    server.read_delay_ms.store(0, Ordering::SeqCst);
    settle(&mut source);
    std::thread::sleep(Duration::from_millis(500));
    settle(&mut source);
    // The page is on screen: the second account's own values were read without a reopening.
    let facts = source.security_facts();
    assert_eq!(facts.two_factor, Some(false), "the second account's own mark");
    assert_eq!(facts.sessions, Some(1), "the second account's own list");
}

/// The page says "Loading" while the list it counts is still on its way: never "Not known" for a list that is
/// about to answer.
#[test]
fn the_security_page_loads_until_its_first_count_answered() {
    let server = server();
    server.read_delay_ms.store(1_500, Ordering::SeqCst);
    let mut source = Source::new(signed_in(&server));
    source.opened(Pane::Security);
    // The page's own read (the profile) lands first: it is not slowed.
    let began = Instant::now();
    while source.identity().email.is_empty() {
        assert!(
            began.elapsed() < Duration::from_secs(1),
            "the profile did not answer before the list"
        );
        source.pump();
        std::thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(source.security_facts().sessions, None);
    assert_eq!(
        source.pane_state(Pane::Security),
        PaneState::Loading,
        "the list is on its way"
    );
    assert_eq!(
        source.pane_state(Pane::Account),
        PaneState::Ready,
        "Account does not wait for it"
    );
    settle(&mut source);
    assert_eq!(source.pane_state(Pane::Security), PaneState::Ready);
    assert_eq!(source.security_facts().sessions, Some(3));
}

/// A sign-out answered while the Security page shows: the page's count is of a list read AFTER that answer.
#[test]
fn a_sign_out_answered_under_the_security_page_is_followed_by_a_new_count() {
    let server = server();
    let mut source = Source::new(signed_in(&server));
    opened(&mut source, Pane::Sessions);
    server.change_delay_ms.store(300, Ordering::SeqCst);
    assert!(source.sign_out_device("s-phone"));
    // The person goes to Security before Beebeeb answered.
    source.opened(Pane::Security);
    settle(&mut source);
    assert_eq!(server.deleted.lock().unwrap().clone(), ["s-phone"]);
    assert_eq!(
        source.security_facts().sessions,
        Some(2),
        "counted after the sign-out, not before"
    );
}

/// The same when a list read was already on its way: that list is older than the sign-out, so it is not counted,
/// and the list is read once more.
#[test]
fn a_list_read_before_a_sign_out_is_not_the_security_pages_count() {
    let server = server();
    let mut source = Source::new(signed_in(&server));
    opened(&mut source, Pane::Sessions);
    // A slow read of the list starts; then the sign-out, which Beebeeb answers first.
    server.read_delay_ms.store(700, Ordering::SeqCst);
    source.reload(Pane::Sessions);
    std::thread::sleep(Duration::from_millis(150));
    server.change_delay_ms.store(100, Ordering::SeqCst);
    assert!(source.sign_out_device("s-phone"));
    source.opened(Pane::Security);
    server.read_delay_ms.store(0, Ordering::SeqCst);
    settle(&mut source);
    assert_eq!(server.deleted.lock().unwrap().clone(), ["s-phone"]);
    assert_eq!(
        source.security_facts().sessions,
        Some(2),
        "the older list is not what is counted"
    );
}

/// A sign-out that was not done, said when Devices and sessions shows again: the list is read with it, even
/// when the Security page's count read it a moment ago.
#[test]
fn a_sign_out_said_later_reads_the_list_again_however_fresh() {
    let server = server();
    let mut source = Source::new(signed_in(&server));
    opened(&mut source, Pane::Sessions);
    server.change_mode.store(SERVER_ERROR, Ordering::SeqCst);
    server.change_delay_ms.store(300, Ordering::SeqCst);
    assert!(source.sign_out_device("s-phone"));
    // The person leaves; the answer lands under Security, whose count reads the list.
    opened(&mut source, Pane::Security);
    assert_eq!(source.notice(), None, "not said over another pane");
    let asked = server.asked("GET /api/v1/account/sessions");
    opened(&mut source, Pane::Sessions);
    assert!(
        source.notice().is_some() && source.notice_failed(),
        "said now, over its own pane"
    );
    assert_eq!(
        server.asked("GET /api/v1/account/sessions"),
        asked + 1,
        "read again with the words"
    );
}

#[test]
fn the_device_count_is_not_read_for_a_page_that_does_not_show_it() {
    let server = server();
    let mut source = Source::new(signed_in(&server));
    opened(&mut source, Pane::Account);
    assert_eq!(
        server.asked("GET /api/v1/account/sessions"),
        0,
        "Account does not count devices"
    );
    opened(&mut source, Pane::Security);
    assert_eq!(server.asked("GET /api/v1/account/sessions"), 1);
    // Fresh: opening it again asks nothing more.
    opened(&mut source, Pane::Security);
    assert_eq!(server.asked("GET /api/v1/account/sessions"), 1);
}
