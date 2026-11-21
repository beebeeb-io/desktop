//! The screen lock's password path: the wrong-try accounting, the server's grant-nothing password proof for the
//! current session, and the local check record that decides only when the server cannot be reached.
//!
//! DECISION 2509 (lead, recommendation A): this file is a COPY of the ORDER of steps in core's lock service,
//! `beebeeb-client/src/auth/lock_password.rs` (`Client::unlock_screen_with_password`, `set_ask_password_at_start`,
//! `counted_password_check`, `judge_password`, `prove_password_online`), because this engine cannot call that client
//! (it keeps its own session and credential store). The pure rules are core's and are called, not restated:
//! `screen_lock::{AttemptState, next_try, wait_after_ms, LOCAL_CHECK_OFF_AT, stretch, enroll, verify}`. A core
//! follow-up task, after 0.9.0's test build, exposes that order as a core function over a store and a transport
//! trait; this copy is then deleted. Until then each step below is pinned by a test, and the copy is reviewed against
//! core's function of the same name.
//!
//! Every method BLOCKS on the handle's runtime: call it from a worker thread, never from the UI thread.
//!
//! Must not: log or keep the typed password, the record's bytes or the OPAQUE messages; count a refusal that judged
//! nothing (the server's limit, an ended session, an offline check without a record) as a wrong try; let a local
//! record decide while the server can be reached; check two passwords at once.

use std::sync::Mutex;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use beebeeb_api::flows::LockedAction;
use beebeeb_api::proposed::{self, password_proof_codes};
use beebeeb_core::api_error::ServerError;
use beebeeb_core::opaque_protocol::{LoginFinishError, client_login_finish_judged, client_login_start};
use beebeeb_core::screen_lock::{
    self, AttemptState, CheckOutcome, LOCAL_CHECK_OFF_AT, PendingScreenCheck, ScreenCheckRecord, TryDecision, next_try,
    wait_after_ms,
};
use zeroize::Zeroizing;

#[path = "app_lock_api.rs"]
mod app_lock;
pub use app_lock::{AppLockProof, AppLockRefused, Away, AwayAnswer, LockAfter};

use super::lock_api::{LockRefused, LockState};
use super::{CredentialStores, Engine};
use crate::account::SessionGeneration;
use crate::account_dto::AccountProfile;
use crate::keychain::{AuthVault, LockFlag};

/// A request that gets no answer in this long is an offline one.
const WAIT: Duration = Duration::from_secs(30);

/// The 409 the server answers when the proof's start was used up or timed out (the server's header table); core's
/// constants at the pinned revision do not name it.
const PROOF_STALE: &str = "password_proof_stale";

/// What became of one typed password at the screen lock. Same variants and meanings as core's
/// `beebeeb_client::auth::ScreenUnlock`. Only `Wrong` and `LocalCheckOff` are wrong tries.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScreenUnlock {
    /// Accepted (by the server, or by the local record while the server could not be reached): the window opens.
    Opened,
    /// A wrong password. `wait_ms` is the wait before the next try (0: none yet).
    Wrong { wait_ms: u64 },
    /// Nothing was checked: a wait after earlier wrong tries is still running.
    Wait { wait_ms: u64 },
    /// The server's own limit on password checks, not a wrong try. `None`: it named no wait.
    ServerLimit { wait_ms: Option<u64> },
    /// This wrong try was the tenth in a row: the local check is off and its record is deleted.
    LocalCheckOff,
    /// The server cannot be reached and this PC has no usable record of the password.
    OfflineNoRecord,
    /// The account is still on the legacy key stretch: signing in again finishes its upgrade.
    AccountUpgradeNeeded,
    /// The server no longer accepts the session: the vault is closed, and the strong unlock opens it.
    SessionEnded,
}

