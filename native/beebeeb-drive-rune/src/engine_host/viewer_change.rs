//! The viewer's changes that leave the window's thread: Available offline and saving a text. The press stamps the
//! session and the account, a worker makes the engine call for that session (`Engine::set_kept_for`,
//! `save_text_for`, `save_text_copy_for`), and the answer is taken by `ViewerSource::settled` only while it is
//! still the open file's session and account. Nothing is drawn as done before the engine answered.
//!
//! The read of the open file's version (DW-1519's banner) runs on a worker too, but only reads: what it found is
//! weighed on the window's thread (`take_settled`), against the version the view holds (`Opening::seen`), which
//! only the window's thread writes.

use std::sync::mpsc;
use std::time::{Duration, Instant};

use zeroize::Zeroizing;

use super::*;
use crate::frames::viewers::source::{Conflict, Newer};

/// How often the open file's version is read from the local state.
pub(super) const NEWER_EVERY: Duration = Duration::from_millis(1500);

/// What a save says when an earlier change to the file on this PC (another opening's save, or one made in another app)
/// has not been sent yet: nothing was stored, and the edit stays open (delta 3 of desktop#432); Save a copy keeps it
/// as a new file next to this one (task 2530, item 1). Once that change lands, the banner offers it and a save from
/// here asks the two-versions question.
pub(super) const EARLIER_NOT_SENT: &str = "Not saved: an earlier change to this file on this PC has not been sent yet. \
     Your edit stays open: Save a copy keeps it as a new file.";

/// As [`EARLIER_NOT_SENT`], when that earlier change is held back (parked, or a held edit): it is not sent until the
/// person settles it, so the words promise no send (task 2530, item 1).
pub(super) const EARLIER_HELD: &str = "Not saved: an earlier change to this file on this PC is held back and is not \
     sent until it is settled in Sync history. Your edit stays open: Save a copy keeps it as a new file.";

/// What the window says of a save that was on its way when the screen locked and that the engine could not confirm
/// (the lock landed while it ran: it may have been stored just before). Never "not saved": that is not known. Saving
/// again tells: an earlier change still unsent refuses the save and says so; one that landed is the two-versions
/// question.
pub(super) const NOT_CONFIRMED_AT_LOCK: &str =
    "The save on its way when Beebeeb locked was not confirmed. Your edit stays open: save it again.";

/// One answer on its way back: for which file, under which session and account, for which opening.
pub(super) struct Message {
    file_id: String,
    session: SessionGeneration,
    account: Option<String>,
    /// The opening the work began under (`Opening::serial`).
    opening: u64,
    answer: Answer,
}

enum Answer {
    /// A change's answer, and the version the view holds once it is applied (`None`: not moved: a conflict, a
    /// refusal, the switch, a Keep both copy, which is another file). `worker`: the answer of the change that made the
    /// source busy (`Changes::busy`); a refusal said at once (`refuse`) is not, and never ends a change in flight.
    Change {
        settled: Settled,
        holds: Option<i64>,
        worker: bool,
    },
    /// A read of the open file's version: what the local state holds, the version the view's own queued save
    /// became (`EditFacts::own`), and when this PC read it ("09:14").
    Read { version: i64, own: bool, at: String },
}

pub(super) struct Changes {
    tx: mpsc::Sender<Message>,
    rx: mpsc::Receiver<Message>,
    /// A change is with a worker: one at a time. The file is not opened again meanwhile (`begin`).
    pub(super) busy: bool,
    /// A read of the open file's version is with a worker (it never blocks a change).
    pub(super) checking: Arc<AtomicBool>,
    /// When the version was last asked for.
    pub(super) checked: Option<Instant>,
    /// How long between two reads of the version.
    pub(super) every: Duration,
    /// How many files this source opened: each opening's `serial`.
    pub(super) openings: u64,
    /// What came back in the same drain as a change's answer, in order: reads, and any later change's answer. Taken on
    /// the next pump, against what the view holds after that answer (never dropped, never wiped unread).
    pending: Vec<Message>,
}

impl Message {
    /// A change's text, overwritten: the answer is never taken.
    fn wipe(self) {
        if let Answer::Change { settled, .. } = self.answer {
            wipe(settled);
        }
    }
}

