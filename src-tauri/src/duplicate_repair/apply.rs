//! Task 2274, spec phase 3: apply one pair the person marked. The order is fixed (spec I1): re-check the pair (A5),
//! back up the state DB, forget B's row on this PC, then move B to Beebeeb's trash, then read A and B back and find
//! B's `file_trash` op, the echo the app will apply. Forgetting first is what keeps the echo off A's file: with no row
//! for B the app's `file_trash` changes nothing on disk (D2); with B's row present it acts on B's path, which is A's
//! file (D1, 2283).
use super::list::{self, Observed};
use super::model::{Action, Applied, Class, Outcome, Pair, Reason, RunFile, ServerFacts, ServerFile};
use super::server::{Op, RepairApi, server_read};
use crate::state_db::StateDb;
use std::path::Path;

/// What `apply-one` works on, on this PC.
pub(crate) struct Local<'a> {
    pub db: &'a StateDb,
    /// The state DB file the app uses; its WAL sits next to it.
    pub db_path: &'a Path,
    pub root: &'a Path,
    /// Where the backup goes: the out dir, next to the run file.
    pub out_dir: &'a Path,
    pub observe: &'a dyn Fn(&Path) -> Observed,
}

/// What the server says about the pair now, against the server read.
#[derive(Debug, PartialEq, Eq)]
enum ServerNow {
    AsListed,
    /// A retry only: B is in the trash, and nothing else changed (the first try's trash went through, review F2).
    BTrashed,
    Changed,
}

/// Apply pair `n` as marked, and record what was done in the run file. A run file that fails [`RunFile::check`] is
/// refused before anything is read or written (T11). A pair whose trash failed, or whose trash was not read back, is
/// tried again (T12, review F2, F6): B's row is gone then, so the re-check expects A's row alone; the first try's
/// backup stands (F4), and the app must not have run since (F3).
pub(crate) async fn apply_one(local: &Local<'_>, api: &RepairApi, run: &mut RunFile, n: u32) -> anyhow::Result<Outcome> {
    run.check()?;
    let index = usize::try_from(n)
        .ok()
        .and_then(|n| n.checked_sub(1))
        .filter(|index| *index < run.pairs.len())
        .ok_or_else(|| anyhow::anyhow!("pair {n} is not in the run file; nothing was changed."))?;
    let pair = run.pairs[index].clone();
    let first: Option<Applied> = match &pair.applied {
        None => None,
        Some(applied) => match applied.outcome {
            Outcome::Skipped { .. } => None,
            Outcome::TrashFailed { .. } | Outcome::Unconfirmed { .. } => Some(applied.clone()),
            Outcome::Trashed { .. } | Outcome::Forgotten | Outcome::Unexpected { .. } => {
                return Ok(Outcome::Skipped { reason: Reason::AlreadyDone });
            }
        },
    };
    let retry = first.is_some();
    if pair.action == Action::None {
        return Ok(Outcome::Skipped { reason: Reason::NotMarked });
    }
    let cursor = local
        .db
        .get_sync_cursor()?
        .ok_or_else(|| anyhow::anyhow!("The app has not synced yet (no op cursor); nothing was changed."))?;
    if first.as_ref().is_some_and(|first| first.cursor != cursor) {
        return Ok(Outcome::Skipped { reason: Reason::AppRanSinceTheTry });
    }
    if !unchanged_on_this_pc(local, &pair, retry)? {
        return Ok(Outcome::Skipped { reason: Reason::ChangedOnThisPc });
    }
    let server = server_now(api, &pair, retry).await?;
    // Review P1: every op after the cursor is one the app applies when it starts. One that names a listed B whose row
    // is still here acts on that row, whose path is A's file (D1, 2283); so does a removal of the folder that row names
    // as its parent, when this PC holds no row for that folder (see `listed_bs_still_here`). The one exception is this
    // forget-b pair's own trash or delete with nothing else waiting: the forget below leaves those ops no row to act on
    // (D2).
    let ops = api.ops_after(cursor).await.map_err(|failure| failure.at(n))?;
    let forget_defuses = server == ServerNow::AsListed
        && pair.action == Action::ForgetB
        && ops.iter().all(|op| op.id.as_deref() == Some(pair.b.file_id.as_str()) && removes(op));
    let here = listed_bs_still_here(local.db, run)?;
    if !forget_defuses && ops.iter().any(|op| here.reached_by(op)) {
        return Ok(Outcome::Skipped { reason: Reason::BChangePending });
    }
    match server {
        ServerNow::Changed => return Ok(Outcome::Skipped { reason: Reason::ChangedOnServer }),
        // F3: an op after the cursor is one the stopped app would apply when it starts, with B's row gone.
        ServerNow::AsListed if !ops.is_empty() && !forget_defuses => return Ok(Outcome::Skipped { reason: Reason::PendingOps }),
        ServerNow::AsListed | ServerNow::BTrashed => {}
    }

    let backup = match &first {
        Some(first) => first.backup.clone(),
        None => back_up(local, n)?,
    };
    let at = chrono::Utc::now().timestamp();
    let mut record = |outcome: Outcome| {
        run.pairs[index].applied = Some(Applied { outcome: outcome.clone(), cursor, backup: backup.clone(), at });
        outcome
    };
    if !retry {
        // D6, I2b: by id only, never by path: A's row shares the path.
        local.db.delete_file(&pair.b.file_id)?;
    }
    if pair.action == Action::ForgetB {
        return Ok(record(Outcome::Forgotten));
    }
    if server == ServerNow::AsListed {
        if let Err(failure) = api.trash(&pair.b.file_id).await {
            return Ok(record(Outcome::TrashFailed { status: failure.0 }));
        }
    }
    Ok(record(read_back(api, &pair, cursor).await))
}

