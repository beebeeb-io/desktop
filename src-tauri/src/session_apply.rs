//! `session_apply` items split out of `lib.rs`.

use super::*;

/// The server's answer to "is this key the signed-in account's?" (`POST /api/v1/auth/verify-recovery-check`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum RecoveryCheckAnswer {
    Matches,
    /// A `400` whose code is exactly [`INVALID_RECOVERY_PHRASE_CODE`]: the key is not the account's. A server that has
    /// no distinct code for it sends the same answer for an account with no recovery check on file (spec §5.6).
    Mismatch,
    /// A `400` whose code is exactly [`RECOVERY_CHECK_MISSING`]: the account has no recovery check on file.
    NoCheckOnFile,
}

/// The `error` code the server sends when the key's check is not the account's. Matched exactly; nothing else is.
pub(super) const INVALID_RECOVERY_PHRASE_CODE: &str = "invalid_recovery_phrase";

/// The `error` code the server sends when the account has no recovery check on file. Matched exactly; nothing else is.
pub(super) const RECOVERY_CHECK_MISSING: &str = "recovery_check_missing";

/// Ask the server whether `master_key` is the signed-in account's key by comparing its `recovery_check` (HKDF of the
/// key; never the key itself) against the stored one. Only a completed answer is `Ok`: a `200` that says valid, or a
/// `400` with one of the two exact codes above. A network error, a 401, a 5xx, an unreadable success body, and any other
/// `400` (no JSON, another code, a body the server's extractor rejected, an intermediary's page) are `Err`: they say
/// nothing about the key, and an unreachable verifier is never read as "valid" (the caller fails closed).
pub(super) async fn recovery_check_answer(
    client: &reqwest::Client,
    base_url: &str,
    session_token: &str,
    master_key: &beebeeb_core::kdf::MasterKey,
) -> Result<RecoveryCheckAnswer, String> {
    // The check stands in for the key at the server, so its text is wiped after the request (fix round 1, item 10),
    // and the body is serialized from a borrow of it (no second copy in a JSON value).
    let recovery_check = zeroize::Zeroizing::new(encode_base64(&*beebeeb_core::opaque::compute_recovery_check(
        master_key,
    )));
    #[derive(serde::Serialize)]
    struct RecoveryCheckBody<'a> {
        recovery_check: &'a str,
    }
    let resp = client
        .post(format!("{base_url}/api/v1/auth/verify-recovery-check"))
        .bearer_auth(session_token)
        .json(&RecoveryCheckBody {
            recovery_check: &recovery_check,
        })
        .send()
        .await
        .map_err(|e| format!("Could not verify the recovery phrase (network error: {e}). Try again."))?;
    let status = resp.status();
    if status == reqwest::StatusCode::BAD_REQUEST {
        let body = resp.text().await.unwrap_or_default();
        let code = serde_json::from_str::<serde_json::Value>(&body)
            .ok()
            .and_then(|value| value.get("error").and_then(|code| code.as_str()).map(str::to_string));
        return match code.as_deref() {
            Some(INVALID_RECOVERY_PHRASE_CODE) => Ok(RecoveryCheckAnswer::Mismatch),
            Some(RECOVERY_CHECK_MISSING) => Ok(RecoveryCheckAnswer::NoCheckOnFile),
            // Any other 400 says nothing about the key: answered like a server error, never as a mismatch.
            _ => Err(format!(
                "Could not verify the recovery phrase (server returned {status}). Try again."
            )),
        };
    }
    if status == reqwest::StatusCode::UNAUTHORIZED {
        return Err("Your session expired. Sign in again before unlocking the vault.".to_string());
    }
    if !status.is_success() {
        return Err(format!(
            "Could not verify the recovery phrase (server returned {status}). Try again."
        ));
    }
    let body: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| format!("Could not verify the recovery phrase (unreadable response: {e}). Try again."))?;
    if body.get("valid").and_then(|v| v.as_bool()) == Some(true) {
        Ok(RecoveryCheckAnswer::Matches)
    } else {
        Err("Could not verify the recovery phrase (unexpected response). Try again.".to_string())
    }
}

/// The recovery-phrase unlock's question: is the key from this phrase the signed-in account's? `Ok(true)` only on a
/// match. An account with no check on file proves nothing here either, so it stays `Ok(false)` (web applies the same
/// policy); see [`recovery_check_answer`] for what is `Err`.
pub(super) async fn recovered_key_matches_account(
    client: &reqwest::Client,
    base_url: &str,
    session_token: &str,
    master_key: &beebeeb_core::kdf::MasterKey,
) -> Result<bool, String> {
    recovery_check_answer(client, base_url, session_token, master_key)
        .await
        .map(|answer| answer == RecoveryCheckAnswer::Matches)
}

