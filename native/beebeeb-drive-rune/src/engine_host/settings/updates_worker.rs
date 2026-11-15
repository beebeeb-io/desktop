//! About's updates: the worker that owns the updater (see the parent module's doc). The launch check, each order of
//! the window, a background check whenever one is due, and the press's one order ([`one_press`]).

use super::*;

/// The worker: the launch check, then each order, and a background check whenever one is due. Ends when the
/// window's end is dropped.
pub(super) fn work(
    make: MakeDrive,
    sync_back: SyncBack,
    presence: Arc<Presence>,
    channel: beebeeb_updater::Channel,
    orders: mpsc::Receiver<Order>,
    reports: mpsc::Sender<Report>,
    tuning: Tuning,
) {
    let mut updater = make(sync_back);
    let mut channel = channel;
    let to = Reports {
        reports: &reports,
        wake: &*tuning.wake,
    };
    let background = |channel| Event::Check {
        channel,
        intent: Intent::Background,
    };
    let Some(mut state) = run(&mut *updater, 0, background(channel), true, &to) else {
        return;
    };
    loop {
        let reached = match orders.recv_timeout(tuning.look_every) {
            Ok(Order {
                serial,
                work: Work::Event(event),
            }) => {
                if let Event::Check { channel: asked, .. } = &event {
                    channel = *asked;
                }
                run(&mut *updater, serial, event, true, &to)
            }
            // Only the press's order: a background check is the arm below, and it never downloads nor hands off.
            Ok(Order {
                serial,
                work: Work::Update(version),
            }) => one_press(&mut *updater, serial, version, state, channel, &presence, &to),
            Err(mpsc::RecvTimeoutError::Timeout) if updater.background_check_due() => {
                run(&mut *updater, 0, background(channel), true, &to)
            }
            Err(mpsc::RecvTimeoutError::Timeout) => continue,
            Err(mpsc::RecvTimeoutError::Disconnected) => None,
        };
        match reached {
            Some(reached) => state = reached,
            None => return,
        }
    }
}

/// Where the worker reports, and how it wakes the window after each report.
struct Reports<'a> {
    reports: &'a mpsc::Sender<Report>,
    wake: &'a dyn Fn(),
}

impl Reports<'_> {
    /// `false`: the window's end is gone.
    fn send(&self, updater: &dyn Drive, serial: u64, state: &State, done: bool) -> bool {
        let sent = self
            .reports
            .send(Report {
                serial,
                state: state.clone(),
                freshness: updater.freshness(),
                verified: updater.last_verified(),
                at: Local::now(),
                done,
            })
            .is_ok();
        if sent {
            (self.wake)();
        }
        sent
    }
}

/// One event, then `Tick`s until the machine has no work left; the window is woken after each report. The state it
/// settled in (its last report is `done` when `ends` says this event ends its order); `None`: the window's end is
/// gone.
fn run(updater: &mut dyn Drive, serial: u64, event: Event, ends: bool, to: &Reports) -> Option<State> {
    let mut state = updater.step(event);
    if !to.send(updater, serial, &state, ends && !busy(&state)) {
        return None;
    }
    let mut said = Instant::now();
    while busy(&state) {
        state = updater.step(Event::Tick);
        if !busy(&state) || said.elapsed() >= PROGRESS_EVERY {
            if !to.send(updater, serial, &state, ends && !busy(&state)) {
                return None;
            }
            said = Instant::now();
        }
    }
    Some(state)
}

/// `state` offers `version` itself for a download (on offer, or a download that can be resumed): never an older
/// version than this build's.
fn to_download(state: &State, version: Version) -> bool {
    matches!(
        state,
        State::Available { offer } | State::Failed { resume: Some(offer), .. }
            if offer.version == version && !offer.downgrade
    )
}

/// `state` holds the verified file of `version` itself.
fn verified(state: &State, version: Version) -> bool {
    matches!(state, State::ReadyToRestart { offer, .. } if offer.version == version && !offer.downgrade)
}

/// "Restart to update" for `version`, as ONE order (owner ruling 2026-11-15): when the offer is not in front of the
/// machine (the file was not the signed one and the machine dropped it), a check the person asked for first; then the
/// download and its signature check; then, only on the `ReadyToRestart` this order's own download reached, for
/// `version`, the hand-off (`Event::Apply`: verified again, sync stops, the installer starts). Each leg runs only on
/// what the leg before reached; a leg that did not reach it ends the order there, with nothing installed. The
/// verified file is reported before the hand-off runs (its stop can take seconds), and the last report is `done`.
///
/// The hand-off runs only for the person who pressed (review of #444): when Beebeeb was locked, or the session
/// changed (a sign-out, another sign-in) since the press, nothing stops and nothing starts, the verified file stays
/// (`ReadyToRestart` at rest: About says it waits for the person), and the press is offered again once the lock is
/// open. The hand-off asks the same again right before the installer starts (`EngineApply::launch`).
fn one_press(
    updater: &mut dyn Drive,
    serial: u64,
    version: Version,
    mut state: State,
    channel: beebeeb_updater::Channel,
    presence: &Presence,
    to: &Reports,
) -> Option<State> {
    if !to_download(&state, version) {
        let check = Event::Check {
            channel,
            intent: Intent::Manual,
        };
        state = run(updater, serial, check, false, to)?;
    }
    if to_download(&state, version) {
        state = run(updater, serial, Event::Download, false, to)?;
        if verified(&state, version) {
            if presence.holds() {
                state = run(updater, serial, Event::Apply, false, to)?;
            } else {
                log("settings: Beebeeb was locked or the session changed since the press; the update waits");
            }
        }
    }
    to.send(updater, serial, &state, true).then_some(state)
}
