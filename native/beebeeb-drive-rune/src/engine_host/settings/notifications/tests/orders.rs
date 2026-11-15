//! A read and a write of the account's notifications answered in either order: what a person sees in the end is
//! the same. Every order is made with the part's own barriers (a worker waits before it calls the engine), never
//! with a sleep.

use super::*;

/// What a person sees of the account: every title with its "New", the two states, "may be out of date", and the
/// account's switches.
fn seen_of(source: &Source) -> (Vec<(String, bool)>, ReadState, ReadState, bool, Vec<Option<bool>>) {
    (
        titles(source),
        source.recent_state(),
        source.alerts_state(0),
        source.notifications_unconfirmed(),
        source.notifications().groups[0].1.iter().map(|a| a.on).collect(),
    )
}

/// A switch Beebeeb takes, a mark Beebeeb takes, a switch Beebeeb refuses: each beside a read that began before
/// it and fails. The write's answer first or the read's first: the same toasts, the same reads, and what was held
/// stays drawn (with the switch as Beebeeb answered it), said to be possibly out of date. A failed read that
/// began before a write was answered is never the verdict on what Beebeeb holds after it.
#[test]
fn a_failed_read_beside_a_write_ends_the_same_whichever_answer_lands_first() {
    const SWITCH_TAKEN: u8 = 0;
    const MARK_TAKEN: u8 = 1;
    const SWITCH_REFUSED: u8 = 2;
    for kind in [SWITCH_TAKEN, MARK_TAKEN, SWITCH_REFUSED] {
        let mut ends = Vec::new();
        for read_first in [true, false] {
            let fake = fake(&format!("orders-{kind}-{read_first}"));
            let mut source = Source::new(signed_in(&fake));
            opened(&mut source, Pane::Recent);
            let on = switch(&source, 0, 1);
            source.alerts.expire();
            fake.read_mode.store(SERVER_ERROR, Ordering::SeqCst);
            if kind == SWITCH_REFUSED {
                fake.put_mode.store(SERVER_ERROR, Ordering::SeqCst);
            }
            let read = source.alerts.hold_next_read();
            source.opened(Pane::Notifications);
            let write = source.alerts.hold_next_write();
            if kind == MARK_TAKEN {
                source.mark_recent_seen();
                assert_eq!(source.alerts.marking, 1, "precondition: a mark is out");
            } else {
                source.set_alert(0, 1, !on);
                assert!(source.alerts.switching, "precondition: a change is out");
            }
            let mut toasts = Vec::new();
            if read_first {
                read.send(()).expect("the read's worker is waiting");
                toasts.extend(pump_until(&mut source, |s| s.alerts.reading.is_none()));
                write.send(()).expect("the write's worker is waiting");
            } else {
                write.send(()).expect("the write's worker is waiting");
                toasts.extend(pump_until(&mut source, |s| {
                    s.alerts.marking == 0 && !s.alerts.switching
                }));
                assert!(
                    source.alerts.reading.is_some(),
                    "precondition: the read from before is out"
                );
                read.send(()).expect("the read's worker is waiting");
            }
            toasts.extend(settle(&mut source));
            let now = seen_of(&source);
            assert_eq!(
                (now.1, now.2, now.3),
                (ReadState::Ready, ReadState::Ready, true),
                "kind {kind}, read first {read_first}: kept, and said to be possibly out of date"
            );
            let taken = kind == SWITCH_TAKEN;
            assert_eq!(
                now.4[1],
                Some(on != taken),
                "kind {kind}, read first {read_first}: the switch as Beebeeb answered it"
            );
            ends.push((now, toasts, asked(&fake.fake, LIST)));
        }
        assert_eq!(ends[0], ends[1], "kind {kind}: the same in either order");
    }
}

/// A refused mark arms "unconfirmed"; the read after it is out. The person opens the pane again (a read is asked
/// for behind it). The read after the mark lands whole: it began after the mark was answered, so it HAS confirmed
/// what Beebeeb holds, whoever asked for another read since. The next read fails: judged as any failed read
/// (nothing of the account stays drawn), not kept as "may be out of date".
#[test]
fn a_whole_read_from_after_the_write_takes_unconfirmed_back_whoever_asked_since() {
    let fake = fake("orders-confirmed");
    let mut source = Source::new(signed_in(&fake));
    opened(&mut source, Pane::Recent);
    fake.mark_mode.store(SERVER_ERROR, Ordering::SeqCst);
    let after_mark = source.alerts.hold_next_read();
    source.mark_recent_seen();
    let mut toasts = pump_until(&mut source, |s| s.alerts.marking == 0);
    assert!(
        source.alerts.unconfirmed.is_some() && source.alerts.reading.is_some(),
        "precondition: said, and the read after the mark is out"
    );
    let reads = source.alerts.reads;
    source.alerts.expire();
    let asked_since = source.alerts.hold_next_read();
    source.opened(Pane::Notifications);
    assert!(source.alerts.again, "precondition: the person's read waits behind it");
    after_mark.send(()).expect("the read's worker is waiting");
    toasts.extend(pump_until(&mut source, |s| s.alerts.reads == reads + 1));
    assert!(
        source.alerts.unconfirmed.is_none(),
        "confirmed by the read that began after the mark"
    );
    assert!(!source.notifications_unconfirmed());
    fake.read_mode.store(SERVER_ERROR, Ordering::SeqCst);
    asked_since.send(()).expect("the read's worker is waiting");
    toasts.extend(settle(&mut source));
    assert_eq!(toasts, [NOT_MARKED, NOT_LOADED]);
    assert_eq!(source.recent_state(), ReadState::Failed, "a failed read, judged as one");
    assert_eq!(source.alerts_state(0), ReadState::Failed);
    assert!(!source.notifications_unconfirmed());
}

/// A switch change is told the session ended (401). The change was not made, and it is said; what is drawn of the
/// account is asked for again at once, and that read is judged: told the same, nothing of the account stays
/// drawn; answered, what Beebeeb holds is drawn.
#[test]
fn a_switch_told_the_session_ended_reads_again_and_keeps_nothing_of_an_ended_session() {
    for ended in [true, false] {
        let fake = fake(&format!("orders-ended-{ended}"));
        let mut source = Source::new(signed_in(&fake));
        opened(&mut source, Pane::Notifications);
        let on = switch(&source, 0, 1);
        let reads = asked(&fake.fake, SWITCHES);
        fake.put_mode.store(SESSION_ENDED, Ordering::SeqCst);
        if ended {
            fake.read_mode.store(SESSION_ENDED, Ordering::SeqCst);
        }
        source.set_alert(0, 1, !on);
        let toasts = settle(&mut source);
        assert_eq!(toasts, [ALERT_NOT_SAVED], "ended {ended}: the change was not made");
        assert_eq!(
            asked(&fake.fake, SWITCHES),
            reads + 1,
            "ended {ended}: read again at once, once"
        );
        if ended {
            assert_eq!(source.alerts_state(0), ReadState::Failed, "nothing stays drawn");
            assert_eq!(source.recent_state(), ReadState::Failed);
            assert!(source.notifications().groups[0].1.iter().all(|a| a.on.is_none()));
        } else {
            assert_eq!(source.alerts_state(0), ReadState::Ready);
            assert_eq!(switch(&source, 0, 1), on, "as Beebeeb holds it");
        }
        assert!(!source.notifications_unconfirmed(), "ended {ended}");
    }
}
