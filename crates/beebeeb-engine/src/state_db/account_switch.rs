//! An account switch's guard (w26-account-switch): what of the leaving account's still needs a person's answer.

use super::*;

impl StateDb {
    /// The first kind of the previous account's work a person has not answered for, as a fixed word (never a name):
    /// a queued change, a staged copy, an upload to finish (held for review or not), a held edit, a placeholder left
    /// at another path with unknown bytes (sign-out's D11 rule: never deleted unasked), or a file in a state that is
    /// not at rest. Any of it keeps another account out of this data with nothing deleted; the account that owns it
    /// signs in and settles it. A file row of unknown content is not decided here: see [`Self::guarded_paths`].
    pub fn work_waiting_for_a_person(&self) -> Result<Option<&'static str>> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        let kinds: [(&str, String); 6] = [
            ("queued change", "SELECT COUNT(*) FROM operation_queue".into()),
            ("staged copy", "SELECT COUNT(*) FROM staged_payloads".into()),
            ("upload to finish", "SELECT COUNT(*) FROM upload_finalizations".into()),
            ("held edit", "SELECT COUNT(*) FROM held_edits".into()),
            ("file of unknown content", "SELECT COUNT(*) FROM unknown_orphans".into()),
            (
                "file not at rest",
                "SELECT COUNT(*) FROM files WHERE status IN ('uploading', 'conflict', 'error', 'trashing')".into(),
            ),
        ];
        for (word, sql) in kinds {
            if conn.query_row(&sql, [], |row| row.get::<_, i64>(0))? != 0 {
                return Ok(Some(word));
            }
        }
        Ok(None)
    }

    /// RC2-5: the waiting work of [`Self::work_waiting_for_a_person`] whose every byte is an item in the sync folder,
    /// by the path of that item: a new file or an edit queued while sync was paused, a new
    /// folder queued, an edit held for review (a rebuilt state.db holds every file of unknown content so), and the
    /// staged copies of those queued changes. A person's answer about those items (keep them aside, or discard them)
    /// settles all of it. `None` when any other work waits (another kind of change, an upload the server may already
    /// hold, a file of unknown content at another path, a conflict, an error, a trash): that needs its account.
    pub fn work_in_the_folder(&self) -> Result<Option<Vec<String>>> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        if other_work_waits(&conn)? {
            return Ok(None);
        }
        let mut items = conn.prepare(
            "SELECT f.path FROM operation_queue o JOIN files f ON f.file_id = o.file_id
             UNION SELECT f.path FROM held_edits h JOIN files f ON f.file_id = h.file_id
             ORDER BY 1",
        )?;
        let items = items.query_map([], |row| row.get::<_, String>(0))?;
        items.collect::<Result<Vec<_>>>().map(Some)
    }

    /// RC3-1: forgets the waiting work of [`Self::work_in_the_folder`], in one transaction, once a person's answer
    /// took its items out of the folder (or they hold nothing): the queued changes, their staged copies' rows and
    /// upload sessions, and the edits held for review. A row this PC made and never sent goes with its queued
    /// create (its item is no longer the account's to send); a row the server knows stays, back at rest as an item
    /// with no bytes on this PC. The caller's removal of the synced items refuses while any of this is left (the
    /// Windows purge's first gate), which is what kept every answered switch out (the Windows test machine, rc3/f1).
    ///
    /// `Some`: the payload paths the forgotten rows named, for the caller to delete outside the sync folder.
    /// `None`, nothing changed: other work waits (see [`Self::work_in_the_folder`]).
    pub fn forget_work_in_the_folder(&self) -> Result<Option<Vec<String>>> {
        let mut conn = self.0.lock().expect("state_db mutex poisoned");
        let tx = conn.transaction()?;
        if other_work_waits(&tx)? {
            return Ok(None);
        }
        let payloads = {
            let mut paths = tx.prepare(
                "SELECT payload_path FROM operation_queue WHERE payload_path IS NOT NULL
                 UNION SELECT path FROM staged_payloads
                 UNION SELECT r.payload_path FROM upload_resume r JOIN operation_queue o ON o.op_id = r.op_id",
            )?;
            let paths = paths.query_map([], |row| row.get::<_, String>(0))?;
            paths.collect::<Result<Vec<_>>>()?
        };
        // Before the queue goes: the rows it made that the server never had.
        tx.execute(
            "DELETE FROM files WHERE current_version = 0 AND remote_updated_at = 0
               AND file_id IN (SELECT file_id FROM operation_queue WHERE file_id IS NOT NULL)",
            [],
        )?;
        tx.execute(
            "UPDATE files SET status = 'cloud_only' WHERE status = 'uploading'
               AND (file_id IN (SELECT file_id FROM operation_queue WHERE file_id IS NOT NULL)
                    OR file_id IN (SELECT file_id FROM held_edits))",
            [],
        )?;
        tx.execute(
            "DELETE FROM upload_resume WHERE op_id IN (SELECT op_id FROM operation_queue)",
            [],
        )?;
        tx.execute("DELETE FROM staged_payloads", [])?;
        tx.execute("DELETE FROM held_edits", [])?;
        tx.execute("DELETE FROM operation_queue", [])?;
        tx.commit()?;
        self.1.fetch_add(1, std::sync::atomic::Ordering::AcqRel);
        Ok(Some(payloads))
    }

    /// Where each file row of unknown content is, as the database writes paths (w27). Such a row alone is no work of
    /// a person's: a document made in the app is one, with nothing of it in the folder. It keeps another account out
    /// only while the item there may hold bytes on this PC, which the OS answers (`SyncRootPort::may_hold_bytes`).
    pub fn guarded_paths(&self) -> Result<Vec<String>> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        let mut rows = conn.prepare(&format!("SELECT path FROM files WHERE {GUARDED_SQL} ORDER BY rowid"))?;
        let paths = rows.query_map([], |row| row.get::<_, String>(0))?;
        paths.collect()
    }
}

