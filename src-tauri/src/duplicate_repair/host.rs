//! Task 2274, plan Task 7: one run of the tool, one phase (spec section 6), on this PC's files. `run` in `mod.rs` fills
//! [`Env`] from the app's own settings, state dir and stored session on Windows; tests fill it with a scratch PC and
//! the loopback server.
//!
//! What the person reads goes to the out dir on this PC: the run file (`run.json`, where the actions are marked) and
//! the list (`list.txt`, written again after every phase), with names, paths and ids. The screen gets counts, classes
//! and pair numbers only (spec A3, I5).
//!
//! Every phase but `check-echo` holds the app's sync lock, so the app cannot start meanwhile (T15). `check-echo` runs
//! while the app runs and only reads.
use super::Command;
use super::apply::{Local, apply_one};
use super::compare::compare;
use super::echo::{EchoResult, check_echo};
use super::list::{Observed, list_pairs};
use super::model::{Class, Outcome, Pair, Reason, RunFile, kebab};
use super::server::{RepairApi, server_read};
use crate::state_db::StateDb;
use rusqlite::{Connection, OpenFlags};
use std::fmt::Write as _;
use std::path::Path;
use std::time::Duration;

pub(crate) const RUN_FILE: &str = "run.json";
pub(crate) const LIST_FILE: &str = "list.txt";
/// The L8 amendment's words, in the list's header.
pub(crate) const FOLDER_SHARES: &str = "Shares of the folders that hold these files were not checked.";
/// OP5: restoring B from the trash brings its row back (T3), and with it the two rows at one path this tool removes.
pub(crate) const DO_NOT_RESTORE: &str = "Do not restore these from the trash; ask first.";
const SCHEMA_CHANGED: &str =
    "This build of the tool does not match the app's state database (opening a copy changed its schema). Nothing was \
     changed; ask for a tool built from the app's own version.";

/// This PC, as one run of the tool sees it.
pub(crate) struct Env<'a> {
    /// The app's state DB.
    pub db_path: &'a Path,
    pub root: &'a Path,
    /// Where the run file, the list, the backups and the scratch copy go.
    pub out_dir: &'a Path,
    pub observe: &'a dyn Fn(&Path) -> Observed,
    /// Claims the app's sync lock for the whole phase; refused while the app runs (T15).
    pub lock: &'a dyn Fn() -> anyhow::Result<Box<dyn std::any::Any>>,
    /// The server client on the app's stored session (OP2), read in-process when a phase first needs the server.
    pub api: &'a dyn Fn() -> anyhow::Result<RepairApi>,
    /// `check-echo`'s wait for the app's cursor, and how often it reads it (spec: 3 minutes, every 5 s).
    pub echo_wait: Duration,
    pub echo_poll: Duration,
}

/// Run one phase. Returns the exit code and what goes on the screen.
pub(crate) async fn execute(command: Command, env: &Env<'_>) -> anyhow::Result<(i32, String)> {
    let run_path = env.out_dir.join(RUN_FILE);
    match command {
        Command::List => {
            let _lock = (env.lock)()?;
            if run_path.exists() {
                anyhow::bail!("A run file is in the out dir already. Move it aside to list again; nothing was changed.");
            }
            let scratch = guarded_copy(env)?;
            let listing = list_pairs(&scratch, env.root, env.observe)?;
            let run = RunFile { pairs: listing.pairs, more_than_two: listing.more_than_two, retention_days: None };
            save(env, &run)?;
            let screen = format!(
                "Listed {} pairs; {} paths with more than two rows are counted, not listed.\n{}\nThe list is in the out dir.",
                run.pairs.len(),
                run.more_than_two,
                classes(&run.pairs)
            );
            Ok((0, screen))
        }
        Command::Retention { days } => {
            let mut run = RunFile::load(&run_path)?;
            run.retention_days = Some(days);
            save(env, &run)?;
            Ok((0, format!("Trash retention recorded: {days} days. The list shows the purge date of each trashed B.")))
        }
        Command::ServerRead => {
            let _lock = (env.lock)()?;
            let mut run = RunFile::load(&run_path)?;
            server_read(&(env.api)()?, &mut run.pairs).await?;
            save(env, &run)?;
            Ok((0, format!("Read the server for the pairs the local rows left open.\n{}", classes(&run.pairs))))
        }
        Command::Compare { download_ok } => {
            let _lock = (env.lock)()?;
            let mut run = RunFile::load(&run_path)?;
            let counts = compare(&(env.api)()?, &mut run.pairs, download_ok).await?;
            save(env, &run)?;
            Ok((
                0,
                format!(
                    "Compared: {} identical, {} differ, {} changed on the server since the read (not downloaded), {} could \
                     not be read or opened (left waiting).\n{}",
                    counts.identical,
                    counts.differs,
                    counts.changed,
                    counts.unreadable,
                    classes(&run.pairs)
                ),
            ))
        }
        Command::ApplyOne { pair } => {
            let _lock = (env.lock)()?;
            let mut run = RunFile::load(&run_path)?;
            if run.retention_days.is_none() {
                anyhow::bail!(
                    "Read the trash retention in the admin panel and record it with `retention <days>` first; nothing \
                     was changed."
                );
            }
            drop(guarded_copy(env)?);
            let db = StateDb::open(env.db_path).map_err(|_| anyhow::anyhow!("The app's state DB could not be opened."))?;
            let local = Local { db: &db, db_path: env.db_path, root: env.root, out_dir: env.out_dir, observe: env.observe };
            let outcome = apply_one(&local, &(env.api)()?, &mut run, pair).await?;
            save(env, &run)?;
            Ok(applied(pair, &outcome))
        }
        Command::CheckEcho { pair } => {
            let run = RunFile::load(&run_path)?;
            let result = check_echo(env.db_path, env.root, &run, pair, env.echo_wait, env.echo_poll, env.observe)?;
            Ok(match result {
                EchoResult::Unchanged => (
                    0,
                    format!("pair {pair}: the app applied the echo; the file at the path is unchanged and A's row is its only row."),
                ),
                EchoResult::Changed(facts) => {
                    (1, format!("pair {pair}: changed after the echo: {}. Stop here; leave the app running.", facts.join(", ")))
                }
                EchoResult::CursorNeverReached => (
                    1,
                    format!("pair {pair}: the app did not apply B's trash within {} s. Stop here.", env.echo_wait.as_secs()),
                ),
            })
        }
    }
}

