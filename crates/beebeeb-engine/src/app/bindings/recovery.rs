//! Slice K6 (plan40 batch 2, A12): recovering the account with the twelve words and setting a new password, in the
//! app (DW-A45 to DW-A50). Beebeeb cannot reset a password: the words re-derive the same account key on this
//! computer, the server checks a value derived from it, and only the account's OPAQUE password file changes.
//!
//! Every rule is core's `recovery_flow::PhraseRecovery` (sans-IO, the one the web runs): this module sends the
//! requests it hands out, reads the answers back into it, and maps core's codes to the kinds a screen draws. The
//! server's numbers (the password minimum, the breach check) come from its onboarding document, read once per
//! recovery, never hard-coded. It adds no cryptography.
//!
//! Two steps, each session-bound like its neighbours (`*_for(turn)`): asked FOR the session turn the host read at
//! its press, and refused (`Overtaken`) when that turn is no longer the one in memory.
//! 1. [`Engine::recovery_verify_for`]: the email and the words. A word that is not in the word list is marked and
//!    NOTHING is sent. Otherwise core derives the key here and only the recovery check leaves this computer; the
//!    flow core then holds (the key, the server's recovery token) is kept by THIS engine, never handed to the host.
//! 2. [`Engine::recovery_set_password_for`]: the new password, by sign-up's rules (the minimum, then the breach
//!    check), the OPAQUE re-registration and the finalize. `SignedIn`: the session and the key are stored through
//!    the password sign-in's own store (`store_first_sign_in`), the vault opens. `SignInRequired` (two-factor on):
//!    the password changed, nothing is stored, the person signs in normally next.
//!
//! The held flow: one per engine, one at a time. A new verify replaces it; a stale turn, a refused recovery token
//! and the last step (which consumes it) drop it; [`Engine::recovery_forget`] drops it on demand. It is kept in a
//! slot of this module, keyed by the engine's account runtime (held weakly), because the handle has no field for
//! it; it is taken out of the slot for one step and never held across a request by the slot's lock.
//!
//! Every method BLOCKS on the handle's runtime: call it from a worker thread, never from the UI thread, except
//! [`Engine::recovery_password_verdict`], which never waits on the network or on a step.
//!
//! Secrets: the phrase, the password, the recovery token, the session token and the key are `Zeroizing` (core's
//! buffers, or this module's) and are wiped as they drop. Must not: log or format a word, a password, a token, a
//! key, an email or a server's body. Only kinds leave this module.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, Weak};

use beebeeb_core::auth_flow::HttpResponse;
use beebeeb_core::onboarding::PasswordHint;
use beebeeb_core::recovery_flow::{PhraseRecovery, Recovered, unknown_word_positions};
use beebeeb_core::signup_flow::{SignupError, SignupPolicy};
use zeroize::Zeroizing;

use crate::account::{AccountRuntime, SessionGeneration};
use crate::app::lock_api::SessionMoved;
use crate::app::signup_api::{SignupHint, SignupVerdict};
use crate::app::{CredentialStores, Engine};
use crate::auth_transport;
use crate::keychain::SecretBytes;
use crate::session::Session;
use crate::signin::{key_owner, store};

/// The most of a breach answer that is read (a range answer is a few hundred lines of a few dozen bytes).
const BREACH_BODY_CAP: usize = 4 * 1024 * 1024;

/// Why a recovery step did not go on. Kinds only: never a word, a password, an address or a server's body.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RecoveryRefused {
    /// The server did not accept these words for this address, or they are not twelve valid words (a checksum).
    /// Nothing changed, and the password is as it was.
    WrongPhrase,
    /// The words at these positions (counted from zero) are not in the word list. Nothing was sent.
    UnknownWords(Vec<u32>),
    /// Shorter than the server's minimum. Nothing was sent; the words stay verified.
    Weak,
    /// Found in a list of leaked passwords. The password was not set; the words stay verified.
    Breached,
    /// No answer from Beebeeb (or the breach check could not be made and the server does not allow going on
    /// without it).
    Offline,
    /// Too many tries: the server's wait, in seconds.
    RateLimited {
        seconds: u32,
    },
    /// The session on this computer changed since the press, the recovery was left or ended (a new verify, a
    /// refused recovery token, the last step's answer), or no recovery is held. Nothing was stored: start again
    /// from the words.
    Overtaken,
    Other,
}

/// How a recovery ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecoveryDone {
    /// Signed in with the new password on this computer, and the vault is open.
    SignedIn,
    /// The password was changed, and the server asks for a normal sign-in next (two-factor on, or an older server).
    /// Nothing was stored.
    SignInRequired,
}