pub(super) fn normalize_recovery_phrase_input(input: &str) -> Result<String, String> {
    let words = input
        .split_whitespace()
        .map(|word| {
            word.trim()
                .to_ascii_lowercase()
                .chars()
                .filter(|c| c.is_ascii_alphabetic())
                .collect::<String>()
        })
        .filter(|word| !word.is_empty())
        .collect::<Vec<_>>();

    if words.len() != 12 {
        return Err("Recovery phrase must contain exactly 12 words.".to_string());
    }

    Ok(words.join(" "))
}

/// Decode a base64 string (standard alphabet, with or without padding).
pub(super) fn decode_base64(s: &str) -> Result<Vec<u8>, String> {
    use base64::Engine as _;
    base64::engine::general_purpose::STANDARD
        .decode(s.trim())
        .map_err(|e| e.to_string())
}

pub(super) fn encode_base64(bytes: &[u8]) -> String {
    use base64::Engine as _;
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

/// Verify a freshly-minted session by fetching `/api/v1/auth/me`, returning
/// the parsed [`account_dto::AccountProfile`].
///
/// This used to discard the payload after reading only `totp_enabled`. We
/// now return the whole profile so the caller can both gate on 2FA AND cache
/// the profile into `AppState` — that's what powers the `account_profile`
/// IPC without a second round-trip (the "all pages empty" data-layer fix).
/// The email a session is known by (R2, fix round 2 of Task 10; fix round 3): the SERVER's own spelling of it, from
/// the account's profile, and nothing else. Typed text is never an identity: a password login lowercases what was
/// typed client-side and a browser login does not, so the same account would have two, and signing in the other way
/// would look like another account. The profile's spelling and user id are what name an account here. Without a profile a session
/// has no email and is unidentified: no engine starts for it (see `authorize_engine_start`).
pub(super) fn session_email(profile: Option<&account_dto::AccountProfile>) -> Option<String> {
    profile
        .map(|profile| profile.email.trim())
        .filter(|email| !email.is_empty())
        .map(str::to_string)
}

/// The server's record of the account behind `session_token`, for a sign-in that only has the credentials a browser
/// handed over. `None` when the server cannot be reached or does not answer with a profile: the sign-in then has no
/// identity, and no engine starts until [`establish_session_identity`] gets one.
pub(super) async fn fetch_canonical_profile(session_token: &str) -> Option<account_dto::AccountProfile> {
    fetch_canonical_profile_at(&runner::api_base_url(), session_token).await
}

pub(super) async fn fetch_canonical_profile_at(base_url: &str, session_token: &str) -> Option<account_dto::AccountProfile> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .default_headers(api_client::provenance_headers())
        .build()
        .ok()?;
    fetch_session_profile(&client, base_url, session_token).await.ok()
}

/// A session whose account is not known by its user id (its sign-in could not fetch the account record, the Keychain
/// kept no email, or the startup probe could not name it) asks the server again, and ONE fetch that succeeds names it
/// (lead ruling 5, Task 12): see [`name_session_after_profile`]. The startup restore calls this before it starts an
/// engine; app activation and "Try again" call it too ([`identify_unidentified_session`]), so recovering from a failed
/// fetch needs no relaunch. A failed fetch changes nothing (`false`), so the engine start keeps refusing; a session that
/// is identified already, that was replaced while the fetch ran, or whose turn ended (a Lock, a Sign-out) is left
/// alone. `owner_dir`: where the owner record a naming completes lives (`None`: no owner record is touched).
pub(super) async fn establish_session_identity(
    state: &AppState,
    acct: &account::AccountRuntime,
    base_url: &str,
    owner_dir: Option<&Path>,
) -> bool {
    // One session transition: captured before the session is read, and the write below happens only in its turn.
    let turn = acct.session_generation();
    let (token, email) = match acct.session.lock() {
        Ok(guard) => match guard.as_ref() {
            Some(session) => (zeroize::Zeroizing::new(session.token.clone()), session.email.clone()),
            None => return false,
        },
        Err(_) => return false,
    };
    if session_knows_its_user_id(acct, email.as_deref()) {
        return false;
    }
    let Some(profile) = fetch_canonical_profile_at(base_url, &token).await else {
        return false;
    };
    // Checked, not yet moved on (fix round 1, item 5): a fetch that names nothing must not refuse a sign-in in flight.
    let Ok(write) = check_session_turn(acct, &turn) else {
        return false;
    };
    let named = name_session_after_profile(&write, state, acct, &token, &profile, owner_dir);
    if named {
        // Who the session is known as changed: a transition that began before this one is refused from now on.
        acct.advance_session_generation_holding_session_write_lock(account::SessionTransition::Write);
    }
    drop(write);
    named
}