/// Answers never taken (the source is let go: the file closed, the window went) are overwritten with it: an answer
/// kept for the next pump (`pending`) and one still on its way back alike.
impl Drop for Changes {
    fn drop(&mut self) {
        for message in self.pending.drain(..).chain(self.rx.try_iter()) {
            message.wipe();
        }
    }
}

impl Default for Changes {
    fn default() -> Self {
        let (tx, rx) = mpsc::channel();
        Self {
            tx,
            rx,
            busy: false,
            checking: Arc::default(),
            checked: None,
            every: NEWER_EVERY,
            openings: 0,
            pending: Vec::new(),
        }
    }
}

/// "09:41", this PC's clock.
fn hhmm() -> String {
    chrono::Local::now().format("%H:%M").to_string()
}

/// What this PC is called in the name of a Keep both copy (also This PC's Keep both, task 2530).
pub(crate) fn this_pc() -> String {
    ["COMPUTERNAME", "HOSTNAME"]
        .iter()
        .find_map(|name| std::env::var(name).ok().filter(|value| !value.trim().is_empty()))
        .unwrap_or_else(|| "This PC".to_string())
}

impl Opening {
    /// Stop everything of this opening: the download is cancelled, the picture released, an answer on its way
    /// dropped (its text overwritten).
    pub(super) fn end(&mut self) {
        self.cancel.store(true, Ordering::SeqCst);
        self.picture = None;
        // The worker that holds the document ends when nobody asks any more: the document and its bytes go.
        self.pages = None;
        while self.inbox.try_recv().is_ok() {}
        self.fetching = false;
        self.holds = false;
        self.ended = true;
    }
}

impl<E: ViewerEngine> EngineViewerSource<E> {
    /// The open file and what the local state said of it, while its session is the session.
    pub(super) fn change_facts(&self) -> Option<(&Opening, EditFacts)> {
        let opening = self
            .open
            .as_ref()
            .filter(|opening| !opening.ended && Self::is_current(&self.engine, opening))?;
        Some((opening, opening.edit?))
    }

    /// The press's stamp for `file_id`: the session, the account, the opening, what the file is shown as and its
    /// name.
    fn stamp_of(&self, file_id: &str) -> Option<Stamp> {
        let (opening, _) = self.change_facts().filter(|(opening, _)| opening.file_id == file_id)?;
        Some(Stamp {
            session: opening.session,
            account: opening.account.clone(),
            opening: opening.serial,
            epoch: opening.epoch,
            shows: opening.shows,
            name: opening.name.clone(),
        })
    }

