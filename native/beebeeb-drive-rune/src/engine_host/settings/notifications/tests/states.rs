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
    window_sized(engine, (958.0, 638.0))
}

fn window_sized(engine: &Arc<Engine<MemoryCredentials>>, size: (f32, f32)) -> Runtime<State> {
    let state = crate::shell::state::init_on_engine(crate::shell::host::Shell::fixture(), engine.clone());
    let env = Env {
        system_dark: false,
        scale: 2.0,
    };
    let mut rt = Runtime::new(state, crate::view, size, env);
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

    let stopped = rt.state.settings.source.notifications().groups[1].1[0]
        .on
        .expect("this PC's are read");
    let account: Vec<Option<bool>> = rt.state.settings.source.notifications().groups[0]
        .1
        .iter()
        .map(|a| a.on)
        .collect();
    click_at(&mut rt, aim);
    poll(&mut rt);
    let now = rt.state.settings.source.notifications();
    assert_eq!(now.groups[1].1[0].on, Some(!stopped), "the press changed Sync stopped");
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
    assert!(
        !seen.contains("Nothing was changed"),
        "not claimed: a mark may have been taken: {seen}"
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
    assert!(
        source.notifications_unconfirmed(),
        "and is said to be possibly out of date"
    );

    // What was kept is not fresh: the next opening reads again, and takes the answer.
    fake.mark_mode.store(ANSWER, Ordering::SeqCst);
    fake.read_mode.store(ANSWER, Ordering::SeqCst);
    assert!(opened(&mut source, Pane::Recent).is_empty());
    assert_eq!(asked(&fake.fake, LIST), lists + 2, "kept, never stamped fresh");

    assert!(!source.notifications_unconfirmed(), "confirmed again");

    // A failed read that follows no failed write leaves nothing of the account drawn.
    fake.read_mode.store(SERVER_ERROR, Ordering::SeqCst);
    source.alerts.expire();
    assert_eq!(opened(&mut source, Pane::Recent), [NOT_LOADED]);
    assert!(titles(&source).is_empty());
    assert_eq!(source.recent_state(), ReadState::Failed);
    assert_eq!(source.alerts_state(0), ReadState::Failed);
    assert!(
        source.notifications().groups[0].1.iter().all(|a| a.on.is_none()),
        "a switch that was not read has no value"
    );
    assert!(
        source.notifications().groups[1].1.iter().all(|a| a.on.is_some()),
        "this PC's switches are read from its own file"
    );
    assert!(!source.notifications_unconfirmed(), "nothing is held to be unsure of");
}

/// Beebeeb ended the session: nothing of it stays drawn, whichever call was told so, and a read that fails for
/// another reason after that is said (it is not "after a mark that said it failed").
#[test]
fn a_session_beebeeb_ended_keeps_nothing_whichever_call_was_told() {
    // The read after a refused mark is told.
    let fake = fake("mark-ended");
    let mut source = Source::new(signed_in(&fake));
    opened(&mut source, Pane::Recent);
    fake.mark_mode.store(SERVER_ERROR, Ordering::SeqCst);
    fake.read_mode.store(SESSION_ENDED, Ordering::SeqCst);
    source.mark_recent_seen();
    assert_eq!(settle(&mut source), [NOT_MARKED]);
    assert!(titles(&source).is_empty(), "nothing of it stays drawn");
    assert_eq!(source.alerts_state(0), ReadState::Failed);
    assert!(!source.notifications_unconfirmed());

    // The mark itself is told (no toast of its own), and the read after it fails for want of an answer: said, and
    // nothing stays.
    let fake = super::fake("mark-told");
    let mut source = Source::new(signed_in(&fake));
    opened(&mut source, Pane::Recent);
    assert!(!titles(&source).is_empty());
    fake.mark_mode.store(SESSION_ENDED, Ordering::SeqCst);
    fake.read_mode.store(SERVER_ERROR, Ordering::SeqCst);
    source.mark_recent_seen();
    assert_eq!(
        settle(&mut source),
        [NOT_LOADED],
        "never silent: the failed read is said"
    );
    assert!(
        titles(&source).is_empty(),
        "nothing of a session Beebeeb ended stays drawn"
    );
    assert_eq!(source.recent_state(), ReadState::Failed, "which offers Try again");
    assert_eq!(source.alerts_state(0), ReadState::Failed);
    assert!(!source.notifications_unconfirmed());

    // One half of the read after a refused mark is told, the other fails for want of an answer: the read is
    // judged as a whole.
    let fake = super::fake("half-told");
    let mut source = Source::new(signed_in(&fake));
    opened(&mut source, Pane::Recent);
    fake.mark_mode.store(SERVER_ERROR, Ordering::SeqCst);
    fake.switches_mode.store(SESSION_ENDED, Ordering::SeqCst);
    fake.read_mode.store(SERVER_ERROR, Ordering::SeqCst);
    source.mark_recent_seen();
    assert_eq!(settle(&mut source), [NOT_MARKED]);
    assert!(
        titles(&source).is_empty(),
        "the list is not kept beside switches Beebeeb refused"
    );
    assert_eq!(source.recent_state(), ReadState::Failed);
    assert_eq!(source.alerts_state(0), ReadState::Failed);
    assert!(!source.notifications_unconfirmed());
}

/// What a person sees of the account: every title with its "New", the two states, and "may be out of date".
fn seen_of(source: &Source) -> (Vec<(String, bool)>, ReadState, ReadState, bool, Vec<Option<bool>>) {
    (
        titles(source),
        source.recent_state(),
        source.alerts_state(0),
        source.notifications_unconfirmed(),
        source.notifications().groups[0].1.iter().map(|a| a.on).collect(),
    )
}

/// Leaving Recent for Notifications with no connection: the mark and the read that opening starts are both out,
/// and both fail. Whichever answer lands first, what was held stays, one toast (the mark's), "may be out of date".
#[test]
fn a_refused_mark_keeps_what_was_held_whichever_answer_lands_first() {
    let mut ends = Vec::new();
    for read_first in [true, false] {
        let fake = fake(if read_first { "order-read" } else { "order-mark" });
        let mut source = Source::new(signed_in(&fake));
        opened(&mut source, Pane::Recent);
        let held = seen_of(&source);
        assert_eq!(held.0[0], (FIRST_SHARE.to_string(), true));
        source.alerts.expire();
        fake.mark_mode.store(SERVER_ERROR, Ordering::SeqCst);
        fake.read_mode.store(SERVER_ERROR, Ordering::SeqCst);
        let read = source.alerts.hold_next_read();
        source.opened(Pane::Notifications);
        assert!(source.alerts.reading.is_some(), "the opening's read is out");
        let mark = source.alerts.hold_next_write();
        source.mark_recent_seen();
        assert_eq!(source.alerts.marking, 1, "and so is the mark");

        let mut toasts = Vec::new();
        if read_first {
            read.send(()).expect("the read's worker is waiting");
            toasts.extend(pump_until(&mut source, |s| s.alerts.reading.is_none()));
            assert_eq!(source.alerts.marking, 1, "the mark is still out");
            assert!(
                toasts.is_empty(),
                "a read that fails beside a write on its way says nothing yet"
            );
            assert_eq!(
                (source.recent_state(), source.alerts_state(0)),
                (ReadState::Ready, ReadState::Ready),
                "and drops nothing"
            );
            mark.send(()).expect("the mark's worker is waiting");
        } else {
            mark.send(()).expect("the mark's worker is waiting");
            toasts.extend(pump_until(&mut source, |s| s.alerts.marking == 0));
            assert!(source.alerts.reading.is_some(), "the read is still out");
            read.send(()).expect("the read's worker is waiting");
        }
        toasts.extend(settle(&mut source));
        assert_eq!(toasts, [NOT_MARKED], "read first: {read_first}: one toast, the mark's");
        let now = seen_of(&source);
        assert_eq!(
            (&now.0, now.1, now.2, &now.4),
            (&held.0, held.1, held.2, &held.4),
            "read first: {read_first}: what was held stays, New back on what was not marked"
        );
        assert!(
            now.3,
            "read first: {read_first}: and is said to be possibly out of date"
        );
        ends.push((now, toasts, asked(&fake.fake, LIST)));
    }
    assert_eq!(ends[0], ends[1], "the same in either order");
}

/// A read that failed beside a write on its way is not judged; when the write is taken, what Beebeeb holds is read
/// again, and that read is the one drawn.
#[test]
fn a_read_that_failed_beside_a_write_is_read_again_when_the_write_answers() {
    let fake = fake("recheck");
    let mut source = Source::new(signed_in(&fake));
    opened(&mut source, Pane::Recent);
    source.alerts.expire();
    fake.read_mode.store(SERVER_ERROR, Ordering::SeqCst);
    let read = source.alerts.hold_next_read();
    source.opened(Pane::Notifications);
    let mark = source.alerts.hold_next_write();
    source.mark_recent_seen();
    assert_eq!(source.alerts.marking, 1);
    read.send(()).expect("the read's worker is waiting");
    assert!(pump_until(&mut source, |s| s.alerts.reading.is_none()).is_empty());
    assert_eq!(source.recent_state(), ReadState::Ready, "kept while the mark is out");
    assert!(!source.notifications_unconfirmed(), "nothing was judged yet");

    // The mark is taken; the read after it answers.
    fake.read_mode.store(ANSWER, Ordering::SeqCst);
    let lists = asked(&fake.fake, LIST);
    mark.send(()).expect("the mark's worker is waiting");
    assert!(settle(&mut source).is_empty(), "nothing failed in the end");
    assert_eq!(asked(&fake.fake, LIST), lists + 1, "read again, once");
    assert_eq!(
        titles(&source)[0],
        (FIRST_SHARE.to_string(), false),
        "marked, as Beebeeb says"
    );
    assert!(!source.notifications_unconfirmed());
    // Stamped fresh by the read that answered: opening again asks nothing.
    assert!(opened(&mut source, Pane::Notifications).is_empty());
    assert_eq!(asked(&fake.fake, LIST), lists + 1);

    // The same, and the read after the write fails too: now it is judged (nothing said it failed before).
    source.alerts.expire();
    fake.read_mode.store(SERVER_ERROR, Ordering::SeqCst);
    let read = source.alerts.hold_next_read();
    source.opened(Pane::Notifications);
    let on = switch(&source, 0, 1);
    let change = source.alerts.hold_next_write();
    source.set_alert(0, 1, !on);
    assert!(source.alerts.switching);
    read.send(()).expect("the read's worker is waiting");
    assert!(pump_until(&mut source, |s| s.alerts.reading.is_none()).is_empty());
    change.send(()).expect("the change's worker is waiting");
    assert_eq!(
        settle(&mut source),
        [ALERT_SAVED, NOT_LOADED],
        "saved, and then the read the person asked for failed"
    );
    assert_eq!(
        source.notifications().groups[0].1[1].on,
        Some(!on),
        "the switch Beebeeb just took is not dropped by that read"
    );
    assert_eq!(source.recent_state(), ReadState::Ready, "nor the list that was held");
    assert!(
        source.notifications_unconfirmed(),
        "it is said to be possibly out of date"
    );
}

/// A refused mark, with the opening's read still out: that read answers, and the read asked for after the mark
/// fails. What was said about the mark stands for the read after it, although one landed whole in between.
#[test]
fn a_read_that_lands_whole_before_the_read_after_a_refused_mark_does_not_unsay_it() {
    let fake = fake("order-whole");
    let mut source = Source::new(signed_in(&fake));
    opened(&mut source, Pane::Recent);
    let held = seen_of(&source);
    source.alerts.expire();
    fake.mark_mode.store(SERVER_ERROR, Ordering::SeqCst);
    let first = source.alerts.hold_next_read();
    source.opened(Pane::Notifications);
    let reads = source.alerts.reads;
    // The read after the mark is held too, before it calls the engine.
    let second = source.alerts.hold_next_read();
    source.mark_recent_seen();
    let mut toasts = pump_until(&mut source, |s| s.alerts.marking == 0);
    assert!(
        source.alerts.again,
        "a read was asked for after the mark, behind the one on its way"
    );
    first.send(()).expect("the first read's worker is waiting");
    toasts.extend(pump_until(&mut source, |s| s.alerts.reads == reads + 1));
    fake.read_mode.store(SERVER_ERROR, Ordering::SeqCst);
    second.send(()).expect("the second read's worker is waiting");
    toasts.extend(settle(&mut source));
    assert_eq!(toasts, [NOT_MARKED], "one toast, the mark's");
    let now = seen_of(&source);
    assert_eq!((&now.0, now.1, now.2), (&held.0, held.1, held.2), "what was held stays");
    assert!(now.3, "said to be possibly out of date");
}

/// A write of the session before, still on its way, does not hold up the verdict of the new session's read.
#[test]
fn a_write_of_the_session_before_does_not_defer_the_next_sessions_failed_read() {
    let fake = fake("old-write");
    let engine = signed_in(&fake);
    let mut source = Source::new(engine.clone());
    opened(&mut source, Pane::Notifications);
    let held_write = source.alerts.hold_next_write();
    source.set_alert(0, 1, true);
    assert!(source.alerts.switching);
    let _ = engine.lock();
    engine
        .unlock_with_phrase(Zeroizing::new(fake.first_phrase.clone()))
        .expect("the phrase unlocks");
    fake.read_mode.store(SERVER_ERROR, Ordering::SeqCst);
    let toasts = pump_until(&mut source, |s| s.alerts.reading.is_none() && s.alerts.reads >= 2);
    assert!(source.alerts.switching, "the old session's change is still out");
    assert_eq!(toasts, [NOT_LOADED], "the new session's failed read is said at once");
    assert_eq!(
        source.alerts_state(0),
        ReadState::Failed,
        "and drawn as failed, with its way to try again"
    );
    drop(held_write);
    settle(&mut source);
}

/// No connection, and a switch is pressed: its result is unknown, and the read that asks Beebeeb fails too. What
/// was on screen stays (the switches and the list), one toast, "may be out of date" with a way to try again.
#[test]
fn an_unknown_switch_result_and_its_failed_read_keep_what_is_drawn_and_say_it_once() {
    let fake = fake("switch-offline");
    let mut source = Source::new(signed_in(&fake));
    opened(&mut source, Pane::Notifications);
    let held = seen_of(&source);
    assert!(!held.0.is_empty() && held.4 == [Some(true), Some(false), Some(true)]);
    // The change reaches Beebeeb and its answer does not read; the read after it gets no answer.
    fake.put_mode.store(UNREADABLE, Ordering::SeqCst);
    fake.read_mode.store(SERVER_ERROR, Ordering::SeqCst);
    source.set_alert(0, 1, true);
    assert_eq!(settle(&mut source), [ALERT_UNKNOWN], "one toast: the change's");
    let now = seen_of(&source);
    assert_eq!(
        (&now.0, now.1, now.2, &now.4),
        (&held.0, held.1, held.2, &held.4),
        "the switches and the list stay as they were drawn"
    );
    assert!(now.3, "said to be possibly out of date");
    assert_eq!(places(&source), all_places());

    // "Try again" with still no answer: said (the person asked), and what is held still stays.
    assert_eq!(
        {
            source.reload_notifications();
            settle(&mut source)
        },
        [NOT_LOADED]
    );
    assert_eq!(seen_of(&source), now);
    // Opening the pane again asks again (never stamped fresh), and says so when it fails.
    let lists = asked(&fake.fake, LIST);
    assert_eq!(opened(&mut source, Pane::Notifications), [NOT_LOADED]);
    assert_eq!(asked(&fake.fake, LIST), lists + 1);
    assert_eq!(seen_of(&source), now);

    // "Try again" with an answer: what Beebeeb has (the change had landed), and nothing is unsure any more.
    fake.read_mode.store(ANSWER, Ordering::SeqCst);
    source.reload_notifications();
    assert!(settle(&mut source).is_empty());
    let after = seen_of(&source);
    assert_eq!(after.4, [Some(true), Some(true), Some(true)], "File updates is on");
    assert!(!after.3, "confirmed");

    // The next failed read follows no failed write: nothing stays, as for any read.
    fake.read_mode.store(SERVER_ERROR, Ordering::SeqCst);
    source.alerts.expire();
    assert_eq!(opened(&mut source, Pane::Notifications), [NOT_LOADED]);
    assert_eq!(source.alerts_state(0), ReadState::Failed);

    // Another account never sees "may be out of date" of the one before.
    let fake = super::fake("switch-offline-next");
    let engine = signed_in(&fake);
    let mut source = Source::new(engine.clone());
    opened(&mut source, Pane::Notifications);
    fake.put_mode.store(UNREADABLE, Ordering::SeqCst);
    fake.read_mode.store(SERVER_ERROR, Ordering::SeqCst);
    source.set_alert(0, 1, true);
    assert_eq!(settle(&mut source), [ALERT_UNKNOWN]);
    assert!(source.notifications_unconfirmed());
    fake.put_mode.store(ANSWER, Ordering::SeqCst);
    fake.read_mode.store(ANSWER, Ordering::SeqCst);
    engine.sign_out_keeping_local_data().expect("signed out");
    assert!(
        !source.notifications_unconfirmed(),
        "gone with the session, before any poll"
    );
    sign_in(&engine, &fake.second, &fake.second_phrase);
    assert!(!source.notifications_unconfirmed());
    opened(&mut source, Pane::Notifications);
    assert!(
        !source.notifications_unconfirmed(),
        "the next account's own read is not unsure"
    );
}

/// A change of the session before is taken by Beebeeb while the new session's read is out: that read's switches
/// may be older than the change, so they are not drawn, and with none held they are read again: the rows never
/// say "Couldn't load" for a read that did not fail.
#[test]
fn switches_skipped_as_older_than_a_change_are_read_again_not_called_failed() {
    let fake = fake("skipped");
    let engine = signed_in(&fake);
    let mut source = Source::new(engine.clone());
    opened(&mut source, Pane::Notifications);
    let (gate, held) = std::sync::mpsc::channel();
    *fake.put_gate.lock().unwrap() = Some(held);
    source.set_alert(0, 1, true);
    // The change is at Beebeeb, not yet stored.
    let began = Instant::now();
    while asked(&fake.fake, "PUT /api/v1/notifications/preferences") == 0 {
        assert!(
            began.elapsed() < Duration::from_secs(10),
            "the change did not reach the fake"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
    let _ = engine.lock();
    engine
        .unlock_with_phrase(Zeroizing::new(fake.first_phrase.clone()))
        .expect("the phrase unlocks");
    // The new session's read is asked for (the pane is showing) and held before it calls the engine.
    let read = source.alerts.hold_next_read();
    let mut toasts = pump_until(&mut source, |s| s.alerts.reading.is_some());
    assert_eq!(source.alerts_state(0), ReadState::Loading);
    // The change lands first, then the read that began before it.
    gate.send(()).expect("the fake is waiting");
    toasts.extend(pump_until(&mut source, |s| !s.alerts.switching));
    assert_eq!(puts(&fake), 1, "the change was stored");
    let switches = asked(&fake.fake, SWITCHES);
    read.send(()).expect("the read's worker is waiting");
    toasts.extend(pump_until(&mut source, |_| asked(&fake.fake, SWITCHES) > switches + 1));
    assert_ne!(
        source.alerts_state(0),
        ReadState::Failed,
        "no read failed: the rows do not say Couldn't load"
    );
    toasts.extend(settle(&mut source));
    assert_eq!(asked(&fake.fake, SWITCHES), switches + 2, "read again, once");
    assert_eq!(source.alerts_state(0), ReadState::Ready);
    assert_eq!(
        source.notifications().groups[0].1[1].on,
        Some(true),
        "what Beebeeb holds after the change"
    );
    assert!(!toasts.contains(&NOT_LOADED), "{toasts:?}");
}

/// The default window: the Notifications pane is taller than it, so its last group is below the fold.
const DEFAULT: (f32, f32) = (958.0, 638.0);

/// The rect of the element keyed `target`, which must be wholly inside the default window.
fn in_view(rt: &Runtime<State>, target: &str) -> [f32; 4] {
    let r = find(rt, target)
        .unwrap_or_else(|| panic!("no element keyed {target}"))
        .rect;
    assert!(
        r[0] >= 0.0 && r[1] >= 0.0 && r[0] + r[2] <= DEFAULT.0 && r[1] + r[3] <= DEFAULT.1,
        "{target} is outside the window: {r:?}"
    );
    r
}

/// A press in the middle of the element keyed `target`, which must be inside the window.
fn press_in_view(rt: &mut Runtime<State>, target: &str) {
    let r = in_view(rt, target);
    click_at(rt, (r[0] + r[2] / 2.0, r[1] + r[3] / 2.0));
}

/// Poll settings as the window does (and only so: no toast is taken past the window) until `done`, then draw.
fn poll_until(rt: &mut Runtime<State>, what: &str, done: impl Fn(&Runtime<State>) -> bool) {
    let began = Instant::now();
    loop {
        rt.update(|s, tx| {
            crate::frames::settings::poll(s, tx);
        });
        draw(rt);
        if done(rt) {
            return;
        }
        assert!(began.elapsed() < Duration::from_secs(10), "{what}: {}", words(rt));
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn text_of(rt: &Runtime<State>, key: &str) -> Option<String> {
    find(rt, key).and_then(|e| e.text)
}

/// The Notifications pane in the DEFAULT window, where its last group is below the fold: what the rows cannot
/// say is said over the pane with "Try again" (a failed read; switches that stay drawn but could not be
/// confirmed), in view without scrolling, and no row moves when it comes or goes. What a press did is said there
/// too, above a notice that stands, and the rows under them still take a press.
#[test]
fn the_pane_says_it_in_view_with_try_again_and_no_row_moves() {
    use crate::frames::settings::notifications::{NOT_READ, UNCONFIRMED};
    let fake = fake("pane-retry");
    fake.read_mode.store(SERVER_ERROR, Ordering::SeqCst);
    let engine = signed_in(&fake);
    let mut rt = window(&engine);
    rt.update(|s, _| crate::frames::settings::go(s, "DW-0750"));
    poll_until(&mut rt, "the read never failed", |rt| find(rt, "notice-line").is_some());
    let seen = words(&rt);
    assert!(seen.contains("Couldn't load"), "the rows say it: {seen}");
    assert_eq!(text_of(&rt, "notice-text").as_deref(), Some(NOT_READ), "{seen}");
    assert!(find(&rt, "item-3-0-switch").is_none(), "no switch without a value");
    let rows = row_rects(&rt);
    let last = rows.last().unwrap().1;
    assert!(
        last[1] + last[3] > DEFAULT.1,
        "precondition: the last row is below the fold: {last:?}"
    );
    in_view(&rt, "notice");
    assert!(
        find(&rt, "toast-room").is_some(),
        "room under the last row to scroll it clear of the notice"
    );

    // "Try again", pressed where it is drawn: one read, and its answer is drawn where the rows already were.
    fake.read_mode.store(ANSWER, Ordering::SeqCst);
    let lists = asked(&fake.fake, LIST);
    press_in_view(&mut rt, "notice-action");
    poll_until(&mut rt, "the read never answered", |rt| {
        find(rt, "item-3-0-switch").is_some()
    });
    assert_eq!(asked(&fake.fake, LIST), lists + 1, "one read for the one press");
    assert!(find(&rt, "notice-line").is_none(), "nothing left to say");
    assert!(find(&rt, "toast-room").is_none());
    assert_eq!(row_rects(&rt), rows, "no row moved when the notice went");

    // A switch Beebeeb takes: said where the toast sits.
    press_in_view(&mut rt, "item-3-1-switch");
    poll_until(&mut rt, "the change was never said", |rt| find(rt, "toast-0").is_some());
    assert_eq!(text_of(&rt, "toast-0-text").as_deref(), Some(ALERT_SAVED));
    assert_eq!(find(&rt, "toast-0").map(|e| e.role), Some("status".into()));
    assert_eq!(puts(&fake), 1, "the press was sent");
    in_view(&rt, "toast-0");
    let on = rt.state.settings.source.notifications().groups[0].1[1].on;

    // A switch whose result is unknown, and the read after it fails: the switches stay, said to be unsure, with
    // the way to ask again in view; what the press itself came to is said above it.
    fake.put_mode.store(UNREADABLE, Ordering::SeqCst);
    fake.read_mode.store(SERVER_ERROR, Ordering::SeqCst);
    press_in_view(&mut rt, "item-3-1-switch");
    poll_until(&mut rt, "never said to be unsure", |rt| {
        find(rt, "notice-line").is_some()
    });
    assert_eq!(puts(&fake), 2, "the press was sent");
    let seen = words(&rt);
    assert_eq!(text_of(&rt, "notice-text").as_deref(), Some(UNCONFIRMED), "{seen}");
    assert!(!seen.contains("Couldn't load"), "nothing was dropped: {seen}");
    assert!(find(&rt, "item-3-1-switch").is_some(), "the switches stay drawn");
    assert_eq!(row_rects(&rt), rows, "no row moved when the notice came");
    assert!(
        find(&rt, "toast-0").is_none(),
        "\"checking\" does not stay beside the check's failure: {seen}"
    );
    in_view(&rt, "notice");
    // A press Beebeeb refuses while that stands: said above the notice, both in view.
    fake.put_mode.store(SERVER_ERROR, Ordering::SeqCst);
    press_in_view(&mut rt, "item-3-1-switch");
    poll_until(&mut rt, "the refusal was never said", |rt| {
        find(rt, "toast-0").is_some()
    });
    assert_eq!(text_of(&rt, "toast-0-text").as_deref(), Some(ALERT_NOT_SAVED));
    assert_eq!(
        find(&rt, "toast-0").map(|e| e.role),
        Some("alert".into()),
        "a refusal is drawn as one: never with the mark of a thing that worked"
    );
    let (notice, toast) = (in_view(&rt, "notice"), in_view(&rt, "toast-0"));
    assert!(
        toast[1] + toast[3] <= notice[1],
        "above the notice, not over it: {toast:?} {notice:?}"
    );
    in_view(&rt, "notice-action");
    // With both up, a row still takes a press (this PC's switch at the top of the pane).
    let allow = rt.state.settings.source.notifications().allow;
    press_in_view(&mut rt, "item-0-0-switch");
    assert_eq!(rt.state.settings.source.notifications().allow, !allow);
    // Recent says the same over its list, which stays.
    rt.update(|s, _| crate::frames::settings::go(s, "DW-0791"));
    draw(&mut rt);
    let seen = words(&rt);
    assert!(seen.contains(UNCONFIRMED) && seen.contains(FIRST_SHARE), "{seen}");
    in_view(&rt, "notice-action");
    rt.update(|s, _| crate::frames::settings::go(s, "DW-0750"));
    draw(&mut rt);

    // "Try again" with an answer: what Beebeeb has, and the notice goes.
    fake.read_mode.store(ANSWER, Ordering::SeqCst);
    press_in_view(&mut rt, "notice-action");
    poll_until(&mut rt, "never confirmed", |rt| find(rt, "notice-line").is_none());
    assert!(
        find(&rt, "toast-0").is_none(),
        "the refusal's toast went with the press"
    );
    assert_eq!(
        rt.state.settings.source.notifications().groups[0].1[1].on,
        on.map(|on| !on),
        "the change had landed"
    );
    assert_eq!(row_rects(&rt), rows);
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
    assert!(source.alerts.switching, "a change is on its way before the sign-out");
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
    // Without this the block proves nothing: a press that started no change also sends nothing.
    assert!(source.alerts.switching, "a change is on its way before the lock");
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
    assert_eq!(source.alerts.marking, 1, "a mark is on its way before the sign-out");
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
