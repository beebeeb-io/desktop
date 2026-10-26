//! Task 2191 S2: durable recording, scheduling and conditional promotion.
use super::*;

#[derive(Debug, Clone, Default, PartialEq)]
pub struct UnknownCheck {
    pub state: Option<String>,
    pub attempts: i64,
    pub stops: i64,
    pub next_attempt_at: i64,
    pub last_error: Option<String>,
    pub copied_hash: Option<String>,
    pub recorded_hash: Option<String>,
    pub recorded_size: Option<i64>,
    pub recorded_mtime: Option<i64>,
    pub resume_candidate: Option<String>,
    pub resume_chunk: Option<i64>,
    pub resume_digest: Option<String>,
    pub updated_at: i64,
}

#[derive(Debug, Clone)]
pub struct UnknownRow {
    pub file_id: String,
    pub path: String,
    pub current_version: i64,
    pub record: UnknownCheck,
}

impl StateDb {
    pub(crate) fn update_unknown_budget<T>(
        &self,
        update: impl FnOnce(&mut crate::unknown_check::budget::Ledger) -> T,
    ) -> anyhow::Result<T> {
        let mut conn = self.0.lock().expect("state_db mutex poisoned");
        let tx = conn.transaction()?;
        let raw: Option<String> = tx
            .query_row(
                "SELECT value FROM sync_state WHERE key='unknown_check_ledger'",
                [],
                |r| r.get(0),
            )
            .optional()?;
        // Corruption fails closed; never silently reset an allowance.
        let mut ledger = match raw {
            Some(raw) => serde_json::from_str(&raw)?,
            None => Default::default(),
        };
        let result = update(&mut ledger);
        tx.execute(
            "INSERT INTO sync_state(key,value) VALUES ('unknown_check_ledger',?1)
            ON CONFLICT(key) DO UPDATE SET value=excluded.value",
            [serde_json::to_string(&ledger)?],
        )?;
        tx.commit()?;
        Ok(result)
    }