/// Whether the app applies `op` by removing the row it names: a trash or a permanent delete (`engine_bridge.rs`
/// `apply_sync_op`).
fn removes(op: &Op) -> bool {
    matches!(op.kind.as_str(), "file_trash" | "file_delete")
}

/// The listed Bs whose row is still on this PC, and the ops the app would apply to those rows.
struct ListedBsHere {
    ids: Vec<String>,
    /// The parents those rows name that have no row on this PC.
    absent_parents: Vec<String>,
}

impl ListedBsHere {
    /// Whether the app, applying `op`, would act on one of these rows: an op naming the row's own id, or a trash or
    /// delete of a folder this PC holds no row for, which prunes that folder's children by `parent_id`
    /// (`delete_orphaned_children_of_absent_folder`). A folder with a row here prunes by path, and a listed B's row is
    /// at A's path, under a folder only when A's row is too.
    fn reached_by(&self, op: &Op) -> bool {
        op.id.as_ref().is_some_and(|id| self.ids.contains(id) || (removes(op) && self.absent_parents.contains(id)))
    }
}

fn listed_bs_still_here(db: &StateDb, run: &RunFile) -> anyhow::Result<ListedBsHere> {
    let mut here = ListedBsHere { ids: Vec::new(), absent_parents: Vec::new() };
    for pair in &run.pairs {
        let Some(row) = db.get_file(&pair.b.file_id)? else { continue };
        here.ids.push(pair.b.file_id.clone());
        if let Some(parent) = row.parent_id
            && db.get_file(&parent)?.is_none()
        {
            here.absent_parents.push(parent);
        }
    }
    Ok(here)
}

/// Review P1, the snapshot half (`engine_tests` :154): the listed pairs whose B is gone or in the trash, by the server
/// read, while B's row is still on this PC. A full re-sync of the folder lists no B there and prunes B's row, at A's
/// path, so the app must not start until those pairs are forgotten.
pub(crate) fn unsafe_to_start(db: &StateDb, run: &RunFile) -> anyhow::Result<usize> {
    let mut count = 0;
    for pair in run.pairs.iter().filter(|pair| matches!(pair.class, Class::BGone | Class::BInTrash)) {
        if db.get_file(&pair.b.file_id)?.is_some() {
            count += 1;
        }
    }
    Ok(count)
}

/// After the trash: A live, B in the trash, and the ops after the app's cursor are B's `file_trash` alone, the echo the
/// app applies when it starts (review F3). A read that fails leaves the pair unconfirmed, to be read back again (F6).
async fn read_back(api: &RepairApi, pair: &Pair, cursor: i64) -> Outcome {
    let a = api.file(&pair.a.file_id).await;
    let b = api.file(&pair.b.file_id).await;
    let ops = api.ops_after(cursor).await;
    let (a, b, ops) = match (a, b, ops) {
        (Ok(a), Ok(b), Ok(ops)) => (a, b, ops),
        (a, b, ops) => {
            let status = [a.err(), b.err(), ops.err()].into_iter().flatten().next().and_then(|failure| failure.0);
            return Outcome::Unconfirmed { status };
        }
    };
    let a_live = matches!(a, ServerFile::Found(a) if !a.trashed);
    let (b_trashed, b_trashed_at) = match b {
        ServerFile::Found(b) if b.trashed => (true, b.trashed_at),
        _ => (false, None),
    };
    let is_b_trash = |op: &&super::server::Op| op.kind == "file_trash" && op.id.as_deref() == Some(pair.b.file_id.as_str());
    let seq = ops.iter().find(is_b_trash).map(|op| op.seq);
    let other_ops = ops.iter().filter(|op| !is_b_trash(op)).count();
    match seq {
        Some(seq) if a_live && b_trashed && other_ops == 0 && ops.len() == 1 => Outcome::Trashed { seq, b_trashed_at },
        _ => Outcome::Unexpected { a_live, b_trashed, seq, other_ops },
    }
}