/// Does the session know its account's user id: is a profile cached whose email is this session's own (see
/// [`identity_of_session`])? A session that does not is compared by email alone at an engine start.
pub(super) fn session_knows_its_user_id(acct: &account::AccountRuntime, email: Option<&str>) -> bool {
    let profile = acct.cached_profile.lock().ok().and_then(|guard| guard.clone());
    identity_of_session(email, profile.as_ref()).user_id.is_some()
}

/// Name the session in memory after the server's own record of its account (R2; lead rulings 4 and 5, Task 12): its
/// email becomes the server's spelling, the profile is cached (from now on its user id is known), and the Account page
/// and the Keychain follow. Only the session whose token is `token`, and only when it has no email yet or the same
/// address in its canonical form (`account_binding::same_email`): an answer never names a session it was not fetched for, nor one another name was
/// given meanwhile. Inside a checked turn (`write`). `true` when it changed the session.
///
/// The owner record comes FIRST (fix round 1, item 1). With an owner record (`owner_dir`, macOS and Linux) the session is
/// named only after the record has been completed with the account's id and the server's spelling
/// ([`backfill_local_data_owner`]); a backfill that fails names nothing, so the stored email stays exactly as it was and
/// the next engine start still knows the account's own data (a later fetch tries again). Without one (Windows) a
/// session that has an email is never renamed to another spelling: only an email-less session is named, as before.
pub(super) fn name_session_after_profile(
    write: &SessionWrite,
    state: &AppState,
    acct: &account::AccountRuntime,
    token: &str,
    profile: &account_dto::AccountProfile,
    owner_dir: Option<&Path>,
) -> bool {
    let Some(email) = session_email(Some(profile)) else {
        return false;
    };
    let known = {
        let Ok(guard) = acct.session.lock() else {
            return false;
        };
        match guard.as_ref() {
            Some(session)
                if session.token == token
                    && session
                        .email
                        .as_deref()
                        .is_none_or(|known| account_binding::same_email(known, &email)) =>
            {
                session.email.clone()
            }
            _ => return false,
        }
    };
    let renames = known.as_deref().is_some_and(|known| known != email);
    match owner_dir {
        Some(dir) => match backfill_local_data_owner(write, dir, profile) {
            Ok(true) => tracing::info!("R10: the owner record of the local data now names its account by user id"),
            Ok(false) => {}
            Err(error) => {
                tracing::warn!(%error, "R10: the owner record could not be completed, so the session is not renamed; a later fetch tries again");
                return false;
            }
        },
        // No owner record to complete (Windows): another spelling of a known email is never taken over.
        None if renames => return false,
        None => {}
    }
    {
        let Ok(mut guard) = acct.session.lock() else {
            return false;
        };
        match guard.as_mut() {
            Some(session) if session.token == token => session.email = Some(email.clone()),
            _ => return false,
        }
    }
    if let Ok(mut cached) = acct.cached_profile.lock() {
        *cached = Some(profile.clone());
    }
    set_auth_email(state, Some(email.clone()));
    write_session_email_to_keychain(write, acct, &email);
    true
}

/// The Keychain copy of the session's email, written in the naming's turn (Task 12 fix round 2, item 3). A write that
/// fails is not ignored: it is logged as `keychain_email_write_failed` with the kind of store error only (never its
/// text or the address) and marked on the account, so the next naming trigger (app activation, "Try again", the next
/// launch's background request) writes it again. A store that cannot hold secrets (Linux) has nothing to retry.
/// `true` when the Keychain holds it now.
pub(super) fn write_session_email_to_keychain(_write: &SessionWrite, acct: &account::AccountRuntime, email: &str) -> bool {
    match AuthVault::new(platform_keychain_store_for(acct.id.as_str())).store_account_email(email) {
        Ok(()) | Err(keychain::AuthStoreError::Unsupported(_)) => {
            acct.keychain_email_pending.store(false, Ordering::SeqCst);
            true
        }
        Err(error) => {
            acct.keychain_email_pending.store(true, Ordering::SeqCst);
            let kind = match error {
                keychain::AuthStoreError::Unsupported(_) => "unsupported",
                keychain::AuthStoreError::NotFound => "not_found",
                keychain::AuthStoreError::InvalidSecret(_) => "invalid_secret",
                keychain::AuthStoreError::Backend(_) => "backend",
            };
            tracing::warn!(
                event = "keychain_email_write_failed",
                kind,
                "the session's email is written to the Keychain again at the next naming"
            );
            false
        }
    }
}

