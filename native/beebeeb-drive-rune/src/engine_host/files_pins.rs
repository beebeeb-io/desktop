//! The sidebar's Pinned folders on the real engine: [`Pins`], the part of `EngineFilesSource` behind `pinned`,
//! `pinned_ids`, `set_pinned` and `refresh_pins`.
//!
//! The pins are the account's `pinned_folders` preference (`Engine::pinned_folders_for`), the one the web app keeps,
//! so a pin made on either shows on the other. Folders only; a folder is drawn once this PC's local state knows its
//! row (its name is encrypted: it is read from there, never from the preference).
//!
//! When it reads: never from a draw. A draw only notes that the list is wanted; [`Pins::pump`] (the window's poll,
//! through `FilesSource::changes`) starts the read on a worker: the first time it is wanted in a session, when the
//! window got focus (`refresh`), when a write failed, and when the engine's sync word changes (a folder may have
//! arrived). A read that fails keeps what is held. A pin or unpin shows at once and is written on a worker, in the
//! order pressed; a read that lands while a write is out is dropped (it would show the list from before it).
//!
//! Whose: what is held carries the session AND the account it was read under; a look under another drops it, and an
//! answer that lands under another is dropped. Must not: log an id or a name.

use std::cell::Cell;
use std::sync::{Arc, Mutex, PoisonError, mpsc};

use beebeeb_engine::app::{CredentialStores, Engine};

use super::stamp::{Stamp, stamp};
use super::{files_item_of, log, sync};
use crate::frames::files::source::ActionError;
use crate::source::DriveItem;

/// One pin: the folder's id, and its row once this PC knows it.
type Pin = (String, Option<DriveItem>);

enum Answer {
    Read(Stamp, Option<Vec<Pin>>),
    Wrote(Stamp, bool),
}

/// See the module's doc.
pub struct Pins<C: CredentialStores> {
    engine: Arc<Engine<C>>,
    answers: mpsc::Sender<Answer>,
    inbox: mpsc::Receiver<Answer>,
    seen: Option<Stamp>,
    held: Option<Vec<Pin>>,
    wanted: Cell<bool>,
    reading: bool,
    /// What is held is older than something that happened: read again.
    old: bool,
    /// Writes out and not answered.
    writing: usize,
    /// Writes go one at a time, in the order pressed.
    order: Arc<Mutex<()>>,
    word: Option<&'static str>,
}

impl<C: CredentialStores> Pins<C> {
    /// Starts no thread and reads nothing.
    pub fn new(engine: Arc<Engine<C>>) -> Self {
        let (answers, inbox) = mpsc::channel();
        Self {
            engine,
            answers,
            inbox,
            seen: None,
            held: None,
            wanted: Cell::new(false),
            reading: false,
            old: true,
            writing: 0,
            order: Arc::new(Mutex::new(())),
            word: None,
        }
    }

    fn current(&self) -> Option<&Vec<Pin>> {
        self.wanted.set(true);
        let now = stamp(&self.engine);
        self.held
            .as_ref()
            .filter(|_| self.engine.has_session() && self.seen.as_ref() == Some(&now))
    }

    /// The pinned folders this PC knows, in the order pinned.
    pub fn pinned(&self) -> Vec<DriveItem> {
        self.current()
            .map(|pins| pins.iter().filter_map(|(_, item)| item.clone()).collect())
            .unwrap_or_default()
    }

    pub fn pinned_ids(&self) -> Vec<String> {
        self.current()
            .map(|pins| pins.iter().map(|(id, _)| id.clone()).collect())
            .unwrap_or_default()
    }

    /// Read again at the next poll.
    pub fn refresh(&mut self) {
        self.old = true;
    }