    /// Run `work` on a worker; what it settles, and the version the view then holds, is sent back for the stamp's
    /// session, account and opening.
    fn spawn(
        &mut self,
        file_id: &str,
        stamp: Stamp,
        work: impl FnOnce(&E) -> (Settled, Option<i64>) + Send + 'static,
    ) -> bool {
        let (engine, tx, id) = (self.engine.clone(), self.changes.tx.clone(), file_id.to_string());
        let started = std::thread::Builder::new().name("viewer-change".into()).spawn(move || {
            let (settled, holds) = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| work(&engine)))
                .unwrap_or((Settled::Failed, None));
            let sent = tx.send(Message {
                file_id: id,
                session: stamp.session,
                account: stamp.account,
                opening: stamp.opening,
                answer: Answer::Change {
                    settled,
                    holds,
                    worker: true,
                },
            });
            // Nobody takes it any more (the source is gone): its text is overwritten here.
            if let Err(mpsc::SendError(message)) = sent {
                message.wipe();
            }
            engine.wake();
        });
        if started.is_err() {
            log("viewer: could not start the worker for a change");
            return false;
        }
        self.changes.busy = true;
        true
    }

    /// Say at once, through the same road as an answer, that nothing was done.
    fn refuse(&mut self, file_id: &str, settled: Settled) {
        let Some(stamp) = self.stamp_of(file_id) else {
            return;
        };
        let _ = self.changes.tx.send(Message {
            file_id: file_id.to_string(),
            session: stamp.session,
            account: stamp.account,
            opening: stamp.opening,
            answer: Answer::Change {
                settled,
                holds: None,
                worker: false,
            },
        });
    }

    pub(super) fn start_offline(&mut self, file_id: &str, on: bool) -> bool {
        let Some(stamp) = self.stamp_of(file_id) else {
            return false;
        };
        if self.changes.busy {
            self.refuse(file_id, Settled::Offline { on, done: false });
            return true;
        }
        let (session, id) = (stamp.session, file_id.to_string());
        if !self.spawn(file_id, stamp, move |engine| {
            let done = engine.set_kept(session, &id, on).is_ok();
            (Settled::Offline { on, done }, None)
        }) {
            self.refuse(file_id, Settled::Offline { on, done: false });
        }
        true
    }

    /// Review of #452 (P1-2): the open file ended at a lock of the screen, and the same account is back: the file is
    /// opened again for the session in memory under the edit the window kept, so that edit can be saved. A NEW
    /// opening (the engine's record of the earlier one's saves is not this one's) whose facts are read for this
    /// session, as an open reads them; its content is not read again (the window holds the edit, and the reader
    /// under it stays the session-changed state). The save goes from the version the edit began from (the window's
    /// `base`): a file that changed meanwhile is the two-versions question, never overwritten. Nothing while a change
    /// is on its way or while Beebeeb is locked; never for another account or after a sign-out.
    fn resume(&mut self, file_id: &str) {
        if self.changes.busy {
            return;
        }
        let engine = self.engine.clone();
        // Only while Beebeeb is open (review of #452, P2-B): a press that reaches the view while it is locked (the lock
        // landed, the screen not redrawn yet) uses nothing up, and the next press after the unlock opens the file again.
        let resumable = self.open.as_ref().is_some_and(|opening| {
            opening.file_id == file_id
                && opening.ended
                && opening.resumable
                && engine.has_session()
                && engine.screen_open()
                && engine.account() == opening.account
        });
        if !resumable {
            return;
        }
        let session = engine.session_turn();
        let epoch = engine.begin_opening(session, file_id);
        // Taken up only once its facts were read: the earlier opening stays ended and resumable until then, so a read
        // that fails is tried again at the next press, never left as a Save that fails with no words (P2-B).
        let Some(edit) = engine.edit_facts(session, file_id, epoch) else {
            return;
        };
        let Some(opening) = self.open.as_mut() else {
            return;
        };
        self.changes.openings += 1;
        opening.session = session;
        opening.epoch = epoch;
        opening.edit = Some(edit);
        opening.serial = self.changes.openings;
        opening.ended = false;
        opening.resumable = false;
        // The window holds the edit: the next lock ends this opening too, and a save after it opens the file again.
        opening.holds = true;
        // A fresh road for answers: nothing of the earlier session's worker reaches this opening.
        let (_, inbox) = mpsc::channel();
        opening.inbox = inbox;
        opening.cancel = Arc::new(AtomicBool::new(false));
    }

    pub(super) fn start_save(&mut self, file_id: &str, text: &str, base: u32) -> SaveOutcome {
        self.resume(file_id);
        let Some(stamp) = self.stamp_of(file_id) else {
            return SaveOutcome::Failed;
        };
        if self.changes.busy || !self.can_save() {
            return SaveOutcome::Failed;
        }
        let (session, epoch, shows, name, id) = (
            stamp.session,
            stamp.epoch,
            stamp.shows,
            stamp.name.clone(),
            file_id.to_string(),
        );
        let text = Zeroizing::new(text.to_string());
        if self.spawn(file_id, stamp, move |engine| {
            let saved = engine.save_text(session, &id, &text, i64::from(base), epoch);
            settle_save(saved, &id, &name, &text, shows)
        }) {
            SaveOutcome::Working
        } else {
            SaveOutcome::Failed
        }
    }

    pub(super) fn start_resolve(&mut self, file_id: &str, text: &str, keep: Keep) -> SaveOutcome {
        self.resume(file_id);
        let Some(stamp) = self.stamp_of(file_id) else {
            return SaveOutcome::Failed;
        };
        if self.changes.busy || !self.can_save() {
            return SaveOutcome::Failed;
        }
        let (session, epoch, shows, name, id) = (
            stamp.session,
            stamp.epoch,
            stamp.shows,
            stamp.name.clone(),
            file_id.to_string(),
        );
        let text = Zeroizing::new(text.to_string());
        let started = self.spawn(file_id, stamp, move |engine| match keep {
            // Mine becomes the newest version: saved from the version the file is at now.
            Keep::Mine => match engine.edit_facts(session, &id, epoch) {
                Some(now) => settle_save(
                    engine.save_text(session, &id, &text, now.version, epoch),
                    &id,
                    &name,
                    &text,
                    shows,
                ),
                None => (Settled::Failed, None),
            },
            // My edit is dropped; the file as Beebeeb has it is read again. The version is read BEFORE the content
            // (as an open reads it): a version that lands during the download makes the label older than the
            // bytes, never newer (a save from there is then a conflict, never an overwrite).
            Keep::Theirs => {
                let Some(now) = engine.edit_facts(session, &id, epoch) else {
                    return (Settled::Failed, None);
                };
                let cancel = AtomicBool::new(false);
                let recorded = Recorded {
                    here: false,
                    unsettled: false,
                };
                let Ok((read_for, bytes, _)) = read(engine, &id, recorded, IN_MEMORY_MAX, &|_, _| {}, &cancel) else {
                    return (Settled::Failed, None);
                };
                if read_for != session {
                    return (Settled::Failed, None);
                }
                match content_of(shows, &bytes, &cancel).body.take() {
                    Some(body) => (
                        Settled::Opened {
                            body,
                            version: u32::try_from(now.version).unwrap_or(0),
                        },
                        Some(now.version),
                    ),
                    None => (Settled::Failed, None),
                }
            }
            // Mine next to theirs, under this PC's name. The copy is another file: what the view holds of this one
            // does not move.
            Keep::Both => {
                let at = hhmm();
                let settled = match engine.save_copy(session, &id, &text, &this_pc(), &at) {
                    Ok(copy) => match body_of_text(&text, shows) {
                        Some(body) => Settled::Saved {
                            id: copy.id,
                            name: copy.name,
                            body,
                            version: 1,
                            at,
                        },
                        None => Settled::Failed,
                    },
                    Err(_) => Settled::Failed,
                };
                (settled, None)
            }
        });
        if started {
            SaveOutcome::Working
        } else {
            SaveOutcome::Failed
        }
    }

    /// The answer for `file_id`, while it is still this session's and this account's. Any other is dropped (its
    /// text overwritten with it). One change's answer per pump: what came after it waits for the next one, in order
    /// (delta check of desktop#432, round 3: a second answer in the same drain is never wiped unread).
    pub(super) fn take_settled(&mut self, file_id: &str) -> Option<Settled> {
        let mut found = None;
        let mut queue = std::mem::take(&mut self.changes.pending);
        while let Ok(message) = self.changes.rx.try_recv() {
            queue.push(message);
        }
        let (mut reads, mut after) = (Vec::new(), Vec::new());
        for message in queue {
            if found.is_some() {
                after.push(message);
                continue;
            }
            let current = self.answers_for(&message, file_id);
            let before_a_lock = self.before_a_lock(&message, file_id);
            match message.answer {
                Answer::Read { .. } => reads.push(message),
                Answer::Change { settled, holds, worker } => {
                    // Only the change in flight ends it: a refusal said at once (a press while busy) never does.
                    if worker {
                        self.changes.busy = false;
                    }
                    if current && self.opening_as(message.opening).is_some() {
                        self.hold(message.opening, holds);
                        found = Some(settled);
                    } else if current {
                        // Begun under an earlier opening of this file (delta check of #432): it never replaces what
                        // is on screen. Its outcome is said; its text is overwritten.
                        found = earlier(settled, file_id);
                    } else if before_a_lock {
                        // Begun before a lock of the screen, the same account back (review of #452): said the same
                        // way, never its text; what the engine could not confirm is said as that.
                        found = earlier_than_the_lock(settled, file_id);
                    } else {
                        wipe(settled);
                    }
                }
            }
        }
        // A change's answer first: it is what the person pressed. The reads, and whatever came after it, wait for the
        // next pump, and are taken then against what the view holds after this answer.
        if found.is_some() {
            reads.append(&mut after);
            self.changes.pending = reads;
            return found;
        }
        let mut newer = None;
        for message in reads {
            newer = self.weigh(message, file_id).or(newer);
        }
        if newer.is_none() {
            self.check_for_newer(file_id);
        }
        newer.map(Settled::Newer)
    }

    /// The message is for `file_id`, in the session and the account in memory now.
    fn answers_for(&self, message: &Message, file_id: &str) -> bool {
        message.file_id == file_id
            && self.engine.has_session()
            && self.engine.session_turn() == message.session
            && self.engine.account() == message.account
    }

    /// The message is for `file_id`, of an earlier session of the account signed in now: a lock of the screen came
    /// between (a sign-out and a sign-in take the window's file with them).
    fn before_a_lock(&self, message: &Message, file_id: &str) -> bool {
        message.file_id == file_id
            && self.engine.has_session()
            && self.engine.session_turn() != message.session
            && self.engine.account() == message.account
    }

    /// The opening `serial`, while it is the open one.
    fn opening_as(&mut self, serial: u64) -> Option<&mut Opening> {
        self.open
            .as_mut()
            .filter(|opening| opening.serial == serial && !opening.ended)
    }

    /// A change's answer moved the view to the version `holds`: not another device's.
    fn hold(&mut self, serial: u64, holds: Option<i64>) {
        if let (Some(version), Some(opening)) = (holds, self.opening_as(serial)) {
            opening.seen = opening.seen.max(version);
        }
    }

    /// What a read found, on the window's thread: a version past the one the view holds is one this opening did not
    /// make (another device's, or an earlier opening's on this PC), said once (it is then held, so one banner per
    /// version), unless it is the version the view's own queued save became.
    fn weigh(&mut self, message: Message, file_id: &str) -> Option<Newer> {
        let current = self.answers_for(&message, file_id);
        let Answer::Read { version, own, at } = message.answer else {
            return None;
        };
        let opening = self.opening_as(message.opening).filter(|_| current)?;
        if version <= opening.seen {
            return None;
        }
        opening.seen = version;
        (!own).then(|| Newer {
            from: String::new(),
            at,
        })
    }

    /// The open file whose version is read: a text or a note whose version the local state gave.
    fn read_for(&self, file_id: &str) -> Option<&Opening> {
        self.change_facts()
            .map(|(opening, _)| opening)
            .filter(|opening| opening.file_id == file_id && matches!(opening.shows, Shows::Text | Shows::Markdown))
    }

    /// How long until the next read of the version is due, while one is read: the window is woken then, so the
    /// banner comes without a press. Reads still to weigh: at once. A read on its way: none (it wakes the window
    /// itself when it ends, whatever it found).
    pub(super) fn next_check_due(&self) -> Option<Duration> {
        if !self.changes.pending.is_empty() {
            return Some(Duration::ZERO);
        }
        if self.changes.checking.load(Ordering::SeqCst) {
            return None;
        }
        self.read_for(&self.open.as_ref()?.file_id)?;
        let waited = self.changes.checked.map_or(self.changes.every, |at| at.elapsed());
        Some(self.changes.every.saturating_sub(waited))
    }

    /// On a worker: read the version the local state holds for the open file, and send it back for the window's
    /// thread to weigh (`weigh`).
    fn check_for_newer(&mut self, file_id: &str) {
        let due = self.changes.checked.is_none_or(|at| at.elapsed() >= self.changes.every);
        if self.changes.checking.load(Ordering::SeqCst) || !due {
            return;
        }
        let Some(opening) = self.read_for(file_id) else {
            return;
        };
        let (session, account, serial, epoch) =
            (opening.session, opening.account.clone(), opening.serial, opening.epoch);
        let (engine, tx, id) = (self.engine.clone(), self.changes.tx.clone(), file_id.to_string());
        let checking = self.changes.checking.clone();
        checking.store(true, Ordering::SeqCst);
        let started = std::thread::Builder::new().name("viewer-newer".into()).spawn(move || {
            let facts =
                std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| engine.edit_facts(session, &id, epoch)))
                    .ok()
                    .flatten();
            // Sent before the flag is let go: the next read starts after this answer is on its way.
            if let Some(facts) = facts {
                let _ = tx.send(Message {
                    file_id: id,
                    session,
                    account,
                    opening: serial,
                    answer: Answer::Read {
                        version: facts.version,
                        own: facts.own,
                        at: hhmm(),
                    },
                });
            }
            checking.store(false, Ordering::SeqCst);
            // Every end wakes the window, an answer or none: the next read is asked for from there.
            engine.wake();
        });
        self.changes.checked = Some(Instant::now());
        if started.is_err() {
            log("viewer: could not start the worker for a version read");
            self.changes.checking.store(false, Ordering::SeqCst);
        }
    }
}

