//! Notifications on the real engine, the states a person can see: the rows' places while the account's switches
//! are unread, Recent's loading and failed states, what stays after a refused mark, and writes that are only sent
//! under the session of their press. The fake and its helpers are the parent module's.

use super::*;

#[test]
fn a_group_is_all_the_accounts_or_all_this_pcs() {
    for (title, rows) in GROUPS {
        let of_account = rows
            .iter()
            .filter(|(_, _, stored)| matches!(stored, Stored::Account(_)))
            .count();
        assert!(
            of_account == 0 || of_account == rows.len(),
            "{title} mixes the two: its read state would speak for rows it does not cover"
        );
    }
}

fn find(rt: &Runtime<State>, key: &str) -> Option<rune::ElementInfo> {
    rt.elements()
        .into_iter()
        .find(|e| e.key.as_deref() == Some(key) && !e.exiting)
}

fn centre(rt: &Runtime<State>, key: &str) -> (f32, f32) {
    let r = find(rt, key).unwrap_or_else(|| panic!("no element keyed {key}")).rect;
    (r[0] + r[2] / 2.0, r[1] + r[3] / 2.0)
}

fn click_at(rt: &mut Runtime<State>, (x, y): (f32, f32)) {
    rt.pointer_down(x, y);
    rt.pointer_up(x, y);
    rt.settle_all();
    rt.frame(0.0);
}

/// Draw what the source holds now, taking in nothing a worker answered.
fn draw(rt: &mut Runtime<State>) {
    rt.frame(0.0);
    rt.settle_all();
    rt.frame(0.0);
}

fn window(engine: &Arc<Engine<MemoryCredentials>>) -> Runtime<State> {
    let state = crate::shell::state::init_on_engine(crate::shell::host::Shell::fixture(), engine.clone());
    let env = Env {
        system_dark: false,
        scale: 2.0,
    };
    let mut rt = Runtime::new(state, crate::view, (958.0, 638.0), env);
    rt.frame(0.0);
    rt.update(|s, _| {
        crate::shell::host::dispatch(s, Event::OpenSettings(SettingsPane::Notifications));
    });
    rt
}

/// The rows of the three groups as the pane keys them (lists 3, 4, 5), each with its rect.
fn row_rects(rt: &Runtime<State>) -> Vec<(String, [f32; 4])> {
    [(3, 3), (4, 4), (5, 2)]
        .into_iter()
        .flat_map(|(list, rows)| (0..rows).map(move |i| format!("item-{list}-{i}")))
        .map(|key| {
            let rect = find(rt, &key).unwrap_or_else(|| panic!("no row keyed {key}")).rect;
            (key, rect)
        })
        .collect()
}

#[test]
fn a_press_aimed_before_the_read_lands_hits_the_same_switch_after() {
    let fake = fake("aim");
    let engine = signed_in(&fake);
    let mut rt = window(&engine);
    // Drawn with the read on its way (or answered and not yet taken in: the same to the pane).
    rt.update(|s, _| crate::frames::settings::go(s, "DW-0750"));
    draw(&mut rt);
    let seen = words(&rt);
    assert!(
        seen.contains("Shared with you"),
        "the account's rows are there before the read: {seen}"
    );
    assert!(seen.contains("Loading"), "and say they are loading: {seen}");
    for i in 0..3 {
        assert!(
            find(&rt, &format!("item-3-{i}-switch")).is_none(),
            "an unread account switch is not drawn"
        );
    }
    let before = row_rects(&rt);
    let aim = centre(&rt, "item-4-0-switch");
    let unread = centre(&rt, "item-3-0-value");
    // A press where an unread row says "Loading": nothing to press.
    click_at(&mut rt, unread);

    poll(&mut rt);
    assert!(
        find(&rt, "item-3-0-switch").is_some(),
        "the read landed: the account's switches are drawn"
    );
    assert_eq!(row_rects(&rt), before, "no row moved when the read landed");
    assert_eq!(
        centre(&rt, "item-4-0-switch"),
        aim,
        "Sync stopped is where it was aimed at"
    );

    let stopped = rt.state.settings.source.notifications().groups[1].1[0].on;
    let account: Vec<bool> = rt.state.settings.source.notifications().groups[0]
        .1
        .iter()
        .map(|a| a.on)
        .collect();
    click_at(&mut rt, aim);
    poll(&mut rt);
    let now = rt.state.settings.source.notifications();
    assert_eq!(now.groups[1].1[0].on, !stopped, "the press changed Sync stopped");
    assert_eq!(
        now.groups[0].1.iter().map(|a| a.on).collect::<Vec<_>>(),
        account,
        "and no switch of the account"
    );
    assert!(
        fake.writes.lock().unwrap().is_empty(),
        "nothing was sent to the account: {:?}",
        fake.writes.lock().unwrap().len()
    );
    // Put this PC's switch back (its settings file is the test process's own).
    click_at(&mut rt, aim);
    poll(&mut rt);
}

