//! The account's own settings on Beebeeb, for a host without a UI toolkit (the native Settings panes): where new
//! files are stored, the account's notifications, and which alerts the account sends.
//!
//! Every method BLOCKS on the handle's runtime, as every method of the handle does: call it from a worker thread,
//! never from the UI thread (task 2470 W5).
//!
//! One path for all of them ([`Engine::account_call`]): the session's token moves into an [`ApiClient`] of its own
//! for that one call, and the client wipes it when the call is done. The vault key is not handed over: none of these
//! calls decrypts anything. A failure is an [`AccountCallError`], a kind with fixed words: never a server's body, an
//! address, a token or reqwest's own text.
//!
//! Writes account state: [`Engine::set_storage_region`], [`Engine::set_notification_preferences`] and
//! [`Engine::mark_notifications_read`] (and the two `_for` forms of the last two, which refuse when the session is
//! no longer the one the host names). The others only read. Must not: log anything a call sends or receives.

use std::future::Future;

use super::{CredentialStores, Engine};
use crate::account::SessionGeneration;
use crate::account_dto::{
    NotificationList, NotificationPreferences, NotificationPreferencesUpdate, SetPreferredRegionResponse,
    UserRegionResponse,
};
use crate::api_client::{ApiClient, DesktopApiError};

/// Why a call about the account's settings did not answer. Fixed words only.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AccountCallError {
    /// No session in memory: signed out, or locked.
    NotSignedIn,
    /// Beebeeb could not be reached, or did not answer in time.
    Offline,
    /// Beebeeb no longer accepts this session (it answered 401).
    SessionEnded,
    /// Beebeeb answered with an error.
    ServerError,
    /// Beebeeb's answer did not read as the expected shape.
    Unreadable,
}

impl AccountCallError {
    /// The kind in a few words, for a log line or a host's own sentence.
    pub fn words(self) -> &'static str {
        match self {
            Self::NotSignedIn => "not signed in",
            Self::Offline => "offline",
            Self::SessionEnded => "the session ended",
            Self::ServerError => "the server answered with an error",
            Self::Unreadable => "the server's answer could not be read",
        }
    }
}

impl std::fmt::Display for AccountCallError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.words())
    }
}

impl std::error::Error for AccountCallError {}

impl<C: CredentialStores> Engine<C> {
    /// Where the account stores new files, and the places it can choose from (`GET /api/v1/me/region`). By country,
    /// never a city (`account_dto::region`).
    pub fn storage_region(&self) -> Result<UserRegionResponse, AccountCallError> {
        self.account_call(|api| async move { api.account_region().await })
    }

    /// WRITES: store where new files go (`PUT /api/v1/me/region`): `preferred_region` is a choice's id as the list
    /// names it (`europe`, a country), `region_strict` is "only store in this country". The server replaces the
    /// whole choice, so a caller sends both.
    pub fn set_storage_region(
        &self,
        preferred_region: Option<&str>,
        region_strict: Option<bool>,
    ) -> Result<SetPreferredRegionResponse, AccountCallError> {
        let preferred_region = preferred_region.map(str::to_string);
        self.account_call(move |api| async move {
            api.set_preferred_region(preferred_region.as_deref(), region_strict)
                .await
        })
    }

    /// The account's notifications, newest first, and how many are unread (`GET /api/v1/notifications`).
    pub fn notifications(&self) -> Result<NotificationList, AccountCallError> {
        self.account_call(|api| async move { api.notifications().await })
    }

    /// Which alerts the account sends (`GET /api/v1/notifications/preferences`).
    pub fn notification_preferences(&self) -> Result<NotificationPreferences, AccountCallError> {
        self.account_call(|api| async move { api.notification_preferences().await })
    }

    /// WRITES: change some of the account's alert switches (`PUT /api/v1/notifications/preferences`); a switch
    /// `update` leaves out keeps its value. The answer is the server's whole set after the change.
    pub fn set_notification_preferences(
        &self,
        update: &NotificationPreferencesUpdate,
    ) -> Result<NotificationPreferences, AccountCallError> {
        let update = update.clone();
        self.account_call(move |api| async move { api.update_notification_preferences(&update).await })
    }

    /// WRITES: [`Engine::set_notification_preferences`], for the session a person pressed under and no other.
    /// `session` is [`Engine::session_turn`] as the host read it AT THE PRESS (on its window's thread, before any
    /// worker started). A session that moved on since (a lock, a sign-out, another sign-in, even back to the same
    /// account): [`AccountCallError::NotSignedIn`], and nothing is sent.
    pub fn set_notification_preferences_for(
        &self,
        session: SessionGeneration,
        update: &NotificationPreferencesUpdate,
    ) -> Result<NotificationPreferences, AccountCallError> {
        let update = update.clone();
        self.account_call_for(session, move |api| async move {
            api.update_notification_preferences(&update).await
        })
    }

