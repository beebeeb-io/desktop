//! App lock on the real engine: the "Lock Beebeeb" switch of Sign-in and security (DW-S03, DW-S03b), bound to core's
//! rule through the engine ([`Engine::app_lock_for`], [`Engine::set_app_lock_for`]). Off: the app never locks by
//! itself and opens directly for anyone signed in to this PC; on (the default): it opens screen-locked. The rule
//! lives in core; this part only asks and draws.
//!
//! - Read on a worker when Sign-in and security opens; nothing is drawn as the setting before the engine answered
//!   (the row says "Not known yet"), and an answer for another session or account is dropped.
//! - Turning it ON asks nothing. Turning it OFF lowers what the screen lock protects, so core wants a proof given for
//!   this change: Windows Hello is asked first (the lock screen's prompt), and only its "verified" goes to the engine.
//!   A cancelled or failed prompt changes nothing and says so; a PC where no prompt can be shown keeps it on and says
//!   why. Either way the row then offers "Use your password" (no board frame: D-2): the account password is the
//!   engine's other proof ([`AppLockProof::Password`]), checked online or by the local check and counted as the screen
//!   unlock's tries are. The offer is for that change, in that session and account, while the page shows.
//! - The password is moved into the engine call and wiped there; nothing of it is kept, logged or put in an answer. A
//!   wrong one keeps the step open with the lock screen's words; any other answer ends the offer.
//! - The switch shows the stored value until the engine answered: it never flips first. One change at a time.
//! - "Locks again after" (DW-S03): the delay is read with the switch ([`Engine::lock_after_for`], the engine's
//!   `lock.after`, five minutes while nothing is stored) and drawn only while app lock is on. A shorter delay is
//!   written at once; a longer one lowers what the lock protects, so Windows Hello is asked first, as for off, and
//!   only its "verified" goes to the engine ([`Engine::set_lock_after_for`]). A stored value that is not one of the
//!   menu's is drawn as not known (the engine counts it as the strictest). What the page holds can be stale: when the
//!   engine wants a proof for a change the page thought shorter, Windows Hello is asked then (review of #452). The away lock that acts on it is the shell's (`shell/away.rs`) and the engine's.
//!
//! Must not: log anything of the account; keep a Hello answer for a later change; write before the proof answered;
//! keep, log or format the password; send it twice, or for another session.

use std::sync::{Arc, mpsc};
use std::time::{Duration, Instant};

use beebeeb_engine::app::lock_password_api::{AppLockProof, AppLockRefused, LockAfter};
use beebeeb_engine::app::{CredentialStores, Engine};

use zeroize::Zeroizing;

use super::account::EngineAccountSource;
use super::stamp::{Stamp, account_of, stamp};
use crate::engine_host::hello::{HelloCancel, HelloPrompter, HelloResult, NoHello};
use crate::frames::account::Pane;
use crate::frames::account::app_lock_password::PasswordOffer;

/// How long a toast of this part stays.
const STAYS: Duration = Duration::from_secs(6);

const HELLO_NOT_CONFIRMED: &str = "App lock is still on: Windows Hello did not confirm it's you.";
const HELLO_NOT_SET_UP: &str =
    "App lock is still on: Windows Hello is not set up on this PC. Use your password instead.";
const NOT_OPEN: &str = "App lock was not changed: Beebeeb is locked.";
const NOT_SAVED: &str = "App lock was not changed: this PC could not store the setting.";
const SESSION_ENDED: &str = "App lock was not changed: your session ended.";
const NOT_CHANGED: &str = "App lock was not changed.";
// The lock screen's sentences for the same answers (`frames/signin/FRAMES.md`, "Screen lock: password", and the vault
// level's notice), copied: they are inline there.
const OFFLINE: &str = "App lock is still on: Beebeeb can't be reached, and this PC can't check your password offline yet. Connect to the internet or use Windows Hello.";
const UPGRADE_NEEDED: &str = "App lock is still on: sign in again to finish updating your account.";