#[test]
fn recent_says_loading_then_could_not_load_and_never_no_notifications_unread() {
    let fake = fake("recent-state");
    fake.read_mode.store(SERVER_ERROR, Ordering::SeqCst);
    let engine = signed_in(&fake);
    let mut rt = window(&engine);
    rt.update(|s, _| crate::frames::settings::go(s, "DW-0791"));
    draw(&mut rt);
    let seen = words(&rt);
    assert!(
        seen.contains("Loading your notifications"),
        "the read is on its way: {seen}"
    );
    assert!(!seen.contains("No notifications"), "nothing was read yet: {seen}");

    poll(&mut rt);
    let seen = words(&rt);
    assert!(
        seen.contains("Couldn't load your notifications"),
        "the read failed: {seen}"
    );
    assert!(
        !seen.contains("No notifications"),
        "a failed read is not an empty list: {seen}"
    );

    // "Try again" asks again, and an answer is drawn.
    fake.read_mode.store(ANSWER, Ordering::SeqCst);
    let asked_before = asked(&fake.fake, LIST);
    let again = centre(&rt, "btn-0");
    click_at(&mut rt, again);
    poll(&mut rt);
    assert_eq!(asked(&fake.fake, LIST), asked_before + 1, "one read for the one press");
    let seen = words(&rt);
    assert!(seen.contains(FIRST_SHARE), "the list, once read: {seen}");
    assert!(!seen.contains("Couldn't load your notifications"));

    // Read and empty: only now may the page say there are none.
    fake.read_mode.store(EMPTY, Ordering::SeqCst);
    rt.state.settings.source.reload_notifications();
    poll(&mut rt);
    let seen = words(&rt);
    assert!(seen.contains("No notifications"), "read, and empty: {seen}");
}

#[test]
fn a_mark_and_its_read_again_both_failing_keep_what_was_held_and_say_it_once() {
    let fake = fake("mark-offline");
    let mut source = Source::new(signed_in(&fake));
    opened(&mut source, Pane::Recent);
    let held = titles(&source);
    assert_eq!(held[0], (FIRST_SHARE.to_string(), true));
    let shared = switch(&source, 0, 0);
    // No answer for the mark, none for the read that follows it.
    fake.mark_mode.store(SERVER_ERROR, Ordering::SeqCst);
    fake.read_mode.store(SERVER_ERROR, Ordering::SeqCst);
    let lists = asked(&fake.fake, LIST);
    source.mark_recent_seen();
    assert_eq!(settle(&mut source), [NOT_MARKED], "one toast: the mark's");
    assert_eq!(asked(&fake.fake, LIST), lists + 1, "it was read again");
    assert_eq!(
        titles(&source),
        held,
        "the list that was held stays, New back on what was not marked"
    );
    assert_eq!(source.recent_state(), ReadState::Ready);
    assert_eq!(
        source.alerts_state(0),
        ReadState::Ready,
        "the switches that were held stay"
    );
    assert_eq!(switch(&source, 0, 0), shared);
    assert_eq!(places(&source), all_places());

    // What was kept is not fresh: the next opening reads again, and takes the answer.
    fake.mark_mode.store(ANSWER, Ordering::SeqCst);
    fake.read_mode.store(ANSWER, Ordering::SeqCst);
    assert!(opened(&mut source, Pane::Recent).is_empty());
    assert_eq!(asked(&fake.fake, LIST), lists + 2, "kept, never stamped fresh");

    // Only the read after a refused mark keeps: any other failed read leaves nothing of the account drawn.
    fake.read_mode.store(SERVER_ERROR, Ordering::SeqCst);
    source.alerts.expire();
    assert_eq!(opened(&mut source, Pane::Recent), [NOT_LOADED]);
    assert!(titles(&source).is_empty());
    assert_eq!(source.recent_state(), ReadState::Failed);
    assert_eq!(source.alerts_state(0), ReadState::Failed);
}

#[test]
fn a_session_that_ended_at_the_read_after_a_refused_mark_keeps_nothing() {
    let fake = fake("mark-ended");
    let mut source = Source::new(signed_in(&fake));
    opened(&mut source, Pane::Recent);
    fake.mark_mode.store(SERVER_ERROR, Ordering::SeqCst);
    fake.read_mode.store(SESSION_ENDED, Ordering::SeqCst);
    source.mark_recent_seen();
    assert_eq!(settle(&mut source), [NOT_MARKED]);
    assert!(
        titles(&source).is_empty(),
        "Beebeeb ended the session: nothing of it stays drawn"
    );
    assert_eq!(source.alerts_state(0), ReadState::Failed);
}

