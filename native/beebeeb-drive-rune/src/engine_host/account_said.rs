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
/// The same two sentences for a toast that takes the place of one about ANOTHER session that had the same
/// button: the person can tell that Try again is now about a different session ([`Said::shown`]). Two forms, so
/// a third in a row reads differently from the second. Neither says how the one before ended.
pub(super) const NOT_SIGNED_OUT_ANOTHER: &str = "Couldn't sign out another session.";
pub(super) const NOT_SIGNED_OUT_ONE_MORE: &str = "Couldn't sign out one more session.";
pub(super) const NOT_SENT_ANOTHER: &str = "Couldn't sign out another session. Nothing was sent.";
pub(super) const NOT_SENT_ONE_MORE: &str = "Couldn't sign out one more session. Nothing was sent.";
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

/// Where the sign-out a toast is about stands. What the line does with a toast (hold it, let the list correct it,
/// keep it when the pane is left) is read from this, never from its sentence.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Stage {
    /// Said, and nothing more is expected (the list may still correct what it is about).
    Final,
    /// The result is not known and the list is being read to find out: it stays until that read answered.
    Checking,
    /// The result is not known and the check failed too.
    Unconfirmed,
    /// A second press while the first is on its way: over with that one's answer.
    Busy,
    /// What the list answered to a check: said on return to the pane too, as the check was.
    Answered,
}

/// What a toast says. It is kept with the session it was said in: a toast is never drawn in another session.
pub(super) struct Said {
    pub(super) words: &'static str,
    pub(super) stage: Stage,
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
    /// 0, or which "another session" form it is drawn in (`fade` decides when it first shows).
    pub(super) turn: u8,
}

impl Said {
    pub(super) fn done(words: &'static str) -> Self {
        Self {
            words,
            stage: Stage::Final,
            about: None,
            again: false,
            failed: false,
            shown_at: None,
            turn: 0,
        }
    }

    pub(super) fn failed(words: &'static str, about: Option<About>, again: bool) -> Self {
        Self {
            words,
            stage: Stage::Final,
            about,
            again,
            failed: true,
            shown_at: None,
            turn: 0,
        }
    }

    /// A timeout or a broken connection: the sign-out may have landed, and the list is read to find out.
    pub(super) fn checking(about: About) -> Self {
        Self {
            stage: Stage::Checking,
            ..Self::failed(UNKNOWN_SIGN_OUT, Some(about), false)
        }
    }

    /// The check itself failed: a toast of its own, with its own time.
    pub(super) fn unconfirmed(about: Option<About>) -> Self {
        Self {
            stage: Stage::Unconfirmed,
            ..Self::failed(UNCONFIRMED_SIGN_OUT, about, false)
        }
    }

    pub(super) fn busy() -> Self {
        Self {
            stage: Stage::Busy,
            ..Self::failed(STILL_SIGNING_OUT, None, false)
        }
    }

