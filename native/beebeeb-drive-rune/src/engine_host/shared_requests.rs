//! New request (DW-0504) on the real engine: [`EngineSharedSource::create_request`] and `take_link`, handed here by
//! `shared.rs`.
//!
//! - The press: [`Engine::file_request_create_for`] on a worker of its own (it blocks), for the session the lists were
//!   read for, taken on the window's thread before the worker starts. One create at a time; a second press sends
//!   nothing. Nothing is sent without a title or without that session.
//! - The answer: it rides the adapter's change channel (`Answer::Changed`, under the id [`NEW_REQUEST`]), so a refusal
//!   is the "change was not made" notice, an answer under another session is dropped, and after a request that was
//!   made the lists are read again (the request appears from Beebeeb's list, never from here).
//! - The link: held from the worker's answer to the window's next look ([`SharedSource::take_link`]), which puts it on
//!   the clipboard. A link of another session is dropped there, unread. It is never logged or drawn.
//!
//! Must not: log a title, an id or a link; keep a link past the window's next look.

use std::sync::Mutex;

use beebeeb_engine::app::bindings::file_request::RequestChoices;
use beebeeb_engine::app::share_link_api::ShareLinkError;

use super::*;
use crate::frames::files::link::LinkText;
use crate::frames::shared::source::RequestForm;

/// The id a create rides the change channel under (never a row's id: those are UUIDs).
pub(super) const NEW_REQUEST: &str = "new-request";

/// The links made and not yet taken, each with the session it was made for.
#[derive(Default)]
pub(super) struct RequestCalls {
    made: Arc<Mutex<Vec<(SessionGeneration, LinkText)>>>,
}

impl<C: CredentialStores> EngineSharedSource<C> {
    /// See the module's doc. `false`: no title, no session, a create already on its way, or no worker could start.
    pub(super) fn shared_requests_create(&mut self, form: RequestForm) -> bool {
        let title = form.title.trim().to_string();
        if title.is_empty() || !self.engine.has_session() || self.changing.contains(NEW_REQUEST) {
            return false;
        }
        // The session of the press: the lists' own when they are current, else the one in memory now.
        let session = self
            .current()
            .map(|held| held.session)
            .unwrap_or_else(|| self.engine.session_turn());
        let note = form.note.trim();
        let choices = RequestChoices {
            title,
            note: (!note.is_empty()).then(|| note.to_string()),
            folder_id: form.target_folder,
            max_files: form.max_files,
            max_bytes: form.max_total_bytes,
            expires_in_days: form.expires_in_hours.map(|hours| hours.div_ceil(24)),
        };
        let (engine, answers, made) = (
            self.engine.clone(),
            self.answers.clone(),
            self.shared_requests.made.clone(),
        );
        let started = std::thread::Builder::new()
            .name("shared-request".into())
            .spawn(move || {
                let done = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    engine.file_request_create_for(session, &choices)
                }))
                .unwrap_or(Err(ShareLinkError::Refused));
                let done = match done {
                    Ok(link) => {
                        if let Ok(mut held) = made.lock() {
                            held.push((link.session, LinkText::new(link.expose())));
                        }
                        Ok(())
                    }
                    Err(why) => {
                        log(format!("shared: a request was not made ({why:?})"));
                        Err(ShareChangeError::Refused)
                    }
                };
                let _ = answers.send(Answer::Changed(NEW_REQUEST.to_string(), session, done));
                engine.wake_host();
            });
        if started.is_err() {
            log("shared: could not start a thread for a request");
            return self.refuse(Notice::ChangeRefused);
        }
        self.changing.insert(NEW_REQUEST.to_string());
        self.dirty = true;
        true
    }

    /// The next link made under the session in memory; links of any other session are dropped.
    pub(super) fn shared_requests_take_link(&mut self) -> Option<LinkText> {
        let now = self.engine.session_turn();
        let mut held = self.shared_requests.made.lock().ok()?;
        let mine = held.iter().position(|(session, _)| *session == now);
        held.retain(|(session, _)| *session == now);
        mine.map(|_| held.remove(0).1)
    }

    /// The session ended: no link is kept.
    pub(super) fn shared_requests_forget(&mut self) {
        if let Ok(mut held) = self.shared_requests.made.lock() {
            held.clear();
        }
    }
}
