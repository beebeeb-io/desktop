//! Task 2274, spec phase 1: the read-only list. It reads a copy of the state DB and the platform's view of each path two
//! rows share. It fetches nothing and changes nothing; the server's rows (L4-L9) come later, in `server-read`.
use super::model::{Class, Pair, RowFacts, ViewFacts};
use crate::state_db::{FileEntry, StateDb};
use crate::unknown_check::{OnDisk, PlatformView};
use std::path::Path;

/// What the platform shows at a path, with the attributes the echo check compares (spec I4).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Observed {
    pub view: PlatformView,
    pub size: Option<u64>,
    pub modified: Option<u64>,
    pub read_only: Option<bool>,
}

/// The pairs, and the paths with more than two rows (spec L0: counted, never listed).
#[derive(Debug)]
pub(crate) struct Listing {
    pub pairs: Vec<Pair>,
    pub more_than_two: usize,
}

/// Every path that exactly two rows share, numbered from 1 in path order, with its local class (L1-L3b, or
/// `Unknown` until the server read). `observe` reads the platform at a path without fetching anything.
pub(crate) fn list_pairs(db: &StateDb, root: &Path, observe: &dyn Fn(&Path) -> Observed) -> anyhow::Result<Listing> {
    let mut listing = Listing { pairs: Vec::new(), more_than_two: 0 };
    for (path, rows) in db.shared_paths()? {
        if rows > 2 {
            listing.more_than_two += 1;
            continue;
        }
        let Ok([first, second]) = <[FileEntry; 2]>::try_from(db.rows_at_folded_path(&path)?) else {
            anyhow::bail!("the state DB changed while it was read; list a fresh copy");
        };
        let (first, second) = (row_facts(db, &first)?, row_facts(db, &second)?);
        // A path the app itself would refuse is left alone; its error would name the path (A3).
        let observed = match crate::engine_bridge::local_file_path_under_sync_root(root, &path) {
            Ok(on_disk) => observe(&on_disk),
            Err(_) => Observed { view: PlatformView::Unreadable, size: None, modified: None, read_only: None },
        };
        let identity = match &observed.view {
            PlatformView::Placeholder { identity, .. } => identity.clone(),
            _ => None,
        };
        // A is the row the file on this PC is bound to; bound to neither, the row with known content.
        let (a, b) = if identity.as_deref() == Some(second.file_id.as_str())
            || (identity.as_deref() != Some(first.file_id.as_str())
                && first.local_hash.is_none()
                && second.local_hash.is_some())
        {
            (second, first)
        } else {
            (first, second)
        };
        let class = local_class(db, &a, &b, &observed, identity.as_deref())?;
        listing.pairs.push(Pair { n: 0, path, a, b, view: view_facts(&observed, identity), class });
    }
    listing.pairs.sort_by(|left, right| left.path.cmp(&right.path));
    for (index, pair) in listing.pairs.iter_mut().enumerate() {
        pair.n = u32::try_from(index + 1)?;
    }
    Ok(listing)
}

/// Spec rows L1-L3b, in order; `Unknown` when none applies (the server read decides the rest).
fn local_class(
    db: &StateDb,
    a: &RowFacts,
    b: &RowFacts,
    observed: &Observed,
    identity: Option<&str>,
) -> anyhow::Result<Class> {
    let PlatformView::Placeholder { in_sync, .. } = observed.view else {
        return Ok(Class::Unbound);
    };
    if identity != Some(a.file_id.as_str()) {
        return Ok(Class::Unbound);
    }
    if a.local_hash.is_none() && b.local_hash.is_some() {
        return Ok(Class::BoundToB);
    }
    if a.local_hash.is_none() || !in_sync || db.names_pending_work(&a.file_id)? {
        return Ok(Class::ABusy);
    }
    // B's own unknown-check mark is not B's pending work: it acts on B's path, which is A's file, and forgetting B's
    // row deletes it (trigger `files_unknown_checks_follow_delete`).
    if db.names_pending_work(&b.file_id)? {
        return Ok(Class::BBusy);
    }
    Ok(Class::Unknown)
}

fn row_facts(db: &StateDb, entry: &FileEntry) -> anyhow::Result<RowFacts> {
    let contract = db.get_file_contract_state(&entry.file_id)?;
    Ok(RowFacts {
        file_id: entry.file_id.clone(),
        status: entry.status.as_str().to_string(),
        size_bytes: entry.size_bytes,
        remote_updated_at: entry.remote_updated_at,
        local_hash: contract.as_ref().and_then(|state| state.local_hash.clone()),
        current_version: contract.as_ref().map_or(0, |state| state.current_version),
        shared: contract.as_ref().is_some_and(|state| state.share_id.is_some() || state.shared_root_id.is_some()),
    })
}

fn view_facts(observed: &Observed, identity: Option<String>) -> ViewFacts {
    let (kind, on_disk, in_sync) = match &observed.view {
        PlatformView::Missing => ("missing", None, None),
        PlatformView::Unreadable => ("unreadable", None, None),
        PlatformView::PlainFile => ("plain", None, None),
        PlatformView::Placeholder { on_disk, in_sync, .. } => {
            let on_disk = match on_disk {
                OnDisk::Nothing => "nothing",
                OnDisk::Partly => "partly",
                OnDisk::Wholly => "wholly",
            };
            ("placeholder", Some(on_disk.to_string()), Some(*in_sync))
        }
    };
    ViewFacts {
        kind: kind.to_string(),
        on_disk,
        in_sync,
        identity_names: identity,
        size: observed.size,
        modified: observed.modified,
        read_only: observed.read_only,
    }
}
