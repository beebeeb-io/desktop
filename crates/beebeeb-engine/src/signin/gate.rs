//! The rules the auth gate and the sign-in screens share (task 2445, Guus 2026-11-11: "every rule
//! in Rust, not in TSX"). A non-Tauri caller (a future native UI) calls these directly; the TSX
//! calls them through the `auth_gate_state` and `auth_sign_in_problem` commands.
//!
//! Nothing here returns or keeps a key, a token, or a credential. It reads the flags the sign-in and
//! lock paths already set (`auth_present`, `session`, `sync_status`) and, for a signed-in locked
//! window, asks the credential store whether a usable vault key is there (`stored_key_state_for`;
//! on Windows that check loads the key and drops it at once). The engine start is still gated by
//! `keys_for_engine_start` (session_reauth.rs); this module decides what the WINDOW shows.

use serde::Serialize;

use crate::signin::key_owner::StoredKey;

/// What the main window shows when it opens (or when it checks again on focus). The TypeScript
/// gate reads this once and picks a screen; every screen checks again on `focus` (DW-0014).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum GateState {
    /// The window is ready: signed in, vault unlocked, plan present.
    Ready,
    /// No session token at all: show sign-in (DW-A02/A03).
    SignedOut,
    /// Signed in, vault key IS in the keychain: show the lock screen (DW-0009, Windows Hello).
    Locked,
    /// Signed in, but no vault key in keychain at all: show the recovery phrase (DW-A23).
    /// A first sign-in by password ends here; the 12 words (or another device) provision the key.
    NeedsRecoveryPhrase,
    /// Signed in and unlocked, but the account has no plan: show the choose-a-plan screen (DW-0014).
    NoPlan,
    /// Signed in, but the credential store could not say whether the vault key is there (P2-6). Neither
    /// "locked" (an unlock would fail) nor "needs the phrase" (a key may be there): the window says so
    /// and asks again.
    KeyStoreUnavailable,
}

/// What follows a successful `desktop_login` or `desktop_login_2fa` on this PC.
/// The TSX reads this from the `LoginOutcome` and picks the next screen.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum NextAfterSignIn {
    /// The vault is unlocked (browser handoff, device approval, or the keychain had the key):
    /// go to Files (DW-0201 via DW-0401).
    Drive,
    /// The vault key is not on this PC: ask for the 12 words (DW-A23) or approve from another
    /// device (DW-A07).
    RecoveryPhrase,
    /// A TOTP/backup code is needed first (DW-A26/A28).
    TwoFactor,
    /// Re-sign-in of the same account: vault is already unlocked, go to Files.
    Reauthenticated,
    /// Another account tried to sign in: show the mismatch warning.
    AccountMismatch,
}

/// Decide what follows a sign-in from its outcome fields.
pub fn next_after_sign_in(
    requires_2fa: bool,
    vault_unlocked: bool,
    reauthenticated: bool,
    account_mismatch: bool,
) -> NextAfterSignIn {
    if requires_2fa {
        return NextAfterSignIn::TwoFactor;
    }
    if account_mismatch {
        return NextAfterSignIn::AccountMismatch;
    }
    if reauthenticated {
        return NextAfterSignIn::Reauthenticated;
    }
    if vault_unlocked {
        return NextAfterSignIn::Drive;
    }
    // A password sign-in on a PC that holds no keys: the 12 words or another device (DW-A23).
    NextAfterSignIn::RecoveryPhrase
}

// ── Sign-in error mapping (the board's words for each code) ─────────────────

/// The sign-in form's error state (DW-A19 to DW-A22, DW-A63).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SignInProblem {
    /// Wrong email or password (DW-A19). Anti-enumeration: one message for all.
    Invalid,
    /// The client's own fifth consecutive failure suggests a lock (DW-A22).
    LockedHint,
    /// No connection (DW-A20).
    Offline,
    /// HTTP 429: wait the named seconds (DW-A21).
    RateLimited { seconds: u32 },
    /// Another account's vault key is still stored on this PC and could not be removed (task 2447):
    /// the sign-in was refused and nothing was stored.
    StaleKey,
    /// Anything else: "Sign-in didn't finish."
    Other,
}

