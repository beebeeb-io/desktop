//! `planning` items split out of `held_queue.rs`.

use super::*;

/// The plan, in a safe apply order: landed drops, parent resolves, remaps by ascending depth of the ghost's path,
/// scaffolding drops, then holds, releases and re-arms. `payload_exists` says whether an upload's staged payload is
/// still on disk; `disk_has_file` whether a sync-root-relative directory holds any file (D5).
pub fn plan(
    rows: &[Row],
    ops: &[Op],
    opts: &Options,
    payload_exists: &dyn Fn(&str) -> bool,
    disk_has_file: &dyn Fn(&str) -> bool,
) -> (Vec<Unit>, Counts) {
    // D5 reads `disk_has_file`: true when the directory at that sync-root-relative path holds a file anywhere under it,
    // or cannot be read.
    let scaffolding = Scaffolding::read(rows, ops, opts);
    let mut counts = Counts::default();
    let row_of: HashMap<&str, &Row> = rows.iter().map(|r| (r.file_id.as_str(), r)).collect();

    // D1: a create the server already holds under this very id. Any other create_folder of a folder row names a ghost.
    let mut landed = Vec::new();
    let mut create_of: HashMap<&str, &Op> = HashMap::new();
    for op in ops.iter().filter(|op| op.kind == "create_folder") {
        match op.file_id.as_deref().and_then(|id| row_of.get(id)) {
            Some(row) if row.server_known => landed.push(op.op_id.as_str()),
            Some(row) if row.is_folder => {
                create_of.entry(row.file_id.as_str()).or_insert(op);
            }
            _ => {}
        }
    }
    counts.landed = landed.len();
    let landed_ops: HashSet<&str> = landed.iter().copied().collect();
    // A file named by an op that stays: a twin it names takes a pending change, so no ghost is folded into it.
    let busy: HashSet<&str> = ops
        .iter()
        .filter(|op| !landed_ops.contains(op.op_id.as_str()))
        .filter_map(|op| op.file_id.as_deref())
        .collect();
    let mut server_folders: HashMap<Option<&str>, Vec<&Row>> = HashMap::new();
    for row in rows.iter().filter(|r| r.is_folder && r.server_known) {
        server_folders.entry(row.parent_id.as_deref()).or_default().push(row);
    }
    // D6, before D2: the parent of a create queued with none, by path. The rest of the plan reads the parents it sets.
    let by_path = parents_by_path(rows, ops, &create_of, &landed_ops, &row_of, opts, &mut counts);

    let mut ghosts: Vec<&Row> = rows
        .iter()
        .filter(|r| r.is_folder && !r.server_known && create_of.contains_key(r.file_id.as_str()))
        .collect();
    ghosts.sort_by_key(|r| depth(&r.path));
    let mut fate: HashMap<&str, Fate> = HashMap::new();
    let mut remaps: Vec<Unit> = Vec::new();
    let mut remap_at: HashMap<&str, usize> = HashMap::new();
    let (mut holds, mut releases, mut rearms) = (Vec::new(), Vec::new(), Vec::new());
    let (mut drops, mut dropped_ops): (Vec<Unit>, HashSet<&str>) = (Vec::new(), HashSet::new());
    for ghost in ghosts {
        let id = ghost.file_id.as_str();
        if matches!(fate.get(id), Some(Fate::Dropped)) {
            continue; // D5: dropped with the scaffolding above it
        }
        let create = create_of[id];
        if by_path.unresolved.contains(create.op_id.as_str()) {
            // D6 held its create (and counted it): it stays, and its children wait with it.
            fate.insert(id, Fate::Stays);
            continue;
        }
        let row_parent = by_path.of_row(ghost);
        let parent_fate = row_parent.and_then(|p| fate.get(p).copied());
        if matches!(parent_fate, Some(Fate::Stays)) {
            counts.waiting_under_held += 1;
            fate.insert(id, Fate::Stays);
            continue;
        }
        if !pass_decides(create) {
            counts.left_paused += 1;
            fate.insert(id, Fate::Stays);
            continue;
        }
        let parent = match parent_fate {
            Some(Fate::Remapped(twin)) => Some(twin),
            _ => row_parent,
        };
        if parent.is_none() && ghost.path.trim_matches('/').contains('/') {
            // A nested folder whose row names no parent that D6 could not settle by path: never matched at the top
            // level, and held (D4) so the key check never resumes its create to the vault root or next to a twin.
            counts.parent_unresolved += 1;
            fate.insert(id, Fate::Stays);
            if create.paused_reason.as_deref() != Some(NEEDS_REVIEW) {
                holds.push(Unit::Hold {
                    op_id: create.op_id.clone(),
                });
            }
            continue;
        }
        let twins: Vec<&Row> = server_folders
            .get(&parent)
            .map(|rows| {
                rows.iter()
                    .copied()
                    .filter(|t| same_leaf(leaf(&t.path), leaf(&ghost.path), opts.case_insensitive))
                    .collect()
            })
            .unwrap_or_default();
        // D5: a ghost at or under `Backups/unknown` with no twin never replays. Its subtree is dropped whole when it is
        // only scaffolding; otherwise it is held as D4 and the folders under it wait.
        if let Some(scaffolding) = scaffolding
            .as_ref()
            .filter(|s| twins.is_empty() && s.holds(&ghost.path))
        {
            if let Some((unit, ghosts, op_ids)) = scaffolding.drop_at(ghost, &create_of, &fate, disk_has_file) {
                if let Unit::DropScaffold { dirs, .. } = &unit {
                    counts.scaffold_dirs.extend(dirs.iter().cloned());
                }
                dropped_ops.extend(op_ids);
                counts.scaffold_dropped += ghosts.len();
                fate.extend(ghosts.into_iter().map(|g| (g, Fate::Dropped)));
                drops.push(unit);
            } else {
                counts.backup_unknown_kept += 1;
                fate.insert(id, Fate::Stays);
                if create.paused_reason.as_deref() != Some(NEEDS_REVIEW) {
                    holds.push(Unit::Hold {
                        op_id: create.op_id.clone(),
                    });
                }
            }
            continue;
        }
        let held = if ops
            .iter()
            .any(|op| op.file_id.as_deref() == Some(id) && op.op_id != create.op_id)
        {
            Some(&mut counts.held_acts_on_ghost)
        } else if by_path.of_op(create).is_some_and(|p| Some(p) != row_parent) {
            Some(&mut counts.held_parents_disagree)
        } else if twins.iter().any(|t| busy.contains(t.file_id.as_str())) {
            Some(&mut counts.held_twin_busy)
        } else if twins.len() > 1 {
            Some(&mut counts.held_ambiguous)
        } else {
            None
        };
        if let Some(count) = held {
            *count += 1;
            fate.insert(id, Fate::Stays);
            if create.paused_reason.as_deref() != Some(NEEDS_REVIEW) {
                holds.push(Unit::Hold {
                    op_id: create.op_id.clone(),
                });
            }
        } else if let [twin] = twins[..] {
            counts.remapped += 1;
            fate.insert(id, Fate::Remapped(twin.file_id.as_str()));
            remap_at.insert(id, remaps.len());
            remaps.push(Unit::Remap {
                ghost: id.to_string(),
                twin: twin.file_id.clone(),
                ghost_op: create.op_id.clone(),
                rearm: Vec::new(),
                hold: Vec::new(),
            });
        } else {
            counts.replay += 1;
            fate.insert(id, Fate::Replay);
            if create.paused_reason.as_deref() == Some(NEEDS_REVIEW) {
                counts.released += 1;
                releases.push(Unit::Release {
                    op_id: create.op_id.clone(),
                });
            }
        }
    }

    // D10, before D8: a held file twin is never re-armed.
    let twins = file_twins(
        rows,
        ops,
        &row_of,
        &busy,
        &by_path.unresolved,
        |row| effective_parent(row, &by_path, &fate),
        |op| sent_to(op, &by_path, &fate),
        opts,
        &mut counts,
    );

    // D8: an op re-pointed off a remapped ghost (its unit carries it) or under a ghost kept for replay, whose last error
    // is a parent 404, given up or still retrying. A park, a pause the pass owns (`auth`, `needs_review`) and any other
    // error stay as they are. D8 amendment (lead ruling 2026-10-29): an op paused for the person gets its retry budget
    // back and keeps its pause, so "Upload to my Beebeeb" (which only clears the pause) sends it; it is never re-labelled,
    // so a superseded one is left as it is.
    let mut rearmed: HashSet<&str> = HashSet::new();
    for op in ops {
        // Its parent once D6 applied: a create put under a ghost moves (and re-arms) with that ghost's remap.
        let Some(parent) = by_path.of_op(op) else { continue };
        if twins.held.contains(op.op_id.as_str()) {
            continue;
        }
        let remap = match fate.get(parent) {
            Some(Fate::Remapped(_)) => Some(remap_at[parent]),
            Some(Fate::Replay) => None,
            _ => continue,
        };
        let person_paused = op.paused_reason.is_some() && !pass_decides(op);
        if (op.paused_reason.is_some() && !person_paused)
            || op.parked
            || landed_ops.contains(op.op_id.as_str())
            || dropped_ops.contains(op.op_id.as_str())
            || !op.last_error.as_deref().is_some_and(is_parent_404)
        {
            continue;
        }
        let row = op.file_id.as_deref().and_then(|id| row_of.get(id)).copied();
        let op_id = op.op_id.clone();
        let block = presend_block(op, row, payload_exists);
        if block == Some(PresendBlock::Superseded) && person_paused {
            continue;
        }
        if block == Some(PresendBlock::Superseded) {
            counts.rearm_superseded += 1;
            match remap.map(|at| &mut remaps[at]) {
                Some(Unit::Remap { hold, .. }) => hold.push(op_id),
                _ => holds.push(Unit::Hold { op_id }),
            }
        } else if block == Some(PresendBlock::NoPayload) {
            counts.rearm_no_payload += 1;
        } else {
            if person_paused {
                counts.rearmed_paused += 1;
            } else {
                counts.rearmed += 1;
            }
            rearmed.insert(op.op_id.as_str());
            match remap.map(|at| &mut remaps[at]) {
                Some(Unit::Remap { rearm, .. }) => rearm.push(op_id),
                _ => rearms.push(Unit::Rearm { op_id }),
            }
        }
    }
    // A ghost kept for replay whose own create gave up on anything but a parent 404 under a moved or replayed folder.
    counts.replay_given_up = fate
        .iter()
        .filter(|(id, f)| {
            matches!(f, Fate::Replay) && create_of[*id].given_up && !rearmed.contains(create_of[*id].op_id.as_str())
        })
        .count();

    let mut units: Vec<Unit> = landed
        .into_iter()
        .map(|op_id| Unit::DropLanded {
            op_id: op_id.to_string(),
        })
        .collect();
    units.extend(by_path.units);
    units.extend(remaps);
    units.extend(drops);
    units.extend(by_path.holds);
    units.extend(holds);
    units.extend(twins.holds);
    units.extend(releases);
    units.extend(twins.releases);
    units.extend(rearms);
    deepest_first(&mut counts.scaffold_dirs);
    (units, counts)
}