/// The press's stamp: the session, the account and the opening the change is for; what the file is shown as and
/// its name (a Keep both names the copy after it).
struct Stamp {
    session: SessionGeneration,
    account: Option<String>,
    opening: u64,
    /// The engine's opening of the file the change is for (`Opening::epoch`): only its saves are its own.
    epoch: u64,
    shows: Shows,
    name: String,
}

/// The body of `text` as the file is shown.
fn body_of_text(text: &str, shows: Shows) -> Option<Body> {
    text_body(text.as_bytes(), shows == Shows::Markdown)
}

/// What a save came to, for the view, and the version the view then holds: the one the save became; for a save
/// stored here and not sent yet, the base the engine saved it from (its label: the text on screen is based on it;
/// the version it becomes is told apart when it lands: `EditFacts::own`); for a conflict, none (the view still holds
/// its own: the newer version is announced by the next read once the person leaves the question).
fn settle_save(
    saved: Result<SaveEnd, EditRefused>,
    id: &str,
    name: &str,
    text: &str,
    shows: Shows,
) -> (Settled, Option<i64>) {
    match saved {
        Ok(SaveEnd::Saved { version }) => match body_of_text(text, shows) {
            Some(body) => (
                Settled::Saved {
                    id: id.to_string(),
                    name: name.to_string(),
                    body,
                    version: u32::try_from(version).unwrap_or(0),
                    at: hhmm(),
                },
                Some(version),
            ),
            None => (Settled::Failed, None),
        },
        Ok(SaveEnd::Queued { base }) => match body_of_text(text, shows) {
            Some(body) => (
                Settled::Queued {
                    body,
                    version: u32::try_from(base).unwrap_or(0),
                },
                Some(base),
            ),
            None => (Settled::Failed, None),
        },
        // Who saved the other version is not known (the engine cannot tell another device from an earlier opening of
        // this one): `from` stays empty, and the question is worded without it.
        Ok(SaveEnd::Conflict { theirs }) => (
            Settled::Conflict(Conflict {
                from: String::new(),
                at: hhmm(),
                version: u32::try_from(theirs).unwrap_or(0),
            }),
            None,
        ),
        // Delta 3 of desktop#432: an earlier change to this file on this PC has not been sent yet; queued after it,
        // this save would replace it unasked. Nothing was stored: the edit stays, and the person is told why.
        Err(EditRefused::EarlierChangeNotSent) => (Settled::Refused(EARLIER_NOT_SENT.to_string()), None),
        // Task 2530, item 1: that earlier change is held back; the words promise no send.
        Err(EditRefused::EarlierChangeHeld) => (Settled::Refused(EARLIER_HELD.to_string()), None),
        Err(_) => (Settled::Failed, None),
    }
}