/// What `apply-one` did, for the screen.
fn applied(pair: u32, outcome: &Outcome) -> (i32, String) {
    match outcome {
        Outcome::Trashed { seq, .. } => (
            0,
            format!(
                "pair {pair}: B's row forgotten on this PC, B moved to Beebeeb's trash (its trash op is {seq}). Start the \
                 app, then run check-echo {pair}."
            ),
        ),
        Outcome::Forgotten => (
            0,
            format!(
                "pair {pair}: B's row forgotten on this PC (B was in the trash or gone already). Start the app, then run \
                 check-echo {pair}."
            ),
        ),
        Outcome::TrashFailed { status } => (
            1,
            format!(
                "pair {pair}: B's row forgotten on this PC; the trash failed ({}). Run apply-one {pair} again with the app \
                 stopped.",
                status.map_or("no answer".to_string(), |status| format!("HTTP {status}"))
            ),
        ),
        Outcome::Unconfirmed { status } => (
            1,
            format!(
                "pair {pair}: B's row forgotten and the trash sent, but the server could not be read back ({}). Run \
                 apply-one {pair} again with the app stopped: it reads back without a second trash.",
                status.map_or("no answer".to_string(), |status| format!("HTTP {status}"))
            ),
        ),
        Outcome::Unexpected { a_live, b_trashed, seq, .. } => (
            1,
            format!(
                "pair {pair}: B's row forgotten and the trash sent, but the server does not confirm it (A live: {a_live}, \
                 B in the trash: {b_trashed}, trash op found: {}). Stop here.",
                seq.is_some()
            ),
        ),
        Outcome::Skipped { reason } => {
            let next = match reason {
                Reason::PendingOps => {
                    " The server holds changes the app has not applied yet: start the app, let it sync, stop it, then run \
                     apply-one again."
                }
                Reason::AppRanSinceTheTry => " The app ran since the try that failed. Stop here.",
                _ => "",
            };
            (1, format!("pair {pair}: skipped ({}); nothing was changed.{next}", kebab(reason)))
        }
    }
}

/// The count of pairs per class, in the decision table's order.
fn classes(pairs: &[Pair]) -> String {
    let order = [
        Class::Unbound,
        Class::BoundToB,
        Class::ABusy,
        Class::BBusy,
        Class::AGone,
        Class::BGone,
        Class::BInTrash,
        Class::Moved,
        Class::YourCall,
        Class::DiffersBySize,
        Class::Unknown,
        Class::Differs,
        Class::Identical,
    ];
    let counts: Vec<String> = order
        .iter()
        .map(|class| (class, pairs.iter().filter(|pair| pair.class == *class).count()))
        .filter(|(_, count)| *count > 0)
        .map(|(class, count)| format!("{} {count}", kebab(class)))
        .collect();
    if counts.is_empty() { "No pairs.".to_string() } else { format!("By class: {}.", counts.join(", ")) }
}

/// A scratch copy of the app's state DB, opened with this build's own code (which migrates it). L1: if that changes
/// the schema, this build is not the app's and the run stops before the app's own DB is opened.
fn guarded_copy(env: &Env<'_>) -> anyhow::Result<StateDb> {
    let dir = env.out_dir.join("scratch");
    std::fs::create_dir_all(&dir)?;
    let copy = dir.join("state.db");
    for name in ["state.db", "state.db-wal", "state.db-shm"] {
        match std::fs::remove_file(dir.join(name)) {
            Err(error) if error.kind() != std::io::ErrorKind::NotFound => return Err(error.into()),
            _ => {}
        }
    }
    let failed = |_| anyhow::anyhow!("The app's state DB could not be copied to the out dir; nothing was changed.");
    let source = Connection::open_with_flags(env.db_path, OpenFlags::SQLITE_OPEN_READ_ONLY).map_err(failed)?;
    source.execute("VACUUM INTO ?1", [copy.to_string_lossy()]).map_err(failed)?;
    drop(source);
    let before = schema(&copy)?;
    let db = StateDb::open(&copy).map_err(|_| anyhow::anyhow!(SCHEMA_CHANGED))?;
    if schema(&copy)? != before {
        anyhow::bail!(SCHEMA_CHANGED);
    }
    Ok(db)
}

