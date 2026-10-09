//! Ruling R10 (spec 2026-10-06 §5.6): the local data on this computer belongs to the account that
//! created it, and a different account never reuses it. "Local data" is everything in `state.db`
//! (the queue, staged uploads, upload sessions, file rows, Finder anchors, activity) and the files it
//! points at (staged payloads, the decrypted cache). Pure: the caller gathers the facts and this
//! decides. `lib.rs` runs it before every engine start (`spawn_bound_engine`).

/// Who an account is, as far as this computer knows: the server user id when known, else the
/// account email (an offline relaunch, or local data recorded before the id was known).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Identity {
    pub user_id: Option<String>,
    pub email: Option<String>,
}

impl Identity {
    /// Trims both fields; an empty value is unknown.
    pub fn new(user_id: Option<&str>, email: Option<&str>) -> Self {
        let clean = |value: Option<&str>| value.map(str::trim).filter(|v| !v.is_empty()).map(str::to_string);
        Self {
            user_id: clean(user_id),
            email: clean(email),
        }
    }

    pub fn is_known(&self) -> bool {
        self.user_id.is_some() || self.email.is_some()
    }
}

/// The canonical form of an account email: surrounding whitespace trimmed, then Unicode lower-casing. The same rule as
/// the server's `email_canon::canonical_email` (`raw.trim().to_lowercase()`): the form signup stores and every server
/// lookup compares, and the server keeps one account per canonical address (a unique index on the lower-cased email).
/// The client's one email fold (ruling A″): the engine start ([`same_account`]), the owner backfill and sign-in's
/// email rule all compare through [`same_email`].
pub fn canonical_email(email: &str) -> String {
    email.trim().to_lowercase()
}

/// Two account emails name the same account when their canonical forms are equal ([`canonical_email`]). An email that
/// is empty in canonical form never matches anything, not even another empty one.
pub fn same_email(a: &str, b: &str) -> bool {
    let a = canonical_email(a);
    !a.is_empty() && a == canonical_email(b)
}

