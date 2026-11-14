//! The account's passkeys, for a host without a UI toolkit (the native Passkeys pane and the sign-in screen's
//! "Use a passkey"): list them, add one, remove one, and sign in with one. Every rule is core's
//! (`beebeeb_core::passkey::flow`); this file only moves bytes (the HTTP of [`PasskeyHttp`]) and binds each call to a
//! session. The platform call (webauthn.dll on Windows) is a [`PasskeyAuthenticator`] the host's thread runs: the
//! `*_with` methods take any one, the Windows ones are [`Engine::add_passkey`], [`Engine::remove_passkey`] and
//! [`Engine::sign_in_passkey`] (through `crate::passkey_windows`).
//!
//! Every method BLOCKS on the handle's runtime: call it from a worker thread, never from the UI thread. A passkey
//! ceremony shows the system's own dialog and can take a minute: give it a thread of its own.
//!
//! Session rule: a change (add, remove) is sent only for the session the host read its list under (`listed_under`,
//! [`Stamped::session`]): another session sends nothing ([`PasskeyProblem::NotSignedIn`]). The token and the vault
//! key leave the session's lock together, for that session only, and wipe when the call ends. The relying party is
//! this build's own, never the server's answer.
//!
//! Must not: log a password, a token, a key, a passkey's name or id, or a server's words; hand the PRF output, the
//! wrap key or the escrow blob to anything (core keeps them); sign in over a session that is already there.

use std::future::Future;

use beebeeb_core::kdf::MasterKey;
use beebeeb_core::passkey::ceremony::{ExpectedRp, PasskeyError};
use beebeeb_core::passkey::flow::{
    self, ApiRequest, ApiResponse, Method, PasskeyAuthenticator, PasskeyHttp, VaultOutcome,
};
use zeroize::Zeroizing;

use super::account_api::Stamped;
use super::{CredentialStores, Engine};
use crate::account::SessionGeneration;
use crate::keychain::SecretBytes;
use crate::session::Session;
use crate::signin::gate::NextAfterSignIn;
use crate::signin::key_owner;

/// The production relying party (the server's `WEBAUTHN_RP_ID`).
const PRODUCTION_RP_ID: &str = "beebeeb.io";

/// The longest name a passkey gets (the same bound the web client uses).
pub const NAME_MAX: usize = 64;

/// One passkey of the account, as the server lists it.
#[derive(Clone, PartialEq, Eq)]
pub struct PasskeyRow {
    /// The server's id: what a removal names.
    pub id: String,
    pub name: String,
    pub created_at: String,
    /// `None`: the server does not say whether this passkey can unlock the vault.
    pub prf: Option<bool>,
}

// By hand: a passkey's name is the person's own words.
impl std::fmt::Debug for PasskeyRow {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PasskeyRow").finish_non_exhaustive()
    }
}

/// Why a passkey call did not do what it was asked. Fixed words only.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PasskeyProblem {
    /// No session in memory (signed out or locked), or the session of the press is gone: nothing was sent.
    NotSignedIn,
    /// Beebeeb no longer accepts this session (401).
    SessionEnded,
    /// Beebeeb could not be reached.
    Offline,
    /// The password was not the account's.
    WrongPassword,
    /// Too many tries: Beebeeb asks to wait.
    RateLimited,
    /// No password was given where one is needed, or the name is empty or too long.
    InvalidInput,
    /// The person closed the Windows Security dialog, or it timed out.
    Cancelled,
    /// This computer has no passkey support (or this build has no platform call).
    Unavailable,
    /// Beebeeb refused it, or answered something that does not read.
    Refused,
    /// The platform call failed for another reason.
    Platform,
}

/// What adding a passkey did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PasskeyAdded {
    /// It has PRF and its vault key escrow was stored: it can unlock the data on a new device.
    pub can_unlock: bool,
}

/// The production relying party, or the development one in a debug build built for it.
fn expected_rp() -> ExpectedRp {
    ExpectedRp::new(rp_id_for(cfg!(debug_assertions), option_env!("BB_PASSKEY_RP_ID")))
}

fn rp_id_for(debug_build: bool, build_env: Option<&str>) -> &'static str {
    match build_env.map(str::trim) {
        Some("localhost") if debug_build => "localhost",
        _ => PRODUCTION_RP_ID,
    }
}