/// Every `sqlite_master` row and the user version of the DB at `path`, read-only.
fn schema(path: &Path) -> anyhow::Result<(Vec<String>, i64)> {
    let failed = |_| anyhow::anyhow!("The copy of the app's state DB could not be read; nothing was changed.");
    let db = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY).map_err(failed)?;
    let mut statement = db
        .prepare("SELECT type || ' ' || name || ' ' || tbl_name || ' ' || coalesce(sql, '') FROM sqlite_master ORDER BY 1")
        .map_err(failed)?;
    let rows = statement.query_map([], |row| row.get(0)).map_err(failed)?.collect::<Result<Vec<String>, _>>();
    let version = db.query_row("PRAGMA user_version", [], |row| row.get(0)).map_err(failed)?;
    Ok((rows.map_err(failed)?, version))
}

/// Write the run file, then the list from it.
fn save(env: &Env<'_>, run: &RunFile) -> anyhow::Result<()> {
    run.save(&env.out_dir.join(RUN_FILE))?;
    std::fs::write(env.out_dir.join(LIST_FILE), list(run))?;
    Ok(())
}

/// The list the person reads on this PC: the header (L8, OP3, OP5), then every pair with what the tool knows and did.
pub(crate) fn list(run: &RunFile) -> String {
    let mut text = String::new();
    let _ = writeln!(text, "Beebeeb duplicates repair (task 2274). This list stays on this PC.");
    let _ = writeln!(text, "{FOLDER_SHARES}");
    let _ = writeln!(text, "{DO_NOT_RESTORE}");
    let _ = match run.retention_days {
        Some(days) => writeln!(text, "Trash retention: {days} days (read in the admin panel). A trashed B is purged after it."),
        None => writeln!(text, "Trash retention: not recorded yet. Read it in the admin panel, then run `retention <days>`."),
    };
    let _ = writeln!(
        text,
        "Pairs: {}. Paths with more than two rows (counted, not listed): {}. Mark each pair's action in run.json: none, \
         trash-b or forget-b.",
        run.pairs.len(),
        run.more_than_two
    );
    for pair in &run.pairs {
        let _ = writeln!(text);
        let _ = writeln!(text, "pair {} · {} · action {} · {}", pair.n, kebab(&pair.class), kebab(&pair.action), pair.path);
        let _ = writeln!(text, "  A {} ({}, {} bytes)", pair.a.file_id, pair.a.status, pair.a.size_bytes);
        let _ = writeln!(text, "  B {} ({}, {} bytes)", pair.b.file_id, pair.b.status, pair.b.size_bytes);
        if let Some(read) = &pair.server {
            let _ = writeln!(
                text,
                "  B's live links: {}, live invites: {}",
                read.live_links.map_or("-".into(), |n| n.to_string()),
                read.live_invites.map_or("-".into(), |n| n.to_string())
            );
        }
        if let Some(applied) = &pair.applied {
            let _ = writeln!(text, "  done: {}", done(&applied.outcome, run.retention_days));
        }
    }
    text
}

/// One applied outcome, with B's trash time and purge date (L2, OP3).
fn done(outcome: &Outcome, retention_days: Option<u32>) -> String {
    match outcome {
        Outcome::Trashed { seq, b_trashed_at } => {
            let purge = b_trashed_at
                .as_deref()
                .and_then(|at| chrono::DateTime::parse_from_rfc3339(at).ok())
                .zip(retention_days)
                .map(|(at, days)| (at + chrono::Duration::days(i64::from(days))).date_naive().to_string());
            format!(
                "B in Beebeeb's trash since {} (trash op {seq}); purged after {}",
                b_trashed_at.as_deref().unwrap_or("an unknown time"),
                purge.unwrap_or_else(|| "the retention (not recorded)".into())
            )
        }
        Outcome::Forgotten => "B's row forgotten on this PC (B was in the trash or gone already)".into(),
        Outcome::TrashFailed { status } => format!(
            "B's row forgotten; the trash failed ({}); run apply-one again",
            status.map_or("no answer".to_string(), |status| format!("HTTP {status}"))
        ),
        Outcome::Unconfirmed { status } => format!(
            "B's row forgotten, the trash sent, not read back yet ({}); run apply-one again",
            status.map_or("no answer".to_string(), |status| format!("HTTP {status}"))
        ),
        Outcome::Unexpected { a_live, b_trashed, seq, .. } => format!(
            "trash sent, not confirmed (A live: {a_live}, B in the trash: {b_trashed}, trash op: {})",
            seq.map_or("not found".to_string(), |seq| seq.to_string())
        ),
        Outcome::Skipped { reason } => format!("skipped ({})", kebab(reason)),
    }
}