/// Why the "ask for my password when Beebeeb starts" setting, or a password confirmed online, was not done.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StartSettingRefused {
    /// The app is not open: the setting changes only while it is.
    NotOpen,
    /// Turning it off needs the password.
    PasswordNeeded,
    /// A wrong password, counted (core's `AuthError::PasswordWrong`): `wait_ms` before the next try (0: none), and
    /// whether this tenth wrong try turned the offline check off.
    Wrong { wait_ms: u64, local_check_off: bool },
    /// Nothing was checked: a wait after wrong tries is still running.
    Wait { wait_ms: u64 },
    /// The server's own limit on password checks. `None`: it named no wait.
    ServerLimit { wait_ms: Option<u64> },
    /// The server cannot be reached: this needs the password proven online, never the local record.
    Offline,
    /// The account is still on the legacy key stretch.
    UpgradeNeeded,
    /// The server no longer accepts the session: the vault is closed.
    SessionEnded,
    /// The check could not run (a count that cannot be stored, an unknown owner), or a check is already running.
    CouldNotCheck,
    /// The flag could not be stored.
    NotSaved,
}

/// This handle's password-check state: one check at a time, and the sign-in's stretched password waiting for its
/// account.
#[derive(Default)]
pub(super) struct LockPassword {
    one_at_a_time: Mutex<()>,
    /// The typed password's stretch, with the canonical email of the sign-in it came from.
    pending: Mutex<Option<(String, PendingScreenCheck)>>,
}

impl LockPassword {
    /// The sign-in's stretched password is wiped (the session was forgotten).
    pub(super) fn drop_pending(&self) {
        *self.pending.lock().unwrap_or_else(|p| p.into_inner()) = None;
    }

    /// Whether a sign-in's stretched password waits here (the sign-in's wipe tests, `signin_api_tests.rs`).
    #[cfg(test)]
    pub(super) fn holds_a_stretch(&self) -> bool {
        self.pending.lock().unwrap_or_else(|p| p.into_inner()).is_some()
    }
}

/// What the server said about one typed password.
enum OnlineProof {
    /// `None`: the server named no password counter with its answer.
    Proven {
        password_counter: Option<u64>,
    },
    /// Judged and refused: the OPAQUE finish failed on this PC, or the server refused the proof.
    Wrong,
    Unreachable,
    Limited {
        wait_ms: Option<u64>,
    },
    Unauthorized,
    UpgradeNeeded,
    /// Judged nothing (an answer this client does not read, a 5xx, a start that went stale twice).
    Failed,
}

/// One typed password after the wrong-try rule and the checks.
enum Judgement {
    Accepted,
    /// A judged wrong password; `failures` is the count including this one.
    Wrong {
        failures: u32,
    },
    Wait {
        wait_ms: u64,
    },
    ServerLimit {
        wait_ms: Option<u64>,
    },
    OfflineNoRecord,
    UpgradeNeeded,
    SessionEnded,
}

/// Why a check did not run to a judgement.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CheckStopped {
    /// Another check is on its way.
    Busy,
    /// The count could not be stored, the owner is unknown, or the check could not run: nothing was judged.
    Failed,
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |since| u64::try_from(since.as_millis()).unwrap_or(u64::MAX))
}

/// The stored count, and whether it is to be written back at once (an entry that is not a stored state never buys
/// tries: the longest wait, from now).
fn load_attempts<S: crate::keychain::AuthSecretStore>(
    vault: &AuthVault<S>,
    now_ms: u64,
) -> Result<(AttemptState, bool), CheckStopped> {
    match vault.lock_entry(LockFlag::Attempts) {
        Err(_) => Err(CheckStopped::Failed),
        Ok(None) => Ok((AttemptState::new(), false)),
        Ok(Some(bytes)) => Ok(match AttemptState::from_bytes(&bytes) {
            Some(state) => (state, false),
            None => (AttemptState::unreadable(now_ms), true),
        }),
    }
}

fn store_attempts<S: crate::keychain::AuthSecretStore>(
    vault: &AuthVault<S>,
    state: &AttemptState,
) -> Result<(), CheckStopped> {
    let stored = if state.failures() == 0 {
        vault.clear_lock_flag(LockFlag::Attempts)
    } else {
        vault.set_lock_entry(LockFlag::Attempts, &state.to_bytes())
    };
    stored.map_err(|_| CheckStopped::Failed)
}

