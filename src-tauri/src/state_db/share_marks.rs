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