/// DW-A22: the client's own count of consecutive failures at which the lock hint appears.
pub const LOCK_HINT_AFTER: u32 = 5;

/// Map a command's error code to the form's problem.
pub fn problem_from(code: &str, consecutive_failures: u32) -> SignInProblem {
    if code == "offline" || code.contains("network error") || code.contains("could not reach") {
        return SignInProblem::Offline;
    }
    if code.starts_with("rate_limited") {
        let seconds = code.split(':').nth(1).and_then(|s| s.parse::<u32>().ok()).unwrap_or(30);
        return SignInProblem::RateLimited { seconds };
    }
    if code == crate::signin::key_owner::FOREIGN_KEY_NOT_REMOVED {
        return SignInProblem::StaleKey;
    }
    // P3: `account_deleted` is one of the "wrong email or password" codes here. The sign-in form answers before
    // the password is proven, so it must not say that an account exists or existed (DW-A63 moves to a place that
    // follows a proven password).
    if code == "invalid_credentials" || code == "account_deleted" || code.contains("Invalid email or password") {
        return if consecutive_failures >= LOCK_HINT_AFTER {
            SignInProblem::LockedHint
        } else {
            SignInProblem::Invalid
        };
    }
    SignInProblem::Other
}

/// The board's words for each problem (the TSX shows this verbatim).
pub fn problem_text(p: &SignInProblem) -> &'static str {
    match p {
        SignInProblem::Invalid | SignInProblem::LockedHint => {
            "That email and password don\u{2019}t match. Nothing was unlocked."
        }
        SignInProblem::Offline => "You\u{2019}re offline. Nothing was sent.",
        // The countdown is rendered by the caller; this is the static part.
        SignInProblem::RateLimited { .. } => "Too many tries from this network. Wait and try again.",
        SignInProblem::StaleKey => crate::signin::key_owner::FOREIGN_KEY_NOT_REMOVED,
        SignInProblem::Other => "Sign-in didn\u{2019}t finish. Nothing was unlocked. Try again.",
    }
}

/// DW-A22's note (a hint, not a diagnosis).
pub const LOCK_NOTE: &str = "After five wrong tries in a row, your account locks for a short \
     while. It unlocks by itself, and we\u{2019}ve emailed the account owner a link to unlock it now.";

// ── Two-factor step (DW-A26 to DW-A28) ─────────────────────────────────────

/// Two-factor: wrong code (DW-A27).
pub const WRONG_2FA_CODE: &str = "That code didn\u{2019}t work. Check the app and try again.";

/// Two-factor: the partial token expired (~5 min) before the code was sent.
pub const TWO_FACTOR_EXPIRED: &str = "Your sign-in expired. Start again from the password.";

/// Map a 2FA verify error to the board's words. A wrong code stays retryable;
/// an expired partial token starts over; anything else is honest.
pub fn two_factor_error(code: &str) -> &'static str {
    if code == "invalid_credentials" || code.contains("Invalid authentication code") {
        return WRONG_2FA_CODE;
    }
    if code.contains("expired") || code.contains("No pending") {
        return TWO_FACTOR_EXPIRED;
    }
    if code == "offline" || code.contains("network error") || code.contains("could not reach") {
        return "You\u{2019}re offline. Nothing was sent.";
    }
    if code.starts_with("rate_limited") {
        return "Too many tries from this network. Wait and try again.";
    }
    "The code check didn\u{2019}t finish. Try again."
}

// ── Recovery-phrase step (DW-A23 to DW-A25) ─────────────────────────────────

/// The recovery-phrase step's error text (DW-A24).
pub const WRONG_PHRASE: &str = "That phrase doesn\u{2019}t open this account. Nothing was restored.";

