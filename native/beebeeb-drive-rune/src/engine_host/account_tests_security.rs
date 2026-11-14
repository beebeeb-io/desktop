//! Sign-in and security on the real engine against the parent module's fake: what the page is told
//! (`AccountSource::security_facts`), for which session and account, and when its count is read. Every name
//! is invented.

use super::*;

/// Everything this source never reads stays "not known", whatever else is held.
fn the_rest_is_not_known(facts: &SecurityFacts) {
    assert!(facts.app_lock.is_none() && facts.password_changed.is_none() && facts.backup_codes.is_none());
    assert!(facts.passkeys.is_none() && facts.activity.is_none() && facts.factors.is_none());
    assert!(!facts.can_sign_out_all, "Drive is the one app on this PC");
}

#[test]
fn the_security_page_says_two_factor_and_the_devices_of_this_account_and_nothing_else() {
    let server = server();
    server.two_factor.lock().unwrap()[0] = Some(true);
    let mut source = Source::new(signed_in(&server));
    // Before anything is read: nothing is known, and nothing is a guess.
    let before = source.security_facts();
    assert_eq!((before.two_factor, before.sessions), (None, Count::NotKnown));
    the_rest_is_not_known(&before);
    assert_eq!(source.pane_state(Pane::Security), PaneState::Loading);
    opened(&mut source, Pane::Security);
    assert_eq!(source.pane_state(Pane::Security), PaneState::Ready);
    let facts = source.security_facts();
    assert_eq!(facts.two_factor, Some(true));
    assert_eq!(facts.sessions, Count::Known(3), "one per session of the list");
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

/// A list that was never read is "not known" (never 0). A re-read that fails over a list that was read keeps
/// that list's count, as Devices and sessions keeps its rows (no sign-out was sent since it was read).
#[test]
fn a_sessions_list_that_fails_is_never_counted_as_none() {
    let server = server();
    server.two_factor.lock().unwrap()[0] = Some(true);
    // The first read of the list fails: there is nothing to count.
    let mut source = Source::new(signed_in(&server));
    opened(&mut source, Pane::Account);
    server.read_mode.store(SERVER_ERROR, Ordering::SeqCst);
    source.opened(Pane::Security);
    settle(&mut source);
    let facts = source.security_facts();
    assert_eq!(facts.sessions, Count::NotKnown, "never 0 for a list that was not read");
    assert_eq!(source.pane_state(Pane::Security), PaneState::Ready, "the page stays up");
    // Reading the page again reads the list again too.
    server.read_mode.store(ANSWER, Ordering::SeqCst);
    source.reload(Pane::Security);
    settle(&mut source);
    assert_eq!(source.security_facts().sessions, Count::Known(3));
    assert_eq!(source.security_facts().two_factor, Some(true));
    // A later re-read that fails keeps what was read.
    server.read_mode.store(SERVER_ERROR, Ordering::SeqCst);
    source.reload(Pane::Sessions);
    settle(&mut source);
    assert_eq!(
        source.security_facts().sessions,
        Count::Known(3),
        "the list that was read stands"
    );
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
    assert_eq!((facts.two_factor, facts.sessions), (None, Count::NotKnown));
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
    assert_eq!(source.security_facts().sessions, Count::Known(3));
    // The first account's answers are slow: they land after the second account signed in.
    server.read_delay_ms.store(400, Ordering::SeqCst);
    source.reload(Pane::Security);
    source.reload(Pane::Sessions);
    engine.sign_out_keeping_local_data().expect("signs out");
    sign_in(&engine, &server.people[1]);
    let facts = source.security_facts();
    assert_eq!(facts.two_factor, None, "before the poll: nothing of the first");
    assert!(
        !matches!(facts.sessions, Count::Known(_)),
        "before the poll: no count of the first"
    );
    server.read_delay_ms.store(0, Ordering::SeqCst);
    settle(&mut source);
    std::thread::sleep(Duration::from_millis(500));
    settle(&mut source);
    // The page is on screen: the second account's own values were read without a reopening.
    let facts = source.security_facts();
    assert_eq!(facts.two_factor, Some(false), "the second account's own mark");
    assert_eq!(facts.sessions, Count::Known(1), "the second account's own list");
}

/// The page does not wait for its count: what its own read said shows at once, and the count's row says it is
/// loading (never "Not known" for a list that is on its way, never the whole page on "Loading").
#[test]
fn the_security_page_shows_at_once_and_its_count_says_loading_until_it_answered() {
    let server = server();
    // Only the list is slow (the profile is the accounts' own route here, which does not wait).
    server.read_delay_ms.store(1_500, Ordering::SeqCst);
    let mut source = Source::new(signed_in(&server));
    source.opened(Pane::Security);
    let began = Instant::now();
    while source.pane_state(Pane::Security) != PaneState::Ready {
        assert!(began.elapsed() < Duration::from_secs(1), "the page waited for the list");
        source.pump();
        std::thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(
        source.security_facts().sessions,
        Count::Reading,
        "the count's row says it is loading"
    );
    settle(&mut source);
    assert_eq!(source.security_facts().sessions, Count::Known(3));
    // Read again, slowly, with nothing in between: the count that is held stands while the new one is out.
    source.reload(Pane::Sessions);
    assert_eq!(source.security_facts().sessions, Count::Known(3));
    assert_eq!(source.pane_state(Pane::Security), PaneState::Ready);
    server.read_delay_ms.store(0, Ordering::SeqCst);
    settle(&mut source);
}

/// From the moment Beebeeb took a sign-out, the list read before it is not counted: the row says it is being
/// read again, for as long as that takes.
#[test]
fn a_count_is_never_of_a_list_read_before_a_sign_out_not_even_while_the_new_one_is_out() {
    let server = server();
    let mut source = Source::new(signed_in(&server));
    opened(&mut source, Pane::Sessions);
    opened(&mut source, Pane::Security);
    assert_eq!(source.security_facts().sessions, Count::Known(3));
    source.opened(Pane::Sessions);
    // The sign-out answers quickly; the list read after it is slow.
    server.read_delay_ms.store(1_500, Ordering::SeqCst);
    assert!(source.sign_out_device("s-phone"));
    source.opened(Pane::Security);
    let began = Instant::now();
    loop {
        source.pump();
        let taken = !server.deleted.lock().unwrap().is_empty();
        if taken && source.security_facts().sessions != Count::Known(3) {
            break;
        }
        assert!(
            began.elapsed() < Duration::from_secs(1),
            "still counts the list read before the sign-out"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(source.security_facts().sessions, Count::Reading);
    server.read_delay_ms.store(0, Ordering::SeqCst);
    settle(&mut source);
    assert_eq!(source.security_facts().sessions, Count::Known(2));
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
        Count::Known(2),
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
    // A slow read of the list is under way at Beebeeb (it has taken its list) when the sign-out is sent.
    let (under_way, seen) = std::sync::mpsc::channel();
    *server.during_read.lock().unwrap() = Some(Box::new(move || {
        let _ = under_way.send(());
    }));
    server.read_delay_ms.store(1_500, Ordering::SeqCst);
    source.reload(Pane::Sessions);
    seen.recv_timeout(Duration::from_secs(5))
        .expect("the read reached Beebeeb");
    assert!(source.sign_out_device("s-phone"));
    source.opened(Pane::Security);
    // Beebeeb takes the sign-out while that read is still out.
    let began = Instant::now();
    while server.deleted.lock().unwrap().is_empty() {
        assert!(
            began.elapsed() < Duration::from_secs(1),
            "the sign-out waited for the read"
        );
        source.pump();
        std::thread::sleep(Duration::from_millis(10));
    }
    // (Only now: the slow read took its delay long ago; the read after it is quick.)
    server.read_delay_ms.store(0, Ordering::SeqCst);
    settle(&mut source);
    assert_eq!(server.deleted.lock().unwrap().clone(), ["s-phone"]);
    assert_eq!(
        source.security_facts().sessions,
        Count::Known(2),
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
