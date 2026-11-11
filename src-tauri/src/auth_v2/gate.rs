//! The rules the auth gate and the sign-in screens share (task 2445, Guus 2026-11-11: "every rule
//! in Rust, not in TSX"). A non-Tauri caller (a future native UI) calls these directly; the TSX
//! calls them through the `auth_gate_state` and `auth_sign_in_problem` commands.
//!
//! Nothing here reads a key, a token, or a credential; it reads only the flags the sign-in and
//! lock paths already set (`auth_present`, `session`, `sync_status`). The engine start is still
//! gated by `keys_for_engine_start` (session_reauth.rs); this module decides what the WINDOW shows.

use serde::Serialize;

/// What the main window shows when it opens (or when it checks again on focus). The TypeScript
/// gate reads this once and picks a screen; every screen checks again on `focus` (DW-0014).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum GateState {
    /// The window is ready: signed in, vault unlocked, plan present.
    Ready,
    /// No session token at all: show sign-in (DW-A02/A03).
    SignedOut,
    /// Signed in but the vault key is not in memory: show the lock screen (DW-0009, Windows Hello)
    /// or the recovery phrase (DW-A23) if this is a first sign-in with no keychain key.
    Locked,
    /// Signed in and unlocked, but the account has no plan: show the choose-a-plan screen (DW-0014).
    NoPlan,
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
    /// The account was deleted (DW-A63).
    Deleted,
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
    if code == "account_deleted" {
        return SignInProblem::Deleted;
    }
    if code == "invalid_credentials" || code.contains("Invalid email or password") {
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
        SignInProblem::Deleted => "This account has been deleted. We can\u{2019}t recover it.",
        SignInProblem::Other => "Sign-in didn\u{2019}t finish. Nothing was unlocked. Try again.",
    }
}

/// DW-A22's note (a hint, not a diagnosis).
pub const LOCK_NOTE: &str = "After five wrong tries in a row, your account locks for a short while. It unlocks by itself, \
     and we\u{2019}ve emailed the account owner a link to unlock it now.";

/// The recovery-phrase step's error text (DW-A24).
pub const WRONG_PHRASE: &str = "That phrase doesn\u{2019}t open this account. Nothing was restored.";

/// DW-A24: after the server's attempt limit.
pub const TOO_MANY_PHRASE_TRIES: &str = "Too many tries. Wait an hour.";

/// Two-factor: wrong code (DW-A27).
pub const WRONG_2FA_CODE: &str = "That code didn\u{2019}t work. Check the app and try again.";

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

/// Decide the gate state from the values the window already has (from `sync_status`
/// and `account_subscription`). The TSX calls this once after reading both; the
/// decision is in Rust.
pub fn decide_gate(logged_in: bool, vault_unlocked: bool, has_no_plan: bool) -> GateState {
    if !logged_in {
        return GateState::SignedOut;
    }
    if !vault_unlocked {
        return GateState::Locked;
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

/// The gate state for the main window: what screen to show. Reads the session
/// and auth_present flags synchronously; the plan check is driven by the caller.
///
/// The TSX calls this as: 1) `sync_status` (already exists), 2) if unlocked,
/// `account_subscription` (already exists), 3) `auth_gate_state` with the results.
/// This way the plan check stays one command call, and all rules live in Rust.
#[tauri::command]
pub(crate) fn auth_gate_state(
    state: tauri::State<'_, crate::AppState>,
    plan_state: Option<String>,
    plan: Option<String>,
    plan_status: Option<String>,
    quota_bytes: Option<i64>,
) -> Result<GateState, String> {
    let acct = state.active_account()?;
    let vault_unlocked = acct.session.lock().map(|g| g.is_some()).unwrap_or(false);
    let auth_present = state.auth_present.lock().map(|g| *g).unwrap_or(false);
    let logged_in = vault_unlocked || auth_present;
    let no_plan = has_no_plan(
        plan_state.as_deref(),
        plan.as_deref(),
        plan_status.as_deref(),
        quota_bytes,
    );
    Ok(decide_gate(logged_in, vault_unlocked, no_plan))
}

/// Map a sign-in error code and the failure count to the board's problem and text.
/// The TSX calls this instead of doing the mapping itself.
#[tauri::command]
pub(crate) fn auth_sign_in_problem(code: String, consecutive_failures: u32) -> AuthSignInProblemDto {
    let problem = problem_from(&code, consecutive_failures);
    let text = problem_text(&problem).to_string();
    let lock_note = matches!(problem, SignInProblem::LockedHint).then(|| LOCK_NOTE.to_string());
    let wait_seconds = match &problem {
        SignInProblem::RateLimited { seconds } => Some(*seconds),
        _ => None,
    };
    AuthSignInProblemDto {
        problem,
        text,
        lock_note,
        wait_seconds,
    }
}

/// What `auth_sign_in_problem` returns.
#[derive(Debug, Serialize)]
pub(crate) struct AuthSignInProblemDto {
    pub problem: SignInProblem,
    pub text: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lock_note: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub wait_seconds: Option<u32>,
}

/// What screen follows a sign-in. The TSX calls this with the `LoginOutcome`
/// fields and picks the screen from the result.
#[tauri::command]
pub(crate) fn auth_next_after_sign_in(
    requires_2fa: bool,
    vault_unlocked: bool,
    reauthenticated: bool,
    account_mismatch: bool,
) -> NextAfterSignIn {
    next_after_sign_in(requires_2fa, vault_unlocked, reauthenticated, account_mismatch)
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
        assert_eq!(problem_from("account_deleted", 0), SignInProblem::Deleted);
        assert_eq!(problem_from("server_error", 0), SignInProblem::Other);
    }

    #[test]
    fn problem_text_returns_the_boards_words() {
        assert!(problem_text(&SignInProblem::Invalid).contains("don\u{2019}t match"));
        assert!(problem_text(&SignInProblem::Offline).contains("offline"));
        assert!(problem_text(&SignInProblem::Deleted).contains("deleted"));
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
        assert_eq!(decide_gate(false, false, false), GateState::SignedOut);
    }

    #[test]
    fn decide_gate_locked() {
        assert_eq!(decide_gate(true, false, false), GateState::Locked);
    }

    #[test]
    fn decide_gate_no_plan() {
        assert_eq!(decide_gate(true, true, true), GateState::NoPlan);
    }

    #[test]
    fn decide_gate_ready() {
        assert_eq!(decide_gate(true, true, false), GateState::Ready);
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
}