/// `Some(true)`: the same account. `Some(false)`: a different one. `None`: nothing to compare.
/// The user id decides whenever both sides know it. Otherwise the email decides by its canonical form ([`same_email`],
/// ruling A″): equal canonical emails are the same account (an exact match, an ASCII or a non-ASCII letter-case
/// variant, surrounding whitespace aside), any other email is another account.
pub fn same_account(a: &Identity, b: &Identity) -> Option<bool> {
    if let (Some(x), Some(y)) = (&a.user_id, &b.user_id) {
        return Some(x == y);
    }
    match (&a.email, &b.email) {
        (Some(x), Some(y)) => Some(same_email(x, y)),
        _ => None,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Binding {
    /// This session's account owns the local data, or there is none: start, keep everything.
    Proceed,
    /// The local data belongs to another account, or to no recorded account: reset it first.
    Reset,
    /// An owner is recorded but cannot be compared with this session: start nothing, delete nothing.
    Refuse,
}

pub fn decide(owner: Option<&Identity>, session: &Identity, has_local_data: bool) -> Binding {
    // A session that does not know who it is (a sign-in whose account record could not be fetched) is never
    // compared with, adopted into or reset against anything: nothing starts and nothing is deleted until a fetch
    // names it, whatever the data says (owned, unowned or none).
    if !session.is_known() {
        return Binding::Refuse;
    }
    match owner.filter(|owner| owner.is_known()) {
        Some(owner) => match same_account(owner, session) {
            Some(true) => Binding::Proceed,
            Some(false) => Binding::Reset,
            None => Binding::Refuse,
        },
        None if has_local_data => Binding::Reset,
        None => Binding::Proceed,
    }
}

/// The local data, as the binding needs it. `lib.rs` `StateDbLocalData` is the real one.
pub trait LocalData {
    fn owner(&self) -> Result<Option<Identity>, String>;
    fn has_account_data(&self) -> Result<bool, String>;
    /// The sign-out purge plus every remaining row. An error means nothing may start.
    fn reset(&self) -> Result<(), String>;
    fn record_owner(&self, owner: &Identity) -> Result<(), String>;
    /// The upgrade path as ONE conditional step, at most once per database: record `candidate` as the owner of
    /// unowned local data, and shut the window whatever the outcome (`candidate` may be `None`). Never replaces
    /// a recorded owner. See `StateDb::adopt_owner_once`.
    fn adopt_once(&self, candidate: Option<&Identity>) -> Result<bool, String>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Bound {
    Kept,
    Reset,
}

pub const IDENTITY_UNKNOWN: &str = "Beebeeb couldn’t confirm which account this computer’s local files belong to, so sync didn’t start. Connect to the internet and open Beebeeb again.";
pub const OTHER_ACCOUNT_ON_WINDOWS: &str =
    "This PC still holds local files of another Beebeeb account, so sync didn’t start. Sign out, then sign in again.";

/// The sentence for an engine start refused because an earlier engine stop was never confirmed: its task may still
/// run with the keys, so every start refuses until Beebeeb restarts (spec §5.6).
pub const ENGINE_STOP_UNCONFIRMED: &str =
    "Beebeeb’s sync didn’t confirm it stopped. Quit and reopen Beebeeb before syncing again.";

/// An engine-start refusal the person is shown (Task 12 fix round 2, ruling P): a closed code and its constant
/// sentence, recorded by the engine start and exposed as `engine_refusal` on `sync_status`. The two binding refusals,
/// and a start refused because an earlier engine stop was never confirmed (spec §5.6). Nothing in it names an account.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Refusal {
    IdentityUnknown,
    OtherAccountOnWindows,
    EngineStopUnconfirmed,
}

impl Refusal {
    pub fn code(self) -> &'static str {
        match self {
            Self::IdentityUnknown => "identity_unknown",
            Self::OtherAccountOnWindows => "other_account_on_windows",
            Self::EngineStopUnconfirmed => "engine_stop_unconfirmed",
        }
    }

    pub fn sentence(self) -> &'static str {
        match self {
            Self::IdentityUnknown => IDENTITY_UNKNOWN,
            Self::OtherAccountOnWindows => OTHER_ACCOUNT_ON_WINDOWS,
            Self::EngineStopUnconfirmed => ENGINE_STOP_UNCONFIRMED,
        }
    }

    /// The binding refusal an engine start's error is, when it is exactly one of the two binding sentences; any other
    /// error is none. An unconfirmed engine stop is recorded where the start refuses on it (`engine_start_refusal`), not
    /// from an error string.
    pub fn of(error: &str) -> Option<Self> {
        [Self::IdentityUnknown, Self::OtherAccountOnWindows]
            .into_iter()
            .find(|refusal| refusal.sentence() == error)
    }
}

/// Run before every engine start. `reset_allowed` is false on Windows, where an account change goes
/// through the Windows sign-out instead. An error means: do not start the engine.
pub fn bind_before_engine_start(
    data: &dyn LocalData,
    session: &Identity,
    reset_allowed: bool,
) -> Result<Bound, String> {
    let owner = data.owner()?.filter(Identity::is_known);
    let has_local_data = data.has_account_data()?;
    match decide(owner.as_ref(), session, has_local_data) {
        Binding::Proceed => {
            // The same account, or nothing here yet: keep everything and keep the record current
            // (an id learned later completes an email-only record; a changed email replaces the old).
            let current = Identity {
                user_id: session
                    .user_id
                    .clone()
                    .or_else(|| owner.as_ref().and_then(|o| o.user_id.clone())),
                email: session
                    .email
                    .clone()
                    .or_else(|| owner.as_ref().and_then(|o| o.email.clone())),
            };
            if current.is_known() && owner.as_ref() != Some(&current) {
                data.record_owner(&current)?;
            }
            Ok(Bound::Kept)
        }
        Binding::Reset if !reset_allowed => Err(OTHER_ACCOUNT_ON_WINDOWS.to_string()),
        Binding::Reset => {
            data.reset()?;
            if session.is_known() {
                data.record_owner(session)?;
            }
            Ok(Bound::Reset)
        }
        Binding::Refuse => Err(IDENTITY_UNKNOWN.to_string()),
    }
}

