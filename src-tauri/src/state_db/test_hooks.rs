//! Test-only hooks: failing landings and direct SQL (split out of `state_db.rs`).

use super::*;

impl StateDb {
    /// Test hook: drop `table.column` from a database, as a test simulates an older or damaged one. SQLite refuses
    /// to drop a column a trigger names ("error in trigger … after drop column"), and LD-0113's triggers name
    /// `operation_queue.payload_path`, so every trigger on `table` whose SQL names `column` goes first. Opening the
    /// database again recreates the LD-0113 triggers (`CREATE TRIGGER IF NOT EXISTS`).
    #[cfg(test)]
    pub(crate) fn drop_column_for_test(conn: &Connection, table: &str, column: &str) -> rusqlite::Result<()> {
        let naming: Vec<String> = conn
            .prepare("SELECT name FROM sqlite_master WHERE type = 'trigger' AND tbl_name = ?1 AND instr(sql, ?2) > 0")?
            .query_map(params![table, column], |row| row.get(0))?
            .collect::<rusqlite::Result<_>>()?;
        for trigger in naming {
            conn.execute(&format!("DROP TRIGGER IF EXISTS {trigger}"), [])?;
        }
        conn.execute(&format!("ALTER TABLE {table} DROP COLUMN {column}"), [])?;
        Ok(())
    }

    /// Test hook: hold the database lock so a test can park every writer behind
    /// it and force two requests to overlap deterministically (task 1684).
    #[cfg(test)]
    pub(crate) fn hold_lock_for_test(&self) -> std::sync::MutexGuard<'_, Connection> {
        self.0.lock().expect("state_db mutex poisoned")
    }
    #[cfg(all(test, target_os = "macos"))]
    pub(crate) fn set_created_at_for_test(&self, op_id: &str, created_at: i64) {
        let conn = self.0.lock().unwrap();
        conn.execute(
            "UPDATE operation_queue SET created_at = ?2 WHERE op_id = ?1",
            params![op_id, created_at],
        )
        .unwrap();
    }
    /// T14: an op whose base went missing after it was queued.
    #[cfg(test)]
    pub(crate) fn set_base_version_for_test(&self, op_id: &str, base_version: Option<i64>) {
        let conn = self.0.lock().unwrap();
        conn.execute(
            "UPDATE operation_queue SET base_version = ?2 WHERE op_id = ?1",
            params![op_id, base_version],
        )
        .unwrap();
    }
    /// T6, T62: an alias the id swap would record (spec §7.1), without a landing.
    #[cfg(test)]
    pub(crate) fn insert_alias_for_test(&self, provisional_id: &str, server_id: &str, created_at: i64) {
        let conn = self.0.lock().unwrap();
        conn.execute(
            "INSERT INTO id_aliases (provisional_id, server_id, created_at) VALUES (?1, ?2, ?3)",
            params![provisional_id, server_id, created_at],
        )
        .unwrap();
    }
    /// Writes the held pair as given, including the half-set row
    /// (a write id without its base) that no writer produces and every read refuses.
    #[cfg(test)]
    pub(crate) fn set_held_pair_for_test(&self, file_id: &str, write_id: Option<&str>, base: Option<i64>) {
        let conn = self.0.lock().unwrap();
        conn.execute(
            "UPDATE files SET held_write_id = ?2, held_base = ?3 WHERE file_id = ?1",
            params![file_id, write_id, base],
        )
        .unwrap();
    }
    #[cfg(test)]
    pub(crate) fn alias_count_for_test(&self) -> i64 {
        let conn = self.0.lock().unwrap();
        conn.query_row("SELECT COUNT(*) FROM id_aliases", [], |row| row.get(0))
            .unwrap()
    }
    /// T26 (m-2): an op as an earlier build would have queued it (spec §10.2).
    #[cfg(test)]
    pub(crate) fn set_write_origin_for_test(&self, op_id: &str, origin: WriteOrigin) {
        let conn = self.0.lock().unwrap();
        conn.execute(
            "UPDATE operation_queue SET write_origin = ?2 WHERE op_id = ?1",
            params![op_id, origin.as_str()],
        )
        .unwrap();
    }
    /// A resume row whose `complete` answered (spec §8.6 rule 1), without an upload.
    #[cfg(test)]
    pub(crate) fn set_completed_for_test(&self, op_id: &str, version: i64) {
        let conn = self.0.lock().unwrap();
        let n = conn
            .execute(
                "UPDATE upload_resume SET completed_version = ?2 WHERE op_id = ?1",
                params![op_id, version],
            )
            .unwrap();
        assert_eq!(n, 1, "no resume row for {op_id}");
    }
    /// P10: where the landing's alias sends a provisional id (spec §7.1).
    #[cfg(test)]
    pub(crate) fn alias_target_for_test(&self, provisional_id: &str) -> Option<String> {
        let conn = self.0.lock().unwrap();
        conn.query_row(
            "SELECT server_id FROM id_aliases WHERE provisional_id = ?1",
            params![provisional_id],
            |row| row.get(0),
        )
        .optional()
        .unwrap()
    }
    /// T26: an op that used up its attempts (parked, with its bytes).
    #[cfg(test)]
    pub(crate) fn park_for_test(&self, op_id: &str) {
        let conn = self.0.lock().unwrap();
        conn.execute(
            "UPDATE operation_queue SET attempts = max_attempts WHERE op_id = ?1",
            params![op_id],
        )
        .unwrap();
    }
    /// P2: a Finder write is an op with a write id; the restore/upload wait is keyed on it.
    #[cfg(test)]
    pub(crate) fn set_write_id_for_test(&self, op_id: &str, write_id: &str) {
        let conn = self.0.lock().unwrap();
        conn.execute(
            "UPDATE operation_queue SET write_id = ?2 WHERE op_id = ?1",
            params![op_id, write_id],
        )
        .unwrap();
    }
    /// Tests outside this module: run one SQL statement (e.g. park an op by hand).
    #[cfg(test)]
    pub(crate) fn test_execute(&self, sql: &str) -> Result<usize> {
        self.0.lock().expect("state_db mutex poisoned").execute(sql, [])
    }
}