/// DW-A24: after the server's attempt limit.
pub const TOO_MANY_PHRASE_TRIES: &str = "Too many tries. Wait an hour.";

/// P3: what a recovery-phrase unlock returns when the phrase was RIGHT (the vault is unlocked) and the engine start
/// then failed. Its own sentence, so no screen can show it as a wrong phrase; it is a sentence and not a code because
/// the older sign-in screens print the command's error as it is.
pub const UNLOCKED_SYNC_NOT_STARTED: &str =
    "Your recovery phrase is right and this computer is unlocked. Sync didn\u{2019}t start yet. Try again.";

/// The recovery-phrase command's error for an engine start that failed after the phrase was accepted.
pub fn engine_not_started_after_unlock(error: String) -> String {
    tracing::warn!(%error, "the recovery phrase unlocked the vault, but the engine did not start");
    UNLOCKED_SYNC_NOT_STARTED.to_string()
}

/// Map a recovery-phrase error to the board's words.
pub fn phrase_error(code: &str) -> &'static str {
    if code == UNLOCKED_SYNC_NOT_STARTED {
        return UNLOCKED_SYNC_NOT_STARTED;
    }
    if code.contains("Incorrect recovery phrase") {
        return WRONG_PHRASE;
    }
    if code.contains("too many") || code.contains("Too many") {
        return TOO_MANY_PHRASE_TRIES;
    }
    if code == "offline" || code.contains("network error") || code.contains("could not reach") {
        return "You\u{2019}re offline. Nothing was sent.";
    }
    if code.contains("Sign in before") {
        return "Sign in before entering the recovery phrase.";
    }
    "The phrase check didn\u{2019}t finish. Try again."
}

// ── Another-device step (DW-A07 to DW-A10) ─────────────────────────────────

/// Map a device-request status to the board's text (DW-A08 to DW-A10).
pub fn device_status_text(status: &str) -> &'static str {
    match status {
        "denied" => "The request was denied on the other device.",
        "expired" => "Nobody approved within 5 minutes. Ask again.",
        "keys_did_not_arrive" => "The other device approved, but the keys didn\u{2019}t arrive. Ask again.",
        "failed" => "Something went wrong. Ask again.",
        "signed_in" => "Signed in.",
        "approved" => "Approved. Setting up\u{2026}",
        "reconnecting" => "Reconnecting\u{2026}",
        _ => "",
    }
}

// ── Approve step (DW-A11 to DW-A17) ────────────────────────────────────────

/// Map an approve error to the board's words.
pub fn approve_error(code: &str) -> &'static str {
    if code == "request_not_found" {
        return "We could not find a waiting request with that code. Check it on the other device.";
    }
    if code == "invalid_credentials" {
        return "That password isn\u{2019}t right. Nothing was shared.";
    }
    if code == "hello_unavailable" {
        return "Windows Hello isn\u{2019}t set up on this PC. Set it up in Windows Settings to \
                approve sign-ins.";
    }
    if code == "hello_failed" {
        return "Windows Hello didn\u{2019}t recognise you. Nothing was shared.";
    }
    if code == "offline" || code.contains("network error") {
        return "You\u{2019}re offline. Nothing was shared.";
    }
    "The approval didn\u{2019}t finish. Nothing was shared."
}

// ── Engine gate ─────────────────────────────────────────────────────────────

/// Whether the engine may start. A caller (the TSX gate, the startup path, or a
/// future native caller) checks this before asking for an engine start. This is
/// a READ of the existing session state, never a write.
///
/// The actual engine start guard is `keys_for_engine_start` in `session_reauth.rs`,
/// which reads the session under the engine slot. This function is for the UI: it
/// tells the person whether sync is possible right now, without acquiring locks.
pub fn engine_allowed(vault_unlocked: bool, has_plan: bool) -> bool {
    vault_unlocked && has_plan
}

// ── Tauri commands ──────────────────────────────────────────────────────────