/// Where the owner record of this computer's local data lives, for the identity steps that complete it (lead ruling 4).
/// `None` on Windows, which keeps today's binding (it refuses on an owner that differs instead).
pub(super) fn owner_record_dir() -> Option<PathBuf> {
    #[cfg(target_os = "windows")]
    {
        None
    }
    #[cfg(not(target_os = "windows"))]
    {
        state_paths::beebeeb_state_dir().ok()
    }
}

/// R10 (lead ruling 4, Task 12): while the session still works, the server's record of its account completes the owner
/// record of this computer's local data. An owner recorded before user ids were known (an email only, possibly spelled
/// with other letter case) gains the account's user id and the server's spelling of the email, so a later sign-in by
/// another method compares ids and never takes the account for another. Only an owner that IS this account is
/// completed: the same user id, or (with no id recorded) the same address in its canonical form (the engine start's
/// own comparison, `account_binding::same_account`). Any other owner is left to the binding at the next engine start. No database yet, or no owner: nothing to complete. Inside a checked
/// turn (`_write`).
pub(super) fn backfill_local_data_owner(
    _write: &SessionWrite,
    state_dir: &Path,
    profile: &account_dto::AccountProfile,
) -> Result<bool, String> {
    let (Some(email), false) = (session_email(Some(profile)), profile.user_id.trim().is_empty()) else {
        return Ok(false);
    };
    let Some(db) = state_db_from_state_dir(state_dir)? else {
        return Ok(false);
    };
    let Some(owner) = db
        .owner()
        .map_err(|e| format!("read the local data owner: {e}"))?
        .filter(account_binding::Identity::is_known)
    else {
        return Ok(false);
    };
    let complete = account_binding::Identity::new(Some(&profile.user_id), Some(&email));
    // The engine start's own comparison (ruling A″): the ids when the owner has one, else the canonical email.
    let this_account = account_binding::same_account(&owner, &complete) == Some(true);
    if !this_account || owner == complete {
        return Ok(false);
    }
    db.set_owner(&complete)
        .map_err(|e| format!("record the local data owner: {e}"))?;
    Ok(true)
}

pub(super) async fn fetch_session_profile(
    client: &reqwest::Client,
    base_url: &str,
    session_token: &str,
) -> Result<account_dto::AccountProfile, String> {
    let resp = client
        .get(format!("{base_url}/api/v1/auth/me"))
        .bearer_auth(session_token)
        .send()
        .await
        .map_err(|e| format!("session verification failed: {e}"))?;
    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return Err(format!("Session verification failed ({status}): {body}"));
    }
    resp.json::<account_dto::AccountProfile>()
        .await
        .map_err(|e| format!("parse session verification response: {e}"))
}

/// How `POST /api/v1/auth/logout` for one session ended (task 1886). Closed: logged by its code, never with the token,
/// the request or reqwest's text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum LogoutOutcome {
    /// The server answered 2xx: the session is ended there too.
    Revoked,
    /// The server answered 401: it had already ended the session.
    AlreadyEnded,
    /// The server answered with this other status.
    Refused(u16),
    /// The request did not reach the server, or no answer came back (offline, refused, DNS, TLS).
    Unreachable,
    /// The request could not be built (the API address is not a URL).
    Misconfigured,
    /// No answer within [`SIGN_OUT_REVOKE_TIMEOUT`] (a sign-out's revoke only).
    TimedOut,
    /// The HTTP client could not be built (a sign-out's revoke only).
    NoClient,
}

impl LogoutOutcome {
    pub(super) fn code(self) -> &'static str {
        match self {
            Self::Revoked => "revoked",
            Self::AlreadyEnded => "already_ended",
            Self::Refused(_) => "refused",
            Self::Unreachable => "unreachable",
            Self::Misconfigured => "misconfigured",
            Self::TimedOut => "timed_out",
            Self::NoClient => "no_client",
        }
    }
}

/// End `session_token`'s session on the server. Never logs; the token goes only into the request's header.
pub(super) async fn revoke_desktop_session(client: &reqwest::Client, base_url: &str, session_token: &str) -> LogoutOutcome {
    let response = match client
        .post(format!("{base_url}/api/v1/auth/logout"))
        .bearer_auth(session_token)
        .send()
        .await
    {
        Ok(response) => response,
        Err(error) if error.is_builder() => return LogoutOutcome::Misconfigured,
        Err(_) => return LogoutOutcome::Unreachable,
    };
    match response.status() {
        status if status.is_success() => LogoutOutcome::Revoked,
        reqwest::StatusCode::UNAUTHORIZED => LogoutOutcome::AlreadyEnded,
        status => LogoutOutcome::Refused(status.as_u16()),
    }
}