    /// The sentence that is drawn.
    pub(super) fn shown(&self) -> &'static str {
        match (self.words, self.turn) {
            (NOT_SIGNED_OUT, 1) => NOT_SIGNED_OUT_ANOTHER,
            (NOT_SIGNED_OUT, 2) => NOT_SIGNED_OUT_ONE_MORE,
            (NOT_SENT, 1) => NOT_SENT_ANOTHER,
            (NOT_SENT, 2) => NOT_SENT_ONE_MORE,
            (words, _) => words,
        }
    }

    /// Its button sends the sign-out of ONE session again.
    fn sends_one_again(&self) -> bool {
        self.again && self.about.as_ref().is_some_and(|about| !about.elsewhere())
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

/// A list read after the last answer landed: every toast about a sign-out follows what the list says, the ones
/// that wait too (`lines`). None of its sessions is listed any more: they are signed out, whatever the answer
/// was, and there is nothing to send again. One is still listed although the result was not known: it was not
/// signed out, and the toast offers to send it again. Each toast goes through the line again, so its rules hold
/// for the new words too (what was not done first, the same sentence once).
pub(super) fn follow(lines: [&mut Line; 2], turn: SessionGeneration, list: &[Row]) {
    let others = list.iter().any(|row| !row.current);
    let follow = |said: Said| {
        let Some(about) = said.about.clone() else {
            return said;
        };
        if !about.listed_in(list) {
            let words = match (about.elsewhere(), others) {
                (false, _) => IS_SIGNED_OUT,
                // The list has sessions that were not confirmed (made since): not "everywhere else".
                (true, true) => SIGNED_OUT_THOSE,
                (true, false) => SIGNED_OUT_ELSEWHERE,
            };
            let stage = match said.stage {
                Stage::Checking | Stage::Unconfirmed => Stage::Answered,
                _ => Stage::Final,
            };
            Said {
                stage,
                ..Said::done(words)
            }
        } else if matches!(said.stage, Stage::Checking | Stage::Unconfirmed) {
            Said::failed(about.not_signed_out(), Some(about), true)
        } else {
            said
        }
    };
    for line in lines {
        for (session, said) in std::mem::take(line) {
            if session == turn {
                line_up(line, session, follow(said));
            }
        }
    }
}

/// The check after a sign-out whose result is not known has itself failed: no longer "checking". It is a new
/// toast, put in the line again: with its own time from when it shows, so the failure is drawn (the old one's
/// time may be long over: it was held for the check).
pub(super) fn check_failed(lines: [&mut Line; 2]) {
    for line in lines {
        for (session, said) in std::mem::take(line) {
            let said = match said.stage {
                Stage::Checking => Said::unconfirmed(said.about),
                _ => said,
            };
            line_up(line, session, said);
        }
    }
}

/// The toast with a button for ONE session's sign-out that was drawn last: that session, and the form of its
/// sentence. A type of its own, with no `Debug`: it holds a session's id.
#[derive(Default)]
pub(super) struct Drawn(Option<(String, u8)>);

/// The poll's part of the line: the first toast shows, for its time from the moment it first showed, and then
/// the next. "Checking what Beebeeb has" stays while the check is out (`check_out`): the list's answer has the
/// last word, and it must find the toast to give it to. Answers when to look again.
///
/// `drawn`: a toast whose button sends one session's sign-out again, and that takes the place of one about
/// ANOTHER session (its time was up, or its button was pressed), is drawn in other words (`Said::shown`): the
/// same sentence with the same button would read as the same toast. About the same session it keeps its words.
pub(super) fn fade(notices: &mut Line, now: Instant, check_out: bool, drawn: &mut Drawn) -> Option<Duration> {
    while let Some((_, said)) = notices.front_mut() {
        if said.shown_at.is_none() && said.sends_one_again() {
            let key = said.about.as_ref().map(|about| about.key.clone()).unwrap_or_default();
            said.turn = match &drawn.0 {
                Some((before, turn)) if *before == key => *turn,
                Some((_, 1)) => 2,
                Some(_) => 1,
                None => 0,
            };
            drawn.0 = Some((key, said.turn));
        }
        let shown_at = *said.shown_at.get_or_insert(now);
        let left = said.stays().saturating_sub(now.saturating_duration_since(shown_at));
        if left.is_zero() && !(said.stage == Stage::Checking && check_out) {
            notices.pop_front();
        } else {
            return (!left.is_zero()).then_some(left);
        }
    }
    // Nothing shows: the next toast is not "another" one.
    drawn.0 = None;
    None
}

/// The window shows another pane: the toasts go. Two kinds are kept in `later`, for when Devices and sessions
/// shows again: a sign-out that was not done and whose toast never showed (it waited behind another), and one
/// whose result is still not known, shown or not (the person was told it is being checked, and is told how it
/// ended). Kept ones show for their whole time again.
pub(super) fn leave(notices: &mut Line, later: &mut Line) {
    for (session, mut said) in std::mem::take(notices) {
        let unseen = said.shown_at.is_none();
        let open = matches!(said.stage, Stage::Checking | Stage::Unconfirmed);
        if said.about.is_some() && said.failed && (unseen || open) || said.stage == Stage::Answered && unseen {
            said.shown_at = None;
            line_up(later, session, said);
        }
    }
}
