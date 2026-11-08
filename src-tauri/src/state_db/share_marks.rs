//! v2 B05e: the local "compared" marks for share safety numbers. One `sync_state` row per
//! (account id, peer user id): `share_compared:{account}:{peer}` = `{sha256(peer key) hex}:{1|0|a}`.
//! Local only (the server never learns who compared whom); a sign-out purge removes them with the
//! rest of `sync_state`.

use super::*;

const PREFIX: &str = "share_compared";

fn mark_key(account_id: &str, peer_user_id: &str) -> String {
    format!("{PREFIX}:{account_id}:{peer_user_id}")
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

impl StateDb {
    /// The stored mark: the peer key fingerprint (hex) it was made against and what it says.
    pub fn share_mark(&self, account_id: &str, peer_user_id: &str) -> Result<Option<(String, MarkKind)>> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        let value: Option<String> = conn
            .query_row(
                "SELECT value FROM sync_state WHERE key = ?1",
                params![mark_key(account_id, peer_user_id)],
                |row| row.get(0),
            )
            .optional()?;
        Ok(value.and_then(|v| {
            let (fp, kind) = v.split_once(':')?;
            Some((fp.to_owned(), MarkKind::decode(kind)?))
        }))
    }

    pub fn set_share_mark(
        &self,
        account_id: &str,
        peer_user_id: &str,
        fingerprint_hex: &str,
        kind: MarkKind,
    ) -> Result<()> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        conn.execute(
            "INSERT INTO sync_state (key, value) VALUES (?1, ?2) ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            params![
                mark_key(account_id, peer_user_id),
                format!("{fingerprint_hex}:{}", kind.encode())
            ],
        )?;
        Ok(())
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
