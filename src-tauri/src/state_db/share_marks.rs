//! v2 B05e: the local "compared" marks for share safety numbers. One `sync_state` row per
//! (account id, person), each `{sha256(peer key) hex}:{1|0|a}`, under two keys:
//! - `share_compared:{account}:{peer user id}`, the person's account id;
//! - `share_compared_email:{account}:{sha256("{account}:{normalized email}") hex}`, the address: the
//!   server's sent list names an account id only while an invite to it exists, so the address is what
//!   a later share to the same person can always find (task 2437 item 2 adds peer ids to more of the
//!   server's lists). Only a digest of it is kept, so a row a sign-out keeps never holds the address.
//!
//! Local only (the server never learns who compared whom). A sign-out keeps them, as it keeps the
//! custody ids (lead ruling 2026-11-09, `state_db/signout.rs`); a reset of local data removes them.

use super::*;

const PREFIX: &str = "share_compared";
const EMAIL_PREFIX: &str = "share_compared_email";

/// The `sync_state` rows of B05e (the marks, and the sealed, offered and accepted keys below), as
/// `LIKE` patterns. A sign-out keeps them; a reset of local data removes them.
pub(super) const SHARE_TRUST_ROWS: [&str; 5] = [
    "share_compared:%",
    "share_compared_email:%",
    "share_sealed:%",
    "share_offered:%",
    "share_accepted:%",
];

/// ` AND key NOT LIKE '…'` for each of [`SHARE_TRUST_ROWS`]: appended to a `sync_state` statement that
/// leaves them alone (the sign-out's purge, and the count of account rows they are not part of).
pub(super) fn keep_share_trust_rows_sql() -> String {
    SHARE_TRUST_ROWS
        .iter()
        .map(|pattern| format!(" AND key NOT LIKE '{pattern}'"))
        .collect()
}

fn mark_key(account_id: &str, peer_user_id: &str) -> String {
    format!("{PREFIX}:{account_id}:{peer_user_id}")
}

/// The form an address is kept and compared in: no surrounding space, lower case.
pub fn normalized_email(email: &str) -> String {
    email.trim().to_lowercase()
}

/// The address row's key: a digest of the account id and the normalized address, never the address.
pub(super) fn email_mark_key(account_id: &str, email: &str) -> String {
    use sha2::{Digest, Sha256};
    let digest = Sha256::digest(format!("{account_id}:{}", normalized_email(email)));
    format!("{EMAIL_PREFIX}:{account_id}:{}", crate::hexfmt::encode(&digest))
}

/// What a mark says about the key it was made against. Every kind pins that key: the next member
/// seal to the person passes its fingerprint to core (`PeerKeyExpectation::Compared`), so a server
/// key that differs is refused again.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MarkKind {
    /// They match: the numbers were compared and matched (`1`).
    Matched,
    /// They don't match (`0`).
    NotMatched,
    /// DW-0515 "Share anyway": the key a changed-key warning carried was accepted without comparing
    /// (`a`). It never reads as compared.
    Accepted,
}

impl MarkKind {
    fn encode(self) -> &'static str {
        match self {
            Self::Matched => "1",
            Self::NotMatched => "0",
            Self::Accepted => "a",
        }
    }

    fn decode(v: &str) -> Option<Self> {
        match v {
            "1" => Some(Self::Matched),
            "0" => Some(Self::NotMatched),
            "a" => Some(Self::Accepted),
            _ => None,
        }
    }
}

/// A stored mark: `{fingerprint}:{kind}`. A row that is there but cannot be decoded (no `:`, or a
/// kind this build does not know, for example one a newer build wrote) is an error, never "no mark".
fn decode_mark(value: &str) -> Result<(String, MarkKind)> {
    value
        .split_once(':')
        .and_then(|(fp, kind)| Some((fp.to_owned(), MarkKind::decode(kind)?)))
        .ok_or_else(|| {
            rusqlite::Error::FromSqlConversionFailure(
                0,
                rusqlite::types::Type::Text,
                "a share mark this build cannot read".into(),
            )
        })
}

/// Whether a read failed on the mark itself (a row this build cannot decode, or a value that is not
/// text), not on the database (busy, locked, I/O).
pub fn is_unreadable_mark(error: &rusqlite::Error) -> bool {
    matches!(
        error,
        rusqlite::Error::FromSqlConversionFailure(..) | rusqlite::Error::InvalidColumnType(..)
    )
}

/// Every key a mark of one person is kept under: their address (when known) and each account id.
fn person_keys<'a>(
    account_id: &'a str,
    email: Option<&'a str>,
    peers: &'a [String],
) -> impl Iterator<Item = String> + 'a {
    email
        .map(|e| email_mark_key(account_id, e))
        .into_iter()
        .chain(peers.iter().map(move |p| mark_key(account_id, p)))
}

fn mark_in(conn: &Connection, key: &str) -> Result<Option<(String, MarkKind)>> {
    let value: Option<String> = conn
        .query_row("SELECT value FROM sync_state WHERE key = ?1", params![key], |row| {
            row.get(0)
        })
        .optional()?;
    value.as_deref().map(decode_mark).transpose()
}

fn set_mark_in(conn: &Connection, key: &str, fingerprint_hex: &str, kind: MarkKind) -> Result<()> {
    conn.execute(
        "INSERT INTO sync_state (key, value) VALUES (?1, ?2) ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        params![key, format!("{fingerprint_hex}:{}", kind.encode())],
    )?;
    Ok(())
}

