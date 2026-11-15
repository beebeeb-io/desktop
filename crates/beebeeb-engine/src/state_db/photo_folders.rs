//! Photo sources' "Folders from Files" (DW-0604): one `sync_state` row per folder an account added,
//! `photo_folder:{user id}:{file id}`, its value `1` (Photos shows it) or `0` (switched off: its photos, with its
//! subfolders', are left out of the library).
//!
//! The rows are keyed by the ACCOUNT's own user id, as the guide mark is (`guide_mark`): the native sign-out keeps
//! this PC's local data, so a row must never be readable under another account. Every method takes the user id and
//! reads or writes only that account's rows; who may ask for which user id is the engine handle's
//! (`app::bindings::photos_folders`, through the account-bound state): only the session's own. A sign-out that
//! clears the account's rows (`signout::sign_out_sync_state_purge`) removes these with them: they have no keep
//! pattern, on purpose.
//!
//! The key holds the user id and the folder's file id, nothing else: never an email, a name or a path. A user id is
//! the server's id (no `:` in it), so the prefix `photo_folder:{user id}:` names one account's rows and no other's;
//! the prefix is matched as a plain string, never as a `LIKE` pattern (a `_` or `%` in it would be a wildcard).

use super::*;

/// The start of every row of the account `user_id`, the separator after the id included.
fn prefix(user_id: &str) -> String {
    format!("photo_folder:{user_id}:")
}

fn key(user_id: &str, file_id: &str) -> String {
    format!("{}{file_id}", prefix(user_id))
}

impl StateDb {
    /// The folders the account `user_id` added to Photo sources, in the order they were added, each with whether
    /// Photos shows it. Never another account's rows.
    pub fn photo_folders(&self, user_id: &str) -> Result<Vec<(String, bool)>> {
        let start = prefix(user_id);
        let conn = self.0.lock().expect("state_db mutex poisoned");
        let mut stmt =
            conn.prepare("SELECT key, value FROM sync_state WHERE substr(key, 1, length(?1)) = ?1 ORDER BY rowid")?;
        let rows = stmt.query_map(params![start], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?;
        let mut out = Vec::new();
        for row in rows {
            let (key, value) = row?;
            let Some(file_id) = key.strip_prefix(start.as_str()) else {
                continue;
            };
            if file_id.is_empty() {
                continue;
            }
            out.push((file_id.to_string(), value != "0"));
        }
        Ok(out)
    }

    /// The account `user_id` adds the folder `file_id`, shown. `false`: it was added already (nothing changed).
    pub fn add_photo_folder(&self, user_id: &str, file_id: &str) -> Result<bool> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        let added = conn.execute(
            "INSERT INTO sync_state (key, value) VALUES (?1, '1') ON CONFLICT(key) DO NOTHING",
            params![key(user_id, file_id)],
        )?;
        Ok(added > 0)
    }

