//! A file's versions, for a host without a UI toolkit (the native Files window's Version history): the list the
//! server holds ([`Engine::file_versions_for`]) and the restore of one ([`Engine::restore_file_version_for`]), each
//! bound to the session of the press and answering only for it.
//!
//! - The id asked about must be a FILE of the signed-in account's local state, read through the account-bound
//!   handle first: an id of another account, a folder or an id nothing here holds sends nothing.
//! - The list is sorted here, current version first, then the newest; its dates and sizes are the server's.
//! - A restore makes one more version on the server, so it is sent at most once per call and never repeated: what
//!   is known of it is said exactly. An answer that came back and refused it is [`VersionsRefused::Call`] (it was
//!   NOT made); a request that went out and got no answer, or a gateway's 5xx, is [`VersionsRefused::Unconfirmed`]
//!   (it may have been made: the host reads the list again to see).
//! - A session that moved on (a lock, a sign-out, another account's sign-in, even back to the same account) is
//!   [`VersionsRefused::NotSignedIn`], with nothing sent; a list made for a session that ended is not handed over.
//!
//! BLOCKS on the handle's runtime: both calls are a worker's. Must not: log a name, an id or anything the server
//! sends; copy a token or a key outside the session-write lock's question.

use std::sync::Arc;

use zeroize::Zeroizing;

use super::settings_api::{AccountCallError, kind_of};
use super::{CredentialStores, Engine, LocalStateRefused};
use crate::account::SessionGeneration;
use crate::api_client::ApiClient;
use crate::drive_v2::dto::VersionEntry;
use crate::drive_v2::model::versions_from_payload;
use crate::engine_bridge::{EngineBridge, EngineError, RestoreSent, error_http_status};

#[cfg(test)]
#[path = "versions_api_tests.rs"]
mod tests;

/// The versions of one file as the server lists them, and the session they were read for.
#[derive(Clone)]
pub struct FileVersions {
    /// The session the list belongs to: a host compares it with [`Engine::session_turn`] and drops a list whose
    /// session is gone.
    pub session: SessionGeneration,
    /// The current version first, then the newest first.
    pub versions: Vec<VersionEntry>,
}

/// How many versions, never their dates: a `{:?}` prints no content of a file's history.
impl std::fmt::Debug for FileVersions {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FileVersions")
            .field("session", &self.session)
            .field("versions", &self.versions.len())
            .finish()
    }
}

/// Why a file's versions were not listed, or a version not restored.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VersionsRefused {
    /// The press's session is gone or moved on: nothing was sent, nothing is handed over.
    NotSignedIn,
    /// No such FILE in this account's local state (another account's id, a folder): nothing was sent.
    NotFound,
    /// The account's local state is not handed out.
    NoLocalState,
    /// The call failed, by its kind. For a restore: the server answered, and it was NOT made.
    Call(AccountCallError),
    /// A restore only: the request went out and no answer came back that says whether it was made.
    Unconfirmed,
}

/// The local state's refusal, as a refusal of a versions call.
fn local(refusal: LocalStateRefused) -> VersionsRefused {
    match refusal {
        LocalStateRefused::SessionChanged
        | LocalStateRefused::SessionEnded
        | LocalStateRefused::SignedOut
        | LocalStateRefused::Locked => VersionsRefused::NotSignedIn,
        _ => VersionsRefused::NoLocalState,
    }
}

/// The kind of a restore's error: read from its status, never its text.
fn kind_of_engine(error: &EngineError) -> AccountCallError {
    match error {
        EngineError::Api(error) => kind_of(error),
        _ => AccountCallError::ServerError,
    }
}

