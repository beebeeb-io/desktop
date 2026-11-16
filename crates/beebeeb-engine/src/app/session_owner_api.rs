//! Whose session is this? The backfill of task 2447's owner record for the native engine (review of desktop#424).
//!
//! A session stored before the owner tag (2026-11-11) comes back from a restart with no recorded owner id
//! (`restore` reads the tag, and there is none). The Tauri app records it at its startup probe
//! (`key_owner::settle_owner_after_probe`, called once the server named the account of the stored token); the native
//! engine has no startup probe (`restore` is network-free), so it asks here, at the first moment this run can prove
//! it: Beebeeb's `GET /auth/me` for the token of the session in memory.
//!
//! - The id Beebeeb names is recorded for the session in memory (`session_user_id`), and written as the stored
//!   session's owner tag only while the stored token is still the one asked about (core's rule, as Tauri does). It is
//!   recorded under the session-write lock and only for the session that was asked about; the turn does NOT move
//!   (learning whose session it is changes nothing a call in flight was made for).
//! - The stored key is judged against that id (`settle_owner_after_probe`'s answer). Another account's key (tagged
//!   with another id) is never in memory here: beside an untagged session the start judged it `SessionUnknown` and
//!   did not load it, and an untagged key stays "unknown" after the tag is written. So that answer changes nothing in
//!   memory and is logged (no id); the next start's loader refuses the key. A stored token that is not the one asked
//!   about (`SessionUnknown`) gets no tag; the id is still recorded for the session in memory.
//! - A session that came back with no email is named by the same answer (`name_unnamed_session`), before its id is
//!   recorded: the session in memory, the account label and, while the stored token is the one asked about, the
//!   credential store. The turn does not move for it either. A session whose id is known at its start is not asked
//!   again, so one that has its id but no stored email stays unnamed until its next sign-in.
//! - Asked as a person's call (`person_session`): never under a locked screen, so the first ask is after the unlock.
//!   The session watcher asks at most once a minute while the owner is unknown; a call that needs the owner now (the
//!   two-factor calls) asks at once.
//!
//! BLOCKS on the handle's runtime: call it from a worker thread or the session watcher, never from the UI thread or a
//! task of that runtime. Must not: log an id, an address or a token.

use std::sync::Arc;
use std::time::{Duration, Instant};

use zeroize::Zeroizing;

use super::super::{CredentialStores, Engine};
use crate::signin::key_owner;

/// How often the session watcher asks while the owner is unknown.
const ASK_AGAIN_AFTER: Duration = Duration::from_secs(60);

/// What [`Engine::settle_session_owner`] did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OwnerSettled {
    /// The session's owner was known already: nothing was asked.
    Known,
    /// Beebeeb named the account of this session's token, and it is recorded.
    Recorded,
    /// Nothing recorded: no session in memory or a locked screen, the session moved on while Beebeeb answered,
    /// Beebeeb could not be asked or did not name an account, or the watcher asked less than a minute ago.
    NotNow,
}

impl<C: CredentialStores> Engine<C> {
    fn owner_known(&self) -> bool {
        self.session_user_id
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .is_some()
    }

    /// Whether an ask may start now: the owner is unknown, a session is in memory and the screen is open, and no ask
    /// is out or started less than a minute ago. `true`: the flight is taken (end it with [`Self::end_owner_flight`]).
    fn take_owner_flight(&self) -> bool {
        // Nothing to ask for (no session, a locked screen): the minute is not spent on it.
        if self.owner_known() || self.person_session(|_, _| ()).is_err() {
            return false;
        }
        let mut flight = self
            .acct
            .identify_flight
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if flight.in_flight || flight.last_started.is_some_and(|at| at.elapsed() < ASK_AGAIN_AFTER) {
            return false;
        }
        flight.in_flight = true;
        flight.last_started = Some(Instant::now());
        true
    }

    fn end_owner_flight(&self) {
        self.acct
            .identify_flight
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .in_flight = false;
    }

    /// While the owner of the session in memory is unknown, at most once a minute (and one at a time), Beebeeb is
    /// asked whose session it is. See the module's doc.
    pub fn settle_session_owner(&self) -> OwnerSettled {
        if self.owner_known() {
            return OwnerSettled::Known;
        }
        if !self.take_owner_flight() {
            return OwnerSettled::NotNow;
        }
        let settled = self.settle_session_owner_now();
        self.end_owner_flight();
        settled
    }