/// How long a sign-out waits for the server to end its sessions, all of them together (task 1886, as Windows always
/// waited): after this the sign-out goes on without the answers.
pub(super) const SIGN_OUT_REVOKE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(3);

/// The session tokens a sign-out took off this computer (task 1886), each kept once: the one it began with, and every one
/// its clears removed (a session written while it ran included). Each is ended on the server once the local clear has
/// run ([`end_dropped_sessions`]). Wiped on drop; never printed (no `Debug`).
#[derive(Default)]
pub(super) struct DroppedTokens(Vec<zeroize::Zeroizing<String>>);

impl DroppedTokens {
    pub(super) fn add(&mut self, token: Option<zeroize::Zeroizing<String>>) {
        if let Some(token) = token
            && !self.0.iter().any(|kept| **kept == *token)
        {
            self.0.push(token);
        }
    }
}

/// The token a sign-out begins with: the session's in memory, else the one the Keychain holds (a locked computer keeps
/// only that). Read once, before the sign-out takes the engine slot: from the slot on, nothing in the sign-out touches
/// the session until its clear (`sign_out_holds_the_engine_slot_until_the_session_is_cleared`).
pub(super) fn session_token_for_revoke(acct: &AccountRuntime) -> Option<zeroize::Zeroizing<String>> {
    acct.session
        .lock()
        .ok()
        .and_then(|session| session.as_ref().map(|session| zeroize::Zeroizing::new(session.token.clone())))
        .or_else(|| {
            load_session_token_from_keychain(acct.id.as_str())
                .ok()
                .flatten()
                .map(zeroize::Zeroizing::new)
        })
}

/// The session token one Keychain layout holds, if it can say (task 1886: a clear reads what it is about to remove).
pub(super) fn stored_session_token<S: keychain::AuthSecretStore>(vault: &AuthVault<S>) -> Option<zeroize::Zeroizing<String>> {
    vault
        .session_token()
        .ok()
        .flatten()
        .map(|token| zeroize::Zeroizing::new(token.expose_for_request().to_string()))
}

/// Every session token this computer still holds for `acct`: in memory and in both Keychain layouts. `None` when a
/// store cannot say (one that holds no secrets, `Unsupported`, or none, `NotFound`, holds none).
pub(super) fn session_tokens_held_here(acct: &AccountRuntime) -> Option<Vec<zeroize::Zeroizing<String>>> {
    let mut held: Vec<zeroize::Zeroizing<String>> = acct
        .session
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .as_ref()
        .map(|session| zeroize::Zeroizing::new(session.token.clone()))
        .into_iter()
        .collect();
    for vault in [
        AuthVault::new(platform_keychain_store_for(acct.id.as_str())),
        AuthVault::new(keychain::legacy_platform_keychain_store()),
    ] {
        match vault.session_token() {
            Ok(token) => held.extend(token.map(|token| zeroize::Zeroizing::new(token.expose_for_request().to_string()))),
            Err(keychain::AuthStoreError::NotFound | keychain::AuthStoreError::Unsupported(_)) => {}
            Err(_) => return None,
        }
    }
    Some(held)
}

/// Task 1886 (P0): end these sessions on the server, all at once and within one [`SIGN_OUT_REVOKE_TIMEOUT`]. Best
/// effort: each outcome is logged by its closed code, never with a token. A 401 means the server had already ended the
/// session, which is no warning.
pub(super) async fn revoke_sessions_for_sign_out(tokens: &[&str], base_url: &str) -> Vec<LogoutOutcome> {
    let deadline = tokio::time::Instant::now() + SIGN_OUT_REVOKE_TIMEOUT;
    let outcomes = match reqwest::Client::builder()
        .default_headers(api_client::provenance_headers())
        .build()
    {
        Err(_) => vec![LogoutOutcome::NoClient; tokens.len()],
        Ok(client) => {
            futures_util::future::join_all(tokens.iter().map(|token| {
                let client = &client;
                async move {
                    tokio::time::timeout_at(deadline, revoke_desktop_session(client, base_url, token))
                        .await
                        .unwrap_or(LogoutOutcome::TimedOut)
                }
            }))
            .await
        }
    };
    for &outcome in &outcomes {
        match outcome {
            LogoutOutcome::Revoked => tracing::info!(outcome = outcome.code(), "sign-out: the server ended the session"),
            LogoutOutcome::AlreadyEnded => {
                tracing::info!(outcome = outcome.code(), "sign-out: the server had already ended the session")
            }
            LogoutOutcome::Refused(status) => tracing::warn!(
                outcome = outcome.code(),
                status,
                "sign-out: the server did not end the session; signed out on this computer anyway"
            ),
            _ => tracing::warn!(
                outcome = outcome.code(),
                "sign-out: the server could not be asked to end the session; signed out on this computer anyway"
            ),
        }
    }
    outcomes
}