/// Core's error as a kind. A server's words never come this far.
pub(crate) fn problem_of(error: &PasskeyError) -> PasskeyProblem {
    match error {
        PasskeyError::Cancelled => PasskeyProblem::Cancelled,
        PasskeyError::Unavailable => PasskeyProblem::Unavailable,
        PasskeyError::Platform { .. } => PasskeyProblem::Platform,
        PasskeyError::Network => PasskeyProblem::Offline,
        PasskeyError::Unauthorized => PasskeyProblem::SessionEnded,
        PasskeyError::StepUp { code } => match *code {
            "invalid_credentials" => PasskeyProblem::WrongPassword,
            "rate_limited" => PasskeyProblem::RateLimited,
            "invalid_input" => PasskeyProblem::InvalidInput,
            "network" | "opaque_unavailable" => PasskeyProblem::Offline,
            _ => PasskeyProblem::Refused,
        },
        PasskeyError::Server { status: 429, .. } => PasskeyProblem::RateLimited,
        PasskeyError::Server { status: 503, .. } => PasskeyProblem::Offline,
        _ => PasskeyProblem::Refused,
    }
}

/// The client's HTTP for core's flows: this handle's client and API base.
struct EngineHttp {
    client: reqwest::Client,
    base: String,
}

impl PasskeyHttp for EngineHttp {
    fn send(&self, r: ApiRequest) -> impl Future<Output = Result<ApiResponse, PasskeyError>> + Send {
        let client = self.client.clone();
        let url = format!("{}{}", self.base, r.path);
        async move {
            let mut req = match r.method {
                Method::Get => client.get(&url),
                Method::Post => client.post(&url),
                Method::Delete => client.delete(&url),
            };
            if let Some(token) = &r.bearer {
                req = req.bearer_auth(token.as_str());
            }
            if let Some(confirm) = &r.confirm_token {
                req = req.header("X-Confirm-Token", confirm.as_str());
            }
            if let Some(body) = &r.body {
                req = req.json(body);
            }
            let response = req.send().await.map_err(|_| PasskeyError::Network)?;
            let status = response.status().as_u16();
            let body = response
                .json::<serde_json::Value>()
                .await
                .unwrap_or(serde_json::Value::Null);
            Ok(ApiResponse { status, body })
        }
    }
}

/// A name for a passkey: trimmed, one to [`NAME_MAX`] characters.
pub fn clean_name(name: &str) -> Option<String> {
    let name = name.trim();
    (!name.is_empty() && name.chars().count() <= NAME_MAX).then(|| name.to_string())
}

impl<C: CredentialStores> Engine<C> {
    fn passkey_http(&self) -> EngineHttp {
        EngineHttp {
            client: self.client.clone(),
            base: self.config.api_base.clone(),
        }
    }

    /// How a call ended, for the engine's run of 401s: only a 401 advances it, an answer clears it. Only while the
    /// session in memory is still `session`.
    fn note_passkey_call(&self, session: SessionGeneration, ended: Option<PasskeyProblem>) {
        let failed_as = match ended {
            None => None,
            Some(PasskeyProblem::SessionEnded) => Some(super::settings_api::AccountCallError::SessionEnded),
            // Any other failure leaves the run as it is.
            Some(_) => return,
        };
        self.note_for_session(session, failed_as);
    }

    /// The token (and the vault key, when `with_key`) of the session `session` names, copied together, and only
    /// while that is the session in memory, asked with no session change half done. A session Beebeeb no longer
    /// accepts: `SessionEnded`, nothing is asked.
    fn passkey_secrets(
        &self,
        session: SessionGeneration,
        with_key: bool,
    ) -> Result<(Zeroizing<String>, Option<Zeroizing<[u8; 32]>>), PasskeyProblem> {
        if self.acct.session.is_poisoned() {
            return Err(PasskeyProblem::NotSignedIn);
        }
        let copied = self
            .acct
            .while_session_is(&self.writes, session, |current| {
                (
                    Zeroizing::new(current.token.clone()),
                    with_key.then(|| Zeroizing::new(current.master_key)),
                )
            })
            .ok_or(PasskeyProblem::NotSignedIn)?;
        if self.acct.auth_health.is_expired() {
            return Err(PasskeyProblem::SessionEnded);
        }
        Ok(copied)
    }