/// Core's `replace_screen_check`: the record is stored, or (no record, or a store that refuses) the old one is
/// deleted: a stale record never stays.
fn replace_screen_check<S: crate::keychain::AuthSecretStore>(vault: &AuthVault<S>, record: Option<ScreenCheckRecord>) {
    let stored = record.is_some_and(|record| vault.set_lock_entry(LockFlag::ScreenCheck, &record.to_bytes()).is_ok());
    if !stored {
        let _ = vault.clear_lock_flag(LockFlag::ScreenCheck);
    }
}

/// One answer of the proof's legs, read as core's `refusal` reads it.
fn refusal(status: u16, body: &str, retry_after_header: Option<&str>) -> OnlineProof {
    let mut server = ServerError::parse_body(status, body);
    if let Some(header) = retry_after_header {
        server = server.with_retry_after_header(header);
    }
    match status {
        401 => OnlineProof::Unauthorized,
        429 => OnlineProof::Limited {
            wait_ms: server.retry_after().map(|seconds| seconds.saturating_mul(1_000)),
        },
        403 if server.is_code(password_proof_codes::NOT_PROVEN) => OnlineProof::Wrong,
        409 if server.is_code(password_proof_codes::UNAVAILABLE) => OnlineProof::UpgradeNeeded,
        _ => OnlineProof::Failed,
    }
}

/// One leg's answer: the body of a 2xx, or what the refusal means.
enum Leg {
    Body(Zeroizing<String>),
    Stale,
    Refused(OnlineProof),
}

impl<C: CredentialStores> Engine<C> {
    /// The screen lock's password path. `Err` leaves the screen locked and counts nothing, except where the answer
    /// says otherwise. See [`ScreenUnlock`].
    pub fn unlock_screen_with_password(&self, password: Zeroizing<String>) -> Result<ScreenUnlock, LockRefused> {
        self.unlock_screen_with_password_at(password, now_ms())
    }

    /// [`Self::unlock_screen_with_password`] on the caller's clock (milliseconds since the epoch), for a test.
    pub(crate) fn unlock_screen_with_password_at(
        &self,
        password: Zeroizing<String>,
        now_ms: u64,
    ) -> Result<ScreenUnlock, LockRefused> {
        if self.lock_state() != LockState::ScreenLocked {
            return Err(LockRefused::NotAtThisLevel);
        }
        if password.is_empty() {
            return Err(LockRefused::NothingTyped);
        }
        let turn = self.acct.session_generation();
        let judged = self
            .runtime
            .block_on(self.counted_password_check(&password, now_ms, true))
            .map_err(|stopped| match stopped {
                CheckStopped::Busy => LockRefused::AlreadyChecking,
                CheckStopped::Failed => LockRefused::CouldNotCheck,
            })?;
        drop(password);
        Ok(match judged {
            Judgement::Accepted => {
                {
                    let _writes = self.writes.write().unwrap_or_else(|p| p.into_inner());
                    // The answer belongs to the session it was asked for: never open another one.
                    if !self.acct.session_unchanged_since(turn) {
                        return Err(LockRefused::Overtaken);
                    }
                    self.lock_password_accepted()?;
                }
                self.publish();
                ScreenUnlock::Opened
            }
            Judgement::Wrong { failures } if failures == LOCAL_CHECK_OFF_AT => ScreenUnlock::LocalCheckOff,
            Judgement::Wrong { failures } => ScreenUnlock::Wrong {
                wait_ms: wait_after_ms(failures),
            },
            Judgement::Wait { wait_ms } => ScreenUnlock::Wait { wait_ms },
            Judgement::ServerLimit { wait_ms } => ScreenUnlock::ServerLimit { wait_ms },
            Judgement::OfflineNoRecord => ScreenUnlock::OfflineNoRecord,
            Judgement::UpgradeNeeded => ScreenUnlock::AccountUpgradeNeeded,
            Judgement::SessionEnded => {
                self.end_session_the_server_refused(turn);
                ScreenUnlock::SessionEnded
            }
        })
    }

    /// The counted online-only check alone (no lock change): for a switch that weakens the app lock on a PC where
    /// Windows Hello cannot answer. Same refusals as [`Self::set_ask_password_at_start`]'s turning off.
    pub fn confirm_password_online(&self, password: Zeroizing<String>) -> Result<(), StartSettingRefused> {
        self.confirm_password_online_at(password, now_ms())
    }

