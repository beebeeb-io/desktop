//! Add a folder and Stop backing up (DW-0737, DW-0738, DW-1410). Part of
//! [`EngineBackupsSource`] (`backups.rs`), which hands `add_folder`, `remove_folder`, `remove_folder_to_trash` and
//! `stopping` here, and drains this slice's answers in its `pump`.
//!
//! - Add a folder…: the shell opens Windows' folder picker (DW-0738, `shell::intents::pick_folder_for`) and hands its
//!   answer to `add_folder`, which asks `Engine::add_backup_folder_for` on a worker of its own, for the session of the
//!   press. The picker is modeless, so the session can change while it is open: the pending pick carries the press's
//!   session and the shell drops its answer once a poll saw that session change (`shell::intents::take_picked_folder`);
//!   an answer the shell hands over is for the press's session, which `add_folder` stamps. The rule is the engine's (Beebeeb's own folder and anything inside it, a folder that holds it, one backed up
//!   already, not a folder): a refusal comes back in the engine's words through `pump` (`Pumped::refusal`). Nothing is
//!   listed before the engine answered: an add that happened reads the list again, and the row comes from that read.
//!   The runner is asked for a pass at once, so the first backup starts (DW-1411).
//! - Stop backing up (DW-1410): "Stop and keep the backup" asks `Engine::remove_backup_folder_for` with no password.
//!   "Stop and move the backup to Trash" asks the account password first (the dialog's second step, a flagged
//!   deviation: the board draws no password field) and hands a copy of it to the engine, which proves it to Beebeeb
//!   (core's step-up) before anything moves; its answer comes through `pump` (`Pumped::stopped`). Asked without a
//!   password, no move to Trash is sent: a step-up is never made up.
//! - Session-bound: every press stamps the session and the account (`DevicesEngine::stamp`); an answer that lands under
//!   another stamp, or after the vault closed, is dropped and never shown.
//!
//! Must not: log a path, a name, an id or the password; keep the password beyond its worker; show a folder as added or
//! removed before the engine said so.

use beebeeb_engine::app::bindings::backup_folders::{AddRefused, RemoveRefused};
use zeroize::Zeroizing;

use super::super::stamp::Stamp;
use super::*;
use crate::frames::backups::source::Stopped;

#[cfg(test)]
#[path = "backup_folders_tests.rs"]
mod tests;

/// What the section says when "Stop and keep the backup" did not happen (it adds that nothing was changed).
pub const NOT_STOPPED: &str = "Couldn't stop backing up this folder.";

/// One add's or stop's answer, with the stamp of its press.
enum FolderAnswer {
    Added {
        pressed: Stamp,
        result: Result<(), AddRefused>,
    },
    Removed {
        pressed: Stamp,
        /// "Stop and move the backup to Trash" (with the password).
        trash: bool,
        result: Result<(), RemoveRefused>,
    },
}

impl FolderAnswer {
    fn pressed(&self) -> &Stamp {
        match self {
            Self::Added { pressed, .. } | Self::Removed { pressed, .. } => pressed,
        }
    }
}

/// What this file keeps between a press and its answer: the inbox its workers answer into, how many are out, and the
/// one move to Trash on its way. The adapter holds one (`EngineBackupsSource::backup_folders`).
pub(super) struct BackupFolderCalls {
    answers: mpsc::Sender<FolderAnswer>,
    inbox: mpsc::Receiver<FolderAnswer>,
    pending: Arc<AtomicUsize>,
    /// The chosen folder a "Stop and move the backup to Trash" was sent for, until its answer: no second one is sent.
    stopping: Option<String>,
    /// Tests: the next worker waits for this before it asks the engine (an answer that outlives its session).
    #[cfg(test)]
    hold_next: Option<mpsc::Receiver<()>>,
}

impl Default for BackupFolderCalls {
    fn default() -> Self {
        let (answers, inbox) = mpsc::channel();
        Self {
            answers,
            inbox,
            pending: Arc::default(),
            stopping: None,
            #[cfg(test)]
            hold_next: None,
        }
    }
}

impl<E: BackupsEngine> EngineBackupsSource<E> {
    /// No add or stop is waiting or running (tests wait on it).
    #[cfg(test)]
    pub(super) fn backup_folders_idle(&self) -> bool {
        self.backup_folders.pending.load(Ordering::SeqCst) == 0
    }

    /// Start `job` on a worker of its own; its answer goes down this slice's channel and wakes the window. `false`: no
    /// worker could be started (nothing was asked).
    fn backup_folders_spawn(&mut self, job: impl FnOnce(&E) -> FolderAnswer + Send + 'static) -> bool {
        let (engine, answers, pending) = (
            self.engine.clone(),
            self.backup_folders.answers.clone(),
            self.backup_folders.pending.clone(),
        );
        #[cfg(test)]
        let hold = self.backup_folders.hold_next.take();
        pending.fetch_add(1, Ordering::SeqCst);
        let started = std::thread::Builder::new()
            .name("backup-folders".into())
            .spawn(move || {
                #[cfg(test)]
                if let Some(hold) = hold {
                    let _ = hold.recv();
                }
                let answer = job(&*engine);
                let _ = answers.send(answer);
                pending.fetch_sub(1, Ordering::SeqCst);
                engine.wake();
            });
        if started.is_err() {
            self.backup_folders.pending.fetch_sub(1, Ordering::SeqCst);
            log("backups: could not start a worker for a chosen folder");
            return false;
        }
        true
    }