const AFTER_HELLO_NOT_CONFIRMED: &str = "The lock delay was not changed: Windows Hello did not confirm it's you.";
const AFTER_HELLO_NOT_SET_UP: &str =
    "The lock delay was not changed: Windows Hello is not set up on this PC. Use your password instead.";
const AFTER_NOT_OPEN: &str = "The lock delay was not changed: Beebeeb is locked.";
const AFTER_NOT_SAVED: &str = "The lock delay was not changed: this PC could not store the setting.";
const AFTER_SESSION_ENDED: &str = "The lock delay was not changed: your session ended.";
const AFTER_NOT_CHANGED: &str = "The lock delay was not changed.";
const AFTER_OFFLINE: &str = "The lock delay was not changed: Beebeeb can't be reached, and this PC can't check your password offline yet. Connect to the internet or use Windows Hello.";
const AFTER_UPGRADE_NEEDED: &str = "The lock delay was not changed: sign in again to finish updating your account.";

/// A stored delay that is not one of the menu's: the button says so and no row of the menu is checked.
const AFTER_NOT_KNOWN: &str = "Not known";

/// The menu's words for each delay (`frames::account::app_lock::DELAYS`, the board's).
fn label(after: LockAfter) -> &'static str {
    match after {
        LockAfter::Immediately => "Immediately",
        LockAfter::Minutes5 => "5 min",
        LockAfter::Minutes30 => "30 min",
        LockAfter::Minutes60 => "60 min",
    }
}

fn after_of(words: &str) -> Option<LockAfter> {
    LockAfter::ALL.into_iter().find(|after| label(*after) == words)
}

/// One change of this part: the switch, or the delay.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Change {
    AppLock(bool),
    After(LockAfter),
}

/// What the engine said is stored: the switch, and the delay (`None`: not one of the menu's).
type Stored = (bool, Option<LockAfter>);

/// What a write carried to the engine.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Proved {
    /// No proof: a change that needs none.
    Nothing,
    /// Windows Hello's yes.
    Hello,
    /// The account password, typed in the step.
    Password,
}

/// "Use your password" (no board frame: D-2): Windows Hello did not confirm `change`, asked under `made_for`. Never the
/// password itself.
#[cfg_attr(test, derive(Debug))]
pub(super) struct Offer {
    made_for: Stamp,
    change: Change,
    /// The password is with the engine.
    checking: bool,
    /// The last answer that kept the step open.
    said: Option<AppLockRefused>,
}

impl Offer {
    fn new(made_for: Stamp, change: Change) -> Self {
        Self {
            made_for,
            change,
            checking: false,
            said: None,
        }
    }
}

#[cfg_attr(test, derive(Debug))]
enum Answer {
    Read {
        number: u64,
        made_for: Stamp,
        result: Result<Stored, AppLockRefused>,
    },
    Hello {
        /// The prompt's number (`AppLockPart::prompts`).
        number: u64,
        made_for: Stamp,
        change: Change,
        result: HelloResult,
    },
    Set {
        made_for: Stamp,
        change: Change,
        /// What the write carried.
        proved: Proved,
        result: Result<(), AppLockRefused>,
    },
}

/// A toast and when it first showed.
struct Said {
    words: &'static str,
    session: beebeeb_engine::account::SessionGeneration,
    at: Instant,
}