    pub(crate) fn confirm_password_online_at(
        &self,
        password: Zeroizing<String>,
        now_ms: u64,
    ) -> Result<(), StartSettingRefused> {
        if self.lock_state() != LockState::Open {
            return Err(StartSettingRefused::NotOpen);
        }
        if password.is_empty() {
            return Err(StartSettingRefused::PasswordNeeded);
        }
        let turn = self.acct.session_generation();
        let judged = self
            .runtime
            .block_on(self.counted_password_check(&password, now_ms, false))
            .map_err(|_| StartSettingRefused::CouldNotCheck)?;
        drop(password);
        match judged {
            Judgement::Accepted if !self.acct.session_unchanged_since(turn) => Err(StartSettingRefused::NotOpen),
            Judgement::Accepted => Ok(()),
            Judgement::Wrong { failures } => Err(StartSettingRefused::Wrong {
                wait_ms: if failures == LOCAL_CHECK_OFF_AT {
                    0
                } else {
                    wait_after_ms(failures)
                },
                local_check_off: failures == LOCAL_CHECK_OFF_AT,
            }),
            Judgement::Wait { wait_ms } => Err(StartSettingRefused::Wait { wait_ms }),
            Judgement::ServerLimit { wait_ms } => Err(StartSettingRefused::ServerLimit { wait_ms }),
            Judgement::OfflineNoRecord => Err(StartSettingRefused::Offline),
            Judgement::UpgradeNeeded => Err(StartSettingRefused::UpgradeNeeded),
            Judgement::SessionEnded => {
                self.end_session_the_server_refused(turn);
                Err(StartSettingRefused::SessionEnded)
            }
        }
    }

    /// "Ask for my password when Beebeeb starts". Only while the app is open. Turning it on asks for nothing.
    /// Turning it off needs the password proven to the server (never the local record, never Windows Hello), so it
    /// cannot be turned off offline; the wrong-try rule counts these tries as it counts the screen lock's.
    pub fn set_ask_password_at_start(
        &self,
        on: bool,
        password: Option<Zeroizing<String>>,
    ) -> Result<(), StartSettingRefused> {
        self.set_ask_password_at_start_at(on, password, now_ms())
    }

    pub(crate) fn set_ask_password_at_start_at(
        &self,
        on: bool,
        password: Option<Zeroizing<String>>,
        now_ms: u64,
    ) -> Result<(), StartSettingRefused> {
        if self.lock_state() != LockState::Open {
            return Err(StartSettingRefused::NotOpen);
        }
        if !on {
            let password = password
                .filter(|password| !password.is_empty())
                .ok_or(StartSettingRefused::PasswordNeeded)?;
            self.confirm_password_online_at(password, now_ms)?;
        }
        // Still open, in the hold that writes: a lock that landed during the check changes nothing here.
        let _writes = self.writes.read().unwrap_or_else(|p| p.into_inner());
        if self.lock_state() != LockState::Open {
            return Err(StartSettingRefused::NotOpen);
        }
        self.vault()
            .set_lock_flag(LockFlag::AskPasswordAtStart, on)
            .map_err(|_| StartSettingRefused::NotSaved)
    }

    /// The flow's step for an accepted password (core's `LockFlow::password_checked`), in the caller's hold of the
    /// session-write lock.
    fn lock_password_accepted(&self) -> Result<(), LockRefused> {
        self.lock_flow
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .password_checked()
            .map_err(|error| match error {
                beebeeb_api::flows::FlowError::StepNotRequired => LockRefused::NotAtThisLevel,
                _ => LockRefused::NeedsThePassword,
            })
    }

    /// A 401 that a second question confirmed: the vault closes, and the count and the record go with the key
    /// ([`Self::close_session_the_server_refused`]). Runs after the check's own `block_on` returned.
    fn end_session_the_server_refused(&self, turn: SessionGeneration) {
        if self.close_session_the_server_refused(Some(turn)).is_some() {
            self.publish();
        }
    }