/// A5 on this PC: the path holds A and B's rows (A's alone when B's row was forgotten by an earlier try), every row
/// as listed, the file at the path as listed, and no row of the decision table's local part (L1-L3b) applies now.
fn unchanged_on_this_pc(local: &Local<'_>, pair: &Pair, retry: bool) -> anyhow::Result<bool> {
    let rows = local.db.rows_at_folded_path(&pair.path)?;
    let mut ids: Vec<&str> = rows.iter().map(|row| row.file_id.as_str()).collect();
    let mut expected = vec![pair.a.file_id.as_str()];
    if !retry {
        expected.push(pair.b.file_id.as_str());
    }
    ids.sort_unstable();
    expected.sort_unstable();
    if ids != expected {
        return Ok(false);
    }
    for row in &rows {
        let listed = if row.file_id == pair.a.file_id { &pair.a } else { &pair.b };
        if list::row_facts(local.db, row)? != *listed {
            return Ok(false);
        }
    }
    let (observed, identity) = list::observe_at(local.root, &pair.path, local.observe);
    if list::view_facts(&observed, identity.clone()) != pair.view {
        return Ok(false);
    }
    Ok(list::local_class(local.db, &pair.path, &pair.a, &pair.b, &observed, identity.as_deref())? == Class::Unknown)
}

/// A5 on the server: the server read, done again now, says exactly what it said for the list (A and B, their parents,
/// sizes, times, versions and the counts of B's links and invites), and the pair's class follows from it (review F1:
/// a compared pair's class is the read's L10). On a retry, B in the trash with nothing else changed is the first
/// try's trash (F2).
async fn server_now(api: &RepairApi, pair: &Pair, retry: bool) -> anyhow::Result<ServerNow> {
    let mut now = pair.clone();
    now.class = Class::Unknown;
    now.server = None;
    server_read(api, std::slice::from_mut(&mut now)).await?;
    if now.server.is_some() && now.server == pair.server {
        let read_gives = match pair.class {
            Class::Identical | Class::Differs => Class::Unknown,
            class => class,
        };
        if now.class != read_gives {
            anyhow::bail!(
                "pair {}: its class in the run file does not follow from the server read; nothing was changed.",
                pair.n
            );
        }
        return Ok(ServerNow::AsListed);
    }
    if retry && b_trashed_since(&now, pair) {
        return Ok(ServerNow::BTrashed);
    }
    Ok(ServerNow::Changed)
}

/// A the same and B in the trash, B's facts as listed but for the three fields the trash sets.
fn b_trashed_since(now: &Pair, listed: &Pair) -> bool {
    let (Some(now), Some(listed)) = (&now.server, &listed.server) else { return false };
    let (Some(ServerFile::Found(b_now)), Some(ServerFile::Found(b_listed))) = (&now.b, &listed.b) else { return false };
    let untrashed = ServerFacts {
        trashed: b_listed.trashed,
        trashed_at: b_listed.trashed_at.clone(),
        updated_at: b_listed.updated_at.clone(),
        ..b_now.clone()
    };
    now.a == listed.a && b_now.trashed && untrashed == *b_listed
}

/// The state DB and its WAL, copied to the out dir before any write: `state-<pair>-<unix>.db` (and `…db-wal`). The app
/// is stopped and the tool writes nothing before this, so the two files agree.
fn back_up(local: &Local<'_>, n: u32) -> anyhow::Result<String> {
    let unix = chrono::Utc::now().timestamp();
    let name = (1..)
        .map(|try_| if try_ == 1 { format!("state-{n}-{unix}.db") } else { format!("state-{n}-{unix}-{try_}.db") })
        .find(|name| !local.out_dir.join(name).exists())
        .expect("a free name");
    let failed = |_| anyhow::anyhow!("The state DB could not be copied to the out dir; nothing was changed.");
    std::fs::copy(local.db_path, local.out_dir.join(&name)).map_err(failed)?;
    let mut wal = local.db_path.as_os_str().to_owned();
    wal.push("-wal");
    if Path::new(&wal).exists() {
        std::fs::copy(&wal, local.out_dir.join(format!("{name}-wal"))).map_err(failed)?;
    }
    Ok(name)
}
