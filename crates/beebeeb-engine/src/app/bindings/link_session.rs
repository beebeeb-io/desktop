//! The session plumbing the link flows share (`file_request.rs`, `share_folder_link.rs`): ONE copy of each session
//! check, so a fix to one reaches every flow. A press runs on a [`LinkRun`]: the token and the vault key of the
//! session of the press, copied together while it is the session in memory.
//!
//! The three session checks, each asked of the session of the press ([`Engine::session_turn`] as the host read it):
//! - [`Engine::link_run`]: when the token and the key are copied. A press of a session that is not the one in memory
//!   is refused before the local state or Beebeeb is asked anything (each flow checks the shape of its own input,
//!   an id, a title, a password, first).
//! - [`Engine::link_send`]: before EACH request. A press made under one session never sends under another.
//! - [`Engine::hand_over`]: before the link is handed over. A link made for one session is never handed to another.
//!
//! The token never leaves this file except on a request [`Engine::link_send`] sends. Must not: log or format a
//! token, a key, a request or an answer.

use beebeeb_core::auth_flow::{HttpRequest, HttpResponse, Method};
use serde_json::Value;
use zeroize::Zeroizing;

use crate::account::SessionGeneration;
use crate::app::bindings::share_folder_link::FolderLinkError;
use crate::app::owned_state::LocalStateRefused;
use crate::app::share_link_api::{MadeLink, ShareLinkError};
use crate::app::{CredentialStores, Engine};
use crate::auth_transport;

/// Why the shared plumbing stopped a press, as the same-named refusal of each flow's own error.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum LinkRefusal {
    /// No session in memory, or not the session of the press any more. Nothing (more) was sent.
    NotSignedIn,
    Offline,
    /// Beebeeb no longer accepts this session.
    SessionEnded,
    /// No item of this account's local state.
    NotFound,
    /// The local state could not be read.
    Refused,
}

impl From<LinkRefusal> for ShareLinkError {
    fn from(refused: LinkRefusal) -> Self {
        match refused {
            LinkRefusal::NotSignedIn => Self::NotSignedIn,
            LinkRefusal::Offline => Self::Offline,
            LinkRefusal::SessionEnded => Self::SessionEnded,
            LinkRefusal::NotFound => Self::NotFound,
            LinkRefusal::Refused => Self::Refused,
        }
    }
}

impl From<LinkRefusal> for FolderLinkError {
    fn from(refused: LinkRefusal) -> Self {
        match refused {
            LinkRefusal::NotSignedIn => Self::NotSignedIn,
            LinkRefusal::Offline => Self::Offline,
            LinkRefusal::SessionEnded => Self::SessionEnded,
            LinkRefusal::NotFound => Self::NotFound,
            LinkRefusal::Refused => Self::Refused,
        }
    }
}

/// What one press runs on: the session of the press, its token and vault key (copied together, wiped on drop), and
/// the transport.
pub(super) struct LinkRun<'a> {
    session: SessionGeneration,
    token: Zeroizing<String>,
    key: Zeroizing<[u8; 32]>,
    base: &'a str,
    client: reqwest::Client,
}

impl LinkRun<'_> {
    /// The vault key of the session of the press.
    pub(super) fn key(&self) -> &[u8; 32] {
        &self.key
    }
}

impl<C: CredentialStores> Engine<C> {
    /// The token and the vault key of `session`, copied together while it is the session in memory.
    pub(super) fn link_run(&self, session: SessionGeneration) -> Result<LinkRun<'_>, LinkRefusal> {
        let (token, key) = self
            .person_session(|current, _| {
                (
                    Zeroizing::new(current.token.clone()),
                    Zeroizing::new(current.master_key),
                )
            })
            .map_err(|_| LinkRefusal::NotSignedIn)?;
        if !self.session_is_still(session, &key) {
            return Err(LinkRefusal::NotSignedIn);
        }
        let client = auth_transport::client_for(&self.config.api_base).map_err(|_| LinkRefusal::Offline)?;
        Ok(LinkRun {
            session,
            token,
            key,
            base: self.config.api_base.trim_end_matches('/'),
            client,
        })
    }

    /// Send one request of the press, only while its session is still the session in memory.
    pub(super) fn link_send(&self, run: &LinkRun<'_>, req: HttpRequest) -> Result<HttpResponse, LinkRefusal> {
        #[cfg(test)]
        if let Some(hook) = tests::BEFORE_SEND.with(|hook| hook.borrow_mut().take()) {
            hook();
        }
        if !self.session_is_still(run.session, &run.key) {
            return Err(LinkRefusal::NotSignedIn);
        }
        self.runtime
            .block_on(auth_transport::send_to(run.base, &run.client, Some(&run.token), req))
            .map_err(|_| LinkRefusal::Offline)
    }

    /// The hand-over: only while the session in memory is the very one the token and the key were copied from. The
    /// link goes over as core made it, its key after `#` and nowhere else.
    pub(super) fn hand_over(&self, run: &LinkRun<'_>, link: Zeroizing<String>) -> Result<MadeLink, LinkRefusal> {
        if !self.session_is_still(run.session, &run.key) {
            return Err(LinkRefusal::NotSignedIn);
        }
        Ok(MadeLink {
            session: run.session,
            link,
        })
    }
}

/// Why the account's local state was not handed out, as a link flow says it.
pub(super) fn state_refusal(refused: LocalStateRefused) -> LinkRefusal {
    match refused {
        LocalStateRefused::SessionEnded => LinkRefusal::SessionEnded,
        LocalStateRefused::Unreadable => LinkRefusal::Refused,
        LocalStateRefused::SignedOut | LocalStateRefused::Locked | LocalStateRefused::SessionChanged => {
            LinkRefusal::NotSignedIn
        }
        // Nothing synced, or a local state that is not this account's: no item of it is this account's.
        _ => LinkRefusal::NotFound,
    }
}

/// A read on the session with no confirm token and no body.
pub(in crate::app) fn get(path: &str) -> HttpRequest {
    let mut req = HttpRequest::session_post(path.to_string(), Value::Null);
    req.method = Method::Get;
    req.body = None;
    req
}

#[cfg(test)]
pub(super) mod tests {
    use std::cell::RefCell;

    thread_local! {
        /// Test seam: runs once at the top of the next [`super::Engine::link_send`] on this thread, before its session
        /// check (the session moves between the press's run and its request).
        pub(in crate::app::bindings) static BEFORE_SEND: RefCell<Option<Box<dyn FnOnce()>>> =
            const { RefCell::new(None) };
    }
}
