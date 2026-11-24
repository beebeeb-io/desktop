//! Why a state database could not be opened for reading only ([`StateDb::open_read_only`]): one reason per case a
//! viewer tells apart. A reason is a fixed word; SQLite's own text (which can hold a path) is never passed on.

/// Why [`super::StateDb::open_read_only`] refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReadOnlyOpenError {
    /// No file at the path.
    NotFound,
    /// The file is not an SQLite database.
    NotADatabase,
    /// An SQLite database with no `files` table: some other application's.
    NotAStateDatabase,
    /// A state database from a build too old to list: a column the listing reads is missing. It is not migrated
    /// here (that would be a write); the app that owns it migrates it when it next starts.
    OlderSchema,
    /// Another process holds the database and did not let go in time.
    Locked,
    /// Anything else (a permission, an I/O error).
    Unreadable,
}

impl ReadOnlyOpenError {
    /// The fixed word for a log line or a command-line message.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::NotFound => "not found",
            Self::NotADatabase => "not a database",
            Self::NotAStateDatabase => "not a Beebeeb state database",
            Self::OlderSchema => "from an older version of Beebeeb (open the app once to update it)",
            Self::Locked => "in use by another program",
            Self::Unreadable => "cannot be read",
        }
    }

    pub(super) fn classify(error: &rusqlite::Error) -> Self {
        use rusqlite::ErrorCode;
        match error {
            rusqlite::Error::SqliteFailure(failure, message) => match failure.code {
                ErrorCode::NotADatabase => Self::NotADatabase,
                ErrorCode::DatabaseBusy | ErrorCode::DatabaseLocked => Self::Locked,
                ErrorCode::CannotOpen => Self::NotFound,
                // "no such column: owner_email", "no such table: ...": the statement names what this file lacks.
                _ if message.as_deref().is_some_and(|m| m.starts_with("no such ")) => Self::OlderSchema,
                _ => Self::Unreadable,
            },
            // A statement that does not compile against this file's schema (SQLite reports it with the SQL).
            rusqlite::Error::SqlInputError { msg, .. } if msg.starts_with("no such ") => Self::OlderSchema,
            _ => Self::Unreadable,
        }
    }
}

impl std::fmt::Display for ReadOnlyOpenError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl std::error::Error for ReadOnlyOpenError {}

#[cfg(test)]
mod read_only_tests {
    use super::super::*;
    use super::ReadOnlyOpenError;

    fn entry(id: &str, path: &str) -> FileEntry {
        FileEntry {
            file_id: id.into(),
            path: path.into(),
            status: FileStatus::CloudOnly,
            size_bytes: 1,
            modified_at: 1,
            content_hash: None,
            remote_updated_at: 1,
            parent_id: None,
            item_kind: ItemKind::File,
        }
    }

    /// A state database with two rows, closed (so its WAL is folded back into the file).
    fn seeded(dir: &std::path::Path) -> std::path::PathBuf {
        let path = dir.join("state.db");
        let db = StateDb::open(&path).unwrap();
        db.upsert_file(&entry("a", "/a.txt")).unwrap();
        db.upsert_file(&entry("b", "/b.txt")).unwrap();
        path
    }

    /// Every kind of write the handle offers, one per table family. Each must fail, and none may change the file.
    fn try_each_writer(db: &StateDb) -> Vec<(&'static str, bool)> {
        vec![
            ("upsert_file", db.upsert_file(&entry("c", "/c.txt")).is_ok()),
            (
                "upsert_file_with_contract",
                db.upsert_file_with_contract(&entry("d", "/d.txt"), |_| {}).is_ok(),
            ),
            ("delete_file", db.delete_file("a").is_ok()),
            (
                "set_owner",
                db.set_owner(&crate::account_binding::Identity::new(Some("u"), None))
                    .is_ok(),
            ),
            ("set_mark_pending", db.set_mark_pending("a", true).is_ok()),
            ("test_execute (raw SQL)", db.test_execute("DELETE FROM files").is_ok()),
            (
                "test_execute (schema)",
                db.test_execute("CREATE TABLE planted(x)").is_ok(),
            ),
        ]
    }