pub(super) struct AppLockPart<C: CredentialStores> {
    engine: Arc<Engine<C>>,
    hello: Box<dyn HelloPrompter>,
    held: Option<(Stamp, Stored)>,
    reads: u64,
    reading: Option<u64>,
    showing: bool,
    /// The change on its way (asked under this stamp): no second one starts.
    changing: Option<Stamp>,
    /// The Hello prompt that is up, to take away when the page goes.
    prompt: Option<HelloCancel>,
    /// The number of the last prompt asked: only the answer of the prompt that is up, under this number, is taken in.
    prompts: u64,
    /// "Use your password", while it is offered.
    pub(super) offer: Option<Offer>,
    said: Option<Said>,
    /// Every answer taken in, as `Debug` prints it (the password test reads them).
    #[cfg(test)]
    pub(super) answers_seen: Vec<String>,
    /// The workers this part started (`computing`): a test tells an answer still being computed from one that was
    /// lost. The app lets its workers run on their own.
    #[cfg(test)]
    workers: Vec<std::thread::JoinHandle<()>>,
    tx: mpsc::Sender<Answer>,
    rx: mpsc::Receiver<Answer>,
}

/// What the page is told when a change was refused.
fn words(change: Change, refused: AppLockRefused) -> &'static str {
    let (proof, not_open, not_saved, ended, other) = match change {
        Change::AppLock(_) => (HELLO_NOT_CONFIRMED, NOT_OPEN, NOT_SAVED, SESSION_ENDED, NOT_CHANGED),
        Change::After(_) => (
            AFTER_HELLO_NOT_CONFIRMED,
            AFTER_NOT_OPEN,
            AFTER_NOT_SAVED,
            AFTER_SESSION_ENDED,
            AFTER_NOT_CHANGED,
        ),
    };
    let (offline, upgrade) = match change {
        Change::AppLock(_) => (OFFLINE, UPGRADE_NEEDED),
        Change::After(_) => (AFTER_OFFLINE, AFTER_UPGRADE_NEEDED),
    };
    match refused {
        AppLockRefused::ProofNeeded => proof,
        AppLockRefused::NotOpen => not_open,
        AppLockRefused::NotSaved => not_saved,
        AppLockRefused::SessionEnded => ended,
        // The password's answers that end the step (the lock screen's sentences).
        AppLockRefused::Offline => offline,
        AppLockRefused::UpgradeNeeded => upgrade,
        // A check already running, and the answers that keep the step open (said under its field, `step_words`):
        // in a toast nothing more is known than that it was not changed.
        AppLockRefused::Overtaken
        | AppLockRefused::Wrong { .. }
        | AppLockRefused::Wait { .. }
        | AppLockRefused::ServerLimit { .. }
        | AppLockRefused::CouldNotCheck => other,
    }
}

/// A password answer after which the step stays open: the person can try again (after a wait the engine counts).
fn step_stays(refused: AppLockRefused) -> bool {
    matches!(
        refused,
        AppLockRefused::Wrong { .. }
            | AppLockRefused::Wait { .. }
            | AppLockRefused::ServerLimit { .. }
            | AppLockRefused::CouldNotCheck
    )
}

/// The sentence under the step's field after an answer that keeps it open: the lock screen's sentence for the same
/// outcome (`frames/signin/FRAMES.md`, "Screen lock: password"; inline in `frames::signin::lock`, so copied), without
/// a prefix: the step's title and line say what it is for. Never anything of the password.
fn step_words(refused: AppLockRefused) -> Option<String> {
    let wait = crate::frames::signin::lock::wait_words;
    Some(match refused {
        AppLockRefused::Wrong {
            local_check_off: true,
            ..
        } => "That password isn't right. This PC no longer checks your password offline: connect to the internet or use Windows Hello.".to_string(),
        AppLockRefused::Wrong { wait_ms: 0, .. } => "That password isn't right.".to_string(),
        AppLockRefused::Wrong { wait_ms, .. } => format!("That password isn't right. Try again in {}.", wait(wait_ms)),
        AppLockRefused::Wait { wait_ms } => format!("Too many wrong tries. Try again in {}.", wait(wait_ms)),
        AppLockRefused::ServerLimit { wait_ms: Some(ms) } => format!(
            "Beebeeb is checking too many passwords right now. Try again in {}.",
            wait(ms)
        ),
        AppLockRefused::ServerLimit { wait_ms: None } => {
            "Beebeeb is checking too many passwords right now. Try again in a moment.".to_string()
        }
        AppLockRefused::CouldNotCheck => "Beebeeb couldn't check that password. Nothing was changed.".to_string(),
        _ => return None,
    })
}

