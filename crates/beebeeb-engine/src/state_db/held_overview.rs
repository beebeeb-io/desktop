//! Task 2524: what the engine holds back from sending, as This PC lists it: the held edits whose base is not known one
//! row each (capped), and the files whose agreed content is not recorded (`content_unknown`) as ONE count with the
//! first few names. That reason is the hydrated-before-0.8.13 population (task 2191): one PC held 28,224 files, mostly
//! untouched, so a row each would bury the few real problems (the review feed collapses them the same way).

use super::*;

/// How many `content_unknown` names the summary carries.
pub const HELD_UNCHECKED_NAMES: u32 = 5;

/// `(file_id, path, reason, detected_at)`, as [`StateDb::list_held_edits`] reads it.
pub type HeldRow = (String, String, String, i64);

/// The held edits as This PC lists them ([`StateDb::held_edit_overview`]).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct HeldEdits {
    /// Every held edit but a `content_unknown` one, oldest first, at most the read's limit.
    pub each: Vec<HeldRow>,
    /// How many `content_unknown` files are held (all of them, whatever the limit).
    pub unchecked: i64,
    /// The first [`HELD_UNCHECKED_NAMES`] of them, oldest first: `(file_id, path, detected_at)`.
    pub unchecked_first: Vec<(String, String, i64)>,
}

impl HeldEdits {
    /// When the scan first found the held edit a problem row stands for (`held:<file id>`, or the summary's
    /// `held:content_unknown`: the oldest `content_unknown` one); 0 when not known.
    pub fn detected_at(&self, problem_id: &str) -> i64 {
        match problem_id.strip_prefix("held:") {
            Some("content_unknown") => self.unchecked_first.first().map_or(0, |f| f.2),
            Some(file_id) => self.each.iter().find(|h| h.0 == file_id).map_or(0, |h| h.3),
            None => 0,
        }
    }
}

impl StateDb {
    /// Task 2524: the held edits whose file still has a row, split as [`HeldEdits`] says; `limit` caps `each`.
    /// One read transaction, so the count and the names are the same moment's.
    pub fn held_edit_overview(&self, limit: u32) -> Result<HeldEdits> {
        let mut conn = self.0.lock().expect("state_db mutex poisoned");
        let tx = conn.transaction()?;
        let each = {
            let mut stmt = tx.prepare(
                "SELECT held.file_id, file.path, held.reason, held.detected_at
                 FROM held_edits AS held JOIN files AS file ON file.file_id = held.file_id
                 WHERE held.reason != 'content_unknown'
                 ORDER BY held.detected_at, held.file_id LIMIT ?1",
            )?;
            stmt.query_map(params![i64::from(limit)], |row| {
                Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
            })?
            .collect::<Result<Vec<_>>>()?
        };
        let unchecked: i64 = tx.query_row(
            "SELECT COUNT(*) FROM held_edits AS held JOIN files AS file ON file.file_id = held.file_id
             WHERE held.reason = 'content_unknown'",
            [],
            |row| row.get(0),
        )?;
        let unchecked_first = {
            let mut stmt = tx.prepare(
                "SELECT held.file_id, file.path, held.detected_at
                 FROM held_edits AS held JOIN files AS file ON file.file_id = held.file_id
                 WHERE held.reason = 'content_unknown'
                 ORDER BY held.detected_at, held.file_id LIMIT ?1",
            )?;
            stmt.query_map(params![i64::from(HELD_UNCHECKED_NAMES)], |row| {
                Ok((row.get(0)?, row.get(1)?, row.get(2)?))
            })?
            .collect::<Result<Vec<_>>>()?
        };
        tx.commit()?;
        Ok(HeldEdits {
            each,
            unchecked,
            unchecked_first,
        })
    }
}
