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
//!   why (the engine also takes the account password as the proof: its window is not in this version yet).
//! - The switch shows the stored value until the engine answered: it never flips first. One change at a time.
//! - "Locks again after" is not drawn from here: the away lock it sets is not bound in this version, so the page
//!   gets no delay (an empty one) and draws none, never a value nothing acts on.
//!
//! Must not: log anything of the account; keep a Hello answer for a later change; write before the proof answered.

use std::sync::{Arc, mpsc};
use std::time::{Duration, Instant};

use beebeeb_engine::app::lock_password_api::{AppLockProof, AppLockRefused};
use beebeeb_engine::app::{CredentialStores, Engine};

use super::account::EngineAccountSource;
use super::stamp::{Stamp, account_of, stamp};
use crate::engine_host::hello::{HelloCancel, HelloPrompter, HelloResult, NoHello};
use crate::frames::account::Pane;

/// How long a toast of this part stays.
const STAYS: Duration = Duration::from_secs(6);

const HELLO_NOT_CONFIRMED: &str = "App lock is still on: Windows Hello did not confirm it's you.";
const HELLO_NOT_SET_UP: &str =
    "App lock is still on: turning it off needs Windows Hello, which is not set up on this PC.";
const NOT_OPEN: &str = "App lock was not changed: Beebeeb is locked.";
const NOT_SAVED: &str = "App lock was not changed: this PC could not store the setting.";
const SESSION_ENDED: &str = "App lock was not changed: your session ended.";
const NOT_CHANGED: &str = "App lock was not changed.";

enum Answer {
    Read {
        number: u64,
        made_for: Stamp,
        result: Result<bool, AppLockRefused>,
    },
    Hello {
        made_for: Stamp,
        result: HelloResult,
    },
    Set {
        made_for: Stamp,
        on: bool,
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
    held: Option<(Stamp, bool)>,
    reads: u64,
    reading: Option<u64>,
    showing: bool,
    /// The change on its way (asked under this stamp): no second one starts.
    changing: Option<Stamp>,
    /// The Hello prompt that is up, to take away when the page goes.
    prompt: Option<HelloCancel>,
    said: Option<Said>,
    tx: mpsc::Sender<Answer>,
    rx: mpsc::Receiver<Answer>,
}

/// What the page is told when a change was refused.
fn words(refused: AppLockRefused) -> &'static str {
    match refused {
        AppLockRefused::ProofNeeded => HELLO_NOT_CONFIRMED,
        AppLockRefused::NotOpen => NOT_OPEN,
        AppLockRefused::NotSaved => NOT_SAVED,
        AppLockRefused::SessionEnded => SESSION_ENDED,
        // Answers of the password proof (not asked from this window yet), a check already running, a session
        // that moved on (dropped before it is said): nothing more is known than that it was not changed.
        AppLockRefused::Overtaken
        | AppLockRefused::Wrong { .. }
        | AppLockRefused::Wait { .. }
        | AppLockRefused::ServerLimit { .. }
        | AppLockRefused::Offline
        | AppLockRefused::UpgradeNeeded
        | AppLockRefused::CouldNotCheck => NOT_CHANGED,
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
            said: None,
            tx,
            rx,
        }
    }

    fn open(&self) -> bool {
        self.engine.has_session() && !self.engine.account_session_refused()
    }

    fn is_now(&self, of: &Stamp) -> bool {
        self.open() && of.0 == self.engine.session_turn() && of.1 == account_of(&self.engine)
    }

    /// The stored switch, when it was read for this session and account.
    pub(super) fn known(&self) -> Option<bool> {
        self.held
            .as_ref()
            .filter(|(made_for, _)| self.is_now(made_for))
            .map(|(_, on)| *on)
    }

    /// What Sign-in and security draws: the switch, and no delay (see the module's doc).
    pub(super) fn facts(&self) -> Option<(bool, String)> {
        // RED: today's behaviour, never known.
        None
    }

    pub(super) fn opened(&mut self, pane: Pane) {
        self.showing = pane == Pane::Security;
        if self.showing && self.open() {
            self.read();
        }
    }

    pub(super) fn closed(&mut self) {
        self.showing = false;
        // A prompt for a page that is gone is taken away; its answer (Cancelled) changes nothing.
        if let Some(cancel) = self.prompt.take() {
            cancel();
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
        let started = std::thread::Builder::new()
            .name("account-app-lock".into())
            .spawn(move || {
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| engine.app_lock_for(mine.0)))
                    .unwrap_or(Err(AppLockRefused::CouldNotCheck));
                let _ = tx.send(Answer::Read {
                    number,
                    made_for: mine,
                    result,
                });
                engine.wake_host();
            })
            .is_ok();
        if started {
            self.reading = Some(number);
        }
    }

    /// The switch was pressed: `on` is what it asks for.
    pub(super) fn set(&mut self, on: bool) {
        // RED: today's behaviour, the press does nothing.
        if true || self.changing.is_some() || !self.open() {
            let _ = on;
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
        if on {
            return self.write(made_for, true, None);
        }
        // Off: Windows Hello first, for this change; its answer comes back through the channel.
        let (tx, engine, mine) = (self.tx.clone(), self.engine.clone(), made_for.clone());
        match self.hello.start(Box::new(move |result| {
            let _ = tx.send(Answer::Hello { made_for: mine, result });
            engine.wake_host();
        })) {
            Ok(cancel) => {
                self.prompt = Some(cancel);
                self.changing = Some(made_for);
            }
            Err(()) => self.say(HELLO_NOT_SET_UP),
        }
    }

    fn write(&mut self, made_for: Stamp, on: bool, proof: Option<AppLockProof>) {
        let (engine, tx, mine) = (self.engine.clone(), self.tx.clone(), made_for.clone());
        let started = std::thread::Builder::new()
            .name("account-app-lock".into())
            .spawn(move || {
                let session = mine.0;
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    engine.set_app_lock_for(session, on, proof)
                }))
                .unwrap_or(Err(AppLockRefused::CouldNotCheck));
                let _ = tx.send(Answer::Set {
                    made_for: mine,
                    on,
                    result,
                });
                engine.wake_host();
            })
            .is_ok();
        if started {
            self.changing = Some(made_for);
        } else {
            self.changing = None;
            self.say(NOT_CHANGED);
        }
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
            Answer::Hello { made_for, result } => {
                self.prompt = None;
                if self.changing.as_ref() != Some(&made_for) || !self.is_now(&made_for) {
                    self.changing = None;
                    return true;
                }
                match result {
                    // Windows verified the person, for this change: the engine asks core and writes.
                    HelloResult::Verified => self.write(made_for, false, Some(AppLockProof::PlatformVerified)),
                    HelloResult::Cancelled | HelloResult::Failed => {
                        self.changing = None;
                        self.say(HELLO_NOT_CONFIRMED);
                    }
                    HelloResult::Unavailable => {
                        self.changing = None;
                        self.say(HELLO_NOT_SET_UP);
                    }
                }
            }
            Answer::Set { made_for, on, result } => {
                if self.changing.as_ref() == Some(&made_for) {
                    self.changing = None;
                }
                if !self.is_now(&made_for) {
                    return true;
                }
                match result {
                    Ok(()) => self.held = Some((made_for, on)),
                    Err(refused) => {
                        self.say(words(refused));
                        // What is stored now is the engine's word: read it again.
                        self.held = None;
                        self.read();
                    }
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