fn puts(fake: &Fake) -> usize {
    fake.writes.lock().unwrap().len()
}

#[test]
fn the_engine_sends_no_write_under_a_session_that_moved_on() {
    let fake = fake("engine-turn");
    let engine = signed_in(&fake);
    let update = AccountSwitch::Shared.update(false);
    let pressed_under = engine.session_turn();
    let _ = engine.lock();
    engine
        .unlock_with_phrase(Zeroizing::new(fake.first_phrase.clone()))
        .expect("the phrase unlocks");
    assert_eq!(
        engine.set_notification_preferences_for(pressed_under, &update).err(),
        Some(AccountCallError::NotSignedIn),
        "the same account, another session: refused"
    );
    assert_eq!(
        engine.mark_notifications_read_for(pressed_under, &["n-1".to_string()]),
        Err(AccountCallError::NotSignedIn)
    );
    assert_eq!(puts(&fake), 0, "nothing was sent");

    let now = engine.session_turn();
    let after = engine
        .set_notification_preferences_for(now, &update)
        .expect("the current session's change is sent");
    assert!(!after.preferences.share_received);
    engine
        .mark_notifications_read_for(now, &["n-1".to_string()])
        .expect("the current session's mark is sent");
    assert_eq!(puts(&fake), 2);

    let _ = engine.lock();
    assert_eq!(
        engine
            .set_notification_preferences_for(engine.session_turn(), &update)
            .err(),
        Some(AccountCallError::NotSignedIn),
        "no session in memory: nothing is asked"
    );
    assert_eq!(puts(&fake), 2);
}

#[test]
fn a_switch_pressed_under_one_account_is_never_sent_under_the_next() {
    let fake = fake("press-turn");
    let engine = signed_in(&fake);
    let mut source = Source::new(engine.clone());
    opened(&mut source, Pane::Notifications);
    assert!(switch(&source, 0, 0), "the first account's Shared with you is on");
    // The press is made; its worker has not called the engine yet when another account signs in.
    let release = source.alerts.hold_next_write();
    source.set_alert(0, 0, false);
    engine.sign_out_keeping_local_data().expect("signed out");
    sign_in(&engine, &fake.second, &fake.second_phrase);
    release.send(()).expect("the worker is waiting");
    let toasts = settle(&mut source);
    assert_eq!(puts(&fake), 0, "the first account's press reached no account");
    assert!(
        !toasts.contains(&ALERT_SAVED),
        "and is not said to be saved: {toasts:?}"
    );

    // The same with a lock and an unlock of the same account in between.
    opened(&mut source, Pane::Notifications);
    let on = switch(&source, 0, 1);
    let release = source.alerts.hold_next_write();
    source.set_alert(0, 1, !on);
    let _ = engine.lock();
    engine
        .unlock_with_phrase(Zeroizing::new(fake.second_phrase.clone()))
        .expect("the phrase unlocks");
    release.send(()).expect("the worker is waiting");
    let toasts = settle(&mut source);
    assert_eq!(
        puts(&fake),
        0,
        "a press made before the lock is not sent after the unlock"
    );
    assert!(!toasts.contains(&ALERT_SAVED), "{toasts:?}");

    // A press under the session that is still there is sent, held or not.
    opened(&mut source, Pane::Notifications);
    let release = source.alerts.hold_next_write();
    source.set_alert(0, 1, !on);
    release.send(()).expect("the worker is waiting");
    assert_eq!(settle(&mut source), [ALERT_SAVED]);
    assert_eq!(puts(&fake), 1);
    assert_eq!(switch(&source, 0, 1), !on);
}

#[test]
fn a_mark_made_under_one_account_is_never_sent_under_the_next() {
    let fake = fake("mark-turn");
    let engine = signed_in(&fake);
    let mut source = Source::new(engine.clone());
    opened(&mut source, Pane::Recent);
    assert_eq!(source.notifications().new_recent, 1);
    let release = source.alerts.hold_next_write();
    source.mark_recent_seen();
    engine.sign_out_keeping_local_data().expect("signed out");
    sign_in(&engine, &fake.second, &fake.second_phrase);
    release.send(()).expect("the worker is waiting");
    settle(&mut source);
    assert_eq!(
        puts(&fake),
        0,
        "the first account's ids were sent under no other session"
    );
}