    /// The session watcher's ask ([`Self::settle_session_owner`]) on a thread of its own, so a slow answer never holds
    /// back the watcher's other checks. Nothing starts while no ask is due.
    pub(in crate::app) fn settle_session_owner_aside(self: &Arc<Self>) {
        if !self.take_owner_flight() {
            return;
        }
        let engine = Arc::clone(self);
        let spawned = std::thread::Builder::new()
            .name("session-owner".to_string())
            .spawn(move || {
                engine.settle_session_owner_now();
                engine.end_owner_flight();
            });
        if spawned.is_err() {
            self.end_owner_flight();
        }
    }

    /// [`Self::settle_session_owner`] at once, for a call that needs the owner now.
    pub(in crate::app) fn settle_session_owner_now(&self) -> OwnerSettled {
        if self.owner_known() {
            return OwnerSettled::Known;
        }
        let Ok((token, turn)) = self.person_session(|session, turn| (Zeroizing::new(session.token.clone()), turn))
        else {
            return OwnerSettled::NotNow;
        };
        let profile =
            match self
                .runtime
                .block_on(super::super::fetch_profile(&self.client, &self.config.api_base, &token))
            {
                Ok(profile) => profile,
                Err(_) => return OwnerSettled::NotNow,
            };
        let user_id = profile.user_id.trim();
        if user_id.is_empty() {
            return OwnerSettled::NotNow;
        }
        // With no session change half done, and only for the session that was asked about. The turn stays.
        let _writes = self.writes.write().unwrap_or_else(|poisoned| poisoned.into_inner());
        if !self.acct.session_unchanged_since(turn) {
            return OwnerSettled::NotNow;
        }
        match key_owner::settle_owner_after_probe(&self.vault(), token.as_str(), user_id) {
            // The stored token is not the one asked about (another, or none): no tag is written. The id is still the
            // owner of the session in memory, whose token Beebeeb answered for.
            Ok(key_owner::KeyOwnership::SessionUnknown) => {}
            // A tagged key of another account: one the start never loaded (beside an untagged session it was
            // `SessionUnknown`), so nothing in memory changes; the next start's loader refuses it.
            Ok(ownership) if !ownership.may_load() => {
                tracing::warn!("the stored vault key is not this session's account's; the next start won't load it");
            }
            Ok(_) => {}
            Err(error) => tracing::warn!(%error, "could not record the session's owner in the credential store"),
        }
        // The email first, then the id: a window that reads "the email, else the id" goes from neither to the email.
        self.name_unnamed_session(token.as_str(), profile.email.trim());
        self.remember_session_user_id(Some(user_id.to_string()));
        OwnerSettled::Recorded
    }

    /// A session that came back with no email (none was stored beside it) is named after Beebeeb's answer for its own
    /// token, as a sign-in names it: the session in memory, the account label, and the credential store while it still
    /// holds that token. A session that has an email keeps it (a change of address is `email_changed_for`'s), and an
    /// answer that names no email names nothing. Called under the session-write lock, for the session asked about.
    fn name_unnamed_session(&self, token: &str, email: &str) {
        if email.is_empty() {
            return;
        }
        {
            let mut session = self
                .acct
                .session
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            match session.as_mut() {
                Some(held) if held.token == token && held.email.is_none() => held.email = Some(email.to_string()),
                _ => return,
            }
        }
        if let Ok(mut label) = self.acct.auth_email.lock() {
            label.get_or_insert_with(|| email.to_string());
        }
        let vault = self.vault();
        let stored_is_asked = vault
            .session_token()
            .ok()
            .flatten()
            .is_some_and(|stored| stored.expose_for_request() == token);
        if stored_is_asked
            && vault.account_email().ok().flatten().is_none()
            && let Err(error) = vault.store_account_email(email)
        {
            tracing::warn!(%error, "could not store the account email (the session stays unnamed until the next start)");
        }
    }
}

#[cfg(test)]
#[path = "session_owner_api_tests.rs"]
mod tests;