    /// The account's passkeys (`GET /api/v1/auth/passkeys`), for the session in memory, handed over only while it
    /// is still that one.
    pub fn passkeys(&self) -> Result<Stamped<Vec<PasskeyRow>>, PasskeyProblem> {
        let session = self.acct.session_generation();
        let (token, _) = self.passkey_secrets(session, false)?;
        let http = self.passkey_http();
        let answer = self
            .runtime
            .block_on(flow::list_passkeys(&http, &token))
            .map(|rows| {
                rows.into_iter()
                    .map(|row| PasskeyRow {
                        id: row.id,
                        name: row.name,
                        created_at: row.created_at,
                        prf: row.prf,
                    })
                    .collect::<Vec<_>>()
            })
            .map_err(|error| problem_of(&error));
        self.note_passkey_call(session, answer.as_ref().err().copied());
        if !self.acct.session_unchanged_since(session) {
            return Err(PasskeyProblem::NotSignedIn);
        }
        answer.map(|value| Stamped { session, value })
    }

    /// WRITES: add a passkey over `authenticator` (core's flow: the step-up from `password`, the platform dialog,
    /// the registration, and the vault key escrow when the passkey has PRF). `listed_under`: the session the host
    /// read its list under; another session sends nothing. A new session's list is read again by the caller.
    pub fn add_passkey_with<A: PasskeyAuthenticator>(
        &self,
        authenticator: &A,
        listed_under: SessionGeneration,
        password: Zeroizing<String>,
        name: &str,
    ) -> Result<PasskeyAdded, PasskeyProblem> {
        let name = clean_name(name).ok_or(PasskeyProblem::InvalidInput)?;
        if password.is_empty() {
            return Err(PasskeyProblem::InvalidInput);
        }
        let (token, key) = self.passkey_secrets(listed_under, true)?;
        let key = key.ok_or(PasskeyProblem::NotSignedIn)?;
        let vault_key = MasterKey::from_bytes(*key);
        drop(key);
        let http = self.passkey_http();
        let answer = self
            .runtime
            .block_on(flow::add_passkey(
                &http,
                authenticator,
                &token,
                password.as_bytes(),
                Some(&name),
                Some(&vault_key),
                &expected_rp(),
            ))
            .map(|added| PasskeyAdded {
                can_unlock: added.prf && added.escrowed,
            })
            .map_err(|error| problem_of(&error));
        self.note_passkey_call(listed_under, answer.as_ref().err().copied());
        answer
    }

    /// WRITES: remove a passkey (core's flow: the step-up from `password`, its token spent on the one `DELETE`).
    /// `listed_under` as for [`Engine::add_passkey_with`]. The copy in the person's password manager stays.
    pub fn remove_passkey(
        &self,
        listed_under: SessionGeneration,
        password: Zeroizing<String>,
        id: &str,
    ) -> Result<(), PasskeyProblem> {
        if password.is_empty() {
            return Err(PasskeyProblem::InvalidInput);
        }
        let (token, _) = self.passkey_secrets(listed_under, false)?;
        let http = self.passkey_http();
        let answer = self
            .runtime
            .block_on(flow::remove_passkey(&http, &token, password.as_bytes(), id))
            .map_err(|error| problem_of(&error));
        self.note_passkey_call(listed_under, answer.as_ref().err().copied());
        answer
    }

    /// Sign in with a passkey for `email` over `authenticator`: core proves the account (a challenge only the
    /// passkey answers), the new session is stored as a password sign-in stores it, and a vault key the passkey's
    /// PRF opened is stored with it. A passkey without PRF (or without an escrow) signs in but opens nothing:
    /// the answer is then what the gate asks (the key of this computer, or the twelve words).
    ///
    /// Does not hold the engine's step lock while the dialog is up: the ceremony can take a minute. The result is
    /// stored only if no session came into being meanwhile; else the new one is ended and nothing is kept.
    pub fn sign_in_passkey_with<A: PasskeyAuthenticator>(
        &self,
        authenticator: &A,
        email: &str,
    ) -> Result<NextAfterSignIn, PasskeyProblem> {
        let result = self.sign_in_passkey_step(authenticator, email);
        self.publish();
        result
    }