/// One held recovery: the flow core runs, for ONE engine, in the session turn its words were verified in.
struct Held {
    /// The engine whose recovery this is: its account runtime, held weakly. A live `Weak` keeps the allocation, so no
    /// other engine can have this address while the entry exists; a dead one marks an engine that was dropped.
    owner: Weak<AccountRuntime>,
    /// Which verify made it: a step that took the flow out puts it back only into the entry it took it from.
    id: u64,
    turn: SessionGeneration,
    /// The server's numbers, read once at the verify: the meter and the password rules use them.
    policy: SignupPolicy,
    /// The address as it was typed (core canonicalises its own copy), for a profile that names none.
    email: Zeroizing<String>,
    /// `None` while a step has the flow out.
    flow: Option<PhraseRecovery>,
}

/// Every engine's held recovery. Locked only for one synchronous step, never across a request or a key derivation.
static HELD: Mutex<Vec<Held>> = Mutex::new(Vec::new());

/// The id of the next verify's entry.
static NEXT_ID: AtomicU64 = AtomicU64::new(1);

/// The slot, with the entries of dropped engines removed (their flows are wiped as they drop).
fn held() -> MutexGuard<'static, Vec<Held>> {
    let mut entries = HELD.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    entries.retain(|entry| entry.owner.strong_count() > 0);
    entries
}

/// Whether the slot holds a recovery of the engine whose account runtime `owner` points at, read WITHOUT dropping the
/// entries of dropped engines first (the wipe tests: what an engine's drop leaves behind).
#[cfg(test)]
fn held_for(owner: &Weak<AccountRuntime>) -> bool {
    HELD.lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .iter()
        .any(|entry| entry.owner.ptr_eq(owner))
}

fn seconds_of(retry_after: u64) -> u32 {
    u32::try_from(retry_after).unwrap_or(u32::MAX)
}

/// Core's codes, as the kinds a screen draws.
fn refused(error: SignupError) -> RecoveryRefused {
    match error {
        // An address that cannot be anyone's is answered as words that do not open it (the server answers an address
        // nobody has the same way).
        SignupError::InvalidEmail | SignupError::InvalidPhrase | SignupError::PhraseNotAccepted => {
            RecoveryRefused::WrongPhrase
        }
        SignupError::PasswordTooShort { .. } => RecoveryRefused::Weak,
        SignupError::PasswordBreached => RecoveryRefused::Breached,
        SignupError::BreachCheckBlocked | SignupError::BreachCheckMissing => RecoveryRefused::Offline,
        SignupError::RateLimited { retry_after_secs } => RecoveryRefused::RateLimited {
            seconds: seconds_of(retry_after_secs),
        },
        SignupError::RecoveryExpired => RecoveryRefused::Overtaken,
        _ => RecoveryRefused::Other,
    }
}

/// Core's evaluator, in the sign-up verdict's shape (the same numbers and hints the sign-up meter draws).
fn verdict_of(policy: &SignupPolicy, password: &str) -> SignupVerdict {
    let eval = beebeeb_core::signup_flow::evaluate(policy, password);
    SignupVerdict {
        level: eval.strength.level(),
        meets_minimum: eval.meets_minimum,
        min_length: eval.min_length,
        missing: eval.missing_characters,
        hint: match eval.hint {
            PasswordHint::None => SignupHint::None,
            PasswordHint::NeedMoreCharacters => SignupHint::MoreCharacters,
            PasswordHint::MixCaseAndAddNumberOrSymbol => SignupHint::MixCaseAndNumberOrSymbol,
            PasswordHint::MixCase => SignupHint::MixCase,
            PasswordHint::AddNumberOrSymbol => SignupHint::NumberOrSymbol,
        },
    }
}

/// What a set-password step took out of the slot.
struct Taken {
    id: u64,
    policy: SignupPolicy,
    email: Zeroizing<String>,
    flow: PhraseRecovery,
}

impl<C: CredentialStores> Engine<C> {
    fn owns_recovery(&self, entry: &Held) -> bool {
        std::ptr::eq(entry.owner.as_ptr(), Arc::as_ptr(&self.acct))
    }

    /// The press's session is still the one in memory, and nobody is signed in (recovery starts signed out: one
    /// account at a time, as sign-in and sign-up).
    fn recovery_turn_holds(&self, turn: SessionGeneration) -> bool {
        self.acct.session_unchanged_since(turn) && !self.auth_present.lock().map(|present| *present).unwrap_or(true)
    }