    /// Whether Photos shows the folder `file_id` of the account `user_id`. `false`: that account did not add it
    /// (nothing changed).
    pub fn set_photo_folder_shown(&self, user_id: &str, file_id: &str, shown: bool) -> Result<bool> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        let changed = conn.execute(
            "UPDATE sync_state SET value = ?2 WHERE key = ?1",
            params![key(user_id, file_id), if shown { "1" } else { "0" }],
        )?;
        Ok(changed > 0)
    }

    /// The account `user_id` takes the folder `file_id` out of Photo sources: its photos show again. Nothing is
    /// moved or deleted. `false`: that account had not added it.
    pub fn remove_photo_folder(&self, user_id: &str, file_id: &str) -> Result<bool> {
        let conn = self.0.lock().expect("state_db mutex poisoned");
        let removed = conn.execute("DELETE FROM sync_state WHERE key = ?1", params![key(user_id, file_id)])?;
        Ok(removed > 0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FOLDER: &str = "d0000000-0000-4000-8000-000000000101";
    const OTHER: &str = "d0000000-0000-4000-8000-000000000102";

    fn db() -> (tempfile::TempDir, StateDb) {
        let dir = tempfile::tempdir().unwrap();
        let db = StateDb::open(dir.path().join("state.db")).unwrap();
        (dir, db)
    }

    /// Every photo folder row of the database, by key, whoever's.
    fn all_rows(db: &StateDb) -> Vec<String> {
        let conn = db.0.lock().unwrap();
        let mut stmt = conn
            .prepare("SELECT key FROM sync_state WHERE substr(key, 1, 13) = 'photo_folder:' ORDER BY key")
            .unwrap();
        stmt.query_map([], |row| row.get(0))
            .unwrap()
            .map(|row| row.unwrap())
            .collect()
    }

    #[test]
    fn photo_folders_add_twice_is_one_row() {
        let (_dir, db) = db();
        assert!(db.add_photo_folder("user-a", FOLDER).unwrap(), "the first add adds");
        assert!(
            !db.add_photo_folder("user-a", FOLDER).unwrap(),
            "the second add says it is there already"
        );
        assert_eq!(db.photo_folders("user-a").unwrap(), [(FOLDER.to_string(), true)]);
        assert_eq!(all_rows(&db), [format!("photo_folder:user-a:{FOLDER}")]);

        // A second add leaves a folder that is switched off switched off.
        assert!(db.set_photo_folder_shown("user-a", FOLDER, false).unwrap());
        assert!(!db.add_photo_folder("user-a", FOLDER).unwrap());
        assert_eq!(db.photo_folders("user-a").unwrap(), [(FOLDER.to_string(), false)]);

        // In the order they were added.
        assert!(db.add_photo_folder("user-a", OTHER).unwrap());
        let ids: Vec<String> = db
            .photo_folders("user-a")
            .unwrap()
            .into_iter()
            .map(|(id, _)| id)
            .collect();
        assert_eq!(ids, [FOLDER, OTHER]);

        assert!(db.remove_photo_folder("user-a", FOLDER).unwrap());
        assert!(!db.remove_photo_folder("user-a", FOLDER).unwrap(), "removed already");
        assert!(
            !db.set_photo_folder_shown("user-a", FOLDER, true).unwrap(),
            "a removed folder is not switched"
        );
        assert_eq!(db.photo_folders("user-a").unwrap(), [(OTHER.to_string(), true)]);
    }

    #[test]
    fn photo_folders_of_one_account_are_never_read_for_another() {
        let (_dir, db) = db();
        assert!(db.add_photo_folder("user-a", FOLDER).unwrap());
        assert!(db.set_photo_folder_shown("user-a", FOLDER, false).unwrap());

        // The second account reads none of the first's rows, and cannot switch or remove them.
        assert!(db.photo_folders("user-b").unwrap().is_empty());
        assert!(!db.set_photo_folder_shown("user-b", FOLDER, true).unwrap());
        assert!(!db.remove_photo_folder("user-b", FOLDER).unwrap());
        // A user id that begins with the other's is another account too.
        assert!(db.photo_folders("user-").unwrap().is_empty());
        assert!(db.add_photo_folder("user-ab", OTHER).unwrap());
        assert_eq!(db.photo_folders("user-a").unwrap(), [(FOLDER.to_string(), false)]);
        assert_eq!(db.photo_folders("user-ab").unwrap(), [(OTHER.to_string(), true)]);

        // The second account adding the same folder is a row of its own; the first's stays as it was.
        assert!(db.add_photo_folder("user-b", FOLDER).unwrap());
        assert_eq!(db.photo_folders("user-b").unwrap(), [(FOLDER.to_string(), true)]);
        assert_eq!(db.photo_folders("user-a").unwrap(), [(FOLDER.to_string(), false)]);
        assert_eq!(
            all_rows(&db),
            [
                format!("photo_folder:user-a:{FOLDER}"),
                format!("photo_folder:user-ab:{OTHER}"),
                format!("photo_folder:user-b:{FOLDER}"),
            ]
        );
    }
}
