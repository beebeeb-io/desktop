//! What the account panes say over themselves: the toasts' words, what a toast is about, and the line they wait
//! in (`EngineAccountSource`, the parent module, puts them there and takes them away).
//!
//! Must not: hold or print a name or an email. A session's id is held (to send a sign-out again), never printed:
//! no type here has `Debug`.

use std::collections::VecDeque;
use std::time::{Duration, Instant};

use beebeeb_engine::account::SessionGeneration;

use super::Row;

/// What the toast says after a sign-out of another device.
pub(super) const SIGNED_OUT: &str = "Signed out. It ends the next time that device contacts Beebeeb.";
pub(super) const SIGNED_OUT_ELSEWHERE: &str = "Signed out everywhere else.";
/// The list Beebeeb has no longer holds the session (it answered 404, or the list read after an answer that was
/// not a yes no longer has it).
pub(super) const IS_SIGNED_OUT: &str = "That session is signed out.";
/// Beebeeb answered with an error (a 5xx, or a refusal): whether it was taken is not claimed; the list is read
/// again, and the toast offers to send it again ([`TRY_AGAIN`]).
pub(super) const NOT_SIGNED_OUT: &str = "Couldn't sign out that session.";
pub(super) const NOT_SIGNED_OUT_ELSEWHERE: &str = "Couldn't sign out your other sessions.";
/// Refused before anything was sent: the list is not current, the row is gone, or the session was busy.
pub(super) const NOT_SENT: &str = "Couldn't sign out. Nothing was sent.";
/// A timeout or a broken connection: the sign-out may have landed.
pub(super) const UNKNOWN_SIGN_OUT: &str = "Couldn't confirm the sign-out. Checking what Beebeeb has.";
pub(super) const STILL_SIGNING_OUT: &str = "Still signing out. Try again in a moment.";
/// The check itself failed (the list could not be read after a sign-out whose result is not known).
pub(super) const UNCONFIRMED_SIGN_OUT: &str = "Couldn't confirm the sign-out.";
/// "Everywhere else", sent again for the sessions that were confirmed: some ended, some did not.
pub(super) const NOT_ALL_SIGNED_OUT: &str = "Couldn't sign out all of your other sessions.";
/// The sessions that were confirmed are gone, and the list has others (made since): never "everywhere else".
pub(super) const SIGNED_OUT_THOSE: &str = "Those sessions are signed out.";
/// A read failed, and what the pane draws is what Beebeeb answered before.
pub(super) const NOT_REFRESHED: &str = "Couldn't refresh. This is what Beebeeb answered before.";
/// A part of Account (the name, the plan) was never read.
pub(super) const NOT_ALL_READ: &str = "Couldn't read everything. Some of this is missing.";
/// The toast's button after a sign-out that was not taken, or a read that failed.
pub(super) const TRY_AGAIN: &str = "Try again";

/// How long a toast stays once it shows, as the Files toasts do: something that worked, something that did not
/// (with or without Try again).
pub(super) const STAYS_DONE: Duration = Duration::from_secs(3);
pub(super) const STAYS_FAILED: Duration = Duration::from_secs(8);

/// The key of "every session but this one" among the changes on their way (no session's id is this).
pub(super) const ELSEWHERE: &str = "*";

/// The sign-out a toast is about. No `Debug`: it holds sessions' ids.
#[derive(Clone, PartialEq)]
pub(super) struct About {
    /// A session's id, or [`ELSEWHERE`].
    pub(super) key: String,
    /// The sessions it is about: the one, or for [`ELSEWHERE`] the sessions that were listed when the person
    /// confirmed. A FIXED set: sending again never reaches a session that is not in it.
    pub(super) ids: Vec<String>,
}

impl About {
    pub(super) fn elsewhere(&self) -> bool {
        self.key == ELSEWHERE
    }

    /// Whether `list` still has one of the sessions this is about.
    pub(super) fn listed_in(&self, list: &[Row]) -> bool {
        list.iter().any(|row| !row.current && self.ids.contains(&row.id))
    }

    pub(super) fn not_signed_out(&self) -> &'static str {
        if self.elsewhere() {
            NOT_SIGNED_OUT_ELSEWHERE
        } else {
            NOT_SIGNED_OUT
        }
    }
}

/// What a toast says. It is kept with the session it was said in: a toast is never drawn in another session.
pub(super) struct Said {
    pub(super) words: &'static str,
    /// The sign-out the words are about while the list can still correct them: the next list read after the
    /// answer decides (`EngineAccountSource::words_follow`).
    pub(super) about: Option<About>,
    /// The toast's button sends `about` again.
    pub(super) again: bool,
    /// Something was not done: drawn as a failure, never with the mark of a thing that worked.
    pub(super) failed: bool,
    /// When it first showed (the poll stamps it): it stays for its time from then, never from when it was said,
    /// so one that waited behind another is not gone before it was seen.
    pub(super) shown_at: Option<Instant>,
}

impl Said {
    pub(super) fn done(words: &'static str) -> Self {
        Self {
            words,
            about: None,
            again: false,
            failed: false,
            shown_at: None,
        }
    }

    pub(super) fn failed(words: &'static str, about: Option<About>, again: bool) -> Self {
        Self {
            words,
            about,
            again,
            failed: true,
            shown_at: None,
        }
    }

    pub(super) fn stays(&self) -> Duration {
        if self.failed { STAYS_FAILED } else { STAYS_DONE }
    }
}

/// The toasts that wait, each with the session it was said in. The first one shows.
pub(super) type Line = VecDeque<(SessionGeneration, Said)>;

/// Put `said` in `line`. One toast per sign-out (a newer word about the same one replaces the older), and one
/// per sentence for those about none. Something that was not done goes before anything that worked and is never
/// replaced by it; something that worked replaces only what worked before it.
pub(super) fn line_up(line: &mut Line, session: SessionGeneration, said: Said) {
    line.retain(|(_, old)| match (&old.about, &said.about) {
        (Some(old), Some(new)) => old.key != new.key,
        (None, None) => old.words != said.words,
        _ => true,
    });
    if said.failed {
        let at = line.iter().position(|(_, old)| !old.failed).unwrap_or(line.len());
        line.insert(at, (session, said));
    } else {
        line.retain(|(_, old)| old.failed);
        line.push_back((session, said));
    }
}
