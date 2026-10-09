//! Ruling R8 (spec 2026-10-06 §3): a sign-in on a Mac that already holds an account is either the
//! same account signing in again (only the session token is replaced: Finder, keys, cache and
//! pending edits stay) or an account switch (a full sign-out comes first, after a warning). Pure:
//! the caller gathers the facts; this decides.
//!
//! "The same account" is whichever account this Mac holds: the recorded owner of the local data (R10) when there is
//! one, otherwise the account its credentials name (the cached profile's user id, the email of the session or of the
//! Keychain: what a sign-in leaves behind before its first engine starts). It is decided in this order:
//!
//! -1. An owner record or `state.db` that cannot be read settles nothing: it is not "no owner" and not a switch (a
//!    read error that passes must never purge someone's own pending edits, nor let the credentials decide). The
//!    sign-in is `LocalDataUnreadable`: nothing changes, and trying again is safe.
//! 0. Local DATA (`state.db` rows, the queue, staged payloads, the cache) that no account is recorded as owning is
//!    never "the same account": claiming it is R10's one-time adoption, not a sign-in. The switch (and its purge) is
//!    the way. Credentials alone, with no owner record and no such data, are NOT unowned data; they go on to 1 to 3.
//! 1. Both sides have a user id: different ids are a switch, and nothing overrides them. Equal ids are the same account,
//!    but a vault key this Mac kept (stored, or in a session in memory) is not trusted on the ids alone: an account's
//!    key can be changed on another device, and that change signs out every device (spec §5.6). So with a key here
//!    the server proves it first (`NeedsKeyProof`, then [`after_key_proof`]): a key it confirms, or an account with no
//!    recovery check on file, is `SameAccount`; a key it says is no longer the account's is `KeyOutdated`; no answer
//!    settles nothing. Equal ids and no key here: `SameAccount` at once.
//! 2. Otherwise, if a vault key is here (stored on this Mac, or in a session in memory): the server proves whether the
//!    key the sign-in would keep belongs to the account that is signing in (`NeedsKeyProof`, then [`after_key_proof`]).
//!    A proof that cannot be completed settles nothing. An account with no recovery check on file cannot be proved with
//!    a key at all: the order goes on to 3.
//! 3. Otherwise the emails are compared in their canonical form (`account_binding::same_email`: trimmed, then Unicode
//!    lower-cased, the server's own canonical form, ruling A″). The server keeps one account per canonical address, so
//!    a letter-case variant of the same address is the same account. Anything that cannot be placed is a switch,
//!    unless nothing of an account is here at all (a first sign-in).

use crate::account_binding::{Identity, same_email};

/// What this Mac still holds of an account when someone signs in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct LocalAccount<'a> {
    /// The user id of the account this Mac holds: the recorded owner's, else the cached profile's, if there is one.
    pub user_id: Option<&'a str>,
    /// Its email: the recorded owner's, else the cached profile's, the session's or the Keychain's. Consulted only when
    /// either side lacks a user id.
    pub email: Option<&'a str>,
    /// `LocalTraces::any()`: anything of a previous account is still on this computer.
    pub traces: bool,
    /// A vault key is stored on this Mac (the id-keyed or the legacy Keychain item). With no session in memory, it is
    /// what a server proof checks.
    pub key_stored: bool,
    /// A session in memory holds a vault key (the vault is unlocked). Rules 1 and 2 have the server prove it, as a stored
    /// one; when both are here, the proof checks this one, because a same-account sign-in keeps it.
    pub key_in_memory: bool,
    /// Local DATA (`state.db` rows, the queue, staged payloads, the cache) that no account is recorded as owning.
    /// Credentials alone are not data.
    pub unowned_data: bool,
    /// The owner record or `state.db` could not be read. That is neither "no owner" nor a switch: nothing can be
    /// decided now, and nothing changes (see [`SignInKind::Unconfirmed`]).
    pub unreadable: bool,
}