    /// Drop this engine's held recovery: the key, the recovery token and anything else it held are wiped. A step
    /// that has the flow out finds the recovery gone and stores nothing. Never waits on the network.
    pub fn recovery_forget(&self) {
        held().retain(|entry| !self.owns_recovery(entry));
    }

    /// Step 1 (DW-A47): check the twelve words for `email`, asked for the session turn `turn` the host read at its
    /// press. A word that is not in the word list answers `UnknownWords` with its position and NOTHING is sent.
    /// Otherwise core derives the key here, the server checks the recovery check (the words never leave this
    /// computer), and the flow is kept by this engine for step 2. Any earlier recovery of this engine is dropped
    /// first.
    pub fn recovery_verify_for(
        &self,
        turn: SessionGeneration,
        email: &str,
        phrase: Zeroizing<String>,
    ) -> Result<(), RecoveryRefused> {
        self.recovery_forget();
        if !self.recovery_turn_holds(turn) {
            return Err(RecoveryRefused::Overtaken);
        }
        // The words as the boxes hold them, one space between: positions are the boxes'.
        let unknown = unknown_word_positions(&phrase.split(' ').collect::<Vec<&str>>());
        if !unknown.is_empty() {
            return Err(RecoveryRefused::UnknownWords(unknown));
        }
        self.runtime.block_on(self.recovery_verify_step(turn, email, phrase))
    }

    async fn recovery_verify_step(
        &self,
        turn: SessionGeneration,
        email: &str,
        phrase: Zeroizing<String>,
    ) -> Result<(), RecoveryRefused> {
        let policy = self.recovery_policy().await?;
        // Core canonicalises the address and derives the key; its request carries the address and the check only.
        let (mut flow, request) = PhraseRecovery::start(policy.clone(), email, &phrase).map_err(refused)?;
        drop(phrase);
        let answer = auth_transport::send_to(&self.config.api_base, &self.client, None, request)
            .await
            .map_err(|_| RecoveryRefused::Offline)?;
        flow.on_start_response(&answer).map_err(refused)?;
        let mut entries = held();
        // Kept only while the press's session is still the one in memory (dropped, and wiped, otherwise).
        if !self.recovery_turn_holds(turn) {
            return Err(RecoveryRefused::Overtaken);
        }
        entries.retain(|entry| !self.owns_recovery(entry));
        entries.push(Held {
            owner: Arc::downgrade(&self.acct),
            id: NEXT_ID.fetch_add(1, Ordering::SeqCst),
            turn,
            policy,
            email: Zeroizing::new(email.trim().to_string()),
            flow: Some(flow),
        });
        Ok(())
    }

    /// The server's numbers for a new password (its onboarding document). Refused, not guessed, when it cannot be
    /// read: the breach check it may declare is not skipped by a default.
    async fn recovery_policy(&self) -> Result<SignupPolicy, RecoveryRefused> {
        let answer = auth_transport::send_to(&self.config.api_base, &self.client, None, SignupPolicy::request())
            .await
            .map_err(|_| RecoveryRefused::Offline)?;
        if answer.status == 429 {
            return Err(RecoveryRefused::RateLimited {
                seconds: seconds_of(answer.retry_after_secs.unwrap_or(60)),
            });
        }
        SignupPolicy::from_response(&answer).map_err(|_| RecoveryRefused::Other)
    }

    /// How strong a new password is (DW-A49's meter): core's sign-up evaluator, on the server's minimum as the
    /// verify read it (core's default before that). Local and at once: it runs on every key press, never waits on
    /// the network or a step, and keeps nothing of the password.
    pub fn recovery_password_verdict(&self, password: &str) -> SignupVerdict {
        let policy = held()
            .iter()
            .find(|entry| self.owns_recovery(entry))
            .map(|entry| entry.policy.clone())
            .unwrap_or_default();
        verdict_of(&policy, password)
    }

    /// Take this engine's flow out of the slot for one step, for the press's turn. A stale turn drops the recovery.
    fn recovery_take(&self, turn: SessionGeneration) -> Result<Taken, RecoveryRefused> {
        let mut entries = held();
        let at = entries
            .iter()
            .position(|entry| self.owns_recovery(entry))
            .ok_or(RecoveryRefused::Overtaken)?;
        if entries[at].turn != turn || !self.recovery_turn_holds(turn) {
            // The session moved since the words were verified: the recovery is dropped (wiped as it drops).
            entries.remove(at);
            return Err(RecoveryRefused::Overtaken);
        }
        let entry = &mut entries[at];
        // `None`: another step has it out (a host runs one step at a time; refused rather than waited for).
        let flow = entry.flow.take().ok_or(RecoveryRefused::Other)?;
        Ok(Taken {
            id: entry.id,
            policy: entry.policy.clone(),
            email: entry.email.clone(),
            flow,
        })
    }