    /// Pin or unpin `folder`: shown at once, written on a worker. Refused while the list was not read yet (a write
    /// from nothing would replace the account's pins with one).
    pub fn set(&mut self, folder: &DriveItem, pin: bool) -> Result<(), ActionError> {
        let now = stamp(&self.engine);
        self.look(&now);
        if !self.engine.has_session() {
            return Err(ActionError::Unavailable);
        }
        let Some(held) = self.held.as_mut() else {
            return Err(ActionError::Unavailable);
        };
        held.retain(|(id, _)| *id != folder.id);
        if pin {
            held.push((folder.id.clone(), Some(folder.clone())));
        }
        let ids: Vec<String> = held.iter().map(|(id, _)| id.clone()).collect();
        let (engine, answers, order) = (self.engine.clone(), self.answers.clone(), self.order.clone());
        let started = std::thread::Builder::new().name("files-pins-write".into()).spawn(move || {
            let _one_at_a_time = order.lock().unwrap_or_else(PoisonError::into_inner);
            let wrote = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                engine.set_pinned_folders_for(now.0, ids).is_ok()
            }))
            .unwrap_or(false);
            let _ = answers.send(Answer::Wrote(now, wrote));
            engine.wake_host();
        });
        if started.is_err() {
            log("files: could not start the write of the pins");
            self.old = true;
            return Err(ActionError::Failed);
        }
        self.writing += 1;
        Ok(())
    }

    fn look(&mut self, now: &Stamp) {
        if self.seen.as_ref() == Some(now) {
            return;
        }
        self.seen = Some(now.clone());
        self.held = None;
        self.old = true;
        self.reading = false;
        self.writing = 0;
    }

    /// The window's poll. `true`: what the sidebar draws may have changed.
    pub fn pump(&mut self) -> bool {
        let now = stamp(&self.engine);
        self.look(&now);
        let here = self.engine.has_session();
        let mut drawn = false;
        while let Ok(answer) = self.inbox.try_recv() {
            match answer {
                Answer::Read(under, read) => {
                    if !(here && under == now) {
                        continue;
                    }
                    self.reading = false;
                    if self.writing > 0 {
                        // From before the write that is out: asked again once it landed.
                        self.old = true;
                    } else if let Some(read) = read {
                        let same = self
                            .held
                            .as_ref()
                            .is_some_and(|held| held.len() == read.len() && held.iter().zip(&read).all(|(a, b)| a.0 == b.0 && a.1.is_some() == b.1.is_some()));
                        drawn |= !same;
                        self.held = Some(read);
                    }
                }
                Answer::Wrote(under, wrote) => {
                    if !(here && under == now) {
                        continue;
                    }
                    self.writing = self.writing.saturating_sub(1);
                    if !wrote {
                        // The account did not take it: the sidebar goes back to what the account holds.
                        self.old = true;
                    }
                    drawn = true;
                }
            }
        }
        let word = sync::sync_word(&self.engine.view());
        if word != self.word && self.held.as_ref().is_some_and(|pins| pins.iter().any(|(_, item)| item.is_none())) {
            self.old = true;
        }
        self.word = word;
        if here && self.old && !self.reading && self.writing == 0 && self.wanted.replace(false) {
            self.read(now);
        }
        drawn
    }

    fn read(&mut self, under: Stamp) {
        let (engine, answers) = (self.engine.clone(), self.answers.clone());
        let started = std::thread::Builder::new().name("files-pins".into()).spawn(move || {
            let read = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                let ids = engine.pinned_folders_for(under.0).ok()?;
                let state = engine.local_state_for_changes().ok();
                Some(
                    ids.into_iter()
                        .map(|id| {
                            let item = state
                                .as_ref()
                                .and_then(|state| state.with(|db| files_item_of::item_of(db, &id)).ok().flatten())
                                .filter(|item| item.is_folder);
                            (id, item)
                        })
                        .collect::<Vec<Pin>>(),
                )
            }))
            .unwrap_or(None);
            let _ = answers.send(Answer::Read(under, read));
            engine.wake_host();
        });
        if started.is_err() {
            log("files: could not start the read of the pins");
            return;
        }
        self.reading = true;
        self.old = false;
    }
}