    #[test]
    fn a_read_only_handle_refuses_every_writer_and_leaves_the_file_as_it_was() {
        let dir = tempfile::tempdir().unwrap();
        let path = seeded(dir.path());
        let before = std::fs::read(&path).unwrap();
        let db = StateDb::open_read_only(&path).unwrap();
        let wrote: Vec<&str> = try_each_writer(&db)
            .into_iter()
            .filter(|(_, ok)| *ok)
            .map(|(name, _)| name)
            .collect();
        assert!(
            wrote.is_empty(),
            "a read-only handle let these writers through: {wrote:?}"
        );
        assert_eq!(db.children_paged(None, 0, 10).unwrap().1, 2, "and it still reads");
        drop(db);
        assert_eq!(
            std::fs::read(&path).unwrap(),
            before,
            "the database file is byte-identical"
        );
    }

    /// The control for the test above: the same writers on a handle from `open` all succeed and the file changes,
    /// so that test fails if `open_read_only` is ever made to open like `open`.
    #[test]
    fn the_same_writers_succeed_on_a_writable_handle() {
        let dir = tempfile::tempdir().unwrap();
        let path = seeded(dir.path());
        let before = std::fs::read(&path).unwrap();
        let db = StateDb::open(&path).unwrap();
        let refused: Vec<&str> = try_each_writer(&db)
            .into_iter()
            .filter(|(_, ok)| !*ok)
            .map(|(name, _)| name)
            .collect();
        assert!(refused.is_empty(), "the control cannot write: {refused:?}");
        drop(db);
        assert_ne!(
            std::fs::read(&path).unwrap(),
            before,
            "a writable handle changes the file"
        );
    }

    #[test]
    fn each_refusal_has_its_own_reason_and_nothing_is_created_or_migrated() {
        let dir = tempfile::tempdir().unwrap();
        let open = |path: &std::path::Path| StateDb::open_read_only(path).err();

        let missing = dir.path().join("missing.db");
        assert_eq!(open(&missing), Some(ReadOnlyOpenError::NotFound));
        assert!(!missing.exists(), "a missing file is not created");

        let text = dir.path().join("notes.txt");
        std::fs::write(
            &text,
            b"this is a text file, long enough to be read as a database header",
        )
        .unwrap();
        assert_eq!(open(&text), Some(ReadOnlyOpenError::NotADatabase));

        let other = dir.path().join("other.db");
        Connection::open(&other)
            .unwrap()
            .execute_batch("CREATE TABLE t(x);")
            .unwrap();
        let other_before = std::fs::read(&other).unwrap();
        assert_eq!(open(&other), Some(ReadOnlyOpenError::NotAStateDatabase));
        assert_eq!(
            std::fs::read(&other).unwrap(),
            other_before,
            "another application's database is not touched"
        );

        // A state database from before a column the listing reads: refused, and not migrated.
        let old = seeded(dir.path());
        {
            let conn = Connection::open(&old).unwrap();
            StateDb::drop_column_for_test(&conn, "files", "owner_email").unwrap();
        }
        assert_eq!(open(&old), Some(ReadOnlyOpenError::OlderSchema));
        let columns: i64 = Connection::open(&old)
            .unwrap()
            .query_row(
                "SELECT COUNT(*) FROM pragma_table_info('files') WHERE name = 'owner_email'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(columns, 0, "the missing column was not added back");

        // Held by another connection in exclusive mode: locked, after the short wait (not an empty listing).
        let held_dir = tempfile::tempdir().unwrap();
        let held = seeded(held_dir.path());
        let holder = Connection::open(&held).unwrap();
        holder
            .execute_batch("PRAGMA locking_mode = EXCLUSIVE; BEGIN EXCLUSIVE; DELETE FROM files WHERE file_id = 'a';")
            .unwrap();
        assert_eq!(open(&held), Some(ReadOnlyOpenError::Locked));
        holder.execute_batch("ROLLBACK;").unwrap();
        drop(holder);
        assert!(open(&held).is_none(), "and it opens once the holder lets go");
    }
}
