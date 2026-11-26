//! The found-while-paused card on the real engine (S2b): [`FoundWhilePausedSource`] reads the changes the engine
//! HOLDS (`Engine::found_while_paused_review_for`) and asks it, and only it, what to do with them: Upload
//! (`…_upload_for`), Keep on this PC only (`…_keep_for`), Show in File Explorer (`kept_folder_to_show_for`).
//!
//! Rules:
//! - Nothing is sent, moved or opened by this file's own decision: every press is one engine call, stamped with the
//!   session turn the card was READ under (never the current one), so a Lock, a sign-out or another account in between
//!   is refused by the engine (`FOUND_WHILE_PAUSED_NOT_YOURS`).
//! - Upload and Keep carry exactly the ids of the last review, never an id of their own and never none.
//! - One action at a time; every call runs on a worker (the engine's calls block on its step lock), and each answer
//!   comes back through [`FoundWhilePausedSource::pump`].
//! - Shows the engine's own fixed sentences when it refuses. Logs nothing but counts.
//!
//! Windows only (and tests): the engine's calls and `found_while_paused` exist only there.

use std::path::PathBuf;
use std::sync::{Arc, mpsc};

use beebeeb_engine::account::SessionGeneration;
use beebeeb_engine::app::{CredentialStores, Engine};
use beebeeb_engine::found_while_paused::{KeepResult, Review};

use super::on_worker;
use crate::frames::this_pc::found_while_paused::{Action, Card, Held, Kept};

#[cfg(test)]
#[path = "found_while_paused_tests.rs"]
mod tests;

/// What the card asks of the engine. The app's is `Engine`; a test's records what it was asked.
pub trait Decisions: Send + Sync + 'static {
    fn turn(&self) -> SessionGeneration;
    fn review(&self, turn: SessionGeneration) -> Result<Review, String>;
    fn upload(&self, turn: SessionGeneration, op_ids: &[String]) -> Result<usize, String>;
    fn keep(&self, turn: SessionGeneration, op_ids: &[String]) -> Result<KeepResult, String>;
    fn kept_folder(&self, turn: SessionGeneration) -> Result<PathBuf, String>;
    /// Open a folder the engine named. A worker's.
    fn open_folder(&self, folder: &std::path::Path) -> bool;
}

impl<C: CredentialStores> Decisions for Engine<C> {
    fn turn(&self) -> SessionGeneration {
        self.session_turn()
    }
    fn review(&self, turn: SessionGeneration) -> Result<Review, String> {
        self.found_while_paused_review_for(turn)
    }
    fn upload(&self, turn: SessionGeneration, op_ids: &[String]) -> Result<usize, String> {
        self.found_while_paused_upload_for(turn, op_ids)
    }
    fn keep(&self, turn: SessionGeneration, op_ids: &[String]) -> Result<KeepResult, String> {
        self.found_while_paused_keep_for(turn, op_ids)
    }
    fn kept_folder(&self, turn: SessionGeneration) -> Result<PathBuf, String> {
        self.kept_folder_to_show_for(turn)
    }
    fn open_folder(&self, folder: &std::path::Path) -> bool {
        super::open_path(folder, &|| true).is_ok()
    }
}

/// One worker's answer, with the turn it was asked under.
enum Answer {
    Reviewed(SessionGeneration, Result<Review, String>),
    Uploaded(Result<usize, String>),
    Kept(Result<KeepResult, String>),
    Shown(Result<bool, String>),
}

/// What the window's card holds, and the asks it makes.
pub struct FoundWhilePausedSource<E: Decisions> {
    engine: Arc<E>,
    pub card: Card,
    /// The session turn the held changes were read under: every decision is stamped with it.
    turn: Option<SessionGeneration>,
    tx: mpsc::Sender<Answer>,
    rx: mpsc::Receiver<Answer>,
}

impl<E: Decisions> FoundWhilePausedSource<E> {
    pub fn new(engine: Arc<E>) -> Self {
        let (tx, rx) = mpsc::channel();
        FoundWhilePausedSource {
            engine,
            card: Card::default(),
            turn: None,
            tx,
            rx,
        }
    }

    /// Read the held changes again (a section opened, a sync check finished, a decision was made). Nothing while an
    /// action is out: its answer reads again.
    pub fn refresh(&mut self) {
        if self.card.busy.is_some() {
            return;
        }
        let (engine, tx) = (self.engine.clone(), self.tx.clone());
        let turn = engine.turn();
        on_worker("found-while-paused-review", move || {
            let _ = tx.send(Answer::Reviewed(turn, engine.review(turn)));
        });
    }