    /// The owner every record is sealed to and verified for: the account's user id, as the session's recorded owner.
    fn screen_owner(&self, vault: &AuthVault<C::Store>) -> Result<String, CheckStopped> {
        match vault.session_owner() {
            Ok(Some(owner)) if !owner.trim().is_empty() => Ok(owner.trim().to_string()),
            _ => Err(CheckStopped::Failed),
        }
    }

    /// One typed password, counted once whichever checks it goes through, in core's order.
    ///
    /// The count is written before any check runs, so ending the process mid-check does not buy a free try, and a
    /// count that cannot be written refuses the try. A judged wrong password keeps the count; a success resets it;
    /// an outcome that judged nothing (the server's limit, an ended session, no record offline, an error) restores
    /// what it was.
    async fn counted_password_check(
        &self,
        password: &Zeroizing<String>,
        now_ms: u64,
        offline_allowed: bool,
    ) -> Result<Judgement, CheckStopped> {
        // One check at a time: a second press while one is on its way checks nothing.
        let _one_at_a_time = match self.lock_password.one_at_a_time.try_lock() {
            Ok(guard) => guard,
            Err(std::sync::TryLockError::Poisoned(poisoned)) => poisoned.into_inner(),
            Err(std::sync::TryLockError::WouldBlock) => return Err(CheckStopped::Busy),
        };
        let vault = self.vault();
        let owner = self.screen_owner(&vault)?;
        let (stored, replaced) = load_attempts(&vault, now_ms)?;
        let before = stored.rebased(now_ms);
        if replaced || before != stored {
            store_attempts(&vault, &before)?;
        }
        let local_allowed = match next_try(&before, now_ms) {
            TryDecision::WaitUntil(until) => {
                return Ok(Judgement::Wait {
                    wait_ms: until.saturating_sub(now_ms),
                });
            }
            TryDecision::Allowed => offline_allowed,
            TryDecision::LocalCheckOff => {
                let _ = vault.clear_lock_flag(LockFlag::ScreenCheck);
                false
            }
        };
        let counted = before.counting_try(now_ms);
        store_attempts(&vault, &counted)?;
        let judged = self.judge_password(password, &owner, &vault, local_allowed).await;
        match &judged {
            Ok(Judgement::Accepted) => {
                // A count that cannot be removed stays: it only makes the next wait longer.
                let _ = store_attempts(&vault, &AttemptState::new());
            }
            Ok(Judgement::Wrong { .. }) => {
                if counted.failures() >= LOCAL_CHECK_OFF_AT {
                    let _ = vault.clear_lock_flag(LockFlag::ScreenCheck);
                }
            }
            // An ended session deletes the count with the record, in the close that follows.
            Ok(Judgement::SessionEnded) => {}
            Ok(_) | Err(_) => {
                let _ = store_attempts(&vault, &before);
            }
        }
        judged.map(|judgement| match judgement {
            Judgement::Wrong { .. } => Judgement::Wrong {
                failures: counted.failures(),
            },
            other => other,
        })
    }

    async fn judge_password(
        &self,
        password: &Zeroizing<String>,
        owner: &str,
        vault: &AuthVault<C::Store>,
        local_allowed: bool,
    ) -> Result<Judgement, CheckStopped> {
        match self.prove_password_online(password).await {
            OnlineProof::Proven { password_counter } => {
                // The server has just accepted this exact password: the record is made again from it, every time.
                let record = screen_lock::enroll(password, owner, password_counter.unwrap_or(0)).ok();
                replace_screen_check(vault, record);
                Ok(Judgement::Accepted)
            }
            OnlineProof::Wrong => Ok(Judgement::Wrong { failures: 0 }),
            OnlineProof::Unreachable if !local_allowed => Ok(Judgement::OfflineNoRecord),
            OnlineProof::Unreachable => {
                let Ok(Some(bytes)) = vault.lock_entry(LockFlag::ScreenCheck) else {
                    return Ok(Judgement::OfflineNoRecord);
                };
                let record = ScreenCheckRecord::from_bytes(&bytes);
                match screen_lock::verify(&record, password, owner) {
                    Ok(CheckOutcome::Match) => Ok(Judgement::Accepted),
                    Ok(CheckOutcome::NoMatch) => Ok(Judgement::Wrong { failures: 0 }),
                    Ok(CheckOutcome::Unusable) => {
                        let _ = vault.clear_lock_flag(LockFlag::ScreenCheck);
                        Ok(Judgement::OfflineNoRecord)
                    }
                    Err(_) => Err(CheckStopped::Failed),
                }
            }
            OnlineProof::Limited { wait_ms } => Ok(Judgement::ServerLimit { wait_ms }),
            // A 401 on either leg is never a wrong try: ask whether the session still lives.
            OnlineProof::Unauthorized => match self.session_still_accepted().await {
                Some(true) => Ok(Judgement::ServerLimit { wait_ms: None }),
                Some(false) => Ok(Judgement::SessionEnded),
                None => Err(CheckStopped::Failed),
            },
            OnlineProof::UpgradeNeeded => Ok(Judgement::UpgradeNeeded),
            OnlineProof::Failed => Err(CheckStopped::Failed),
        }
    }