/// Task 1886 (P0): a sign-out ends on the server every session it took off this computer. Before, only Windows did, and
/// only for the token it began with, so a Mac's or a Linux computer's signed-out token stayed valid on the server until
/// it expired (30 days after the sign-in). Runs once the local clear has run (`cleared`: it completed) and the engine
/// slot is free, so `sync_status`, Lock, Repair and an engine start never wait for the server. After a clear that
/// completed nothing of the account is left here, so every dropped token is ended. After one that failed, only the
/// tokens this computer no longer holds are: one it still holds keeps working here, and trying the sign-out again ends
/// it; when a store cannot say what it holds, none is ended.
pub(super) async fn end_dropped_sessions(acct: &AccountRuntime, dropped: DroppedTokens, cleared: bool) {
    let mut tokens = dropped.0;
    if !cleared {
        match session_tokens_held_here(acct) {
            Some(held) => tokens.retain(|token| !held.iter().any(|kept| **kept == **token)),
            None => {
                tracing::warn!(
                    dropped = tokens.len(),
                    "sign-out paused: this computer could not say which sessions it still holds; none was ended on the server"
                );
                return;
            }
        }
    }
    if tokens.is_empty() {
        return;
    }
    let tokens: Vec<&str> = tokens.iter().map(|token| token.as_str()).collect();
    revoke_sessions_for_sign_out(&tokens, &sign_out_api_base()).await;
}


/// Test builds: run `body` with `base` as the API a sign-out's revoke goes to (task 1886), so a test drives the real
/// sign-out against a server it started. Only this task sees it. (No Windows test drives a signed-in sign-out.)
#[cfg(all(test, not(target_os = "windows")))]
pub(super) async fn with_sign_out_api_for_test<T>(base: String, body: impl std::future::Future<Output = T>) -> T {
    SIGN_OUT_API_FOR_TEST.scope(base, body).await
}

/// The API a sign-out's revoke goes to: [`runner::api_base_url`]. A test build never reads `BB_API_BASE` here (a test
/// running beside it may point that at its own server): it uses the server its own test started
/// ([`with_sign_out_api_for_test`]), else the closed loopback port every test build defaults to.
pub(super) fn sign_out_api_base() -> String {
    #[cfg(test)]
    return SIGN_OUT_API_FOR_TEST
        .try_with(Clone::clone)
        .unwrap_or_else(|_| runner::TEST_API_BASE.to_string());
    #[cfg(not(test))]
    runner::api_base_url()
}

// ── IPC commands: session ─────────────────────────────────────────────────────

/// Persist a freshly-authenticated session and start the engine.
///
/// This is the shared core used by `start_browser_login` (the live browser-
/// handoff flow): it arrives at a session token + 32-byte master key + email
/// and needs write-to-credential-store + stash-in-memory + spawn-the-engine
/// side-effects. (Task 1479: this used to also be the core of a `set_session`
/// IPC command invoked by the WebView after an in-webview login — removed
/// because no desktop window ever loads the web client, so nothing could
/// call it. `apply_session` itself is untouched; it's still live.)
pub(crate) async fn apply_session(
    app: tauri::AppHandle,
    state: &State<'_, AppState>,
    token: String,
    master_key: &[u8; 32],
    profile: Option<account_dto::AccountProfile>,
    turn: &mut account::SessionGeneration,
    #[cfg(target_os = "windows")] attempt: &auth_attempts::Attempt,
) -> Result<(), String> {
    let token = zeroize::Zeroizing::new(token);
    // The session is known by the server's spelling of the email and by nothing the browser handed over (R2). With
    // no profile (the fetch failed) it has no email: it is installed, and no engine starts for it until a fetch
    // names it.
    let email = session_email(profile.as_ref());
    #[cfg(target_os = "windows")]
    let _transition = SESSION_TRANSITION.lock().await;
    #[cfg(target_os = "windows")]
    {
        SESSION_COMMANDS.ensure_reactivation_allowed()?;
        windows_cf::ensure_reactivation_allowed()?;
    }
    #[cfg(target_os = "windows")]
    if state.auth_present.lock().map(|present| *present).unwrap_or(true) {
        return Err("Sign out of the current account before signing in again.".into());
    }

    #[cfg(target_os = "windows")]
    AUTH_ATTEMPTS.validate(attempt)?;

    let acct = state.active_account()?;
    install_new_session_or_revoke(
        state,
        &acct,
        turn,
        &token,
        master_key,
        email,
        profile,
        &runner::api_base_url(),
    )
    .await?;
    bump_vault_epoch();
    tracing::info!("session installed");

    // If we already know the sync_root, kick off the engine. Otherwise
    // it'll start when the first-launch picker resolves.
    let started = start_engine_if_possible(
        app,
        state,
        #[cfg(target_os = "windows")]
        &_transition,
    )
    .await;
    keys_arrived(state, KeysFrom::SignIn);
    started
}

