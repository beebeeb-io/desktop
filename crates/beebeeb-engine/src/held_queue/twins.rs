//! `twins` items split out of `held_queue.rs`.

use super::*;

/// What D6 decided: the units it makes and the parents the rest of the plan reads.
#[derive(Default)]
pub(super) struct ByPath<'a> {
    /// `ResolveParent`, applied before any remap, so a ghost's remap re-points what was just put under it.
    pub(super) units: Vec<Unit>,
    pub(super) holds: Vec<Unit>,
    pub(super) op_parent: HashMap<&'a str, &'a str>,
    pub(super) row_parent: HashMap<&'a str, &'a str>,
    /// The creates it holds (or keeps held): no folder, or more than one, at their directory.
    pub(super) unresolved: HashSet<&'a str>,
}

impl<'a> ByPath<'a> {
    /// The op's parent once D6 applied.
    pub(super) fn of_op(&self, op: &'a Op) -> Option<&'a str> {
        self.op_parent
            .get(op.op_id.as_str())
            .copied()
            .or(op.parent_id.as_deref())
    }

    /// The row's parent once D6 applied.
    pub(super) fn of_row(&self, row: &'a Row) -> Option<&'a str> {
        self.row_parent
            .get(row.file_id.as_str())
            .copied()
            .or(row.parent_id.as_deref())
    }
}

/// D6: a create queued with no parent whose `target_path` is nested would land at the top level (anti-goal 1). Its
/// parent is the folder row at `dirname(target_path)` under `≡`: the one server-known folder there, else, when none is,
/// the one ghost (D2 or D3 then applies to it). Both the op and its row get it. Otherwise the create is held as D4,
/// counted `parent_unresolved` (also while it stays held). Only creates: a move with no parent is a move to the top
/// level, and the other kinds send no parent. A create held for the person (2267) gets its parent and keeps its pause;
/// one that matches no single folder is left as it is, never re-labelled. A landed create is D1's.
pub(super) fn parents_by_path<'a>(
    rows: &'a [Row],
    ops: &'a [Op],
    ghosts: &HashMap<&str, &Op>,
    landed_ops: &HashSet<&str>,
    row_of: &HashMap<&str, &Row>,
    opts: &Options,
    counts: &mut Counts,
) -> ByPath<'a> {
    let mut out = ByPath::default();
    let mut folders_at: Option<HashMap<String, Vec<&Row>>> = None;
    for op in ops {
        let is_create = op.kind == "create_folder" || op.create_file;
        if !is_create || op.parent_id.is_some() || landed_ops.contains(op.op_id.as_str()) {
            continue;
        }
        let Some((dir, _)) = op
            .target_path
            .as_deref()
            .and_then(|path| path.trim_matches('/').rsplit_once('/'))
        else {
            continue;
        };
        let folders_at = folders_at.get_or_insert_with(|| {
            let mut at: HashMap<String, Vec<&Row>> = HashMap::new();
            for row in rows.iter().filter(|r| r.is_folder) {
                at.entry(fold(row.path.trim_matches('/'), opts.case_insensitive))
                    .or_default()
                    .push(row);
            }
            at
        });
        let at = folders_at
            .get(&fold(dir, opts.case_insensitive))
            .map(Vec::as_slice)
            .unwrap_or_default();
        let server: Vec<&Row> = at.iter().copied().filter(|r| r.server_known).collect();
        let new: Vec<&Row> = at
            .iter()
            .copied()
            .filter(|r| ghosts.contains_key(r.file_id.as_str()))
            .collect();
        let parent = match (&server[..], &new[..]) {
            ([one], _) | ([], [one]) => Some(*one),
            _ => None,
        };
        if let Some(parent) = parent {
            counts.parent_resolved += 1;
            let file_id = op.file_id.as_deref().filter(|id| row_of.contains_key(id));
            out.op_parent.insert(op.op_id.as_str(), parent.file_id.as_str());
            if let Some(id) = file_id {
                out.row_parent.insert(id, parent.file_id.as_str());
            }
            out.units.push(Unit::ResolveParent {
                op_id: op.op_id.clone(),
                file_id: file_id.map(str::to_string),
                parent: parent.file_id.clone(),
            });
        } else if pass_decides(op) {
            counts.parent_unresolved += 1;
            out.unresolved.insert(op.op_id.as_str());
            if op.paused_reason.as_deref() != Some(NEEDS_REVIEW) {
                out.holds.push(Unit::Hold {
                    op_id: op.op_id.clone(),
                });
            }
        }
    }
    out
}