    pub fn record_baseline_if_unknown(&self, id: &str, path: &str, version: i64, hash: &str) -> Result<bool> {
        if version <= 0 {
            return Ok(false);
        }
        let mut conn = self.0.lock().expect("state_db mutex poisoned");
        let tx = conn.transaction()?;
        let changed = tx.execute(
            "UPDATE files SET local_base_version=?3,local_hash=?4,fetched_min_version=0,fetched_hash=NULL
            WHERE file_id=?1 AND path=?2 AND local_hash IS NULL AND fetched_min_version<0
              AND item_kind='file' AND status IN ('local','cloud_only')",
            params![id, path, version, hash],
        )? == 1;
        if changed {
            tx.execute("DELETE FROM unknown_checks WHERE file_id=?1", [id])?;
        }
        tx.commit()?;
        Ok(changed)
    }
    pub fn reset_after_divergence(&self, id: &str, path: &str) -> Result<bool> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        Ok(conn.execute(
            "UPDATE files SET status='cloud_only',fetched_min_version=-1,fetched_hash=NULL,local_hash=NULL
            WHERE file_id=?1 AND path=?2 AND local_hash IS NULL AND fetched_min_version<0
              AND item_kind='file' AND status IN ('local','cloud_only')",
            params![id, path],
        )? == 1)
    }
    /// Writes preserve the independently maintained platform mark. A moved/resolved row cannot
    /// receive a stale record. Callers retain `removing` until the watcher consumes its event.
    pub fn put_unknown_check(&self, id: &str, path: &str, r: &UnknownCheck) -> Result<bool> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        Ok(conn.execute("INSERT INTO unknown_checks(file_id,state,attempts,stops,next_attempt_at,last_error,copied_hash,
            recorded_hash,recorded_size,recorded_mtime,resume_candidate,resume_chunk,resume_digest,updated_at)
            SELECT ?1,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15 FROM files
            WHERE file_id=?1 AND path=?2 AND local_hash IS NULL AND fetched_min_version<0
              AND item_kind='file' AND status IN ('local','cloud_only')
            ON CONFLICT(file_id) DO UPDATE SET state=excluded.state,attempts=excluded.attempts,stops=excluded.stops,
            next_attempt_at=excluded.next_attempt_at,last_error=excluded.last_error,copied_hash=excluded.copied_hash,
            recorded_hash=excluded.recorded_hash,recorded_size=excluded.recorded_size,recorded_mtime=excluded.recorded_mtime,
            resume_candidate=excluded.resume_candidate,resume_chunk=excluded.resume_chunk,resume_digest=excluded.resume_digest,
            updated_at=excluded.updated_at", params![id,path,r.state,r.attempts,r.stops,r.next_attempt_at,r.last_error,
            r.copied_hash,r.recorded_hash,r.recorded_size,r.recorded_mtime,r.resume_candidate,r.resume_chunk,r.resume_digest,r.updated_at])? == 1)
    }
    pub fn record_unknown_once(&self, id: &str, path: &str, r: &UnknownCheck) -> Result<bool> {
        let mut conn = self.0.lock().expect("state_db mutex poisoned");
        let tx = conn.transaction()?;
        tx.execute(
            "INSERT OR IGNORE INTO unknown_checks(file_id) SELECT file_id FROM files WHERE file_id=?1",
            [id],
        )?;
        let n=tx.execute("UPDATE unknown_checks SET recorded_hash=?3,recorded_size=?4,recorded_mtime=?5,state=NULL
            WHERE file_id=?1 AND recorded_hash IS NULL AND EXISTS(SELECT 1 FROM files f WHERE f.file_id=?1 AND f.path=?2
            AND f.item_kind='file' AND f.local_hash IS NULL AND f.fetched_min_version<0 AND f.status IN ('local','cloud_only'))",
            params![id,path,r.recorded_hash,r.recorded_size,r.recorded_mtime])?;
        tx.commit()?;
        Ok(n == 1)
    }
    fn unknown_rows(&self, predicate: &str, now: i64, limit: usize, order: &str) -> Result<Vec<UnknownRow>> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        let mut stmt = conn.prepare(&format!(
            "SELECT f.file_id,f.path,f.current_version,u.state,COALESCE(u.attempts,0),COALESCE(u.stops,0),
            COALESCE(u.next_attempt_at,0),u.last_error,u.copied_hash,u.recorded_hash,u.recorded_size,u.recorded_mtime,
            u.resume_candidate,u.resume_chunk,u.resume_digest,COALESCE(u.updated_at,0)
            FROM files f LEFT JOIN unknown_checks u ON f.file_id=u.file_id
            WHERE f.item_kind='file' AND f.local_hash IS NULL AND f.fetched_min_version<0
              AND f.status IN ('local','cloud_only') AND ({predicate}) ORDER BY {order},f.file_id LIMIT ?2"
        ))?;
        stmt.query_map(params![now, limit as i64], |r| {
            Ok(UnknownRow {
                file_id: r.get(0)?,
                path: r.get(1)?,
                current_version: r.get(2)?,
                record: UnknownCheck {
                    state: r.get(3)?,
                    attempts: r.get(4)?,
                    stops: r.get(5)?,
                    next_attempt_at: r.get(6)?,
                    last_error: r.get(7)?,
                    copied_hash: r.get(8)?,
                    recorded_hash: r.get(9)?,
                    recorded_size: r.get(10)?,
                    recorded_mtime: r.get(11)?,
                    resume_candidate: r.get(12)?,
                    resume_chunk: r.get(13)?,
                    resume_digest: r.get(14)?,
                    updated_at: r.get(15)?,
                },
            })
        })?
        .collect()
    }
    pub fn list_unknown_to_record(&self, limit: usize) -> Result<Vec<UnknownRow>> {
        self.unknown_rows(
            "u.recorded_hash IS NULL AND u.state IS NULL AND ?1=0",
            0,
            limit,
            "COALESCE(u.updated_at,0)",
        )
    }
    pub fn list_unknown_to_check(&self, now: i64, limit: usize) -> Result<Vec<UnknownRow>> {
        self.unknown_rows(
            "u.recorded_hash IS NOT NULL AND (u.state IS NULL OR (u.state='waiting_retry' AND u.next_attempt_at<=?1))",
            now,
            limit,
            "u.recorded_mtime DESC",
        )
    }
    pub fn list_unknown_local_waiting(&self, limit: usize) -> Result<Vec<UnknownRow>> {
        self.unknown_rows("u.state='waiting' AND ?1=0", 0, limit, "u.updated_at")
    }
    pub fn touch_unknown_waiting(&self, id: &str, now: i64) -> Result<()> {
        self.0.lock().expect("state_db mutex poisoned").execute(
            "UPDATE unknown_checks SET updated_at=?2 WHERE file_id=?1 AND state IN ('waiting','diverged')",
            params![id, now],
        )?;
        Ok(())
    }
    pub fn list_unknown_waiting(&self, limit: usize) -> Result<Vec<UnknownRow>> {
        self.unknown_rows("u.state IN ('waiting','diverged') AND ?1=0", 0, limit, "u.updated_at")
    }
    /// The entry-driven retry also needs a guard after a web trash/move. It is kept until
    /// the watcher consumes the event, even after the conditional reset.
    pub fn set_unknown_removing(&self, id: &str, removing: bool) -> Result<()> {
        self.0.lock().expect("state_db mutex poisoned").execute(
            "INSERT INTO unknown_checks(file_id,state) VALUES (?1,?2)
            ON CONFLICT(file_id) DO UPDATE SET state=excluded.state",
            params![id, if removing { "removing" } else { "diverged" }],
        )?;
        Ok(())
    }
    pub fn take_removing(&self, id: &str) -> Result<bool> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        Ok(conn.execute(
            "UPDATE unknown_checks SET state=NULL WHERE file_id=?1 AND state='removing'",
            [id],
        )? == 1)
    }
    pub fn clear_removing_states(&self) -> Result<usize> {
        self.0
            .lock()
            .expect("state_db mutex poisoned")
            .execute("UPDATE unknown_checks SET state=NULL WHERE state='removing'", [])
    }
    pub fn rearm_could_not_check(&self) -> Result<usize> {
        self.0.lock().expect("state_db mutex poisoned").execute(
            "UPDATE unknown_checks SET state=NULL,attempts=0,stops=0,next_attempt_at=0 WHERE state='could_not_check'",
            [],
        )
    }
    pub fn unknown_unchecked_count(&self) -> Result<usize> {
        self.0.lock().expect("state_db mutex poisoned").query_row("SELECT COUNT(*) FROM unknown_checks u JOIN files f ON f.file_id=u.file_id
            WHERE u.state='could_not_check' AND f.local_hash IS NULL AND f.fetched_min_version<0 AND f.status IN ('local','cloud_only')", [], |r| r.get(0))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn db() -> (tempfile::TempDir, StateDb) {
        let dir = tempfile::tempdir().unwrap();
        let db = StateDb::open(dir.path().join("state.db")).unwrap();
        db.upsert_file(&FileEntry {
            file_id: "u".into(),
            path: "a.txt".into(),
            status: FileStatus::Local,
            size_bytes: 3,
            modified_at: 0,
            content_hash: None,
            remote_updated_at: 0,
            parent_id: None,
            item_kind: ItemKind::File,
        })
        .unwrap();
        (dir, db)
    }
    #[test]
    fn task_2191_s2_recording_is_once_and_check_list_is_due_newest_first() {
        let (_dir, db) = db();
        assert_eq!(db.list_unknown_to_record(4).unwrap().len(), 1);
        let r = UnknownCheck {
            recorded_hash: Some("h".into()),
            recorded_size: Some(3),
            recorded_mtime: Some(9),
            ..Default::default()
        };
        assert!(db.put_unknown_check("u", "a.txt", &r).unwrap());
        assert!(db.list_unknown_to_record(4).unwrap().is_empty());
        assert_eq!(db.list_unknown_to_check(0, 4).unwrap().len(), 1);
        for state in ["waiting", "diverged", "removing", "could_not_check", "waiting_retry"] {
            let r = UnknownCheck {
                state: Some(state.into()),
                next_attempt_at: 100,
                ..r.clone()
            };
            assert!(db.put_unknown_check("u", "a.txt", &r).unwrap());
            assert!(db.list_unknown_to_check(99, 4).unwrap().is_empty(), "{state}");
        }
        assert_eq!(db.list_unknown_to_check(100, 4).unwrap().len(), 1);
        assert!(!db.put_unknown_check("u", "moved", &r).unwrap());
    }
    #[test]
    fn task_2191_s2_removing_survives_until_event_or_startup() {
        let (_dir, db) = db();
        let r = UnknownCheck {
            state: Some("removing".into()),
            ..Default::default()
        };
        db.put_unknown_check("u", "a.txt", &r).unwrap();
        assert!(db.take_removing("u").unwrap());
        assert!(!db.take_removing("u").unwrap());
        db.put_unknown_check("u", "a.txt", &r).unwrap();
        assert_eq!(db.clear_removing_states().unwrap(), 1);
        assert!(!db.take_removing("u").unwrap());
    }
    #[test]
    fn task_2191_s2_counted_failure_rearms_only_at_start() {
        let (_dir, db) = db();
        let r = UnknownCheck {
            state: Some("could_not_check".into()),
            attempts: 8,
            recorded_hash: Some("h".into()),
            ..Default::default()
        };
        db.put_unknown_check("u", "a.txt", &r).unwrap();
        assert_eq!(db.unknown_unchecked_count().unwrap(), 1);
        assert_eq!(db.rearm_could_not_check().unwrap(), 1);
        assert_eq!(db.unknown_unchecked_count().unwrap(), 0);
        assert_eq!(db.list_unknown_to_check(0, 4).unwrap()[0].record.attempts, 0);
    }

    #[test]
    fn task_2191_s2_baseline_is_positive_unknown_and_path_conditional() {
        let (_dir, db) = db();
        assert!(!db.record_baseline_if_unknown("u", "elsewhere", 2, "h").unwrap());
        assert!(!db.record_baseline_if_unknown("u", "a.txt", 0, "h").unwrap());
        assert!(db.record_baseline_if_unknown("u", "a.txt", 2, "h").unwrap());
        assert!(!db.record_baseline_if_unknown("u", "a.txt", 3, "changed").unwrap());
        let conn = db.0.lock().unwrap();
        let result: (i64, String) = conn
            .query_row(
                "SELECT local_base_version,local_hash FROM files WHERE file_id='u'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(result, (2, "h".into()));
    }
    #[test]
    fn task_2191_s2_reset_is_path_and_population_conditional() {
        let (_dir, db) = db();
        assert!(!db.reset_after_divergence("u", "elsewhere").unwrap());
        db.0.lock()
            .unwrap()
            .execute("UPDATE files SET status='trashing'", [])
            .unwrap();
        assert!(!db.reset_after_divergence("u", "a.txt").unwrap());
        db.0.lock()
            .unwrap()
            .execute("UPDATE files SET status='local',fetched_min_version=-2", [])
            .unwrap();
        assert!(db.reset_after_divergence("u", "a.txt").unwrap());
        assert_eq!(db.get_file("u").unwrap().unwrap().status, FileStatus::CloudOnly);
        assert!(db.mark_no_fetched_content("u").unwrap());
        assert!(!db.record_baseline_if_unknown("u", "a.txt", 1, "h").unwrap());
        assert!(!db.reset_after_divergence("u", "a.txt").unwrap());
    }
    #[test]
    fn task_2191_s2_ledger_is_durable_and_account_clear_removes_it() {
        use crate::unknown_check::budget::{METERED_BYTES, Stop};
        let (dir, db) = db();
        db.update_unknown_budget(|l| l.reserve(0, 1, METERED_BYTES, METERED_BYTES, false))
            .unwrap()
            .unwrap();
        drop(db);
        let db = StateDb::open(dir.path().join("state.db")).unwrap();
        assert_eq!(
            db.update_unknown_budget(|l| l.reserve(100, 1, 1, METERED_BYTES, false))
                .unwrap(),
            Err(Stop::Daily)
        );
        db.update_unknown_budget(|l| l.rate_limited(100, 900, 1)).unwrap();
        drop(db);
        let db = StateDb::open(dir.path().join("state.db")).unwrap();
        assert_eq!(
            db.update_unknown_budget(|l| l.reserve(500, 1, 0, METERED_BYTES, false))
                .unwrap(),
            Err(Stop::Until(900100))
        );
        db.clear_account_data(false).unwrap();
        assert_eq!(
            db.update_unknown_budget(|l| l.reserve(500, 1, 0, METERED_BYTES, false))
                .unwrap(),
            Ok(())
        );
    }
}