/// Decide the gate state. `stored_key` (asked only for a signed-in, locked window) distinguishes the lock screen (a
/// key the loader accepts: Windows Hello can unlock) from the recovery-phrase step (no key, or a key that is not this
/// account's: the 12 words or another device provide it) and from a store that could not answer.
pub fn decide_gate(logged_in: bool, vault_unlocked: bool, stored_key: StoredKey, has_no_plan: bool) -> GateState {
    if !logged_in {
        return GateState::SignedOut;
    }
    if !vault_unlocked {
        return match stored_key {
            StoredKey::Usable => GateState::Locked,
            StoredKey::Absent | StoredKey::NotThisAccounts => GateState::NeedsRecoveryPhrase,
            StoredKey::Unreadable => GateState::KeyStoreUnavailable,
        };
    }
    if has_no_plan {
        return GateState::NoPlan;
    }
    GateState::Ready
}

/// No plan yet: the server's `plan_state` says `none`. An older server without
/// it: no plan or no quota. Same logic as plan.ts `hasNoPlan`, in Rust.
pub fn has_no_plan(
    plan_state: Option<&str>,
    plan: Option<&str>,
    status: Option<&str>,
    quota_bytes: Option<i64>,
) -> bool {
    // The server's `plan_state` field (server #422) is the authority when present.
    if let Some(state) = plan_state.filter(|s| !s.is_empty()) {
        return state == "none";
    }
    // Older server: check the plan name, then the status, then the quota.
    if let Some(p) = plan.filter(|s| !s.is_empty()) {
        return p.eq_ignore_ascii_case("none");
    }
    if let Some(s) = status.filter(|s| !s.is_empty()) {
        return s.eq_ignore_ascii_case("none");
    }
    if let Some(q) = quota_bytes {
        return q == 0;
    }
    // No plan fields at all (the subscription call failed): fail open — show Files.
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn next_after_password_sign_in_without_keys_is_recovery_phrase() {
        assert_eq!(
            next_after_sign_in(false, false, false, false),
            NextAfterSignIn::RecoveryPhrase
        );
    }

    #[test]
    fn next_after_sign_in_with_keys_is_drive() {
        assert_eq!(next_after_sign_in(false, true, false, false), NextAfterSignIn::Drive);
    }

    #[test]
    fn next_after_sign_in_with_2fa_is_two_factor() {
        assert_eq!(
            next_after_sign_in(true, false, false, false),
            NextAfterSignIn::TwoFactor
        );
    }

    #[test]
    fn next_after_reauth_is_reauthenticated() {
        assert_eq!(
            next_after_sign_in(false, true, true, false),
            NextAfterSignIn::Reauthenticated
        );
    }

    #[test]
    fn next_after_mismatch_is_account_mismatch() {
        assert_eq!(
            next_after_sign_in(false, false, false, true),
            NextAfterSignIn::AccountMismatch
        );
    }

    #[test]
    fn problem_from_maps_every_board_code() {
        assert_eq!(problem_from("invalid_credentials", 1), SignInProblem::Invalid);
        assert_eq!(problem_from("invalid_credentials", 5), SignInProblem::LockedHint);
        assert_eq!(problem_from("offline", 0), SignInProblem::Offline);
        assert_eq!(
            problem_from("rate_limited:45", 0),
            SignInProblem::RateLimited { seconds: 45 }
        );
        // P3: never its own kind on the sign-in form (it would say the account existed before a password is proven).
        assert_eq!(problem_from("account_deleted", 0), SignInProblem::Invalid);
        assert_eq!(
            problem_text(&problem_from("account_deleted", 0)),
            problem_text(&problem_from("invalid_credentials", 0))
        );
        assert_eq!(problem_from("server_error", 0), SignInProblem::Other);
    }

    #[test]
    fn problem_text_returns_the_boards_words() {
        assert!(problem_text(&SignInProblem::Invalid).contains("don\u{2019}t match"));
        assert!(problem_text(&SignInProblem::Offline).contains("offline"));
        assert!(problem_text(&SignInProblem::StaleKey).contains("Nothing was signed in"));
    }

    #[test]
    fn engine_not_allowed_without_vault() {
        assert!(!engine_allowed(false, true));
        assert!(!engine_allowed(false, false));
        assert!(!engine_allowed(true, false));
        assert!(engine_allowed(true, true));
    }

    #[test]
    fn problem_from_network_errors() {
        assert_eq!(
            problem_from("network error: connection refused", 0),
            SignInProblem::Offline
        );
        assert_eq!(problem_from("could not reach the server", 0), SignInProblem::Offline);
    }

    #[test]
    fn problem_from_invalid_email_or_password_text() {
        assert_eq!(problem_from("Invalid email or password", 1), SignInProblem::Invalid);
        assert_eq!(problem_from("Invalid email or password", 5), SignInProblem::LockedHint);
    }

    #[test]
    fn rate_limited_without_seconds_defaults_to_30() {
        assert_eq!(
            problem_from("rate_limited:", 0),
            SignInProblem::RateLimited { seconds: 30 }
        );
        assert_eq!(
            problem_from("rate_limited", 0),
            SignInProblem::RateLimited { seconds: 30 }
        );
    }

    #[test]
    fn decide_gate_signed_out() {
        assert_eq!(
            decide_gate(false, false, StoredKey::Absent, false),
            GateState::SignedOut
        );
    }

    #[test]
    fn decide_gate_locked_with_key() {
        assert_eq!(decide_gate(true, false, StoredKey::Usable, false), GateState::Locked);
    }

    #[test]
    fn decide_gate_needs_recovery_phrase() {
        assert_eq!(
            decide_gate(true, false, StoredKey::Absent, false),
            GateState::NeedsRecoveryPhrase
        );
    }

    #[test]
    fn decide_gate_no_plan() {
        assert_eq!(decide_gate(true, true, StoredKey::Absent, true), GateState::NoPlan);
    }

    #[test]
    fn decide_gate_ready() {
        assert_eq!(decide_gate(true, true, StoredKey::Absent, false), GateState::Ready);
    }

    #[test]
    fn has_no_plan_with_plan_state_none() {
        assert!(has_no_plan(Some("none"), None, None, None));
        assert!(!has_no_plan(Some("active"), None, None, None));
    }

    #[test]
    fn has_no_plan_with_plan_name() {
        assert!(has_no_plan(None, Some("none"), None, None));
        assert!(!has_no_plan(None, Some("starter"), None, None));
        // An empty plan field is treated like absent (fail open).
        assert!(!has_no_plan(None, Some(""), None, None));
    }

    #[test]
    fn has_no_plan_with_zero_quota() {
        assert!(has_no_plan(None, None, None, Some(0)));
        assert!(!has_no_plan(None, None, None, Some(1073741824)));
    }

    #[test]
    fn has_no_plan_with_status_none() {
        assert!(has_no_plan(None, None, Some("none"), None));
        assert!(!has_no_plan(None, None, Some("active"), None));
    }

    // ── Two-factor error mapping ────────────────────────────────────────

    #[test]
    fn two_factor_wrong_code() {
        assert!(two_factor_error("invalid_credentials").contains("didn\u{2019}t work"));
        assert!(two_factor_error("Invalid authentication code").contains("didn\u{2019}t work"));
    }

    #[test]
    fn two_factor_expired() {
        assert!(two_factor_error("No pending two-factor sign-in").contains("expired"));
        assert!(two_factor_error("partial token expired").contains("expired"));
    }

    #[test]
    fn two_factor_offline() {
        assert!(two_factor_error("offline").contains("offline"));
        assert!(two_factor_error("network error: timeout").contains("offline"));
    }

    #[test]
    fn two_factor_rate_limited() {
        assert!(two_factor_error("rate_limited:30").contains("Too many"));
    }

    #[test]
    fn two_factor_other() {
        assert!(two_factor_error("unknown_server_error").contains("didn\u{2019}t finish"));
    }

    // ── Phrase error mapping ────────────────────────────────────────────

    #[test]
    fn phrase_wrong() {
        assert!(phrase_error("Incorrect recovery phrase").contains("doesn\u{2019}t open"));
    }

    #[test]
    fn phrase_too_many() {
        assert!(phrase_error("Too many tries").contains("Wait an hour"));
        assert!(phrase_error("too many requests").contains("Wait an hour"));
    }

    #[test]
    fn phrase_offline() {
        assert!(phrase_error("offline").contains("offline"));
    }

    #[test]
    fn phrase_no_session() {
        assert!(phrase_error("Sign in before unlocking the vault.").contains("Sign in before"));
    }

    #[test]
    fn phrase_other() {
        assert!(phrase_error("reqwest build: something").contains("didn\u{2019}t finish"));
    }

    // ── Device status text ──────────────────────────────────────────────

    #[test]
    fn device_status_denied() {
        assert!(device_status_text("denied").contains("denied"));
    }

    #[test]
    fn device_status_expired() {
        assert!(device_status_text("expired").contains("5 minutes"));
    }

    #[test]
    fn device_status_keys_did_not_arrive() {
        assert!(device_status_text("keys_did_not_arrive").contains("didn\u{2019}t arrive"));
    }

    // ── Approve error ───────────────────────────────────────────────────

    #[test]
    fn approve_not_found() {
        assert!(approve_error("request_not_found").contains("could not find"));
    }

    #[test]
    fn approve_wrong_password() {
        assert!(approve_error("invalid_credentials").contains("isn\u{2019}t right"));
    }

    #[test]
    fn approve_hello_unavailable() {
        assert!(approve_error("hello_unavailable").contains("isn\u{2019}t set up"));
    }

    #[test]
    fn approve_offline() {
        assert!(approve_error("offline").contains("offline"));
    }

    // ── P2-6 / P3 (task 2447 delta 3) ───────────────────────────────────

    #[test]
    fn a_store_that_cannot_answer_is_its_own_gate_state_and_a_foreign_key_asks_for_the_phrase() {
        assert_eq!(
            decide_gate(true, false, StoredKey::Unreadable, false),
            GateState::KeyStoreUnavailable
        );
        assert_eq!(
            decide_gate(true, false, StoredKey::NotThisAccounts, false),
            GateState::NeedsRecoveryPhrase
        );
        assert_eq!(
            serde_json::to_string(&GateState::KeyStoreUnavailable).unwrap(),
            "\"key_store_unavailable\""
        );
    }

    #[test]
    fn an_engine_start_that_fails_after_a_right_phrase_is_never_the_wrong_phrase_error() {
        // Whatever the start's own text says (it is logged, never shown as the reason).
        for start_error in [
            "network error: Incorrect recovery phrase",
            "offline",
            "Too many open files",
        ] {
            let reason = engine_not_started_after_unlock(start_error.to_string());
            assert_eq!(reason, UNLOCKED_SYNC_NOT_STARTED);
            assert_eq!(phrase_error(&reason), UNLOCKED_SYNC_NOT_STARTED);
        }
        assert_ne!(UNLOCKED_SYNC_NOT_STARTED, WRONG_PHRASE);
        assert_eq!(
            phrase_error("Incorrect recovery phrase. Check your words and try again."),
            WRONG_PHRASE
        );
    }

    #[test]
    fn a_sign_in_refused_over_a_leftover_key_says_so() {
        let problem = problem_from(crate::signin::key_owner::FOREIGN_KEY_NOT_REMOVED, 0);
        assert_eq!(problem, SignInProblem::StaleKey);
        assert_eq!(
            problem_text(&problem),
            crate::signin::key_owner::FOREIGN_KEY_NOT_REMOVED
        );
    }
}