    /// WRITES: [`Engine::mark_notifications_read`], for the session whose page showed these ids and no other
    /// (`session` as in [`Engine::set_notification_preferences_for`]).
    pub fn mark_notifications_read_for(
        &self,
        session: SessionGeneration,
        ids: &[String],
    ) -> Result<(), AccountCallError> {
        let ids = ids.to_vec();
        self.account_call_for(session, move |api| async move {
            for id in &ids {
                api.mark_notification_read(id).await?;
            }
            Ok(())
        })
    }

    /// WRITES: these notifications have been seen (`PATCH /api/v1/notifications/{id}`, one call each, in order); one
    /// the server no longer has counts as seen. Stops at the first that fails: the ones before it are marked.
    pub fn mark_notifications_read(&self, ids: &[String]) -> Result<(), AccountCallError> {
        let ids = ids.to_vec();
        self.account_call(move |api| async move {
            for id in &ids {
                api.mark_notification_read(id).await?;
            }
            Ok(())
        })
    }

    /// The one path of this file's calls: the session's token into a client of its own (wiped when the client is
    /// dropped, at the end of the call), the call on the handle's runtime against the handle's API base, and a
    /// failure as its kind. No session in memory: nothing is asked.
    fn account_call<T, F, Fut>(&self, call: F) -> Result<T, AccountCallError>
    where
        F: FnOnce(ApiClient) -> Fut,
        Fut: Future<Output = anyhow::Result<T>>,
    {
        let token = self.session_token().map_err(|_| AccountCallError::NotSignedIn)?;
        // No vault key: these calls decrypt nothing, so the client is given none.
        let api = ApiClient::new(self.config.api_base.clone(), token, [0; 32]);
        self.runtime.block_on(call(api)).map_err(|error| kind_of(&error))
    }

    /// [`Engine::account_call`] for one session only: the token is copied only while the session in memory is the
    /// one `session` names, asked with no session change half done (the session-write lock is held for the question
    /// and the copy, as `session_is_still` in `trash_api` holds it), so the token of a later session never carries a
    /// change chosen under an earlier one. Never waits on a session change: while one is under way the answer is
    /// `NotSignedIn`, and nothing is sent.
    fn account_call_for<T, F, Fut>(&self, session: SessionGeneration, call: F) -> Result<T, AccountCallError>
    where
        F: FnOnce(ApiClient) -> Fut,
        Fut: Future<Output = anyhow::Result<T>>,
    {
        let token = {
            let _writes = match self.writes.try_read() {
                Ok(writes) => writes,
                Err(std::sync::TryLockError::Poisoned(poisoned)) => poisoned.into_inner(),
                Err(std::sync::TryLockError::WouldBlock) => return Err(AccountCallError::NotSignedIn),
            };
            if !self.acct.session_unchanged_since(session) {
                return Err(AccountCallError::NotSignedIn);
            }
            self.session_token().map_err(|_| AccountCallError::NotSignedIn)?
        };
        let api = ApiClient::new(self.config.api_base.clone(), token, [0; 32]);
        self.runtime.block_on(call(api)).map_err(|error| kind_of(&error))
    }
}

/// The kind of a failed call, read from the error's chain (never its text). Also the Trash listing's
/// (`app::trash_api`).
pub(super) fn kind_of(error: &anyhow::Error) -> AccountCallError {
    for cause in error.chain() {
        if let Some(api) = cause.downcast_ref::<DesktopApiError>() {
            return match api {
                DesktopApiError::HttpStatus { status, .. } if status.as_u16() == 401 => AccountCallError::SessionEnded,
                DesktopApiError::HttpStatus { .. } => AccountCallError::ServerError,
                DesktopApiError::Transport(error) => transport_kind(error),
                _ => AccountCallError::ServerError,
            };
        }
        if let Some(error) = cause.downcast_ref::<reqwest::Error>() {
            return transport_kind(error);
        }
        if cause.downcast_ref::<serde_json::Error>().is_some() {
            return AccountCallError::Unreadable;
        }
    }
    AccountCallError::ServerError
}

fn transport_kind(error: &reqwest::Error) -> AccountCallError {
    match error.status() {
        Some(status) if status.as_u16() == 401 => AccountCallError::SessionEnded,
        Some(_) => AccountCallError::ServerError,
        None if error.is_decode() || error.is_body() => AccountCallError::Unreadable,
        None => AccountCallError::Offline,
    }
}
