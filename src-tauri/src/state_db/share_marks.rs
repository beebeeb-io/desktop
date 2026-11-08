//! v2 B05e: the local "compared" marks for share safety numbers. One `sync_state` row per
//! (account id, peer user id): `share_compared:{account}:{peer}` = `{sha256(peer key) hex}:{1|0}`.
//! Local only (the server never learns who compared whom); a sign-out purge removes them with the
//! rest of `sync_state`.

use super::*;

const PREFIX: &str = "share_compared";

fn mark_key(account_id: &str, peer_user_id: &str) -> String {
    format!("{PREFIX}:{account_id}:{peer_user_id}")
}

impl StateDb {
    /// The stored mark: the peer key fingerprint (hex) it was made against and whether the numbers matched.
    pub fn share_mark(&self, account_id: &str, peer_user_id: &str) -> Result<Option<(String, bool)>> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        let value: Option<String> = conn
            .query_row(
                "SELECT value FROM sync_state WHERE key = ?1",
                params![mark_key(account_id, peer_user_id)],
                |row| row.get(0),
            )
            .optional()?;
        Ok(value.and_then(|v| {
            let (fp, matched) = v.split_once(':')?;
            Some((fp.to_owned(), matched == "1"))
        }))
    }

    pub fn set_share_mark(
        &self,
        account_id: &str,
        peer_user_id: &str,
        fingerprint_hex: &str,
        matched: bool,
    ) -> Result<()> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        conn.execute(
            "INSERT INTO sync_state (key, value) VALUES (?1, ?2) ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            params![
                mark_key(account_id, peer_user_id),
                format!("{fingerprint_hex}:{}", if matched { 1 } else { 0 })
            ],
        )?;
        Ok(())
    }

    pub fn clear_share_mark(&self, account_id: &str, peer_user_id: &str) -> Result<()> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        conn.execute(
            "DELETE FROM sync_state WHERE key = ?1",
            params![mark_key(account_id, peer_user_id)],
        )?;
        Ok(())
    }
}

/// The exact recipient key a share was sealed to (B05e), per (account id, invite id), hex. The
/// safety number and the "compared" mark are computed from these bytes, never from a separate key
/// lookup. `offered` holds a key the seal refused (`peer_key_changed`) until the user accepts it.
const SEALED: &str = "share_sealed";
const OFFERED: &str = "share_offered";

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
}