    /// Put a flow back for a retry, into the entry it came from. Forgotten or replaced meanwhile: it is wiped here.
    fn recovery_put_back(&self, id: u64, flow: PhraseRecovery) {
        let mut entries = held();
        if let Some(entry) = entries
            .iter_mut()
            .find(|entry| entry.id == id && entry.flow.is_none() && self.owns_recovery(entry))
        {
            entry.flow = Some(flow);
        }
    }

    /// Remove the entry `id`. `true`: it was still there (the recovery was not forgotten or replaced meanwhile).
    fn recovery_end(&self, id: u64) -> bool {
        let mut entries = held();
        let before = entries.len();
        entries.retain(|entry| entry.id != id);
        entries.len() != before
    }

    /// Step 2 (DW-A49, DW-A50): the new password (the window compared the two fields; it is its own confirmation
    /// here), asked for the session turn `turn` the host read at its press. Sign-up's rules first: the server's
    /// minimum (`Weak`, nothing sent), then the breach check (only a prefix of the password's hash leaves this
    /// computer). Then the OPAQUE re-registration and the finalize. A wait or an outage before the finalize keeps
    /// the words verified (the password is asked again); the finalize consumes the recovery, whatever its answer, so
    /// a retry after it starts from the words.
    ///
    /// `SignedIn`: stored through the password sign-in's own store, the vault opened, the lock follows. The turn is
    /// re-checked right before anything is stored: a session that moved meanwhile, a sign-in that happened
    /// meanwhile, or a recovery that was forgotten meanwhile stores nothing (`Overtaken`) and the new session is
    /// ended at the server. `SignInRequired`: nothing is stored.
    pub fn recovery_set_password_for(
        &self,
        turn: SessionGeneration,
        password: Zeroizing<String>,
    ) -> Result<RecoveryDone, RecoveryRefused> {
        let result = self.runtime.block_on(self.recovery_set_password_step(turn, password));
        self.publish();
        result
    }

    async fn recovery_set_password_step(
        &self,
        turn: SessionGeneration,
        password: Zeroizing<String>,
    ) -> Result<RecoveryDone, RecoveryRefused> {
        let Taken {
            id,
            policy,
            email,
            mut flow,
        } = self.recovery_take(turn)?;
        if !verdict_of(&policy, &password).meets_minimum {
            self.recovery_put_back(id, flow);
            return Err(RecoveryRefused::Weak);
        }
        if let Some(request) = flow.breach_request(&password) {
            let (status, text) = self.recovery_breach_answer(&request.path).await;
            let _ = flow.on_breach_response(status, &text);
        }
        let request = match flow.set_password(&password, &password) {
            Ok(request) => request,
            Err(error) => {
                self.recovery_put_back(id, flow);
                return Err(refused(error));
            }
        };
        drop(password);
        let base = &self.config.api_base;
        let answer = match auth_transport::send_to(base, &self.client, None, request).await {
            Ok(answer) => answer,
            Err(_) => {
                // No answer: told to core as an outage, which puts the flow back at "words accepted" (the password and
                // the registration state are wiped), so a retry asks for the password, not the words.
                let _ = flow.on_register_response(&HttpResponse::new(503, serde_json::Value::Null));
                self.recovery_put_back(id, flow);
                return Err(RecoveryRefused::Offline);
            }
        };
        let request = match flow.on_register_response(&answer) {
            Ok(request) => request,
            // A wait or an outage: core put the flow back at "words accepted", so the password can be tried again.
            Err(error @ (SignupError::RateLimited { .. } | SignupError::Server { .. })) => {
                self.recovery_put_back(id, flow);
                return Err(refused(error));
            }
            // The server refused the recovery token (`RecoveryExpired`), or its answer could not be read: this
            // recovery is over, a retry starts from the words.
            Err(error) => {
                drop(flow);
                self.recovery_end(id);
                return Err(refused(error));
            }
        };
        let answer = match auth_transport::send_to(base, &self.client, None, request).await {
            Ok(answer) => answer,
            Err(_) => {
                // No answer to the last step: whether the password changed is not known here. This recovery is over.
                drop(flow);
                self.recovery_end(id);
                return Err(RecoveryRefused::Offline);
            }
        };
        // The last step consumes the flow: from here this recovery is over, whatever the answer.
        let recovered = flow.on_finalize_response(&answer);
        let still_held = self.recovery_end(id);
        match recovered {
            Ok(Recovered::SignedIn {
                session_token,
                master_key,
            }) => {
                let key = master_key.to_bytes();
                self.recovery_install(turn, still_held, session_token, key, email).await
            }
            // The password changed; the server wants a normal sign-in (two-factor). Nothing to store.
            Ok(Recovered::SignInRequired) => Ok(RecoveryDone::SignInRequired),
            Err(error) => Err(refused(error)),
        }
    }

