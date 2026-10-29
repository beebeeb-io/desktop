//! Task 2274, spec phase 4: the echo check. After `apply-one`, the person starts the app, and the app applies B's
//! `file_trash` op: the echo of the tool's trash. `check-echo` reads the app's op cursor from the state DB, read-only,
//! until it reaches that op's seq (a cursor already past it counts, Review Focus 3), then checks the path (I4): the
//! file there has the identity, on-disk state, in-sync state, size, modified time and read-only attribute the list
//! saw, and A's row is the path's only row. The app keeps running while the tool reads; the tool writes nothing.
use super::list::{self, Observed};
use super::model::{Outcome, RunFile, ViewFacts};
use rusqlite::{Connection, OpenFlags, OptionalExtension};
use std::path::Path;
use std::time::{Duration, Instant};

/// What the check found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum EchoResult {
    /// The echo was applied and the path is as the list saw it.
    Unchanged,
    /// The echo was applied and these facts of the path differ (`kind`, `on-disk`, `in-sync`, `identity`, `size`,
    /// `modified`, `read-only`, `rows`). The run stops.
    Changed(Vec<&'static str>),
    /// The app's cursor did not reach B's op within the wait. The run stops.
    CursorNeverReached,
}

/// The `sync_state` key the app keeps its op cursor under (`StateDb::SYNC_CURSOR_KEY`; the tests set the cursor
/// through `StateDb` and read it here).
const CURSOR_KEY: &str = "sync_ops_cursor";

/// Check pair `n`'s echo, polling the cursor every `poll` for at most `wait` (spec: every 5 s for up to 3 minutes).
pub(crate) fn check_echo(
    db_path: &Path,
    root: &Path,
    run: &RunFile,
    n: u32,
    wait: Duration,
    poll: Duration,
    observe: &dyn Fn(&Path) -> Observed,
) -> anyhow::Result<EchoResult> {
    let pair = usize::try_from(n)
        .ok()
        .and_then(|n| n.checked_sub(1))
        .and_then(|index| run.pairs.get(index))
        .ok_or_else(|| anyhow::anyhow!("pair {n} is not in the run file; nothing was read."))?;
    // A forgotten pair's B was already in the trash or gone: the tool sent no trash, so there is no echo to wait for.
    let seq = match pair.applied.as_ref().map(|applied| &applied.outcome) {
        Some(Outcome::Trashed { seq, .. }) => Some(*seq),
        Some(Outcome::Forgotten) => None,
        _ => anyhow::bail!("pair {n} has no confirmed trash or forget to check; nothing was read."),
    };
    let db = Connection::open_with_flags(db_path, OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX)
        .map_err(|_| anyhow::anyhow!("The app's state DB could not be opened read-only."))?;
    if let Some(seq) = seq {
        let started = Instant::now();
        while cursor(&db)?.is_none_or(|cursor| cursor < seq) {
            let left = wait.saturating_sub(started.elapsed());
            if left.is_zero() {
                return Ok(EchoResult::CursorNeverReached);
            }
            std::thread::sleep(poll.min(left));
        }
    }
    let (observed, identity) = list::observe_at(root, &pair.path, observe);
    let mut changed = differences(&list::view_facts(&observed, identity), &pair.view);
    if rows_at(&db, &pair.path)? != [pair.a.file_id.as_str()] {
        changed.push("rows");
    }
    Ok(if changed.is_empty() { EchoResult::Unchanged } else { EchoResult::Changed(changed) })
}

/// The app's op cursor, as `StateDb::get_sync_cursor` reads it.
fn cursor(db: &Connection) -> anyhow::Result<Option<i64>> {
    let raw: Option<String> = db
        .query_row("SELECT value FROM sync_state WHERE key = ?1", [CURSOR_KEY], |row| row.get(0))
        .optional()
        .map_err(|_| anyhow::anyhow!("The app's op cursor could not be read."))?;
    Ok(raw.and_then(|raw| raw.parse().ok()))
}

/// The ids of the rows at `path`, as `StateDb::rows_at_folded_path` matches it.
fn rows_at(db: &Connection, path: &str) -> anyhow::Result<Vec<String>> {
    let failed = |_| anyhow::anyhow!("The app's rows at the path could not be read.");
    let mut statement = db
        .prepare("SELECT file_id FROM files WHERE replace(path, '\\', '/') = ?1 ORDER BY file_id")
        .map_err(failed)?;
    let ids = statement.query_map([path], |row| row.get(0)).map_err(failed)?;
    ids.collect::<Result<_, _>>().map_err(failed)
}

/// The names of the facts that differ, in a fixed order.
fn differences(now: &ViewFacts, listed: &ViewFacts) -> Vec<&'static str> {
    let facts = [
        ("kind", now.kind != listed.kind),
        ("on-disk", now.on_disk != listed.on_disk),
        ("in-sync", now.in_sync != listed.in_sync),
        ("identity", now.identity_names != listed.identity_names),
        ("size", now.size != listed.size),
        ("modified", now.modified != listed.modified),
        ("read-only", now.read_only != listed.read_only),
    ];
    facts.into_iter().filter(|(_, differs)| *differs).map(|(name, _)| name).collect()
}