/// A change's text, overwritten: nobody takes it.
fn wipe(settled: Settled) {
    if let Settled::Saved { mut body, .. } | Settled::Queued { mut body, .. } | Settled::Opened { mut body, .. } =
        settled
    {
        body.wipe();
    }
}

/// A change begun before a lock of the screen: as [`earlier`], except a failure, which the engine answers when the lock
/// landed while the save ran (it may have been stored just before): said as not confirmed, never as not saved.
fn earlier_than_the_lock(settled: Settled, file_id: &str) -> Option<Settled> {
    match settled {
        Settled::Failed => Some(Settled::Said(NOT_CONFIRMED_AT_LOCK.to_string())),
        settled => earlier(settled, file_id),
    }
}

/// A change begun under an earlier opening of `file_id`, for the opening on screen now: what it came to, said, and
/// nothing else (its text is overwritten). The switch (Available offline) is the file's, not the opening's: as it is.
/// Keep theirs, under an earlier opening: nothing to say (the edit was dropped, as asked).
fn earlier(settled: Settled, file_id: &str) -> Option<Settled> {
    let said = match &settled {
        Settled::Offline { .. } => return Some(settled),
        Settled::Saved { id, version, .. } if id == file_id => {
            format!("Your earlier edit was saved \u{00B7} encrypted \u{00B7} version {version}")
        }
        Settled::Saved { .. } => "Your earlier edit was saved as a copy next to this file".to_string(),
        Settled::Queued { .. } => {
            "Your earlier edit was saved on this PC \u{00B7} sends when Beebeeb can be reached".to_string()
        }
        Settled::Conflict(_) => "Your earlier edit was not saved: the file changed since you opened it".to_string(),
        Settled::Failed | Settled::Refused(_) => "Your earlier edit was not saved".to_string(),
        Settled::Opened { .. } | Settled::Newer(_) | Settled::Said(_) => String::new(),
    };
    wipe(settled);
    (!said.is_empty()).then_some(Settled::Said(said))
}