    /// The recovered session and key, stored. The session goes through the password sign-in's own store
    /// (`store_first_sign_in`: a foreign key removed first, then the token, the email and the session's owner); the
    /// key then follows the order sign-up's install uses for a key that arrives with its session (`signup_api`). A
    /// key that cannot be kept signs the session out again. Nothing stored: the new session is ended at the server.
    async fn recovery_install(
        &self,
        turn: SessionGeneration,
        still_held: bool,
        token: Zeroizing<String>,
        key: Zeroizing<[u8; 32]>,
        typed_email: Zeroizing<String>,
    ) -> Result<RecoveryDone, RecoveryRefused> {
        let base = &self.config.api_base;
        let _step = self.transition.lock().await;
        // The turn, re-checked right before anything is stored, in the step's hold: the press's session is still the
        // one in memory, nobody signed in meanwhile, and the recovery was not forgotten meanwhile.
        if !still_held || !self.recovery_turn_holds(turn) {
            crate::app::revoke(&self.client, base, &token).await;
            return Err(RecoveryRefused::Overtaken);
        }
        let profile = match crate::app::fetch_profile(&self.client, base, &token).await {
            Ok(profile) => profile,
            Err(error) => {
                crate::app::revoke(&self.client, base, &token).await;
                return Err(if error == auth_transport::OFFLINE {
                    RecoveryRefused::Offline
                } else {
                    RecoveryRefused::Other
                });
            }
        };
        let email = match profile.email.trim() {
            "" => typed_email.to_string(),
            named => named.to_string(),
        };
        if self.store_first_sign_in(turn, &token, &email, &profile).is_err() {
            crate::app::revoke(&self.client, base, &token).await;
            return Err(RecoveryRefused::Other);
        }
        self.failures.store(0, Ordering::SeqCst);
        let turn = self.acct.session_generation();
        let kept = (|| -> Result<(), String> {
            let _write = self.claim(turn)?;
            let vault = self.vault();
            vault
                .store_wrapped_master_key(SecretBytes::new_master_key(&key))
                .map_err(|e| store::keychain_error("store the account key", e))?;
            // The key was proven to be this account's by the server (it accepted the recovery check derived from it).
            if let Err(error) = key_owner::bind_key_to_session_owner(&vault) {
                tracing::warn!(%error, "could not record the vault key's owner in the credential store");
            }
            let mut guard = self
                .acct
                .session
                .lock()
                .map_err(|_| "session mutex poisoned".to_string())?;
            *guard = Some(Session {
                token: token.to_string(),
                master_key: *key,
                email: Some(email),
            });
            drop(guard);
            self.acct.auth_resume_asked.fetch_add(1, Ordering::SeqCst);
            // The twelve words, accepted by the server for this account, are a strong proof: the lock opens in the
            // same hold as the key's install.
            self.lock_follows(SessionMoved::StrongProof);
            Ok(())
        })();
        if kept.is_err() {
            // Signed in without its key on this computer: ended, so another sign-in can start.
            let _ = self.sign_out_holding_step().await;
            return Err(RecoveryRefused::Other);
        }
        Ok(RecoveryDone::SignedIn)
    }

    /// The range answer of the breach check, as text (core reads lines, not JSON). `None` status: no answer.
    async fn recovery_breach_answer(&self, path: &str) -> (Option<u16>, Zeroizing<String>) {
        let url = format!("{}{}", self.config.api_base, path);
        let Ok(response) = self.client.get(url).send().await else {
            return (None, Zeroizing::new(String::new()));
        };
        let status = response.status().as_u16();
        if response.content_length().is_some_and(|n| n as usize > BREACH_BODY_CAP) {
            return (None, Zeroizing::new(String::new()));
        }
        let text = response.text().await.unwrap_or_default();
        if text.len() > BREACH_BODY_CAP {
            return (None, Zeroizing::new(String::new()));
        }
        (Some(status), Zeroizing::new(text))
    }
}

#[cfg(test)]
#[path = "recovery_tests.rs"]
mod tests;