/// The upgrade path, run once at startup before anything starts: local data from before R10 has no owner,
/// and the account whose VAULT KEY this computer's Keychain holds (`vault_key_account`, `None` when the
/// Keychain holds a token but no key) has been using it. A one-shot migration: it adopts at most once per
/// database, on the first startup after the upgrade, and never replaces a recorded owner or invents one. Any
/// other unowned data is treated as an unknown owner at the next engine start (reset, or refused on Windows).
pub fn adopt_unbound(data: &dyn LocalData, vault_key_account: Option<&Identity>) -> Result<bool, String> {
    data.adopt_once(vault_key_account.filter(|account| account.is_known()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    fn id(user_id: Option<&str>, email: Option<&str>) -> Identity {
        Identity::new(user_id, email)
    }

    #[derive(Default)]
    struct Fake {
        owner: RefCell<Option<Identity>>,
        data: RefCell<bool>,
        reset_fails: bool,
        resets: RefCell<u32>,
        records: RefCell<Vec<Identity>>,
        window_closed: RefCell<bool>,
    }

    impl Fake {
        fn holding(owner: Option<Identity>, data: bool) -> Self {
            let fake = Fake::default();
            *fake.owner.borrow_mut() = owner;
            *fake.data.borrow_mut() = data;
            fake
        }
    }

    impl LocalData for Fake {
        fn owner(&self) -> Result<Option<Identity>, String> {
            Ok(self.owner.borrow().clone())
        }
        fn has_account_data(&self) -> Result<bool, String> {
            Ok(*self.data.borrow())
        }
        fn reset(&self) -> Result<(), String> {
            if self.reset_fails {
                return Err("disk I/O error".into());
            }
            *self.resets.borrow_mut() += 1;
            *self.data.borrow_mut() = false;
            *self.owner.borrow_mut() = None;
            Ok(())
        }
        fn record_owner(&self, owner: &Identity) -> Result<(), String> {
            self.records.borrow_mut().push(owner.clone());
            *self.owner.borrow_mut() = Some(owner.clone());
            Ok(())
        }
        fn adopt_once(&self, candidate: Option<&Identity>) -> Result<bool, String> {
            if std::mem::replace(&mut *self.window_closed.borrow_mut(), true) {
                return Ok(false);
            }
            match candidate {
                Some(candidate) if self.owner.borrow().is_none() && *self.data.borrow() => {
                    self.record_owner(candidate)?;
                    Ok(true)
                }
                _ => Ok(false),
            }
        }
    }

    #[test]
    fn the_r10_table() {
        let a = id(Some("u-a"), Some("a@beebeeb.io"));
        let a_offline = id(None, Some("a@beebeeb.io"));
        let a_case_variant = id(None, Some("A@Beebeeb.io"));
        let b = id(Some("u-b"), Some("b@beebeeb.io"));
        let only_an_id = id(Some("u-a"), None);
        let nobody = Identity::default();
        for (name, owner, session, has_local_data, expected) in [
            ("a fresh computer", None, &b, false, Binding::Proceed),
            ("unrecorded local data (fail closed)", None, &a, true, Binding::Reset),
            ("the same account, by id", Some(&a), &a, true, Binding::Proceed),
            (
                "the same account offline, by its exact email",
                Some(&a),
                &a_offline,
                true,
                Binding::Proceed,
            ),
            // Changed in fix round 3 (ruling A″, superseding A and A′): a letter-case variant of the email is the same
            // account (the server keeps one account per canonical address). Before: a reset (Task 10), then a refusal.
            (
                "a case variant of the email is the same account",
                Some(&a),
                &a_case_variant,
                true,
                Binding::Proceed,
            ),
            (
                "a case variant of the email, nothing left locally",
                Some(&a),
                &a_case_variant,
                false,
                Binding::Proceed,
            ),
            (
                "a non-ASCII case variant is the same account",
                Some(&id(Some("u-e"), Some("Émile@beebeeb.io"))),
                &id(None, Some("émile@beebeeb.io")),
                true,
                Binding::Proceed,
            ),
            ("another account", Some(&a), &b, true, Binding::Reset),
            (
                "another account, nothing left locally",
                Some(&a),
                &b,
                false,
                Binding::Reset,
            ),
            (
                "nothing in common to compare",
                Some(&only_an_id),
                &a_offline,
                true,
                Binding::Refuse,
            ),
            (
                "a session nobody can identify",
                Some(&a),
                &nobody,
                true,
                Binding::Refuse,
            ),
            (
                "a session nobody can identify, unrecorded local data: nothing is reset",
                None,
                &nobody,
                true,
                Binding::Refuse,
            ),
            (
                "a session nobody can identify, nothing here yet: nothing is started",
                None,
                &nobody,
                false,
                Binding::Refuse,
            ),
            (
                "a session nobody can identify, nothing left locally",
                Some(&a),
                &nobody,
                false,
                Binding::Refuse,
            ),
        ] {
            assert_eq!(decide(owner, session, has_local_data), expected, "{name}");
        }
    }

    #[test]
    fn the_user_id_decides_before_the_email() {
        assert_eq!(
            same_account(
                &id(Some("u-a"), Some("x@beebeeb.io")),
                &id(Some("u-b"), Some("x@beebeeb.io"))
            ),
            Some(false)
        );
        assert_eq!(
            same_account(
                &id(Some("u-a"), Some("old@beebeeb.io")),
                &id(Some("u-a"), Some("new@beebeeb.io"))
            ),
            Some(true)
        );
        assert_eq!(
            same_account(&id(None, Some("  ")), &id(None, Some(""))),
            None,
            "an empty email never matches"
        );
    }

    /// Ruling A″: the user id when both sides have one, otherwise the email in its canonical form (trimmed, then Unicode
    /// lower-cased, the server's `canonical_email`). Fix round 2 refused a case-only variant (`None`); before that,
    /// Task 10 read it as another account.
    #[test]
    fn emails_are_compared_in_their_canonical_form() {
        assert_eq!(
            same_account(&id(None, Some("Sam@beebeeb.io")), &id(None, Some("sam@beebeeb.io"))),
            Some(true),
            "an ASCII case variant"
        );
        assert_eq!(
            same_account(&id(None, Some("sam@beebeeb.io")), &id(None, Some("Sam@beebeeb.io"))),
            Some(true),
            "in either direction"
        );
        assert_eq!(
            same_account(&id(None, Some("Émile@beebeeb.io")), &id(None, Some("émile@beebeeb.io"))),
            Some(true),
            "a non-ASCII case variant"
        );
        assert_eq!(
            same_account(&id(None, Some("JOSÉ@beebeeb.io")), &id(None, Some("josé@beebeeb.io"))),
            Some(true),
            "non-ASCII capitals fold too"
        );
        assert_eq!(
            same_account(&id(None, Some("sam@beebeeb.io")), &id(None, Some("sami@beebeeb.io"))),
            Some(false),
            "another email is another account"
        );
        assert_eq!(
            same_account(&id(None, Some("josé@beebeeb.io")), &id(None, Some("jose@beebeeb.io"))),
            Some(false),
            "an accent is not a letter case"
        );
        assert_eq!(
            same_account(
                &id(Some("u-1"), Some("Sam@beebeeb.io")),
                &id(None, Some("sam@beebeeb.io"))
            ),
            Some(true),
            "one id alone: the email decides"
        );
        assert_eq!(
            same_account(&id(None, Some("sam@beebeeb.io")), &id(None, Some("sam@beebeeb.io"))),
            Some(true)
        );
        assert_eq!(
            same_account(&id(None, Some(" sam@beebeeb.io ")), &id(None, Some("sam@beebeeb.io"))),
            Some(true),
            "surrounding whitespace aside"
        );
        // Two ids that differ are different accounts whatever the emails say; with the same id, the id decides.
        assert_eq!(
            same_account(
                &id(Some("u-1"), Some("Sam@beebeeb.io")),
                &id(Some("u-1"), Some("sam@beebeeb.io"))
            ),
            Some(true)
        );
        assert_eq!(
            same_account(
                &id(Some("u-1"), Some("sam@beebeeb.io")),
                &id(Some("u-2"), Some("sam@beebeeb.io"))
            ),
            Some(false)
        );
    }

    /// The one client fold mirrors the server's: `trim()` (Unicode whitespace), then `to_lowercase()` (Unicode).
    #[test]
    fn the_canonical_email_trims_unicode_whitespace_and_lowercases_unicode() {
        assert_eq!(canonical_email("\u{2003}Émile@Beebeeb.IO\u{00A0}"), "émile@beebeeb.io");
        assert_eq!(canonical_email("JOSÉ@beebeeb.io"), "josé@beebeeb.io");
        assert!(
            same_email("\u{2003}SAM@beebeeb.io\n", "sam@beebeeb.io"),
            "surrounding Unicode whitespace aside"
        );
        assert!(!same_email("", ""), "an empty email never matches");
        assert!(!same_email(" \u{00A0} ", ""), "nor one that is only whitespace");
        assert!(
            !same_email("sam@beebeeb.io", "s am@beebeeb.io"),
            "inner whitespace is part of the address"
        );
    }

    /// One row of the identity-order tests: (branch, owner, session, expected result, resets, owner afterwards).
    type Row = (
        &'static str,
        Identity,
        Identity,
        Result<Bound, String>,
        u32,
        Option<Identity>,
    );

    /// Every row is checked; every mismatch is reported.
    fn mismatches(rows: Vec<Row>, reset_allowed: bool) -> Vec<String> {
        let mut mismatches = Vec::new();
        for (name, owner, session, expected, resets, owner_after) in rows {
            let fake = Fake::holding(Some(owner), true);
            let result = bind_before_engine_start(&fake, &session, reset_allowed);
            let seen = (result, *fake.resets.borrow(), fake.owner.borrow().clone());
            let wanted = (expected, resets, owner_after);
            if seen != wanted {
                mismatches.push(format!("{name}: got {seen:?}, want {wanted:?}"));
            }
        }
        mismatches
    }

    /// Rows that hold on every platform (ruling A″): the ids decide when both sides know one (here the same id), and
    /// otherwise equal canonical emails are the same account: an exact match, an ASCII or a non-ASCII letter-case
    /// variant, surrounding Unicode whitespace aside. The data is kept and the record follows the session.
    fn same_account_rows() -> Vec<Row> {
        let a = id(Some("u-a"), Some("a@beebeeb.io"));
        vec![
            (
                "1 ids decide: the same id, another email, is the same account",
                id(Some("u-a"), Some("old@beebeeb.io")),
                id(Some("u-a"), Some("new@beebeeb.io")),
                Ok(Bound::Kept),
                0,
                Some(id(Some("u-a"), Some("new@beebeeb.io"))),
            ),
            (
                "2 an exact email, no id on the session, is the same account",
                a.clone(),
                id(None, Some("a@beebeeb.io")),
                Ok(Bound::Kept),
                0,
                Some(a.clone()),
            ),
            (
                "2 an exact email, no id on the owner, is the same account",
                id(None, Some("a@beebeeb.io")),
                id(None, Some("a@beebeeb.io")),
                Ok(Bound::Kept),
                0,
                Some(id(None, Some("a@beebeeb.io"))),
            ),
            (
                "2 an ASCII case variant is the same account",
                a.clone(),
                id(None, Some("A@Beebeeb.io")),
                Ok(Bound::Kept),
                0,
                Some(id(Some("u-a"), Some("A@Beebeeb.io"))),
            ),
            (
                "2 an ASCII case variant of a legacy record is the same account",
                id(None, Some("Sam@Beebeeb.io")),
                id(None, Some("sam@beebeeb.io")),
                Ok(Bound::Kept),
                0,
                Some(id(None, Some("sam@beebeeb.io"))),
            ),
            (
                "2 an ASCII case variant, with an id on one side only, is the same account",
                id(None, Some("Sam@Beebeeb.io")),
                id(Some("u-1"), Some("sam@beebeeb.io")),
                Ok(Bound::Kept),
                0,
                Some(id(Some("u-1"), Some("sam@beebeeb.io"))),
            ),
            (
                "2 a non-ASCII case variant is the same account",
                id(Some("u-1"), Some("Émile@beebeeb.io")),
                id(None, Some("émile@beebeeb.io")),
                Ok(Bound::Kept),
                0,
                Some(id(Some("u-1"), Some("émile@beebeeb.io"))),
            ),
            (
                "2 surrounding Unicode whitespace aside",
                a.clone(),
                id(None, Some("\u{2003}A@Beebeeb.io\u{00A0}")),
                Ok(Bound::Kept),
                0,
                Some(id(Some("u-a"), Some("A@Beebeeb.io"))),
            ),
        ]
    }

    /// The engine start's identity order (ruling A″, superseding A and A′), branch by branch, through
    /// `bind_before_engine_start` with a reset allowed (macOS and Linux): the same-account rows keep the data; another
    /// id, or another email (an accent is not a letter case), is another account: reset.
    #[test]
    fn the_engine_start_binding_follows_the_identity_order_on_macos_and_linux() {
        let a = id(Some("u-a"), Some("a@beebeeb.io"));
        let mut rows = same_account_rows();
        rows.extend([
            (
                "1 ids decide: another id, the same email, is another account",
                a.clone(),
                id(Some("u-b"), Some("a@beebeeb.io")),
                Ok(Bound::Reset),
                1,
                Some(id(Some("u-b"), Some("a@beebeeb.io"))),
            ),
            (
                "3 another email, no ids, is another account: reset",
                a.clone(),
                id(None, Some("b@beebeeb.io")),
                Ok(Bound::Reset),
                1,
                Some(id(None, Some("b@beebeeb.io"))),
            ),
            (
                "3 an accent is not a letter case: reset",
                id(None, Some("josé@beebeeb.io")),
                id(None, Some("jose@beebeeb.io")),
                Ok(Bound::Reset),
                1,
                Some(id(None, Some("jose@beebeeb.io"))),
            ),
        ]);
        let broke = mismatches(rows, true);
        assert!(broke.is_empty(), "branches that broke:\n{}", broke.join("\n"));
    }

    /// The same order on Windows (no reset allowed): the same-account rows keep the data exactly as on macOS (a case
    /// variant now starts on Windows, as before the binding existed); another id or another email is refused as another
    /// account's data (`OTHER_ACCOUNT_ON_WINDOWS`). Nothing is ever deleted.
    #[test]
    fn the_engine_start_binding_follows_the_identity_order_on_windows() {
        let a = id(Some("u-a"), Some("a@beebeeb.io"));
        let refused_other = Err(OTHER_ACCOUNT_ON_WINDOWS.to_string());
        let mut rows = same_account_rows();
        rows.extend([
            (
                "1 ids decide: another id, the same email, is refused",
                a.clone(),
                id(Some("u-b"), Some("a@beebeeb.io")),
                refused_other.clone(),
                0,
                Some(a.clone()),
            ),
            (
                "3 another email, no ids, is refused",
                a.clone(),
                id(None, Some("b@beebeeb.io")),
                refused_other.clone(),
                0,
                Some(a.clone()),
            ),
        ]);
        let broke = mismatches(rows, false);
        assert!(broke.is_empty(), "branches that broke:\n{}", broke.join("\n"));
    }

    #[test]
    fn a_failed_reset_blocks_the_engine_and_records_nothing() {
        let fake = Fake {
            reset_fails: true,
            ..Fake::holding(Some(id(Some("u-a"), Some("a@beebeeb.io"))), true)
        };
        assert_eq!(
            bind_before_engine_start(&fake, &id(Some("u-b"), Some("b@beebeeb.io")), true),
            Err("disk I/O error".to_string())
        );
        assert!(
            fake.records.borrow().is_empty(),
            "the new account is never recorded over data still there"
        );
    }

    #[test]
    fn windows_refuses_instead_of_resetting() {
        let fake = Fake::holding(Some(id(Some("u-a"), Some("a@beebeeb.io"))), true);
        assert_eq!(
            bind_before_engine_start(&fake, &id(Some("u-b"), None), false),
            Err(OTHER_ACCOUNT_ON_WINDOWS.to_string())
        );
        assert_eq!(*fake.resets.borrow(), 0);
        assert!(fake.records.borrow().is_empty());
    }

    #[test]
    fn refusing_deletes_nothing_and_records_nothing() {
        let fake = Fake::holding(Some(id(Some("u-a"), None)), true);
        assert_eq!(
            bind_before_engine_start(&fake, &id(None, Some("a@beebeeb.io")), true),
            Err(IDENTITY_UNKNOWN.to_string())
        );
        assert_eq!(*fake.resets.borrow(), 0);
        assert!(fake.records.borrow().is_empty());
    }

    #[test]
    fn the_same_account_keeps_everything_and_completes_the_record() {
        let fake = Fake::holding(Some(id(None, Some("a@beebeeb.io"))), true);
        assert_eq!(
            bind_before_engine_start(&fake, &id(Some("u-a"), Some("a@beebeeb.io")), true),
            Ok(Bound::Kept)
        );
        assert_eq!(*fake.resets.borrow(), 0);
        assert_eq!(
            fake.records.borrow().as_slice(),
            &[id(Some("u-a"), Some("a@beebeeb.io"))]
        );
        assert_eq!(
            bind_before_engine_start(&fake, &id(None, Some("a@beebeeb.io")), true),
            Ok(Bound::Kept)
        );
        assert_eq!(
            fake.records.borrow().len(),
            1,
            "an offline start of the same account rewrites nothing"
        );
    }

    #[test]
    fn adoption_happens_once_and_never_replaces_an_owner() {
        let fake = Fake::holding(None, true);
        assert_eq!(adopt_unbound(&fake, Some(&id(None, Some("a@beebeeb.io")))), Ok(true));
        assert_eq!(
            adopt_unbound(&fake, Some(&id(None, Some("b@beebeeb.io")))),
            Ok(false),
            "a recorded owner is never replaced"
        );
        assert_eq!(fake.owner.borrow().clone(), Some(id(None, Some("a@beebeeb.io"))));
        assert_eq!(
            adopt_unbound(&Fake::holding(None, false), Some(&id(None, Some("a@beebeeb.io")))),
            Ok(false),
            "nothing to adopt"
        );
        assert_eq!(
            adopt_unbound(&Fake::holding(None, true), Some(&Identity::default())),
            Ok(false),
            "no identity, no owner"
        );
    }

    /// F2: adoption is a one-shot migration and needs a candidate that holds a vault key. No candidate (a Keychain with
    /// a token but no key) adopts nothing AND uses up the window, so a later startup with a key finds it shut.
    #[test]
    fn adoption_needs_a_candidate_and_uses_up_its_window_even_without_one() {
        let fake = Fake::holding(None, true);
        assert_eq!(adopt_unbound(&fake, None), Ok(false), "no vault key, no candidate");
        assert_eq!(
            adopt_unbound(&fake, Some(&id(None, Some("b@beebeeb.io")))),
            Ok(false),
            "the first startup has passed"
        );
        assert_eq!(fake.owner.borrow().clone(), None, "nobody was adopted");
        assert!(fake.records.borrow().is_empty());
    }
}