/// What [`install_new_session`] did.
#[derive(Debug, PartialEq, Eq)]
pub(super) enum NewSession {
    Installed,
    /// Another session transition happened since the sign-in began: nothing was stored (the caller revokes it).
    Refused(String),
}

/// The browser sign-in's writes, in its turn (lead ruling 1, Task 12): its generation was captured before it looked at
/// this computer, so a Lock, a Sign-out or another sign-in that completed meanwhile stops it here, before anything is
/// stored. The Keychain (token, vault key, email), the cached profile, the session in memory and the flags are written
/// in the same turn. `Err`: a write failed after the turn was taken.
pub(super) fn install_new_session(
    state: &AppState,
    acct: &AccountRuntime,
    turn: &mut account::SessionGeneration,
    token: &str,
    master_key: &[u8; 32],
    email: Option<String>,
    profile: Option<account_dto::AccountProfile>,
) -> Result<NewSession, String> {
    let write = match claim_session_write(acct, turn) {
        Ok(write) => write,
        Err(changed) => return Ok(NewSession::Refused(changed.sign_in_sentence().to_string())),
    };
    persist_session_to_keychain(&write, acct.id.as_str(), token, master_key, email.as_deref())?;
    if let Some(profile) = profile
        && let Ok(mut cached) = acct.cached_profile.lock()
    {
        *cached = Some(profile);
    }
    let mut guard = acct.session.lock().map_err(|_| "session mutex poisoned".to_string())?;
    *guard = Some(Session {
        token: token.to_string(),
        master_key: *master_key,
        email: email.clone(),
    });
    drop(guard);
    set_auth_present(state, true);
    set_auth_email(state, email);
    // Ruling F9: sync resumes; on every OS since spec D11 (task 1888).
    ask_to_resume_auth_paused(&write, acct);
    drop(write);
    Ok(NewSession::Installed)
}

/// [`install_new_session`], and when its turn is refused, the revoke of the session it would have stored (nothing of it
/// is on this computer), against the server at `base_url`, like a sign-in that could not store its token.
#[allow(clippy::too_many_arguments)]
pub(super) async fn install_new_session_or_revoke(
    state: &AppState,
    acct: &AccountRuntime,
    turn: &mut account::SessionGeneration,
    token: &str,
    master_key: &[u8; 32],
    email: Option<String>,
    profile: Option<account_dto::AccountProfile>,
    base_url: &str,
) -> Result<(), String> {
    if let NewSession::Refused(error) = install_new_session(state, acct, turn, token, master_key, email, profile)? {
        if let Ok(client) = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(15))
            .default_headers(api_client::provenance_headers())
            .build()
        {
            let _ = revoke_desktop_session(&client, base_url, token).await;
        }
        return Err(error);
    }
    Ok(())
}

/// What [`clear_session_impl`] actually did, so the native menu can tell
/// "you were already signed out" apart from a completed teardown.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum SignOutOutcome {
    /// Full teardown ran (or completed idempotently).
    Completed,
    /// Nothing was signed in — engine stop and Cloud Files teardown were
    /// skipped; only the idempotent cleanups (keychain clear, local purge)
    /// ran.
    NotSignedIn,
}

/// What a sign-out or a Lock that HAPPENED could not confirm (FB-24): a closed code and its constant sentence,
/// serialized as `{"code": …, "sentence": …}` like `sync_status.engine_refusal`. An `Err` from either command keeps
/// meaning "it did not happen". The variant names mirror the wire codes (`finder_removal_unconfirmed`, …).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(clippy::enum_variant_names)]
pub(super) enum ActionWarning {
    /// Signed out, keys cleared, but the removal from Finder was not confirmed (macOS).
    #[cfg_attr(not(target_os = "macos"), allow(dead_code))]
    FinderRemovalUnconfirmed,
    /// Locked, keys cleared, but the reconciler did not confirm it holds (macOS).
    #[cfg_attr(not(target_os = "macos"), allow(dead_code))]
    FinderLockUnconfirmed,
    /// Locked, keys cleared, but an engine stop (this one or an earlier one) was never confirmed (macOS and Linux).
    #[cfg_attr(target_os = "windows", allow(dead_code))]
    EngineStopUnconfirmed,
}