/// Everything of a previous account that can outlive its session on this computer. A startup 401 keeps
/// the Keychain email and the vault key (R9); an install from before R10 can have an email-only owner.
/// `Fresh` needs none of these.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct LocalTraces {
    pub session_in_memory: bool,
    pub auth_present: bool,
    pub keychain_token: bool,
    pub keychain_email: bool,
    pub vault_key: bool,
    pub recorded_owner: bool,
    pub cached_profile: bool,
    pub queued_or_staged: bool,
}

impl LocalTraces {
    pub fn any(&self) -> bool {
        self.session_in_memory
            || self.auth_present
            || self.keychain_token
            || self.keychain_email
            || self.vault_key
            || self.recorded_owner
            || self.cached_profile
            || self.queued_or_staged
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SignInKind {
    /// Nothing of an account is on this Mac: the ordinary first sign-in.
    Fresh,
    /// The account on this Mac signs in again: replace only the session token.
    SameAccount,
    /// Another account, or one whose identity cannot be established (fail closed): a switch.
    DifferentAccount,
    /// A vault key is here and the server must prove whose it is: a key is stored or in memory, and either the user ids
    /// match (rule 1) or this Mac's account has no user id (rule 2). Never returned by [`after_key_proof`].
    NeedsKeyProof,
    /// The account on this Mac signs in again (the same user id), but the server says the vault key this Mac kept is no
    /// longer the account's: it was changed on another device. The token is replaced without that key, the key is
    /// removed, and the recovery phrase is asked (spec §5.6). Only [`after_key_proof`] returns it.
    KeyOutdated,
    /// The server could not say. Nothing changes: the new session is revoked and the person tries again.
    Unconfirmed,
    /// This computer's own record of its account (the owner record, or `state.db`) could not be read. Nothing changes:
    /// the new session is revoked, and the sign-in says so in its own words (not "connect to the internet").
    LocalDataUnreadable,
}

/// The server's answer to "is the vault key stored on this Mac this account's?".
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyProof {
    /// The server confirmed that the stored key belongs to the signing-in account.
    Matches,
    /// The server confirmed that it does not (its exact answer `invalid_recovery_phrase`).
    Mismatch,
    /// The server holds no recovery check for the signing-in account (its exact answer `recovery_check_missing`), so a
    /// key can prove nothing either way.
    NoCheckOnFile,
    /// No answer (a network or server error, a `400` with neither exact code, a key that could not be read): neither of
    /// the two.
    Unavailable,
}

/// What a key proof makes of a [`SignInKind::NeedsKeyProof`] for the sign-in as `user_id` / `email` on a Mac that holds
/// `local`. A proof the server answered decides; one it could not answer settles nothing. Never `NeedsKeyProof`.
///
/// - Rule 1 (the user ids match): the account is already known, so the proof says only whether the kept key is still
///   its key. Confirmed, or no recovery check on file (nothing to prove with): `SameAccount`. Not the account's key
///   any more: `KeyOutdated`.
/// - Rule 2 (no user id on this Mac's side): the proof says whose key it is. Not the signing-in account's: a switch.
///   An account with no recovery check on file cannot be proved with a key, so the order goes on to its next rule as
///   if no key were here, stored or in memory: the email in its canonical form (ruling A″).
pub fn after_key_proof(proof: KeyProof, local: &LocalAccount<'_>, user_id: &str, email: &str) -> SignInKind {
    if same_user_id(local, user_id) == Some(true) {
        return match proof {
            KeyProof::Matches | KeyProof::NoCheckOnFile => SignInKind::SameAccount,
            KeyProof::Mismatch => SignInKind::KeyOutdated,
            KeyProof::Unavailable => SignInKind::Unconfirmed,
        };
    }
    match proof {
        KeyProof::Matches => SignInKind::SameAccount,
        KeyProof::Mismatch => SignInKind::DifferentAccount,
        KeyProof::NoCheckOnFile => sign_in_kind(
            &LocalAccount {
                key_stored: false,
                key_in_memory: false,
                ..*local
            },
            user_id,
            email,
        ),
        KeyProof::Unavailable => SignInKind::Unconfirmed,
    }
}

/// `Some(equal)` when both this Mac's account and the signing-in one have a user id, `None` otherwise.
fn same_user_id(local: &LocalAccount<'_>, user_id: &str) -> Option<bool> {
    let account = Identity::new(local.user_id, None);
    let signing_in = Identity::new(Some(user_id), None);
    match (account.user_id, signing_in.user_id) {
        (Some(stored), Some(signing)) => Some(stored == signing),
        _ => None,
    }
}

/// Decide what a sign-in as `user_id` / `email` (the server's own record of the account behind the new session,
/// never typed text) is on a Mac that holds `local`. See the module's order of rules.
pub fn sign_in_kind(local: &LocalAccount<'_>, user_id: &str, email: &str) -> SignInKind {
    // -1. What cannot be read cannot be compared with.
    if local.unreadable {
        return SignInKind::LocalDataUnreadable;
    }
    // 0. Unowned local data is not a sign-in's to claim.
    if local.unowned_data {
        return SignInKind::DifferentAccount;
    }
    let account = Identity::new(local.user_id, local.email);
    let signing_in = Identity::new(Some(user_id), Some(email));
    // 1. Both sides have a user id: another id is a switch; the same id keeps a key here only once the server proves it.
    match same_user_id(local, user_id) {
        Some(false) => return SignInKind::DifferentAccount,
        Some(true) if local.key_stored || local.key_in_memory => return SignInKind::NeedsKeyProof,
        Some(true) => return SignInKind::SameAccount,
        None => {}
    }
    // 2. A vault key is here, stored or in memory: the server decides.
    if local.key_stored || local.key_in_memory {
        return SignInKind::NeedsKeyProof;
    }
    // 3. Neither: the same address in its canonical form.
    match (&account.email, &signing_in.email) {
        (Some(stored), Some(signing)) if same_email(stored, signing) => SignInKind::SameAccount,
        // Nothing of an account here and nothing that names one: an ordinary first sign-in.
        _ if !local.traces && !account.is_known() => SignInKind::Fresh,
        _ => SignInKind::DifferentAccount,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use SignInKind::*;

    /// The account this Mac holds, with the given id and email (a recorded owner's, else what the credentials name),
    /// and what else is here. No unowned data.
    fn local(
        user_id: Option<&'static str>,
        email: Option<&'static str>,
        traces: bool,
        key_stored: bool,
    ) -> LocalAccount<'static> {
        LocalAccount {
            user_id,
            email,
            traces,
            key_stored,
            key_in_memory: false,
            unowned_data: false,
            unreadable: false,
        }
    }

    /// The same, with an unlocked session (and so its vault key) in memory.
    fn with_key_in_memory(account: LocalAccount<'static>) -> LocalAccount<'static> {
        LocalAccount {
            key_in_memory: true,
            traces: true,
            ..account
        }
    }

    /// The same, with an owner record or a `state.db` that cannot be read.
    fn unreadable(account: LocalAccount<'static>) -> LocalAccount<'static> {
        LocalAccount {
            unreadable: true,
            traces: true,
            ..account
        }
    }

    /// The same, with local data that no account owns.
    fn with_unowned_data(account: LocalAccount<'static>) -> LocalAccount<'static> {
        LocalAccount {
            unowned_data: true,
            traces: true,
            ..account
        }
    }

    #[test]
    fn the_r8_table() {
        // (name, the signing-in email as the server spells it, what this Mac holds, the expected kind)
        let cases = vec![
            // nothing here
            ("a fresh Mac", "sam@beebeeb.io", local(None, None, false, false), Fresh),
            (
                "an account that names nobody and nothing else here",
                "sam@beebeeb.io",
                local(Some(" "), Some(""), false, false),
                Fresh,
            ),
            // local data that cannot be read settles nothing: not a switch, not the same account
            (
                "an unreadable owner record or database, the same id",
                "sam@beebeeb.io",
                unreadable(local(Some("u-1"), Some("sam@beebeeb.io"), true, false)),
                LocalDataUnreadable,
            ),
            (
                "an unreadable owner record or database, another id",
                "sam@beebeeb.io",
                unreadable(local(Some("u-2"), None, true, false)),
                LocalDataUnreadable,
            ),
            (
                "an unreadable owner record or database, a stored key (no proof is asked)",
                "sam@beebeeb.io",
                unreadable(local(None, None, true, true)),
                LocalDataUnreadable,
            ),
            (
                "an unreadable owner record or database, nothing that names an account",
                "sam@beebeeb.io",
                unreadable(local(None, None, true, false)),
                LocalDataUnreadable,
            ),
            (
                "an unreadable database and unowned data: still nothing is decided",
                "sam@beebeeb.io",
                with_unowned_data(unreadable(local(None, None, true, false))),
                LocalDataUnreadable,
            ),
            // unowned local data is never "the same account", whatever the credentials say
            (
                "unowned data and the same id",
                "sam@beebeeb.io",
                with_unowned_data(local(Some("u-1"), Some("sam@beebeeb.io"), true, false)),
                DifferentAccount,
            ),
            (
                "unowned data and the same email",
                "sam@beebeeb.io",
                with_unowned_data(local(None, Some("sam@beebeeb.io"), true, false)),
                DifferentAccount,
            ),
            (
                "unowned data and a stored key",
                "sam@beebeeb.io",
                with_unowned_data(local(None, None, true, true)),
                DifferentAccount,
            ),
            (
                "unowned data and nothing that names an account",
                "sam@beebeeb.io",
                with_unowned_data(local(None, None, true, false)),
                DifferentAccount,
            ),
            // rule 1: both sides have a user id
            (
                "the same id",
                "sam@beebeeb.io",
                local(Some("u-1"), Some("sam@beebeeb.io"), true, false),
                SameAccount,
            ),
            (
                "the same id wins over another spelling of the email",
                "sam@beebeeb.io",
                local(Some("u-1"), Some("other@beebeeb.io"), true, false),
                SameAccount,
            ),
            (
                "the same id and a stored key: the server proves the key is still the account's",
                "sam@beebeeb.io",
                local(Some("u-1"), None, true, true),
                NeedsKeyProof,
            ),
            (
                "the same id and a key in memory: the server proves it too",
                "sam@beebeeb.io",
                with_key_in_memory(local(Some("u-1"), Some("sam@beebeeb.io"), true, false)),
                NeedsKeyProof,
            ),
            (
                "another id and a key in memory: a switch, nothing is asked",
                "sam@beebeeb.io",
                with_key_in_memory(local(Some("u-2"), Some("sam@beebeeb.io"), true, false)),
                DifferentAccount,
            ),
            (
                "another id",
                "sam@beebeeb.io",
                local(Some("u-2"), Some("sam@beebeeb.io"), true, false),
                DifferentAccount,
            ),
            (
                "another id with the same email: nothing overrides the ids",
                "sam@beebeeb.io",
                local(Some("u-2"), Some("sam@beebeeb.io"), true, true),
                DifferentAccount,
            ),
            (
                "another id, nothing else here",
                "sam@beebeeb.io",
                local(Some("u-2"), None, true, false),
                DifferentAccount,
            ),
            // rule 2: a stored key, and no user id on this side
            (
                "an email and a stored key",
                "sam@beebeeb.io",
                local(None, Some("sam@beebeeb.io"), true, true),
                NeedsKeyProof,
            ),
            (
                "an email in another letter case and a stored key",
                "Sam@beebeeb.io",
                local(None, Some("sam@beebeeb.io"), true, true),
                NeedsKeyProof,
            ),
            (
                "another email and a stored key: the server still decides",
                "sam@beebeeb.io",
                local(None, Some("kim@beebeeb.io"), true, true),
                NeedsKeyProof,
            ),
            (
                "a stored key and nothing else that names an account",
                "sam@beebeeb.io",
                local(None, None, true, true),
                NeedsKeyProof,
            ),
            (
                "an email and a key in memory: proved like a stored one",
                "sam@beebeeb.io",
                with_key_in_memory(local(None, Some("sam@beebeeb.io"), true, false)),
                NeedsKeyProof,
            ),
            // rule 3: no id and no key: the email in its canonical form
            (
                "the same email, spacing only",
                "sam@beebeeb.io",
                local(None, Some("  sam@beebeeb.io "), true, false),
                SameAccount,
            ),
            (
                "an ASCII letter-case variant",
                "Sam@Beebeeb.IO",
                local(None, Some("sam@beebeeb.io"), true, false),
                SameAccount,
            ),
            (
                "another email",
                "sam@beebeeb.io",
                local(None, Some("kim@beebeeb.io"), true, false),
                DifferentAccount,
            ),
            // credentials that name nobody, with something here: the sign-in cannot be placed, so it is a switch
            (
                "traces that name nobody",
                "sam@beebeeb.io",
                local(None, None, true, false),
                DifferentAccount,
            ),
        ];
        for (name, email, account, expected) in cases {
            assert_eq!(sign_in_kind(&account, "u-1", email), expected, "{name}");
        }
    }

    /// Rule 3 compares the canonical form (ruling A″; it folded ASCII only before): a letter-case variant, ASCII or not,
    /// is the same account, an accent is not a letter case, and empty emails never match.
    #[test]
    fn the_email_rule_compares_the_canonical_form_and_never_matches_an_empty_email() {
        let account = |email| local(None, Some(email), true, false);
        assert_eq!(
            sign_in_kind(&account("josé@beebeeb.io"), "u-1", "JOSé@Beebeeb.io"),
            SameAccount,
            "ASCII letters fold"
        );
        assert_eq!(
            sign_in_kind(&account("josé@beebeeb.io"), "u-1", "JOSÉ@beebeeb.io"),
            SameAccount,
            "É folds to é"
        );
        assert_eq!(
            sign_in_kind(&account("ÉMILE@beebeeb.io"), "u-1", "émile@beebeeb.io"),
            SameAccount,
            "and é to É"
        );
        assert_eq!(
            sign_in_kind(&account("josé@beebeeb.io"), "u-1", "jose@beebeeb.io"),
            DifferentAccount,
            "an accent is not a letter case"
        );
        assert_eq!(
            sign_in_kind(&local(None, Some(""), true, false), "u-9", ""),
            DifferentAccount,
            "empty never matches empty"
        );
        assert_eq!(
            sign_in_kind(&account("sam@beebeeb.io"), "u-9", ""),
            DifferentAccount,
            "an empty signing-in email never matches"
        );
    }

    /// A sign-in whose engine has not started yet holds credentials and no owner record. Signing in again as that
    /// account is in place (nothing to warn about); another account is a switch; and unowned DATA is a switch whatever
    /// the credentials say.
    #[test]
    fn credentials_with_no_owner_record_are_judged_by_the_same_order_as_an_owner() {
        // The credentials of a sign-in that has not started an engine: a token, an email, the cached profile's id.
        let before_the_first_engine = local(Some("u-1"), Some("sam@beebeeb.io"), true, false);
        assert_eq!(
            sign_in_kind(&before_the_first_engine, "u-1", "sam@beebeeb.io"),
            SameAccount,
            "the same account twice"
        );
        assert_eq!(
            sign_in_kind(&before_the_first_engine, "u-2", "kim@beebeeb.io"),
            DifferentAccount,
            "another account in that window"
        );
        assert_eq!(
            sign_in_kind(&with_unowned_data(before_the_first_engine), "u-1", "sam@beebeeb.io"),
            DifferentAccount,
            "unowned data"
        );
        // After a relaunch the cached profile is gone: the email decides, or the server's proof when a key is stored.
        let after_a_relaunch = local(None, Some("sam@beebeeb.io"), true, false);
        assert_eq!(sign_in_kind(&after_a_relaunch, "u-1", "Sam@beebeeb.io"), SameAccount);
        assert_eq!(
            sign_in_kind(&after_a_relaunch, "u-2", "kim@beebeeb.io"),
            DifferentAccount
        );
        assert_eq!(
            sign_in_kind(
                &local(None, Some("sam@beebeeb.io"), true, true),
                "u-1",
                "sam@beebeeb.io"
            ),
            NeedsKeyProof
        );
    }

    #[test]
    fn a_completed_key_proof_decides_and_an_unavailable_one_settles_nothing() {
        let held = local(None, Some("sam@beebeeb.io"), true, true);
        assert_eq!(
            after_key_proof(KeyProof::Matches, &held, "u-1", "sam@beebeeb.io"),
            SameAccount
        );
        assert_eq!(
            after_key_proof(KeyProof::Mismatch, &held, "u-1", "sam@beebeeb.io"),
            DifferentAccount
        );
        assert_eq!(
            after_key_proof(KeyProof::Unavailable, &held, "u-1", "sam@beebeeb.io"),
            Unconfirmed
        );
        for proof in [
            KeyProof::Matches,
            KeyProof::Mismatch,
            KeyProof::NoCheckOnFile,
            KeyProof::Unavailable,
        ] {
            for email in ["sam@beebeeb.io", "kim@beebeeb.io"] {
                assert_ne!(
                    after_key_proof(proof, &held, "u-1", email),
                    NeedsKeyProof,
                    "a proof always ends the question"
                );
            }
        }
    }

    /// Spec §5.6: the same user id does not make a kept vault key current. An account's key can be changed on another
    /// device (which signs out every device), so a key stored or in memory is proved first, and a key the server says is
    /// no longer the account's is `KeyOutdated`, never `SameAccount`. Another id is a switch without asking, and the same
    /// id with no key here needs no proof.
    #[test]
    fn with_the_same_id_a_kept_key_is_proved_first_and_one_the_server_rejects_is_outdated() {
        let stored = local(Some("u-1"), Some("sam@beebeeb.io"), true, true);
        let in_memory = with_key_in_memory(local(Some("u-1"), Some("sam@beebeeb.io"), true, false));
        for (name, held) in [("a stored key", stored), ("a key in memory", in_memory)] {
            assert_eq!(
                sign_in_kind(&held, "u-1", "sam@beebeeb.io"),
                NeedsKeyProof,
                "{name}: the ids alone do not make the key current"
            );
            assert_eq!(
                after_key_proof(KeyProof::Matches, &held, "u-1", "sam@beebeeb.io"),
                SameAccount,
                "{name}: a confirmed key is in place"
            );
            assert_eq!(
                after_key_proof(KeyProof::NoCheckOnFile, &held, "u-1", "sam@beebeeb.io"),
                SameAccount,
                "{name}: nothing to prove with, the ids decide"
            );
            assert_eq!(
                after_key_proof(KeyProof::Mismatch, &held, "u-1", "Sam@beebeeb.io"),
                KeyOutdated,
                "{name}: a key the server no longer accepts is outdated, never kept"
            );
            assert_eq!(
                after_key_proof(KeyProof::Unavailable, &held, "u-1", "sam@beebeeb.io"),
                Unconfirmed,
                "{name}: no answer settles nothing"
            );
            assert_eq!(
                sign_in_kind(&held, "u-2", "kim@beebeeb.io"),
                DifferentAccount,
                "{name}: another id is a switch, and the server is not asked"
            );
        }
        assert_eq!(
            sign_in_kind(
                &local(Some("u-1"), Some("sam@beebeeb.io"), true, false),
                "u-1",
                "sam@beebeeb.io"
            ),
            SameAccount,
            "the same id and no key here: in place at once"
        );
        // `KeyOutdated` belongs to the same id only: rule 2's mismatch stays a switch.
        let by_email = local(None, Some("sam@beebeeb.io"), true, true);
        assert_eq!(
            after_key_proof(KeyProof::Mismatch, &by_email, "u-1", "sam@beebeeb.io"),
            DifferentAccount
        );
    }

    /// Lead ruling 12 (Task 12): an account with no recovery check on file cannot be proved with the stored key, so the
    /// email decides (in its canonical form), exactly as if no key were stored. A key proof that says "not this
    /// account's" is still a switch.
    #[test]
    fn no_recovery_check_on_file_falls_through_to_the_email_rule() {
        let held = local(None, Some("sam@beebeeb.io"), true, true);
        assert_eq!(
            sign_in_kind(&held, "u-1", "Sam@beebeeb.io"),
            NeedsKeyProof,
            "the premise: the key is asked first"
        );
        assert_eq!(
            after_key_proof(KeyProof::NoCheckOnFile, &held, "u-1", "Sam@beebeeb.io"),
            SameAccount,
            "the same address"
        );
        assert_eq!(
            after_key_proof(KeyProof::NoCheckOnFile, &held, "u-2", "kim@beebeeb.io"),
            DifferentAccount,
            "another address"
        );
        assert_eq!(
            after_key_proof(KeyProof::Mismatch, &held, "u-1", "Sam@beebeeb.io"),
            DifferentAccount,
            "a mismatch is never overruled by the email"
        );
        let nothing_named = local(None, None, true, true);
        assert_eq!(
            after_key_proof(KeyProof::NoCheckOnFile, &nothing_named, "u-1", "sam@beebeeb.io"),
            DifferentAccount,
            "a key that names nobody: a switch"
        );
    }

    /// Rule 2 covers a key in memory as well as a stored one (spec §5.6): with no user id on this Mac's side, an
    /// unlocked session is proved by the server, never left to the email. With no check on file the email decides, as
    /// for a stored key, so the order goes on as if no key were here at all; and a proof always ends the question.
    #[test]
    fn without_a_user_id_a_key_in_memory_is_proved_like_a_stored_one() {
        let in_memory = with_key_in_memory(local(None, Some("sam@beebeeb.io"), true, false));
        assert_eq!(
            sign_in_kind(&in_memory, "u-1", "Sam@beebeeb.io"),
            NeedsKeyProof,
            "rule 2, not the email rule"
        );
        assert_eq!(
            after_key_proof(KeyProof::Matches, &in_memory, "u-1", "Sam@beebeeb.io"),
            SameAccount
        );
        assert_eq!(
            after_key_proof(KeyProof::Mismatch, &in_memory, "u-1", "Sam@beebeeb.io"),
            DifferentAccount,
            "not this account's key: a switch, whatever the email says"
        );
        assert_eq!(
            after_key_proof(KeyProof::Unavailable, &in_memory, "u-1", "sam@beebeeb.io"),
            Unconfirmed
        );
        let both = LocalAccount {
            key_stored: true,
            ..in_memory
        };
        for (name, held) in [("a key in memory", in_memory), ("a key in memory and stored", both)] {
            assert_eq!(
                after_key_proof(KeyProof::NoCheckOnFile, &held, "u-1", "Sam@beebeeb.io"),
                SameAccount,
                "{name}, no check on file: the same address decides"
            );
            assert_eq!(
                after_key_proof(KeyProof::NoCheckOnFile, &held, "u-2", "kim@beebeeb.io"),
                DifferentAccount,
                "{name}, no check on file: another address is a switch"
            );
            for proof in [
                KeyProof::Matches,
                KeyProof::Mismatch,
                KeyProof::NoCheckOnFile,
                KeyProof::Unavailable,
            ] {
                for email in ["sam@beebeeb.io", "kim@beebeeb.io"] {
                    assert_ne!(
                        after_key_proof(proof, &held, "u-1", email),
                        NeedsKeyProof,
                        "{name}: a proof always ends the question"
                    );
                }
            }
        }
    }

    // ── every retained trace counts ──

    #[test]
    fn every_retained_trace_counts() {
        assert!(!LocalTraces::default().any(), "a computer with nothing left is fresh");
        type Set = fn(&mut LocalTraces);
        let each: [(&str, Set); 8] = [
            ("a session in memory", |t| t.session_in_memory = true),
            ("auth present", |t| t.auth_present = true),
            ("a Keychain token", |t| t.keychain_token = true),
            ("a Keychain email", |t| t.keychain_email = true),
            ("a vault key", |t| t.vault_key = true),
            ("a recorded owner", |t| t.recorded_owner = true),
            ("a cached profile", |t| t.cached_profile = true),
            ("queued or staged data", |t| t.queued_or_staged = true),
        ];
        for (name, set) in each {
            let mut traces = LocalTraces::default();
            set(&mut traces);
            assert!(traces.any(), "{name} alone is a trace");
        }
    }

    /// An install from before R10 whose token was revoked at startup, with nothing queued: an email-only owner was
    /// adopted, and a startup 401 keeps the Keychain email and the vault key (R9). The server decides whose key it is.
    fn after_a_revoked_token_with_nothing_queued() -> LocalAccount<'static> {
        let traces = LocalTraces {
            recorded_owner: true,
            keychain_email: true,
            vault_key: true,
            ..LocalTraces::default()
        };
        LocalAccount {
            user_id: None,
            email: Some("sam@beebeeb.io"),
            traces: traces.any(),
            key_stored: traces.vault_key,
            key_in_memory: false,
            unowned_data: false,
            unreadable: false,
        }
    }

    #[test]
    fn another_account_after_a_revoked_token_with_nothing_queued_is_a_switch_never_fresh() {
        let account = after_a_revoked_token_with_nothing_queued();
        assert_eq!(
            sign_in_kind(&account, "u-2", "kim@beebeeb.io"),
            NeedsKeyProof,
            "never Fresh, never decided by the email alone"
        );
        assert_eq!(
            after_key_proof(KeyProof::Mismatch, &account, "u-2", "kim@beebeeb.io"),
            DifferentAccount,
            "the server says the key is not kim's"
        );
    }

    #[test]
    fn the_same_account_after_a_revoked_token_signs_in_again_in_place() {
        let account = after_a_revoked_token_with_nothing_queued();
        assert_eq!(sign_in_kind(&account, "u-1", "Sam@beebeeb.io"), NeedsKeyProof);
        assert_eq!(
            after_key_proof(KeyProof::Matches, &account, "u-1", "Sam@beebeeb.io"),
            SameAccount,
            "the server says the key is sam's, whatever the letter case"
        );
    }

    /// A key and nothing that names an account is never a first sign-in: the server says whose key it is.
    #[test]
    fn a_retained_vault_key_alone_is_never_fresh_and_goes_to_the_server() {
        let traces = LocalTraces {
            vault_key: true,
            ..LocalTraces::default()
        };
        let local = LocalAccount {
            user_id: None,
            email: None,
            traces: traces.any(),
            key_stored: true,
            key_in_memory: false,
            unowned_data: false,
            unreadable: false,
        };
        assert_eq!(sign_in_kind(&local, "u-2", "kim@beebeeb.io"), NeedsKeyProof);
    }
}