    fn sign_in_passkey_step<A: PasskeyAuthenticator>(
        &self,
        authenticator: &A,
        email: &str,
    ) -> Result<NextAfterSignIn, PasskeyProblem> {
        let email = crate::account_binding::canonical_email(email);
        if email.trim().is_empty() {
            return Err(PasskeyProblem::InvalidInput);
        }
        let turn = self.acct.session_generation();
        if self.auth_present.lock().map(|present| *present).unwrap_or(true) {
            // One account at a time: sign out first.
            return Err(PasskeyProblem::Refused);
        }
        let http = self.passkey_http();
        let signed = self
            .runtime
            .block_on(flow::sign_in(&http, authenticator, &email, &expected_rp()))
            .map_err(|error| problem_of(&error))?;
        let (token, vault) = (signed.session_token, signed.vault);
        self.runtime.block_on(async {
            let _step = self.transition.lock().await;
            let present = self.auth_present.lock().map(|present| *present).unwrap_or(true);
            if present || !self.acct.session_unchanged_since(turn) {
                super::revoke(&self.client, &self.config.api_base, &token).await;
                return Err(PasskeyProblem::Refused);
            }
            let next = match self
                .finish_sign_in(turn, Zeroizing::new(token.to_string()), email)
                .await
            {
                Ok(next) => next,
                Err(_) => return Err(PasskeyProblem::Refused),
            };
            if let VaultOutcome::Unlocked(key) = vault {
                self.keep_passkey_key(&token, key);
                return Ok(crate::signin::gate::next_after_sign_in(
                    false,
                    self.unlocked(),
                    false,
                    false,
                ));
            }
            Ok(next)
        })
    }

    /// The vault key a passkey's PRF opened, stored for the session that was just made, as the twelve words' key
    /// is (`unlock_with_phrase`). Nothing is stored when the vault is already open (a key of this computer was
    /// loaded) or the session moved on; the key is then simply dropped.
    fn keep_passkey_key(&self, token: &str, key: MasterKey) {
        if self.unlocked() {
            return;
        }
        let key = Zeroizing::new(key.to_bytes());
        let turn = self.acct.session_generation();
        let Ok(_write) = self.claim(turn) else {
            return;
        };
        let vault = self.vault();
        let still = vault
            .session_token()
            .ok()
            .flatten()
            .is_some_and(|stored| stored.expose_for_request() == token);
        if !still {
            return;
        }
        if vault
            .store_wrapped_master_key(SecretBytes::new_master_key(&key))
            .is_err()
        {
            return;
        }
        if let Err(error) = key_owner::bind_key_to_session_owner(&vault) {
            tracing::warn!(%error, "could not record the vault key's owner in the credential store");
        }
        let email = self.account_email();
        if let Ok(mut guard) = self.acct.session.lock() {
            *guard = Some(Session {
                token: token.to_string(),
                master_key: *key,
                email,
            });
            self.acct
                .auth_resume_asked
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        }
    }

    /// Whether this computer can show the system's passkey dialog at all (Windows with the WebAuthn API that does
    /// PRF). In memory and quick: a host may ask it on every draw.
    pub fn passkeys_supported(&self) -> bool {
        platform_supported()
    }
}

#[cfg(target_os = "windows")]
fn platform_supported() -> bool {
    crate::passkey_windows::platform_available()
}

#[cfg(not(target_os = "windows"))]
fn platform_supported() -> bool {
    false
}

#[cfg(target_os = "windows")]
impl<C: CredentialStores> Engine<C> {
    /// [`Engine::add_passkey_with`] over the Windows WebAuthn dialog, owned by the window `hwnd` (a `HWND` as a
    /// number).
    pub fn add_passkey(
        &self,
        hwnd: isize,
        listed_under: SessionGeneration,
        password: Zeroizing<String>,
        name: &str,
    ) -> Result<PasskeyAdded, PasskeyProblem> {
        self.add_passkey_with(
            &crate::passkey_windows::WindowsAuthenticator::new(hwnd),
            listed_under,
            password,
            name,
        )
    }

    /// [`Engine::sign_in_passkey_with`] over the Windows WebAuthn dialog, owned by the window `hwnd`.
    pub fn sign_in_passkey(&self, hwnd: isize, email: &str) -> Result<NextAfterSignIn, PasskeyProblem> {
        self.sign_in_passkey_with(&crate::passkey_windows::WindowsAuthenticator::new(hwnd), email)
    }
}

#[cfg(test)]
#[path = "passkeys_api_tests.rs"]
mod tests;