    /// "Upload to my Beebeeb": the ids of the last review, under the turn it was read at. Returns whether it asked.
    pub fn upload(&mut self) -> bool {
        let Some((turn, ids)) = self.decision() else { return false };
        self.begin(Action::Upload);
        let (engine, tx) = (self.engine.clone(), self.tx.clone());
        on_worker("found-while-paused-upload", move || {
            let _ = tx.send(Answer::Uploaded(engine.upload(turn, &ids)));
        });
        true
    }

    /// "Keep on this PC only": the same ids, the same turn.
    pub fn keep(&mut self) -> bool {
        let Some((turn, ids)) = self.decision() else { return false };
        self.begin(Action::Keep);
        let (engine, tx) = (self.engine.clone(), self.tx.clone());
        on_worker("found-while-paused-keep", move || {
            let _ = tx.send(Answer::Kept(engine.keep(turn, &ids)));
        });
        true
    }

    /// "Show in File Explorer": the folder the engine names (never a path of the card's), under the card's turn.
    pub fn show(&mut self) -> bool {
        let Some(turn) = self.turn.filter(|_| self.card.can_show()) else { return false };
        self.begin(Action::Show);
        let (engine, tx) = (self.engine.clone(), self.tx.clone());
        on_worker("found-while-paused-show", move || {
            let _ = tx.send(Answer::Shown(engine.kept_folder(turn).map(|folder| engine.open_folder(&folder))));
        });
        true
    }

    /// The press of the card's button.
    pub fn press(&mut self, action: Action) -> bool {
        match action {
            Action::Upload => self.upload(),
            Action::Keep => self.keep(),
            Action::Show => self.show(),
        }
    }

    fn decision(&self) -> Option<(SessionGeneration, Vec<String>)> {
        if !self.card.can_decide() {
            return None;
        }
        let ids = self.card.held.as_ref()?.op_ids.clone();
        Some((self.turn?, ids)).filter(|(_, ids)| !ids.is_empty())
    }

    fn begin(&mut self, action: Action) {
        self.card.busy = Some(action);
        self.card.said = None;
    }

    /// Take what the workers answered. Returns whether the card changed.
    pub fn pump(&mut self) -> bool {
        let mut changed = false;
        while let Ok(answer) = self.rx.try_recv() {
            changed = true;
            match answer {
                Answer::Reviewed(turn, Ok(review)) => {
                    self.turn = Some(turn);
                    self.card.held = Some(held_of(review));
                    // `said` is not cleared here: the re-read that follows a refused upload would
                    // otherwise wipe the engine's refusal. The next action clears it (begin).
                }
                Answer::Reviewed(_, Err(words)) => {
                    // Not read: nothing is decided on an old read.
                    self.turn = None;
                    self.card.held = None;
                    self.card.said = Some(words);
                }
                Answer::Uploaded(result) => {
                    self.card.busy = None;
                    self.card.said = result.err();
                    self.refresh();
                }
                Answer::Kept(result) => {
                    self.card.busy = None;
                    match result {
                        Ok(kept) => self.card.kept = Some(kept_of(kept)),
                        Err(words) => self.card.said = Some(words),
                    }
                    self.refresh();
                }
                Answer::Shown(result) => {
                    self.card.busy = None;
                    self.card.said = match result {
                        Ok(true) => None,
                        Ok(false) => Some(NOT_OPENED.to_string()),
                        Err(words) => Some(words),
                    };
                }
            }
        }
        changed
    }
}

/// What the card says when Windows would not open the folder the engine named.
pub const NOT_OPENED: &str = "Beebeeb couldn’t open the folder.";

fn held_of(review: Review) -> Held {
    Held {
        new_folders: review.new_folders,
        new_files: review.new_files,
        edits: review.edits,
        deletions: review.deletions,
        in_shared_folders: review.in_shared_folders,
        op_ids: review.op_ids,
        synced_by: review.synced_by,
    }
}

fn kept_of(kept: KeepResult) -> Kept {
    Kept {
        moved_items: kept.moved_items,
        moved_files: kept.moved_files,
        not_moved: kept.not_moved,
        stays_held: kept.stays_held,
        synced_by: kept.synced_by,
    }
}