/// Whether any waiting work is more than items in the sync folder (see [`StateDb::work_in_the_folder`]): another
/// kind of change, a change with no row, a staged copy no queued change names, an upload to finish, a file of unknown
/// content at another path, a conflict, an error, a trash, or a file sending with nothing queued or held for it.
fn other_work_waits(conn: &rusqlite::Connection) -> Result<bool> {
    let other_work = [
        "SELECT COUNT(*) FROM operation_queue WHERE kind NOT IN ('upload_file', 'upload_version', 'create_folder')",
        "SELECT COUNT(*) FROM operation_queue o WHERE NOT EXISTS (SELECT 1 FROM files f WHERE f.file_id = o.file_id)",
        "SELECT COUNT(*) FROM staged_payloads s
           WHERE NOT EXISTS (SELECT 1 FROM operation_queue o WHERE o.payload_path = s.path)",
        // A save made in the app: its bytes are only the staged copy outside the folder, so it is no item there.
        "SELECT COUNT(*) FROM operation_queue WHERE write_id IS NOT NULL",
        "SELECT COUNT(*) FROM upload_finalizations",
        "SELECT COUNT(*) FROM unknown_orphans",
        "SELECT COUNT(*) FROM files WHERE status IN ('conflict', 'error', 'trashing')",
        "SELECT COUNT(*) FROM files f WHERE f.status = 'uploading'
           AND NOT EXISTS (SELECT 1 FROM operation_queue o WHERE o.file_id = f.file_id)
           AND NOT EXISTS (SELECT 1 FROM held_edits h WHERE h.file_id = f.file_id)",
    ];
    for sql in other_work {
        if conn.query_row(sql, [], |row| row.get::<_, i64>(0))? != 0 {
            return Ok(true);
        }
    }
    Ok(false)
}
