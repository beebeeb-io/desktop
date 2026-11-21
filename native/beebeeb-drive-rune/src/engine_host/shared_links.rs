//! Shared's link calls on the real engine (w17-shared-links): Copy link on a link row or a request row, Change expiry
//! and Password on a link, Change expiry on a request. Part of [`EngineSharedSource`] (`shared.rs`), which hands the
//! three `SharedSource` calls here.
//!
//! - Copy link: for a row of the current session's lists only. A link row: [`Engine::share_link_for`] with no choices,
//!   for the ITEM behind the row (`Held::link_items`); a request row: [`Engine::file_request_copy_for`]. Both run on a
//!   worker. The link waits in `made` (a [`LinkText`]: never drawn, never logged, wiped on drop) until the window
//!   takes it with [`EngineSharedSource::shared_links_take_link`] and puts it on the clipboard through a `Tx`; a link
//!   of a session that is gone is dropped there. The answer rides the adapter's change channel (`Answer::Changed`),
//!   so a refusal is the view's "change was not made" and the lists are read again.
//! - Change expiry / Password on a link: [`Engine::share_link_change_for`] on a worker for the row's share id; the
//!   password is a `Zeroizing` that moves into the worker and is wiped there.
//! - Change expiry on a request: `ShareChange::RequestExpiry` through the adapter's `change`.
//!
//! Must not: log a link, a password, a name or an id.

use beebeeb_engine::app::share_link_api::{LinkChange, ShareLinkError};

use super::*;
use crate::frames::files::link::LinkText;
use crate::frames::shared::source::LinkEdit;

/// What the link calls keep between a press and the window taking the link. The adapter holds one.
pub(super) struct SharedLinkCalls {
    made: mpsc::Sender<(SessionGeneration, LinkText)>,
    ready: mpsc::Receiver<(SessionGeneration, LinkText)>,
}

impl Default for SharedLinkCalls {
    fn default() -> Self {
        let (made, ready) = mpsc::channel();
        Self { made, ready }
    }
}

impl<C: CredentialStores> EngineSharedSource<C> {
    /// A link made by Copy link, for the clipboard. Only one of the current session; `None`: nothing waits.
    pub(super) fn shared_links_take_link(&mut self) -> Option<LinkText> {
        while let Ok((session, link)) = self.shared_links.ready.try_recv() {
            if session == self.engine.session_turn() && self.engine.has_session() {
                return Some(link);
            }
        }
        None
    }

    /// Sent once per press, for a link row (its item's link) or a request row of the current session's lists.
    pub(super) fn shared_links_copy_row_link(&mut self, id: &str) -> bool {
        let Some(held) = self.current() else {
            return false;
        };
        let session = held.session;
        let item = held.link_items.get(id).cloned();
        let request = held.requests.iter().any(|r| r.id == id);
        if item.is_none() && !request {
            return false;
        }
        if self.changing.contains(id) {
            return false;
        }
        let (engine, answers, made) = (
            self.engine.clone(),
            self.answers.clone(),
            self.shared_links.made.clone(),
        );
        let sent_for = id.to_string();
        let started = std::thread::Builder::new()
            .name("shared-copy-link".into())
            .spawn(move || {
                let copied = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    let link = match &item {
                        Some(item) => engine.share_link_for(session, item, None).map_err(|_| ()),
                        None => engine.file_request_copy_for(session, &sent_for).map_err(|_| ()),
                    };
                    link.map(|link| LinkText::new(link.expose()))
                }))
                .unwrap_or(Err(()));
                let done = match copied {
                    Ok(link) => {
                        let _ = made.send((session, link));
                        Ok(())
                    }
                    Err(()) => Err(ShareChangeError::Refused),
                };
                let _ = answers.send(Answer::Changed(sent_for, session, done));
                engine.wake_host();
            });
        self.started(started, id)
    }

    /// Sent once per press, for a link row of the current session's lists.
    pub(super) fn shared_links_change_link(&mut self, share_id: &str, edit: LinkEdit) -> bool {
        let Some(session) = self
            .current()
            .filter(|held| held.links.iter().any(|l| l.id == share_id))
            .map(|held| held.session)
        else {
            return false;
        };
        if self.changing.contains(share_id) {
            return false;
        }
        let change = match edit {
            LinkEdit::Expiry(hours) => LinkChange {
                expires_in_hours: Some(hours),
                passphrase: None,
            },
            LinkEdit::Password(password) => LinkChange {
                expires_in_hours: None,
                passphrase: Some(password),
            },
        };
        let (engine, answers, sent_for) = (self.engine.clone(), self.answers.clone(), share_id.to_string());
        let started = std::thread::Builder::new()
            .name("shared-link-change".into())
            .spawn(move || {
                let done = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    engine.share_link_change_for(session, &sent_for, &change)
                }))
                .unwrap_or(Err(ShareLinkError::Refused));
                drop(change);
                let done = done.map_err(|why| match why {
                    ShareLinkError::NotSignedIn => ShareChangeError::NotSignedIn,
                    ShareLinkError::Offline => ShareChangeError::Offline,
                    ShareLinkError::SessionEnded => ShareChangeError::SessionEnded,
                    ShareLinkError::NotFound => ShareChangeError::NotFound,
                    _ => ShareChangeError::Refused,
                });
                let _ = answers.send(Answer::Changed(sent_for, session, done));
                engine.wake_host();
            });
        self.started(started, share_id)
    }

    /// Sent once per press, for a request row of the current session's lists (`None`: never).
    pub(super) fn shared_links_change_request_expiry(&mut self, id: &str, hours: Option<u32>) -> bool {
        let Some(session) = self
            .current()
            .filter(|held| held.requests.iter().any(|r| r.id == id))
            .map(|held| held.session)
        else {
            return false;
        };
        self.change(
            session,
            id,
            ShareChange::RequestExpiry {
                request_id: id.into(),
                expires_in_secs: hours.map(|h| u64::from(h) * 3600),
            },
        )
    }

    /// A worker's start: marks the id as changing, or says the change was not made.
    fn started(&mut self, started: std::io::Result<std::thread::JoinHandle<()>>, id: &str) -> bool {
        if started.is_err() {
            log("shared: could not start a thread for a change");
            return self.refuse(Notice::ChangeRefused);
        }
        self.changing.insert(id.to_string());
        self.dirty = true;
        true
    }
}