/// Why an op is not sent again (spec D8): the reconcile pass's two checks before it re-arms one, and Try again's before it
/// makes a failed change due (task 2414).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PresendBlock {
    /// Replaying it would make old content current: the server already holds this file (a create), or a newer version
    /// of it than the one the edit was made on.
    Superseded,
    /// An upload whose staged copy is gone: sent again, it only fails again.
    NoPayload,
}

/// Spec D8's checks for one op, in the pass's order: superseded first, then the staged copy. `row` is the op's file row,
/// if any; `payload_exists` says whether a staged copy is still on disk.
pub fn presend_block(op: &Op, row: Option<&Row>, payload_exists: &dyn Fn(&str) -> bool) -> Option<PresendBlock> {
    let superseded = row.is_some_and(|row| {
        (op.create_file && row.server_known)
            || (op.kind == "upload_version"
                && !op.create_file
                && op.base_version.is_some_and(|b| row.current_version > b))
    });
    if superseded {
        Some(PresendBlock::Superseded)
    } else if UPLOAD_KINDS.contains(&op.kind.as_str()) && !op.payload_path.as_deref().is_some_and(payload_exists) {
        Some(PresendBlock::NoPayload)
    } else {
        None
    }
}

/// `≡` as a lookup key: two names are `same_leaf` exactly when their keys are equal (spec §3).
pub(super) fn fold(name: &str, case_insensitive: bool) -> String {
    if case_insensitive {
        name.to_lowercase()
    } else {
        name.to_string()
    }
}