    /// The session's token, asked as the one call a locked screen lets through.
    fn proof_token(&self) -> Option<Zeroizing<String>> {
        self.person_session_for(LockedAction::UnlockScreen, |session, _| {
            Zeroizing::new(session.token.clone())
        })
        .ok()
    }

    /// `GET /auth/me`: `Some(true)` the server still accepts the session, `Some(false)` it answered 401, `None` no
    /// answer this client reads.
    async fn session_still_accepted(&self) -> Option<bool> {
        let token = self.proof_token()?;
        let url = format!("{}/api/v1/auth/me", self.config.api_base);
        let response = self
            .client
            .get(url)
            .timeout(WAIT)
            .bearer_auth(token.as_str())
            .send()
            .await
            .ok()?;
        match response.status().as_u16() {
            401 => Some(false),
            status if (200..300).contains(&status) => Some(true),
            _ => None,
        }
    }

    /// One leg of the proof: a POST of `body` on the session's token.
    async fn proof_leg(&self, path: &str, token: &str, body: serde_json::Value) -> Leg {
        let url = format!("{}{path}", self.config.api_base);
        let sent = self
            .client
            .post(url)
            .timeout(WAIT)
            .bearer_auth(token)
            .json(&body)
            .send()
            .await;
        let response = match sent {
            Ok(response) => response,
            // No answer at all (never connected, timed out, hung up) is "cannot be reached"; a request that could not
            // be built or a redirect loop judged nothing. A 5xx is an answer: it is read below, and fails.
            Err(error) if error.is_builder() || error.is_redirect() => return Leg::Refused(OnlineProof::Failed),
            Err(_) => return Leg::Refused(OnlineProof::Unreachable),
        };
        let status = response.status().as_u16();
        let retry_after = response
            .headers()
            .get(reqwest::header::RETRY_AFTER)
            .and_then(|value| value.to_str().ok())
            .map(str::to_string);
        let text = Zeroizing::new(response.text().await.unwrap_or_default());
        if (200..300).contains(&status) {
            return Leg::Body(text);
        }
        if status == 409 && ServerError::parse_body(status, &text).is_code(PROOF_STALE) {
            return Leg::Stale;
        }
        Leg::Refused(refusal(status, &text, retry_after.as_deref()))
    }

    /// The grant-nothing password proof for the current session. A wrong password fails on this PC, in the OPAQUE
    /// finish, before anything is sent. A start that went stale is begun again once.
    async fn prove_password_online(&self, password: &Zeroizing<String>) -> OnlineProof {
        let Some(token) = self.proof_token() else {
            return OnlineProof::Failed;
        };
        for _ in 0..2 {
            if let Some(proof) = self.prove_once(password, &token).await {
                return proof;
            }
        }
        OnlineProof::Failed
    }

