//! The viewer's changes that leave the window's thread: Available offline and saving a text. The press stamps the
//! session and the account, a worker makes the engine call for that session (`Engine::set_kept_for`,
//! `save_text_for`, `save_text_copy_for`), and the answer is taken by `ViewerSource::settled` only while it is
//! still the open file's session and account. Nothing is drawn as done before the engine answered.

use std::sync::mpsc;

use zeroize::Zeroizing;

use super::*;
use crate::frames::viewers::source::Conflict;

/// What the other side of a conflict is called: the engine does not say which device saved.
const OTHER_SIDE: &str = "Another device";

/// One answer on its way back: for which file, under which session and account.
pub(super) struct Message {
    file_id: String,
    session: SessionGeneration,
    account: Option<String>,
    settled: Settled,
}

pub(super) struct Changes {
    tx: mpsc::Sender<Message>,
    rx: mpsc::Receiver<Message>,
    /// A change is with a worker: one at a time.
    busy: bool,
}

impl Default for Changes {
    fn default() -> Self {
        let (tx, rx) = mpsc::channel();
        Self { tx, rx, busy: false }
    }
}

/// "09:41", this PC's clock.
fn hhmm() -> String {
    chrono::Local::now().format("%H:%M").to_string()
}

/// What this PC is called in the name of a Keep both copy.
fn this_pc() -> String {
    ["COMPUTERNAME", "HOSTNAME"]
        .iter()
        .find_map(|name| std::env::var(name).ok().filter(|value| !value.trim().is_empty()))
        .unwrap_or_else(|| "This PC".to_string())
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

    /// The press's stamp for `file_id`: the session, the account, what the file is shown as and its name.
    fn stamp_of(&self, file_id: &str) -> Option<(SessionGeneration, Option<String>, Shows, String)> {
        let (opening, _) = self.change_facts().filter(|(opening, _)| opening.file_id == file_id)?;
        Some((
            opening.session,
            opening.account.clone(),
            opening.shows,
            opening.name.clone(),
        ))
    }

    /// Run `work` on a worker; what it settles is sent back for the stamp's session and account.
    fn spawn(
        &mut self,
        file_id: &str,
        (session, account, ..): (SessionGeneration, Option<String>, Shows, String),
        work: impl FnOnce(&E) -> Settled + Send + 'static,
    ) -> bool {
        let (engine, tx, id) = (self.engine.clone(), self.changes.tx.clone(), file_id.to_string());
        let started = std::thread::Builder::new().name("viewer-change".into()).spawn(move || {
            let settled =
                std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| work(&engine))).unwrap_or(Settled::Failed);
            let _ = tx.send(Message {
                file_id: id,
                session,
                account,
                settled,
            });
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
        let Some((session, account, ..)) = self.stamp_of(file_id) else {
            return;
        };
        let _ = self.changes.tx.send(Message {
            file_id: file_id.to_string(),
            session,
            account,
            settled,
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
        let (session, id) = (stamp.0, file_id.to_string());
        if !self.spawn(file_id, stamp, move |engine| Settled::Offline {
            on,
            done: engine.set_kept(session, &id, on).is_ok(),
        }) {
            self.refuse(file_id, Settled::Offline { on, done: false });
        }
        true
    }

    pub(super) fn start_save(&mut self, file_id: &str, text: &str, base: u32) -> SaveOutcome {
        let Some(stamp) = self.stamp_of(file_id) else {
            return SaveOutcome::Failed;
        };
        if self.changes.busy || !self.can_save() {
            return SaveOutcome::Failed;
        }
        let (session, shows, name, id) = (stamp.0, stamp.2, stamp.3.clone(), file_id.to_string());
        let text = Zeroizing::new(text.to_string());
        if self.spawn(file_id, stamp, move |engine| {
            let saved = engine.save_text(session, &id, &text, i64::from(base));
            settle_save(saved, &id, &name, &text, shows)
        }) {
            SaveOutcome::Working
        } else {
            SaveOutcome::Failed
        }
    }

    pub(super) fn start_resolve(&mut self, file_id: &str, text: &str, keep: Keep) -> SaveOutcome {
        let Some(stamp) = self.stamp_of(file_id) else {
            return SaveOutcome::Failed;
        };
        if self.changes.busy || !self.can_save() {
            return SaveOutcome::Failed;
        }
        let (session, shows, name, id) = (stamp.0, stamp.2, stamp.3.clone(), file_id.to_string());
        let text = Zeroizing::new(text.to_string());
        let started = self.spawn(file_id, stamp, move |engine| match keep {
            // Mine becomes the newest version: saved from the version the file is at now.
            Keep::Mine => match engine.edit_facts(session, &id) {
                Some(now) => settle_save(
                    engine.save_text(session, &id, &text, now.version),
                    &id,
                    &name,
                    &text,
                    shows,
                ),
                None => Settled::Failed,
            },
            // My edit is dropped; the file as Beebeeb has it is read again.
            Keep::Theirs => {
                let cancel = AtomicBool::new(false);
                let recorded = Recorded {
                    here: false,
                    unsettled: false,
                };
                let read = read(engine, &id, recorded, &|_, _| {}, &cancel);
                let (Ok((read_for, bytes, _)), Some(now)) = (read, engine.edit_facts(session, &id)) else {
                    return Settled::Failed;
                };
                if read_for != session {
                    return Settled::Failed;
                }
                match content_of(shows, &bytes, &cancel).body.take() {
                    Some(body) => Settled::Opened {
                        body,
                        version: u32::try_from(now.version).unwrap_or(0),
                    },
                    None => Settled::Failed,
                }
            }
            // Mine next to theirs, under this PC's name.
            Keep::Both => {
                let at = hhmm();
                match engine.save_copy(session, &id, &text, &this_pc(), &at) {
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
                }
            }
        });
        if started {
            SaveOutcome::Working
        } else {
            SaveOutcome::Failed
        }
    }

    /// The answer for `file_id`, while it is still this session's and this account's. Any other is dropped (its
    /// text overwritten with it).
    pub(super) fn take_settled(&mut self, file_id: &str) -> Option<Settled> {
        let mut found = None;
        while let Ok(message) = self.changes.rx.try_recv() {
            self.changes.busy = false;
            let current = self.engine.has_session()
                && self.engine.session_turn() == message.session
                && self.engine.account() == message.account;
            if current && message.file_id == file_id && found.is_none() {
                found = Some(message.settled);
            } else if let Settled::Saved { mut body, .. }
            | Settled::Queued { mut body }
            | Settled::Opened { mut body, .. } = message.settled
            {
                body.wipe();
            }
        }
        found
    }
}

/// The body of `text` as the file is shown.
fn body_of_text(text: &str, shows: Shows) -> Option<Body> {
    text_body(text.as_bytes(), shows == Shows::Markdown)
}

/// What a save's end is, for the view.
fn settle_save(saved: Result<SaveEnd, EditRefused>, id: &str, name: &str, text: &str, shows: Shows) -> Settled {
    match saved {
        Ok(SaveEnd::Saved { version }) => match body_of_text(text, shows) {
            Some(body) => Settled::Saved {
                id: id.to_string(),
                name: name.to_string(),
                body,
                version: u32::try_from(version).unwrap_or(0),
                at: hhmm(),
            },
            None => Settled::Failed,
        },
        Ok(SaveEnd::Queued) => body_of_text(text, shows).map_or(Settled::Failed, |body| Settled::Queued { body }),
        Ok(SaveEnd::Conflict { theirs }) => Settled::Conflict(Conflict {
            from: OTHER_SIDE.to_string(),
            at: hhmm(),
            version: u32::try_from(theirs).unwrap_or(0),
        }),
        Err(_) => Settled::Failed,
    }
}