impl ActionWarning {
    pub(super) fn code(self) -> &'static str {
        match self {
            Self::FinderRemovalUnconfirmed => "finder_removal_unconfirmed",
            Self::FinderLockUnconfirmed => "finder_lock_unconfirmed",
            Self::EngineStopUnconfirmed => "engine_stop_unconfirmed",
        }
    }

    pub(super) fn sentence(self) -> &'static str {
        match self {
            Self::FinderRemovalUnconfirmed => FINDER_SIGN_OUT_UNCONFIRMED_WARNING,
            Self::FinderLockUnconfirmed => FINDER_LOCK_UNCONFIRMED_WARNING,
            Self::EngineStopUnconfirmed => LOCK_ENGINE_UNCONFIRMED_WARNING,
        }
    }
}

impl serde::Serialize for ActionWarning {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeStruct as _;
        let mut warning = serializer.serialize_struct("ActionWarning", 2)?;
        warning.serialize_field("code", self.code())?;
        warning.serialize_field("sentence", self.sentence())?;
        warning.end()
    }
}

/// What `clear_session` and `lock_vault` return when the action happened. `warning` is always serialized (null when
/// every step was confirmed).
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub(super) struct SessionActionOutcome {
    pub(super) warning: Option<ActionWarning>,
}

/// Spec 2026-10-10 §5.2 step 10: the folder the keep step made and its file count. Never logged.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct KeptUnsent {
    pub(super) folder: String,
    pub(super) files: usize,
}

/// Task 2191 (review thread on #157): a sign-out failure after the Windows purge, carrying the
/// folder it kept copies of unknown files in (or the macOS keep step's), so the alert can name it.
#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
pub(super) fn sign_out_failure_after_purge(
    message: String,
    preserved_location: &Option<String>,
    kept_unsent: &Option<KeptUnsent>,
    windows_kept: &Option<KeptUnsent>,
) -> SignOutFailure {
    SignOutFailure {
        message,
        preserved_location: preserved_location.clone(),
        kept_unsent: kept_unsent.clone().or_else(|| windows_kept.clone()),
    }
}

/// What [`clear_session_impl`] did, and what of it could not be confirmed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct SignedOut {
    pub(super) outcome: SignOutOutcome,
    pub(super) warning: Option<ActionWarning>,
    /// Task 1882: the folder where macOS kept the Finder files that had not reached the server when the
    /// sign-out removed the Finder location. Shown to the person in the sign-out's alert
    /// (`surface_sign_out_folders`), never logged.
    pub(super) preserved_location: Option<String>,
    /// Task 1887: the folder the sign-out's keep step copied the waiting Finder saves into (macOS). Never logged.
    pub(super) kept_unsent: Option<KeptUnsent>,
}

#[cfg(test)]
impl SignedOut {
    /// A sign-out whose every step was confirmed.
    pub(super) fn confirmed(outcome: SignOutOutcome) -> Self {
        Self {
            outcome,
            warning: None,
            preserved_location: None,
            kept_unsent: None,
        }
    }
}

/// [`clear_session_impl`]'s failure. Review I1 (task 1882 round 2): a sign-out can still fail AFTER it
/// removed the Finder location, and the folder macOS kept must not be lost with it: the error carries it,
/// and the alert is raised before the error is returned.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct SignOutFailure {
    pub(super) message: String,
    /// The folder macOS kept, when the failure came after the removal. Never logged.
    pub(super) preserved_location: Option<String>,
    /// Task 1887: the folder the keep step made, when the failure came after the keep step. Never logged.
    pub(super) kept_unsent: Option<KeptUnsent>,
}

impl From<String> for SignOutFailure {
    fn from(message: String) -> Self {
        Self {
            message,
            preserved_location: None,
            kept_unsent: None,
        }
    }
}

impl From<&str> for SignOutFailure {
    fn from(message: &str) -> Self {
        message.to_string().into()
    }
}

impl std::fmt::Display for SignOutFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

/// The unconfirmed-stop refusal (Bug A / task 1538 Codex P1). Shared by the
/// fresh-abort path and the Bug-A2 retry gate so a retry cannot be
/// distinguished from a first refusal by its message.
pub(super) const UNCONFIRMED_ENGINE_STOP_ERROR: &str = "Could not stop the sync engine. Please try signing out again; if this keeps \
     happening, restart Beebeeb before signing in with a different account.";
