//! Task 2274, spec phase 3: apply one pair the person marked. The order is fixed (spec I1): re-check the pair (A5),
//! back up the state DB, forget B's row on this PC, then move B to Beebeeb's trash, then read A and B back and find
//! B's `file_trash` op, the echo the app will apply. Forgetting first is what keeps the echo off A's file: with no row
//! for B the app's `file_trash` changes nothing on disk (D2); with B's row present it acts on B's path, which is A's
//! file (D1, 2283).
use super::list::{self, Observed};
use super::model::{Action, Applied, Class, Outcome, Pair, Reason, RunFile, ServerFile};
use super::server::{RepairApi, server_read};
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

/// Apply pair `n` as marked, and record what was done in the run file. A run file that fails [`RunFile::check`] is
/// refused before anything is read or written (T11). A pair whose trash failed is tried again: B's row is gone then,
/// so the re-check expects A's row alone (T12).
pub(crate) async fn apply_one(local: &Local<'_>, api: &RepairApi, run: &mut RunFile, n: u32) -> anyhow::Result<Outcome> {
    run.check()?;
    let index = usize::try_from(n)
        .ok()
        .and_then(|n| n.checked_sub(1))
        .filter(|index| *index < run.pairs.len())
        .ok_or_else(|| anyhow::anyhow!("pair {n} is not in the run file; nothing was changed."))?;
    let pair = run.pairs[index].clone();
    let retry = match pair.applied.as_ref().map(|applied| &applied.outcome) {
        None | Some(Outcome::Skipped { .. }) => false,
        Some(Outcome::TrashFailed { .. }) => true,
        Some(Outcome::Trashed { .. } | Outcome::Forgotten | Outcome::Unexpected { .. }) => {
            return Ok(Outcome::Skipped { reason: Reason::AlreadyDone });
        }
    };
    if pair.action == Action::None {
        return Ok(Outcome::Skipped { reason: Reason::NotMarked });
    }
    let cursor = local
        .db
        .get_sync_cursor()?
        .ok_or_else(|| anyhow::anyhow!("The app has not synced yet (no op cursor); nothing was changed."))?;
    if !unchanged_on_this_pc(local, &pair, retry)? {
        return Ok(Outcome::Skipped { reason: Reason::ChangedOnThisPc });
    }
    if !unchanged_on_server(api, &pair).await? {
        return Ok(Outcome::Skipped { reason: Reason::ChangedOnServer });
    }

    let backup = back_up(local, n)?;
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
    if let Err(failure) = api.trash(&pair.b.file_id).await {
        return Ok(record(Outcome::TrashFailed { status: failure.0 }));
    }
    let a_live = matches!(api.file(&pair.a.file_id).await, Ok(ServerFile::Found(a)) if !a.trashed);
    let (b_trashed, b_trashed_at) = match api.file(&pair.b.file_id).await {
        Ok(ServerFile::Found(b)) if b.trashed => (true, b.trashed_at),
        _ => (false, None),
    };
    let seq = api.trash_seq(cursor, &pair.b.file_id).await.ok().flatten();
    Ok(record(match seq {
        Some(seq) if a_live && b_trashed => Outcome::Trashed { seq, b_trashed_at },
        _ => Outcome::Unexpected { a_live, b_trashed, seq },
    }))
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
    Ok(list::local_class(local.db, &pair.a, &pair.b, &observed, identity.as_deref())? == Class::Unknown)
}

/// A5 on the server: the server read, done again now, says exactly what it said for the list (A and B, their parents,
/// sizes, times, versions and the counts of B's links and invites).
async fn unchanged_on_server(api: &RepairApi, pair: &Pair) -> anyhow::Result<bool> {
    let mut now = pair.clone();
    now.class = Class::Unknown;
    now.server = None;
    server_read(api, std::slice::from_mut(&mut now)).await?;
    Ok(now.server.is_some() && now.server == pair.server)
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
