//! The session gate: the ONE answer to "may the window read or act on the account's files now?".
//!
//! It is closed in three cases, and every one refuses the same things:
//!
//! * the gate is not Ready (signed out, locked, no key, no plan): the route machine shows the
//!   gate's own screen;
//! * the engine's session is no longer the one the window was drawn for (another account, a lock
//!   or a sign-out the next poll has not seen yet);
//! * the session ended (`State::session_ended`): the gate stays Ready and the window keeps what
//!   it shows, but nothing is read or changed until the person signs in again.
//!
//! Who asks:
//!
//! * [`host::dispatch`](super::host::dispatch) and the sidebar's route event in the poll, before
//!   the route machine sees an event that leads to account data ([`refuses_event`]);
//! * the Files line of the registry (`files_frame`), before Search gets a key or draws;
//! * the poll, which closes what was open in the window when the gate closes under it
//!   (`frames::files::reset::session_closed`);
//! * each window's own guard, as a second line (`State::refused_without_session`, Search's
//!   open and run, the viewer's open).
//!
//! A refusal is counted in `State::actions.session.asked`, whoever asked.

use super::app::{Dialog, Event, GateState, Intent, Route, Section};
use crate::State;

/// Is the session open for the account the window shows? Read from the engine as it is NOW, not
/// from what the last poll saw: input can arrive between the engine's change and the next poll.
pub fn open(s: &State) -> bool {
    if s.shell.app.gate() != GateState::Ready || s.session_ended() {
        return false;
    }
    let session = s.shell.actions.session();
    session.gate == GateState::Ready && session.account.as_deref() == s.shell.app.account()
}

/// Something that needs the session was asked for. `true`: it is refused, and counted.
pub fn refuse(s: &mut State) -> bool {
    if open(s) {
        return false;
    }
    s.actions.session.asked += 1;
    true
}

/// A place that shows nothing of the account's files that the window does not already hold: the
/// Files window as it is (it draws its own session-ended state and reads nothing), and the two
/// views of what is on this PC.
fn shows_without_session(route: &Route) -> bool {
    !route.names_account_data()
        && matches!(
            route.section,
            Section::Files | Section::SyncHistory | Section::AvailableOffline
        )
}

fn needs_session(s: &State, event: &Event) -> bool {
    let app = &s.shell.app;
    match event {
        // The session's own events, Esc, Settings: never refused here.
        Event::Gate(_) | Event::Session(_) | Event::Escape | Event::OpenSettings(_) => false,
        Event::OpenItem(_) => true,
        Event::Navigate(route) => !shows_without_session(route),
        // Back out of Settings shows the place the window was on. Any other step goes to a
        // place of the history, which is held to the same rule as a click.
        Event::Back => !app.in_settings() && !app.back_target().is_some_and(shows_without_session),
        Event::Forward => !app.forward_target().is_some_and(shows_without_session),
        Event::OpenDialog(Dialog::Conflict { .. }) => true,
        Event::OpenDialog(Dialog::Shortcuts | Dialog::SyncOff) => false,
        // From outside the window (the b's menu, a link). While the gate is not Ready the route
        // machine keeps or drops it by its own rule; with the gate Ready a request that names a
        // file or a folder is not followed.
        Event::Request(intent) => {
            app.gate() == GateState::Ready
                && match intent {
                    Intent::Go(route) => !shows_without_session(route),
                    Intent::Settings(_) => false,
                    Intent::ReviewConflict { .. } => true,
                }
        }
    }
}

/// Before the route machine sees `event`: is it refused because the session is not open? `true`:
/// the event is dropped (and counted); no window and no source hears of it.
pub fn refuses_event(s: &mut State, event: &Event) -> bool {
    needs_session(s, event) && refuse(s)
}