/// The effective parent of a row (spec §3): its parent once D6 applied, or that parent's twin when the plan remaps it.
pub(super) fn effective_parent<'a>(
    row: &'a Row,
    by_path: &ByPath<'a>,
    fate: &HashMap<&str, Fate<'a>>,
) -> Option<&'a str> {
    let parent = by_path.of_row(row);
    match parent.and_then(|p| fate.get(p)) {
        Some(Fate::Remapped(twin)) => Some(twin),
        _ => parent,
    }
}

/// The folder an op is sent under (task 2271): its parent once D6 applied, or that parent's twin when the plan remaps
/// it. A create's row may name no parent while the op names one; the server places the file by the op's.
pub(super) fn sent_to<'a>(op: &'a Op, by_path: &ByPath<'a>, fate: &HashMap<&str, Fate<'a>>) -> Option<&'a str> {
    let parent = by_path.of_op(op);
    match parent.and_then(|p| fate.get(p)) {
        Some(Fate::Remapped(twin)) => Some(twin),
        _ => parent,
    }
}

/// What D10 decided.
pub(super) struct FileTwins<'a> {
    pub(super) holds: Vec<Unit>,
    pub(super) releases: Vec<Unit>,
    /// Every create it holds or keeps held: D8 re-arms none of them.
    pub(super) held: HashSet<&'a str>,
}

/// D10' (task 2271): a new file's create meets a server-known file that no op names, with the same leaf (`≡`), in the
/// folder the create is sent to (`sent_to`), or in its row's own folder when the row names a different one: held as D4
/// (`needs_review`), counted `file_twin` (also while it stays held). A row that names no folder is never matched at the
/// top level. (0.8.18 read only the row's effective parent, which is NULL for most held new files, and sent 89 of them
/// next to a file of the same name.) No download, no compare, no copy (P1).
/// A held one whose twin is gone is released (`Release`: back to `auth`, so the key check decides, D12). A create the
/// server already holds (its row is server-known) is D8's superseded case; one D6 holds stays D6's; one held for the
/// person (2267) is left as it is.
#[allow(clippy::too_many_arguments)]
pub(super) fn file_twins<'a>(
    rows: &'a [Row],
    ops: &'a [Op],
    row_of: &HashMap<&str, &'a Row>,
    busy: &HashSet<&str>,
    unresolved: &HashSet<&str>,
    parent_of: impl Fn(&'a Row) -> Option<&'a str>,
    sent_to: impl Fn(&'a Op) -> Option<&'a str>,
    opts: &Options,
    counts: &mut Counts,
) -> FileTwins<'a> {
    let mut out = FileTwins {
        holds: Vec::new(),
        releases: Vec::new(),
        held: HashSet::new(),
    };
    let ci = opts.case_insensitive;
    let mut server_files: Option<HashMap<Option<&str>, Vec<&Row>>> = None;
    for op in ops
        .iter()
        .filter(|op| op.create_file && pass_decides(op) && !unresolved.contains(op.op_id.as_str()))
    {
        let Some(row) = op.file_id.as_deref().and_then(|id| row_of.get(id)).copied() else {
            continue;
        };
        if row.server_known {
            continue;
        }
        let server_files = server_files.get_or_insert_with(|| {
            let mut by_parent: HashMap<Option<&str>, Vec<&Row>> = HashMap::new();
            for row in rows
                .iter()
                .filter(|r| !r.is_folder && r.server_known && !busy.contains(r.file_id.as_str()))
            {
                by_parent.entry(row.parent_id.as_deref()).or_default().push(row);
            }
            by_parent
        });
        let same_name = |f: &&Row| f.file_id != row.file_id && same_leaf(leaf(&f.path), leaf(&row.path), ci);
        let twin_under = |parent: Option<&str>| {
            server_files
                .get(&parent)
                .is_some_and(|files| files.iter().any(same_name))
        };
        // Under the folder the create is sent to; and under the row's own parent when it names another one. A row that
        // names no parent names no folder: it is never matched at the top level.
        let (sent, named) = (sent_to(op), parent_of(row));
        let twin = twin_under(sent) || (named.is_some() && named != sent && twin_under(named));
        let held = op.paused_reason.as_deref() == Some(NEEDS_REVIEW);
        if twin {
            counts.file_twin += 1;
            out.held.insert(op.op_id.as_str());
            if !held {
                out.holds.push(Unit::Hold {
                    op_id: op.op_id.clone(),
                });
            }
        } else if held {
            counts.released += 1;
            out.releases.push(Unit::Release {
                op_id: op.op_id.clone(),
            });
        }
    }
    out
}