    /// The folder picker's answer (DW-0738): asked of the engine for the session of the press. The stamp is read here,
    /// when the shell hands the answer over, not when the picker opened: the shell hands over only an answer whose
    /// session no poll saw change while the picker was open (`shell::intents::take_picked_folder`), so it is the
    /// press's. `true`: sent, the answer comes through `pump`. `false`, nothing sent: no one signed in with the vault
    /// open, or no path.
    pub(super) fn backup_folders_add_folder(&mut self, path: &str) -> bool {
        let pressed = self.engine.stamp();
        if !self.current() || path.is_empty() {
            return false;
        }
        let asked = path.to_string();
        self.backup_folders_spawn(move |engine| {
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                engine.add_folder_for(pressed.0, &asked)
            }))
            .unwrap_or_else(|_| {
                log("backups: an add ended early");
                Err(AddRefused::Backups(BackupsError::State))
            });
            // Its first backup starts now rather than at the runner's next pass (DW-1411).
            if result.is_ok() {
                engine.request_pass();
            }
            FolderAnswer::Added { pressed, result }
        })
    }

    /// "Stop and keep the backup" (`move_copy_to_trash == false`), for a chosen folder the list holds. A move to Trash
    /// needs the account password and goes through [`Self::backup_folders_remove_folder_to_trash`]: asked here, nothing
    /// is sent.
    pub(super) fn backup_folders_remove_folder(&mut self, id: &str, move_copy_to_trash: bool) {
        if move_copy_to_trash {
            log("backups: a move to Trash was asked without the account password; nothing was sent");
            return;
        }
        self.backup_folders_send_remove(id, None);
    }

    /// "Stop and move the backup to Trash" with the account password (the dialog's second step). `true`: sent, the
    /// answer comes through `pump` (`Pumped::stopped`). `false`, nothing sent: no password, one already on its way, no
    /// session, or not a chosen folder of the list.
    pub(super) fn backup_folders_remove_folder_to_trash(&mut self, id: &str, password: &str) -> bool {
        if password.is_empty() || self.backup_folders_stopping() {
            return false;
        }
        // The worker's own copy, wiped when it ends (a panic included); it is in no type that prints.
        let secret = Zeroizing::new(password.as_bytes().to_vec());
        let sent = self.backup_folders_send_remove(id, Some(secret));
        if sent {
            self.backup_folders.stopping = Some(id.to_string());
        }
        sent
    }

    /// A move to Trash is on its way for the session that is here.
    pub(super) fn backup_folders_stopping(&self) -> bool {
        self.current() && self.backup_folders.stopping.is_some()
    }

    /// Ask the engine to stop backing up the chosen folder `id` (its backup to Trash when `password` is given).
    fn backup_folders_send_remove(&mut self, id: &str, password: Option<Zeroizing<Vec<u8>>>) -> bool {
        let pressed = self.engine.stamp();
        let listed = self.rows().any(|row| row.key == id && row.chosen_by_you());
        if !self.current() || !listed {
            return false;
        }
        let (asked, trash) = (id.to_string(), password.is_some());
        self.backup_folders_spawn(move |engine| {
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                engine.remove_folder_for(pressed.0, &asked, password.as_deref().map(Vec::as_slice))
            }))
            .unwrap_or_else(|_| {
                log("backups: a stop ended early");
                Err(RemoveRefused::Backups(BackupsError::State))
            });
            drop(password);
            FolderAnswer::Removed { pressed, trash, result }
        })
    }

    /// Read the list again after an add or a stop the engine answered: a read out now was asked before the change, so
    /// one more follows it.
    fn backup_folders_read_again(&mut self) {
        if self.send(Ask::Read) {
            self.reading = true;
        }
    }

    /// Drain the answers of adds and stops (the adapter's `pump`). One asked under another session or account, or that
    /// lands after the vault closed, is dropped. An add or a stop that happened reads the list again (nothing is drawn
    /// as added or removed before that read); a refusal is said in the engine's words.
    pub(super) fn backup_folders_pump(&mut self, pumped: &mut Pumped) {
        pumped.busy |= self.backup_folders.pending.load(Ordering::SeqCst) > 0;
        while let Ok(answer) = self.backup_folders.inbox.try_recv() {
            if matches!(answer, FolderAnswer::Removed { trash: true, .. }) {
                self.backup_folders.stopping = None;
            }
            pumped.changed = true;
            let mine = self.current() && *answer.pressed() == self.engine.stamp();
            if !mine {
                log("backups: an answer for another session was dropped");
                continue;
            }
            match answer {
                FolderAnswer::Added { result: Ok(()), .. } => self.backup_folders_read_again(),
                FolderAnswer::Added {
                    result: Err(refused), ..
                } => {
                    log(format!("backups: a folder was not added ({})", refused.kind()));
                    pumped.refusal = Some(refused.words());
                }
                FolderAnswer::Removed {
                    trash: false, result, ..
                } => {
                    if let Err(refused) = &result {
                        log(format!("backups: a folder was not stopped ({})", refused.kind()));
                        pumped.refusal = Some(NOT_STOPPED);
                    }
                    self.backup_folders_read_again();
                }
                FolderAnswer::Removed {
                    trash: true, result, ..
                } => {
                    pumped.stopped = Some(match &result {
                        Ok(()) => Stopped::Done,
                        Err(refused) if refused.stopped_backing_up() => Stopped::StoppedOnly(refused.words()),
                        Err(refused) => Stopped::Refused(refused.words()),
                    });
                    if let Err(refused) = &result {
                        log(format!("backups: a move to Trash was not done ({})", refused.kind()));
                    }
                    self.backup_folders_read_again();
                }
            }
        }
    }
}