/// What the page is told when Windows Hello did not confirm, or cannot be asked.
fn not_proved(change: Change, unavailable: bool) -> &'static str {
    match (change, unavailable) {
        (Change::After(_), false) => AFTER_HELLO_NOT_CONFIRMED,
        (Change::After(_), true) => AFTER_HELLO_NOT_SET_UP,
        (Change::AppLock(_), false) => HELLO_NOT_CONFIRMED,
        (Change::AppLock(_), true) => HELLO_NOT_SET_UP,
    }
}

impl<C: CredentialStores + 'static> AppLockPart<C> {
    pub(super) fn new(engine: Arc<Engine<C>>) -> Self {
        let (tx, rx) = mpsc::channel();
        Self {
            engine,
            hello: Box::new(NoHello),
            held: None,
            reads: 0,
            reading: None,
            showing: false,
            changing: None,
            prompt: None,
            prompts: 0,
            offer: None,
            said: None,
            #[cfg(test)]
            answers_seen: Vec::new(),
            #[cfg(test)]
            workers: Vec::new(),
            tx,
            rx,
        }
    }

    /// A worker of this part is on its way: the app lets it run on its own; a test keeps it (`computing`).
    fn keep(&mut self, worker: std::thread::JoinHandle<()>) {
        #[cfg(test)]
        self.workers.push(worker);
        #[cfg(not(test))]
        drop(worker);
    }

    /// Whether a worker of this part still runs (tests only). Its answer is then still on its way, never lost: core's
    /// password check takes seconds on a busy PC. A worker that ended sent its answer first, so the next `pump` takes
    /// it in.
    #[cfg(test)]
    pub(super) fn computing(&mut self) -> bool {
        self.workers.retain(|worker| !worker.is_finished());
        !self.workers.is_empty()
    }

    fn open(&self) -> bool {
        self.engine.has_session() && !self.engine.account_session_refused()
    }

    fn is_now(&self, of: &Stamp) -> bool {
        self.open() && of.0 == self.engine.session_turn() && of.1 == account_of(&self.engine)
    }

    /// The stored switch and delay, when they were read for this session and account.
    fn stored(&self) -> Option<Stored> {
        self.held
            .as_ref()
            .filter(|(made_for, _)| self.is_now(made_for))
            .map(|(_, stored)| *stored)
    }

    /// The stored switch, when it was read for this session and account.
    pub(super) fn known(&self) -> Option<bool> {
        self.stored().map(|(on, _)| on)
    }

    /// What Sign-in and security draws: the switch, and the delay in the menu's words (drawn only while on).
    pub(super) fn facts(&self) -> Option<(bool, String)> {
        self.stored()
            .map(|(on, after)| (on, after.map_or(AFTER_NOT_KNOWN, label).to_string()))
    }

    pub(super) fn opened(&mut self, pane: Pane) {
        self.showing = pane == Pane::Security;
        // "Use your password" is the Security page's: it goes when the page goes.
        if !self.showing {
            self.offer = None;
        }
        if self.showing && self.open() {
            self.read();
        }
    }

    pub(super) fn closed(&mut self) {
        self.showing = false;
        self.offer = None;
        // A prompt for a page that is gone is taken away, and the change it was asked for ends with it: after the
        // cancel no answer of it is taken in (`take_in`), so nothing else would free the switch.
        if let Some(cancel) = self.prompt.take() {
            cancel();
            self.changing = None;
        }
    }

    fn read(&mut self) {
        if !self.open() || self.reading.is_some() {
            return;
        }
        self.reads += 1;
        let number = self.reads;
        let made_for = stamp(&self.engine);
        let (engine, tx, mine) = (self.engine.clone(), self.tx.clone(), made_for.clone());
        let worker = std::thread::Builder::new()
            .name("account-app-lock".into())
            .spawn(move || {
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    Ok((engine.app_lock_for(mine.0)?, engine.lock_after_for(mine.0)?))
                }))
                .unwrap_or(Err(AppLockRefused::CouldNotCheck));
                let _ = tx.send(Answer::Read {
                    number,
                    made_for: mine,
                    result,
                });
                engine.wake_host();
            });
        if let Ok(worker) = worker {
            self.keep(worker);
            self.reading = Some(number);
        }
    }

    /// The switch was pressed: `on` is what it asks for.
    pub(super) fn set(&mut self, on: bool) {
        if self.changing.is_some() || !self.open() {
            return;
        }
        let Some(stored) = self.known() else {
            return;
        };
        if stored == on {
            return;
        }
        let made_for = stamp(&self.engine);
        self.said = None;
        self.offer = None;
        if on {
            return self.write(made_for, Change::AppLock(true), None);
        }
        // Off: Windows Hello first, for this change.
        self.prove(made_for, Change::AppLock(false));
    }

    /// A delay was chosen in the menu (its words, `frames::account::app_lock::DELAYS`). Only while app lock is on
    /// (the row is not drawn while it is off); words the menu does not have choose nothing.
    pub(super) fn set_after(&mut self, words: &str) {
        if self.changing.is_some() || !self.open() {
            return;
        }
        let (Some(after), Some((true, stored))) = (after_of(words), self.stored()) else {
            return;
        };
        if stored == Some(after) {
            return;
        }
        let made_for = stamp(&self.engine);
        self.said = None;
        self.offer = None;
        // Shorter than what applies now asks nothing; longer lowers what the lock protects (the engine's rule):
        // Windows Hello first, as for off. A value that is not known counts as the strictest, as in the engine.
        if after <= stored.unwrap_or(LockAfter::STRICTEST) {
            return self.write(made_for, Change::After(after), None);
        }
        self.prove(made_for, Change::After(after));
    }

    /// Windows Hello for `change`; its answer comes back through the channel.
    fn prove(&mut self, made_for: Stamp, change: Change) {
        self.prompts += 1;
        let number = self.prompts;
        let (tx, engine, mine) = (self.tx.clone(), self.engine.clone(), made_for.clone());
        match self.hello.start(Box::new(move |result| {
            let _ = tx.send(Answer::Hello {
                number,
                made_for: mine,
                change,
                result,
            });
            engine.wake_host();
        })) {
            Ok(cancel) => {
                self.prompt = Some(cancel);
                self.changing = Some(made_for);
            }
            // No prompt can be shown on this PC: the password instead.
            Err(()) => {
                self.offer = Some(Offer::new(made_for, change));
                self.say(not_proved(change, true));
            }
        }
    }

    /// "Use your password": the account password for the change Windows Hello did not confirm, sent once, and only in
    /// the session and account it was offered in. The engine checks it (counted, as the screen unlock's tries) and
    /// writes; the buffer moves into the engine call and is wiped there. Nothing of it stays here.
    pub(super) fn with_password(&mut self, password: Zeroizing<String>) {
        if self.changing.is_some() || password.is_empty() {
            return;
        }
        let Some((made_for, change)) = self
            .offer
            .as_ref()
            .filter(|offer| !offer.checking && self.is_now(&offer.made_for))
            .map(|offer| (offer.made_for.clone(), offer.change))
        else {
            return;
        };
        if let Some(offer) = self.offer.as_mut() {
            offer.checking = true;
            offer.said = None;
        }
        self.said = None;
        self.write(made_for, change, Some(AppLockProof::Password(password)));
    }

    /// What Sign-in and security draws for "Use your password": `None` while it is not offered (or the offer is of
    /// another session or account).
    pub(super) fn password_offer(&self) -> Option<PasswordOffer> {
        self.offer
            .as_ref()
            .filter(|offer| self.is_now(&offer.made_for))
            .map(|offer| PasswordOffer {
                turning_off: offer.change == Change::AppLock(false),
                checking: offer.checking,
                said: offer.said.and_then(step_words),
            })
    }

    fn write(&mut self, made_for: Stamp, change: Change, proof: Option<AppLockProof>) {
        let (engine, tx, mine) = (self.engine.clone(), self.tx.clone(), made_for.clone());
        let proved = match &proof {
            None => Proved::Nothing,
            Some(AppLockProof::PlatformVerified) => Proved::Hello,
            Some(AppLockProof::Password(_)) => Proved::Password,
        };
        let worker = std::thread::Builder::new()
            .name("account-app-lock".into())
            .spawn(move || {
                let session = mine.0;
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| match change {
                    Change::AppLock(on) => engine.set_app_lock_for(session, on, proof),
                    Change::After(after) => engine.set_lock_after_for(session, after, proof),
                }))
                .unwrap_or(Err(AppLockRefused::CouldNotCheck));
                let _ = tx.send(Answer::Set {
                    made_for: mine,
                    change,
                    proved,
                    result,
                });
                engine.wake_host();
            });
        if let Ok(worker) = worker {
            self.keep(worker);
            self.changing = Some(made_for);
        } else {
            self.changing = None;
            self.refused(change, proved, AppLockRefused::CouldNotCheck);
        }
    }

    /// A change was refused. A password answer that lets the person try again keeps the step open with its words and
    /// changes nothing stored; any other ends the offer and is said as a toast (a check that was overtaken by another
    /// says nothing), and the stored value is read again.
    fn refused(&mut self, change: Change, proved: Proved, refused: AppLockRefused) {
        if proved == Proved::Password {
            if step_stays(refused)
                && let Some(offer) = self.offer.as_mut()
            {
                offer.checking = false;
                offer.said = Some(refused);
                return;
            }
            self.offer = None;
            if refused == AppLockRefused::Overtaken {
                return;
            }
        }
        self.say(words(change, refused));
        // What is stored now is the engine's word: read it again.
        self.held = None;
        self.read();
    }

    fn say(&mut self, words: &'static str) {
        self.said = Some(Said {
            words,
            session: self.engine.session_turn(),
            at: Instant::now(),
        });
    }

    /// The window's poll: `(changed, busy, how long until a toast goes)`.
    pub(super) fn pump(&mut self) -> (bool, bool, Option<Duration>) {
        let mut changed = false;
        // The session or the account moved on: nothing of this part stays, no change and no toast either.
        if self.held.as_ref().is_some_and(|(made_for, _)| !self.is_now(made_for)) {
            self.held = None;
            changed = true;
        }
        if self.offer.as_ref().is_some_and(|offer| !self.is_now(&offer.made_for)) {
            self.offer = None;
            changed = true;
        }
        if self.changing.as_ref().is_some_and(|made_for| !self.is_now(made_for)) {
            self.changing = None;
            if let Some(cancel) = self.prompt.take() {
                cancel();
            }
            changed = true;
        }
        if self
            .said
            .as_ref()
            .is_some_and(|said| said.session != self.engine.session_turn())
        {
            self.said = None;
            changed = true;
        }
        while let Ok(answer) = self.rx.try_recv() {
            changed |= self.take_in(answer);
        }
        // A new session while the page shows: read once for it.
        if self.showing && self.open() && self.reading.is_none() && self.known().is_none() {
            self.read();
            changed = true;
        }
        let wake_in = self
            .said
            .as_ref()
            .map(|said| STAYS.saturating_sub(said.at.elapsed()))
            .filter(|left| !left.is_zero());
        (changed, self.reading.is_some() || self.changing.is_some(), wake_in)
    }

    fn take_in(&mut self, answer: Answer) -> bool {
        #[cfg(test)]
        self.answers_seen.push(format!("{answer:?}"));
        match answer {
            Answer::Read {
                number,
                made_for,
                result,
            } => {
                if self.reading == Some(number) {
                    self.reading = None;
                }
                // Dropped: made for a session or an account that is not the one here now.
                if !self.is_now(&made_for) {
                    return true;
                }
                self.held = result.ok().map(|on| (made_for, on));
            }
            Answer::Hello {
                number,
                made_for,
                change,
                result,
            } => {
                // Only the prompt that is up answers. One that was taken away (the page went, the session moved) may
                // still have an answer on its way, decided before its cancel (`hello::HelloCancel`): dropped, also
                // when a newer prompt is up under the same stamp, and that one stays up.
                if self.prompt.is_none() || number != self.prompts {
                    return true;
                }
                self.prompt = None;
                if self.changing.as_ref() != Some(&made_for) || !self.is_now(&made_for) {
                    self.changing = None;
                    return true;
                }
                match result {
                    // Windows verified the person, for this change: the engine asks core and writes.
                    HelloResult::Verified => self.write(made_for, change, Some(AppLockProof::PlatformVerified)),
                    // Not confirmed, or no prompt on this PC: nothing changes, and the password is offered instead.
                    HelloResult::Cancelled | HelloResult::Failed => {
                        self.changing = None;
                        self.offer = Some(Offer::new(made_for, change));
                        self.say(not_proved(change, false));
                    }
                    HelloResult::Unavailable => {
                        self.changing = None;
                        self.offer = Some(Offer::new(made_for, change));
                        self.say(not_proved(change, true));
                    }
                }
            }
            Answer::Set {
                made_for,
                change,
                proved,
                result,
            } => {
                if self.changing.as_ref() == Some(&made_for) {
                    self.changing = None;
                }
                if !self.is_now(&made_for) {
                    return true;
                }
                // What the page held was stale: the engine wants a proof for a change it thought needed none.
                // Windows Hello is asked for it now (never "did not confirm" for a prompt that was not shown).
                if matches!(result, Err(AppLockRefused::ProofNeeded)) && proved == Proved::Nothing {
                    self.prove(made_for, change);
                    return true;
                }
                match result {
                    Ok(()) => {
                        if proved == Proved::Password {
                            self.offer = None;
                        }
                        let (on, after) = self.stored().unwrap_or((true, None));
                        let now = match change {
                            Change::AppLock(on) => (on, after),
                            Change::After(after) => (on, Some(after)),
                        };
                        self.held = Some((made_for, now));
                    }
                    Err(refused) => self.refused(change, proved, refused),
                }
            }
        }
        true
    }

    /// The window's poll merged with the Passkeys part's (`account.rs` asks both in one line).
    pub(super) fn pump_with(&mut self, other: (bool, bool, Option<Duration>)) -> (bool, bool, Option<Duration>) {
        let (changed, busy, wake_in) = self.pump();
        (
            changed || other.0,
            busy || other.1,
            [wake_in, other.2].into_iter().flatten().min(),
        )
    }

    /// This part's toast (always a thing that was NOT done) over `other`, the Passkeys part's: one toast at a time.
    pub(super) fn notice_over(&self, other: Option<(&'static str, bool)>) -> Option<(&'static str, bool)> {
        self.notice().map(|words| (words, true)).or(other)
    }

    /// The toast, when it is of this session and not over: always a thing that was NOT done.
    pub(super) fn notice(&self) -> Option<&'static str> {
        self.said
            .as_ref()
            .filter(|said| said.session == self.engine.session_turn() && said.at.elapsed() < STAYS)
            .map(|said| said.words)
    }

    pub(super) fn clear_notice(&mut self) {
        self.said = None;
    }
}

impl<C: CredentialStores + 'static> EngineAccountSource<C> {
    /// The source with the main window's Windows Hello prompt, which turning app lock off asks first.
    pub fn with_hello(mut self, hello: Box<dyn HelloPrompter>) -> Self {
        self.app_lock.hello = hello;
        self
    }
}