impl<C: CredentialStores> Engine<C> {
    /// The versions of the file `file_id`, from the server (`GET /api/v1/files/{id}/versions`), for the session a
    /// person pressed under (`session` is [`Engine::session_turn`] as the host read it AT THE PRESS). The id must
    /// be a file of this account's local state, else [`VersionsRefused::NotFound`] and nothing is sent. The list
    /// is handed over only while that session is still the one in memory.
    pub fn file_versions_for(
        &self,
        session: SessionGeneration,
        file_id: &str,
    ) -> Result<FileVersions, VersionsRefused> {
        let state = self.local_state().map_err(local)?;
        if state.session() != session {
            return Err(VersionsRefused::NotSignedIn);
        }
        let is_file = state
            .with(|db| matches!(db.get_file(file_id), Ok(Some(entry)) if !entry.is_dir()))
            .map_err(local)?;
        if !is_file {
            return Err(VersionsRefused::NotFound);
        }
        drop(state);
        let id = file_id.to_string();
        let body = self
            .account_call_for(session, move |api| async move { api.list_versions(&id).await })
            .map_err(VersionsRefused::Call)?;
        let mut versions =
            versions_from_payload(&body).map_err(|_| VersionsRefused::Call(AccountCallError::Unreadable))?;
        // The current one first, then the newest: a stable sort on the date, which the server writes as one
        // timestamp format, so the text order is the time order.
        versions.sort_by(|a, b| b.current.cmp(&a.current).then_with(|| b.created_at.cmp(&a.created_at)));
        if self.acct.while_session_is(&self.writes, session, |_| ()).is_none() {
            return Err(VersionsRefused::NotSignedIn);
        }
        Ok(FileVersions { session, versions })
    }

    /// WRITES: make the version `version_id` of the file `file_id` the current one
    /// (`POST /api/v1/files/{id}/versions/{version}/restore`), for the session a person pressed under and no other.
    /// The request is sent at most once: each restore mints a version, so it is never repeated here. The local
    /// state is asked to take the server's list again after it was made. See the module's doc for what each refusal
    /// says of what the server did.
    pub fn restore_file_version_for(
        &self,
        session: SessionGeneration,
        file_id: &str,
        version_id: &str,
    ) -> Result<(), VersionsRefused> {
        // A session mutex a panic poisoned: nothing is sent.
        if self.acct.session.is_poisoned() {
            return Err(VersionsRefused::NotSignedIn);
        }
        // The token and the key leave the session's lock together, only while the session is the press's, and are
        // copies of their own, wiped when this call ends.
        let (token, key) = self
            .acct
            .while_session_is(&self.writes, session, |current| {
                (current.token.clone(), Zeroizing::new(current.master_key))
            })
            .ok_or(VersionsRefused::NotSignedIn)?;
        let state = self.local_state_for_changes().map_err(local)?;
        if state.session() != session {
            return Err(VersionsRefused::NotSignedIn);
        }
        let is_file = state
            .with(|db| matches!(db.get_file(file_id), Ok(Some(entry)) if !entry.is_dir()))
            .map_err(local)?;
        if !is_file {
            return Err(VersionsRefused::NotFound);
        }
        let sent = state
            .with_shared(|db| {
                let api = Arc::new(ApiClient::new(self.config.api_base.clone(), token, *key));
                let bridge = EngineBridge::new(self.identity, db, api);
                self.runtime
                    .block_on(bridge.restore_version_staged(file_id, version_id))
            })
            // The session moved while the call ran: whatever it answered is not handed over.
            .map_err(local)?;
        match sent {
            Ok(_) => {
                self.request_resnapshot();
                Ok(())
            }
            Err((RestoreSent::No, error)) => Err(VersionsRefused::Call(kind_of_engine(&error))),
            Err((RestoreSent::Yes, error)) => match status_of(&error) {
                // The server answered, and refused: it was not made (a 401: the session ended).
                Some(401) => Err(VersionsRefused::Call(AccountCallError::SessionEnded)),
                Some(400..=499) => Err(VersionsRefused::Call(AccountCallError::ServerError)),
                // A 5xx (a gateway can answer 502 or 504 after the restore was made) or no answer at all.
                _ => Err(VersionsRefused::Unconfirmed),
            },
        }
    }
}

fn status_of(error: &EngineError) -> Option<u16> {
    match error {
        EngineError::Api(error) => error_http_status(error),
        _ => None,
    }
}