impl StateDb {
    /// Every mark of one person (their address and each account id), read in one go. A row that
    /// cannot be decoded is an error ([`is_unreadable_mark`]), never "no mark".
    pub fn person_marks(
        &self,
        account_id: &str,
        email: Option<&str>,
        peers: &[String],
    ) -> Result<Vec<(String, MarkKind)>> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        let mut marks = Vec::new();
        for key in person_keys(account_id, email, peers) {
            marks.extend(mark_in(&conn, &key)?);
        }
        Ok(marks)
    }

    /// One mark written to every key of a person in one transaction, so a failure leaves every key
    /// as it was and no two keys can pin different fingerprints. With `keep_matched`, a key that
    /// already holds "They match" on this same fingerprint keeps it (DW-0515 Compare, then Share
    /// anyway); reading such a key that cannot be decoded is an error and nothing is written.
    pub fn set_person_marks(
        &self,
        account_id: &str,
        email: Option<&str>,
        peers: &[String],
        fingerprint_hex: &str,
        kind: MarkKind,
        keep_matched: bool,
    ) -> Result<()> {
        let mut conn = self.0.lock().expect("state_db mutex poisoned");
        let tx = conn.transaction()?;
        for key in person_keys(account_id, email, peers) {
            if keep_matched
                && mark_in(&tx, &key)?.is_some_and(|(fp, k)| fp == fingerprint_hex && k == MarkKind::Matched)
            {
                continue;
            }
            set_mark_in(&tx, &key, fingerprint_hex, kind)?;
        }
        tx.commit()
    }
}

/// One key at a time: the tests' view of the rows [`StateDb::person_marks`] and
/// [`StateDb::set_person_marks`] read and write together.
#[cfg(test)]
impl StateDb {
    /// The mark under a person's account id: the peer key fingerprint (hex) it was made against and
    /// what it says. A row that cannot be decoded is an error.
    pub fn share_mark(&self, account_id: &str, peer_user_id: &str) -> Result<Option<(String, MarkKind)>> {
        mark_in(
            &self.0.lock().expect("state_db mutex poisoned"),
            &mark_key(account_id, peer_user_id),
        )
    }

    pub fn set_share_mark(
        &self,
        account_id: &str,
        peer_user_id: &str,
        fingerprint_hex: &str,
        kind: MarkKind,
    ) -> Result<()> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        set_mark_in(&conn, &mark_key(account_id, peer_user_id), fingerprint_hex, kind)
    }

    /// The mark under a person's address (normalized here). A row that cannot be decoded is an error.
    pub fn share_email_mark(&self, account_id: &str, email: &str) -> Result<Option<(String, MarkKind)>> {
        mark_in(
            &self.0.lock().expect("state_db mutex poisoned"),
            &email_mark_key(account_id, email),
        )
    }

    pub fn set_share_email_mark(
        &self,
        account_id: &str,
        email: &str,
        fingerprint_hex: &str,
        kind: MarkKind,
    ) -> Result<()> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        set_mark_in(&conn, &email_mark_key(account_id, email), fingerprint_hex, kind)
    }

    /// Test hook: the key the address row of `email` is kept under.
    pub(crate) fn email_mark_key_for_test(account_id: &str, email: &str) -> String {
        email_mark_key(account_id, email)
    }
}

/// The exact recipient key a share was sealed to (B05e), per (account id, invite id), hex. The
/// safety number and the "compared" mark are computed from these bytes, never from a separate key
/// lookup. `offered` holds a key the seal refused (`peer_key_changed`) until the user accepts it.
const SEALED: &str = "share_sealed";
const OFFERED: &str = "share_offered";
/// DW-0515 over People, "Share anyway": the fingerprint (hex) of the sealed key the person accepted
/// for THIS share. The person's mark is left as it was, so later seals stay guarded.
const ACCEPTED: &str = "share_accepted";

impl StateDb {
    fn share_key_get(&self, kind: &str, account_id: &str, invite_id: &str) -> Result<Option<String>> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        conn.query_row(
            "SELECT value FROM sync_state WHERE key = ?1",
            params![format!("{kind}:{account_id}:{invite_id}")],
            |row| row.get(0),
        )
        .optional()
    }

    fn share_key_set(&self, kind: &str, account_id: &str, invite_id: &str, key_hex: Option<&str>) -> Result<()> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        let key = format!("{kind}:{account_id}:{invite_id}");
        match key_hex {
            Some(v) => conn.execute(
                "INSERT INTO sync_state (key, value) VALUES (?1, ?2) ON CONFLICT(key) DO UPDATE SET value = excluded.value",
                params![key, v],
            )?,
            None => conn.execute("DELETE FROM sync_state WHERE key = ?1", params![key])?,
        };
        Ok(())
    }

    pub fn share_sealed_key(&self, account_id: &str, invite_id: &str) -> Result<Option<String>> {
        self.share_key_get(SEALED, account_id, invite_id)
    }

    pub fn set_share_sealed_key(&self, account_id: &str, invite_id: &str, key_hex: &str) -> Result<()> {
        self.share_key_set(SEALED, account_id, invite_id, Some(key_hex))?;
        self.share_key_set(OFFERED, account_id, invite_id, None)
    }

    pub fn share_offered_key(&self, account_id: &str, invite_id: &str) -> Result<Option<String>> {
        self.share_key_get(OFFERED, account_id, invite_id)
    }

    pub fn set_share_offered_key(&self, account_id: &str, invite_id: &str, key_hex: &str) -> Result<()> {
        self.share_key_set(OFFERED, account_id, invite_id, Some(key_hex))
    }

    pub fn share_accepted_fingerprint(&self, account_id: &str, invite_id: &str) -> Result<Option<String>> {
        self.share_key_get(ACCEPTED, account_id, invite_id)
    }

    pub fn set_share_accepted_fingerprint(
        &self,
        account_id: &str,
        invite_id: &str,
        fingerprint_hex: &str,
    ) -> Result<()> {
        self.share_key_set(ACCEPTED, account_id, invite_id, Some(fingerprint_hex))
    }
}