    /// One start and finish. `None`: the start went stale (to be begun again).
    async fn prove_once(&self, password: &Zeroizing<String>, token: &str) -> Option<OnlineProof> {
        let Ok(started) = client_login_start(password.as_bytes()) else {
            return Some(OnlineProof::Failed);
        };
        let state = Zeroizing::new(started.state);
        let answer = match self
            .proof_leg(
                proposed::password_proof_start().path(),
                token,
                serde_json::json!({ "client_message": STANDARD.encode(started.message) }),
            )
            .await
        {
            Leg::Body(text) => text,
            Leg::Stale => return None,
            Leg::Refused(proof) => return Some(proof),
        };
        let Ok(answer) = serde_json::from_str::<ProofStartAnswer>(&answer) else {
            return Some(OnlineProof::Failed);
        };
        if answer.ksf_version == 0 {
            return Some(OnlineProof::UpgradeNeeded);
        }
        let Ok(server_message) = STANDARD.decode(&answer.server_message) else {
            return Some(OnlineProof::Failed);
        };
        // OPAQUE's finish fails alike for a wrong password and a record made for another one: that is the judgement.
        // A finish that could not run judged nothing.
        let finish = match client_login_finish_judged(
            &state,
            password.as_bytes(),
            &server_message,
            u32::from(answer.ksf_version),
        ) {
            Ok(finish) => finish,
            Err(LoginFinishError::WrongPassword) => return Some(OnlineProof::Wrong),
            Err(LoginFinishError::NotJudged(_)) => return Some(OnlineProof::Failed),
        };
        let finished = match self
            .proof_leg(
                proposed::password_proof_finish().path(),
                token,
                serde_json::json!({
                    "client_message": STANDARD.encode(finish.message),
                    "server_state": answer.server_state,
                }),
            )
            .await
        {
            Leg::Body(text) => text,
            Leg::Stale => return None,
            Leg::Refused(proof) => return Some(proof),
        };
        Some(match serde_json::from_str::<ProofFinishAnswer>(&finished) {
            Ok(proven) if proven.proven => OnlineProof::Proven {
                password_counter: proven.password_counter,
            },
            _ => OnlineProof::Failed,
        })
    }

    // ── the record's lifecycle at sign-in ────────────────────────────────────────────────────────────────────

    /// A password sign-in was just accepted by the server: stretch the typed password for the screen-lock check
    /// (about a second, on the caller's worker; core: "a stretch that cannot run means no record, not a failed
    /// sign-in"). Kept until the install, or until the session is forgotten, and only ever sealed into the sign-in of
    /// the same `email`: a stretch left by a sign-in that stopped at its second factor never reaches another one.
    pub(super) fn stretch_for_sign_in(&self, email: &str, password: &Zeroizing<String>) {
        let pending = screen_lock::stretch(password)
            .ok()
            .map(|stretch| (email.to_string(), stretch));
        *self.lock_password.pending.lock().unwrap_or_else(|p| p.into_inner()) = pending;
    }

    /// The stretched password of the sign-in in progress for `email`, if any. Taken always (a stretch is used for
    /// one install at most); returned only for the sign-in it came from.
    pub(super) fn take_pending_screen_check(&self, email: &str) -> Option<PendingScreenCheck> {
        let taken = self
            .lock_password
            .pending
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .take();
        taken.filter(|(from, _)| from == email).map(|(_, stretch)| stretch)
    }

    /// A strong install just stored `profile`'s session: the check record of the password that was proven replaces
    /// any older one (none when the sign-in carried no stretch), and the wrong-try count is reset. The counter is
    /// the account's `password_counter` (0 when the server names none), as core's `seal_screen_check` does.
    pub(super) fn install_screen_check(&self, pending: Option<PendingScreenCheck>, profile: &AccountProfile) {
        let vault = self.vault();
        let owner = profile.user_id.trim();
        let record = pending.and_then(|pending| pending.seal(owner, profile.password_counter.unwrap_or(0)).ok());
        replace_screen_check(&vault, record);
        let _ = vault.clear_lock_flag(LockFlag::Attempts);
    }
}

#[derive(serde::Deserialize)]
struct ProofStartAnswer {
    server_message: String,
    server_state: String,
    ksf_version: u16,
}

#[derive(serde::Deserialize)]
struct ProofFinishAnswer {
    proven: bool,
    #[serde(default)]
    password_counter: Option<u64>,
}

#[cfg(test)]
#[path = "lock_password_api_tests.rs"]
mod tests;
