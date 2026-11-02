//! Tasks 1888 + 2226 (spec `docs/superpowers/specs/2026-10-28-1888-2226-reconcile-then-resume-design.md`, workspace):
//! the reconcile pass over the queue, planned in memory from plain rows. Pure: no database, no disk, no network.
//!
//! A **ghost** is a folder row made on this PC (not server-known) whose create never reached the server. The same
//! folder reached the server another way, under another id: its **twin**. The plan folds each ghost into its twin
//! (re-points its children, deletes its create and its row), holds what cannot be folded safely, leaves a ghost with no
//! twin to replay, and re-arms the changes that gave up on the 404 the ghost caused. `StateDb::reconcile_held_queue`
//! applies the plan in batches of whole units.
#![cfg_attr(target_os = "macos", allow(dead_code))]

use std::collections::{HashMap, HashSet};

/// A `files` row, as the pass reads it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Row {
    pub file_id: String,
    pub parent_id: Option<String>,
    pub path: String,
    pub is_folder: bool,
    /// `current_version > 0 OR remote_updated_at > 0` (spec §3, corrected).
    pub server_known: bool,
    pub current_version: i64,
}

/// An `operation_queue` row, as the pass reads it.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Op {
    pub op_id: String,
    pub kind: String,
    pub file_id: Option<String>,
    pub parent_id: Option<String>,
    pub target_path: Option<String>,
    /// An `upload_version` whose metadata says `operation: create_file`.
    pub create_file: bool,
    pub paused_reason: Option<String>,
    /// `attempts >= max_attempts AND paused_reason IS NULL AND last_error_class IS NULL` (spec §3).
    pub given_up: bool,
    /// `last_error_class IS NOT NULL`: parked. A park is never re-armed (spec D8).
    pub parked: bool,
    pub last_error: Option<String>,
    pub payload_path: Option<String>,
    pub base_version: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Options {
    /// This device's backup folder name (D9); `None` when the hostname could not be read.
    pub device_segment: Option<String>,
    /// `≡` ignores letter case (Windows: NTFS).
    pub case_insensitive: bool,
}

/// One unit of the plan. Units never split: each is applied whole, in one transaction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Unit {
    /// D1: the create landed (its own row is server-known): delete only its op.
    DropLanded { op_id: String },
    /// D2: fold `ghost` into `twin`: re-point every child row and op, re-arm `rearm` and hold `hold` (D8: superseded),
    /// delete `ghost_op` and the ghost's row. The holds ride in the unit, so no batch separates them from the remap that
    /// takes their ghost away.
    Remap { ghost: String, twin: String, ghost_op: String, rearm: Vec<String>, hold: Vec<String> },
    /// D4 / D8 superseded: `needs_review`.
    Hold { op_id: String },
    /// A `needs_review` op whose cause is gone: back to `auth`, for the key check that follows the pass (D12).
    Release { op_id: String },
    /// D8 under a ghost kept for replay.
    Rearm { op_id: String },
    /// D6: the parent found by path, for the op and for its row (`file_id`, when the row exists).
    ResolveParent { op_id: String, file_id: Option<String>, parent: String },
    /// D5: empty `Backups/unknown` scaffolding.
    DropScaffold { op_ids: Vec<String>, file_ids: Vec<String>, dirs: Vec<String> },
}

/// What the plan does, by D row. The dry run (D14).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Counts {
    pub landed: usize,
    pub remapped: usize,
    pub replay: usize,
    /// A ghost kept for replay whose own create gave up on anything but a parent 404 (D8, builder check Q4).
    pub replay_given_up: usize,
    /// A ghost under a held (or person-paused) ghost: it waits with it, through the parent-first gate (D4, D7).
    pub waiting_under_held: usize,
    /// A ghost whose create waits for the person (2267's `found_while_paused`, quota, a replaced key): left alone.
    pub left_paused: usize,
    /// A `needs_review` op whose cause is gone, released to the key check (D4, D6, D10).
    pub released: usize,
    pub held_ambiguous: usize,
    pub held_acts_on_ghost: usize,
    pub held_parents_disagree: usize,
    pub held_twin_busy: usize,
    pub rearmed: usize,
    /// D8 amendment (lead ruling 2026-10-29): an op paused for the person (2267's `found_while_paused`, quota, a
    /// replaced key) whose retry budget is re-armed on a parent 404; its pause stays.
    pub rearmed_paused: usize,
    pub rearm_no_payload: usize,
    pub rearm_superseded: usize,
    pub parent_resolved: usize,
    pub parent_unresolved: usize,
    pub scaffold_dropped: usize,
    /// D5: the sync-root-relative directories of the dropped scaffolding, for the caller to remove bottom-up with
    /// `std::fs::remove_dir` after the commit. Paths: never logged.
    pub scaffold_dirs: Vec<String>,
    pub backup_unknown_kept: usize,
    pub file_twin: usize,
}

/// The pause reason of an op the pass held for the person to look at (D4).
pub const NEEDS_REVIEW: &str = "needs_review";

/// An upload reads its staged payload; without it a re-armed upload only fails again.
const UPLOAD_KINDS: [&str; 2] = ["upload_version", "upload_file"];

/// `a ≡ b` for two leaf names (spec §3).
pub fn same_leaf(a: &str, b: &str, case_insensitive: bool) -> bool {
    if case_insensitive { a.to_lowercase() == b.to_lowercase() } else { a == b }
}

/// A parent 404: the error text before `" for url ("` contains `404` (spec §3), so a UUID with "404" in the URL never
/// counts.
pub fn is_parent_404(last_error: &str) -> bool {
    last_error.split(" for url (").next().unwrap_or_default().contains("404")
}

fn leaf(path: &str) -> &str {
    path.trim_matches('/').rsplit('/').next().unwrap_or_default()
}

fn depth(path: &str) -> usize {
    path.trim_matches('/').matches('/').count()
}

/// The pass decides about an op that is not paused, paused for `auth` or held by an earlier pass. Any other pause
/// (2267's `found_while_paused`, quota, a replaced key) waits for the person, and the pass leaves it alone.
fn pass_decides(op: &Op) -> bool {
    matches!(op.paused_reason.as_deref(), None | Some("auth") | Some(NEEDS_REVIEW))
}

/// What the plan does with a ghost; its children read it.
#[derive(Clone, Copy)]
enum Fate<'a> {
    Remapped(&'a str),
    Replay,
    /// Held, left for the person, or waiting on one of those: it stays where it is.
    Stays,
    /// D5: in a scaffolding subtree the plan drops whole.
    Dropped,
}

/// D5: where the backup mirror wrote when it could not read this PC's name (D9).
const BACKUPS_UNKNOWN: &str = "Backups/unknown";

/// `path` is `root` or lies under it, whole segment by segment under `≡`.
fn at_or_under(path: &str, root: &str, case_insensitive: bool) -> bool {
    let mut segments = path.trim_matches('/').split('/');
    root.trim_matches('/').split('/').all(|seg| segments.next().is_some_and(|s| same_leaf(s, seg, case_insensitive)))
}

/// `dirs` in an order the caller can remove them in, one at a time: deepest first (then by path, so the order is the
/// same whichever way they were gathered), each once.
fn deepest_first(dirs: &mut Vec<String>) {
    dirs.sort_by(|a, b| depth(b).cmp(&depth(a)).then_with(|| a.cmp(b)));
    dirs.dedup();
}

/// D5: the rows and ops at or under `Backups/unknown`, read once per plan. Only made when D5 applies: this PC's backup
/// folder has a name (D9) and it is not `unknown` under `≡`, so `Backups/unknown` is never this PC's own backup.
struct Scaffolding<'a> {
    /// The rows there, and every row whose parent is one of them.
    rows: Vec<&'a Row>,
    /// The ops that name one of those rows, as their file or their parent, or a target path there.
    ops: Vec<&'a Op>,
    case_insensitive: bool,
}

impl<'a> Scaffolding<'a> {
    fn read(rows: &'a [Row], ops: &'a [Op], opts: &Options) -> Option<Self> {
        let ci = opts.case_insensitive;
        let device = crate::known_folder::sanitize_device_segment(opts.device_segment.as_deref()?);
        if same_leaf(&device, leaf(BACKUPS_UNKNOWN), ci) {
            return None;
        }
        let there: HashSet<&str> =
            rows.iter().filter(|r| at_or_under(&r.path, BACKUPS_UNKNOWN, ci)).map(|r| r.file_id.as_str()).collect();
        let names = |id: Option<&str>| id.is_some_and(|id| there.contains(id));
        Some(Scaffolding {
            rows: rows.iter().filter(|r| there.contains(r.file_id.as_str()) || names(r.parent_id.as_deref())).collect(),
            ops: ops
                .iter()
                .filter(|op| {
                    names(op.file_id.as_deref())
                        || names(op.parent_id.as_deref())
                        || op.target_path.as_deref().is_some_and(|t| at_or_under(t, BACKUPS_UNKNOWN, ci))
                })
                .collect(),
            case_insensitive: ci,
        })
    }

    fn holds(&self, path: &str) -> bool {
        at_or_under(path, BACKUPS_UNKNOWN, self.case_insensitive)
    }

    /// The unit that drops the subtree at `root` (a ghost here with no twin), with the ids of the ghosts and the ops it
    /// drops; `None` when the subtree is more than scaffolding. Scaffolding is ghosts only: every row at or under
    /// `root`, and every row under one of them, is a ghost not decided earlier in this plan (so no server folder is
    /// there, and none of them has a twin); every op on them is a ghost's `create_folder` the pass decides (none waits
    /// for the person); and the directory holds no file on disk.
    fn drop_at(
        &self,
        root: &'a Row,
        ghost_creates: &HashMap<&str, &Op>,
        fate: &HashMap<&str, Fate>,
        disk_has_file: &dyn Fn(&str) -> bool,
    ) -> Option<(Unit, Vec<&'a str>, Vec<&'a str>)> {
        let ci = self.case_insensitive;
        let ghosts: Vec<&'a Row> = self.rows.iter().copied().filter(|r| at_or_under(&r.path, &root.path, ci)).collect();
        let undecided_ghost = |id: &str| ghost_creates.contains_key(id) && !fate.contains_key(id);
        if !ghosts.iter().all(|r| undecided_ghost(r.file_id.as_str())) {
            return None;
        }
        let ids: HashSet<&str> = ghosts.iter().map(|r| r.file_id.as_str()).collect();
        let names = |id: Option<&str>| id.is_some_and(|id| ids.contains(id));
        if self.rows.iter().any(|r| names(r.parent_id.as_deref()) && !ids.contains(r.file_id.as_str())) {
            return None;
        }
        let on_them: Vec<&'a Op> = self
            .ops
            .iter()
            .copied()
            .filter(|op| {
                names(op.file_id.as_deref())
                    || names(op.parent_id.as_deref())
                    || op.target_path.as_deref().is_some_and(|t| at_or_under(t, &root.path, ci))
            })
            .collect();
        if on_them.iter().any(|op| op.kind != "create_folder" || !names(op.file_id.as_deref()) || !pass_decides(op)) {
            return None;
        }
        if disk_has_file(&root.path) {
            return None;
        }
        let mut dirs: Vec<String> = ghosts.iter().map(|r| r.path.clone()).collect();
        deepest_first(&mut dirs);
        let unit = Unit::DropScaffold {
            op_ids: on_them.iter().map(|op| op.op_id.clone()).collect(),
            file_ids: ghosts.iter().map(|r| r.file_id.clone()).collect(),
            dirs,
        };
        Some((unit, ids.into_iter().collect(), on_them.iter().map(|op| op.op_id.as_str()).collect()))
    }
}

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
    let busy: HashSet<&str> =
        ops.iter().filter(|op| !landed_ops.contains(op.op_id.as_str())).filter_map(|op| op.file_id.as_deref()).collect();
    let mut server_folders: HashMap<Option<&str>, Vec<&Row>> = HashMap::new();
    for row in rows.iter().filter(|r| r.is_folder && r.server_known) {
        server_folders.entry(row.parent_id.as_deref()).or_default().push(row);
    }
    // D6, before D2: the parent of a create queued with none, by path. The rest of the plan reads the parents it sets.
    let by_path = parents_by_path(rows, ops, &create_of, &landed_ops, &row_of, opts, &mut counts);

    let mut ghosts: Vec<&Row> =
        rows.iter().filter(|r| r.is_folder && !r.server_known && create_of.contains_key(r.file_id.as_str())).collect();
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
                holds.push(Unit::Hold { op_id: create.op_id.clone() });
            }
            continue;
        }
        let twins: Vec<&Row> = server_folders
            .get(&parent)
            .map(|rows| rows.iter().copied().filter(|t| same_leaf(leaf(&t.path), leaf(&ghost.path), opts.case_insensitive)).collect())
            .unwrap_or_default();
        // D5: a ghost at or under `Backups/unknown` with no twin never replays. Its subtree is dropped whole when it is
        // only scaffolding; otherwise it is held as D4 and the folders under it wait.
        if let Some(scaffolding) = scaffolding.as_ref().filter(|s| twins.is_empty() && s.holds(&ghost.path)) {
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
                    holds.push(Unit::Hold { op_id: create.op_id.clone() });
                }
            }
            continue;
        }
        let held = if ops.iter().any(|op| op.file_id.as_deref() == Some(id) && op.op_id != create.op_id) {
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
                holds.push(Unit::Hold { op_id: create.op_id.clone() });
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
                releases.push(Unit::Release { op_id: create.op_id.clone() });
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
        let row = op.file_id.as_deref().and_then(|id| row_of.get(id));
        // Replaying it would make old content current: the server already holds this file, or a newer version of it.
        let superseded = row.is_some_and(|row| {
            (op.create_file && row.server_known)
                || (op.kind == "upload_version" && !op.create_file && op.base_version.is_some_and(|b| row.current_version > b))
        });
        let op_id = op.op_id.clone();
        if superseded && person_paused {
            continue;
        }
        if superseded {
            counts.rearm_superseded += 1;
            match remap.map(|at| &mut remaps[at]) {
                Some(Unit::Remap { hold, .. }) => hold.push(op_id),
                _ => holds.push(Unit::Hold { op_id }),
            }
        } else if UPLOAD_KINDS.contains(&op.kind.as_str()) && !op.payload_path.as_deref().is_some_and(payload_exists) {
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
        .filter(|(id, f)| matches!(f, Fate::Replay) && create_of[*id].given_up && !rearmed.contains(create_of[*id].op_id.as_str()))
        .count();

    let mut units: Vec<Unit> = landed.into_iter().map(|op_id| Unit::DropLanded { op_id: op_id.to_string() }).collect();
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

/// `≡` as a lookup key: two names are `same_leaf` exactly when their keys are equal (spec §3).
fn fold(name: &str, case_insensitive: bool) -> String {
    if case_insensitive { name.to_lowercase() } else { name.to_string() }
}

/// What D6 decided: the units it makes and the parents the rest of the plan reads.
#[derive(Default)]
struct ByPath<'a> {
    /// `ResolveParent`, applied before any remap, so a ghost's remap re-points what was just put under it.
    units: Vec<Unit>,
    holds: Vec<Unit>,
    op_parent: HashMap<&'a str, &'a str>,
    row_parent: HashMap<&'a str, &'a str>,
    /// The creates it holds (or keeps held): no folder, or more than one, at their directory.
    unresolved: HashSet<&'a str>,
}

impl<'a> ByPath<'a> {
    /// The op's parent once D6 applied.
    fn of_op(&self, op: &'a Op) -> Option<&'a str> {
        self.op_parent.get(op.op_id.as_str()).copied().or(op.parent_id.as_deref())
    }

    /// The row's parent once D6 applied.
    fn of_row(&self, row: &'a Row) -> Option<&'a str> {
        self.row_parent.get(row.file_id.as_str()).copied().or(row.parent_id.as_deref())
    }
}

/// D6: a create queued with no parent whose `target_path` is nested would land at the top level (anti-goal 1). Its
/// parent is the folder row at `dirname(target_path)` under `≡`: the one server-known folder there, else, when none is,
/// the one ghost (D2 or D3 then applies to it). Both the op and its row get it. Otherwise the create is held as D4,
/// counted `parent_unresolved` (also while it stays held). Only creates: a move with no parent is a move to the top
/// level, and the other kinds send no parent. A create held for the person (2267) gets its parent and keeps its pause;
/// one that matches no single folder is left as it is, never re-labelled. A landed create is D1's.
fn parents_by_path<'a>(
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
        let Some((dir, _)) = op.target_path.as_deref().and_then(|path| path.trim_matches('/').rsplit_once('/')) else {
            continue;
        };
        let folders_at = folders_at.get_or_insert_with(|| {
            let mut at: HashMap<String, Vec<&Row>> = HashMap::new();
            for row in rows.iter().filter(|r| r.is_folder) {
                at.entry(fold(row.path.trim_matches('/'), opts.case_insensitive)).or_default().push(row);
            }
            at
        });
        let at = folders_at.get(&fold(dir, opts.case_insensitive)).map(Vec::as_slice).unwrap_or_default();
        let server: Vec<&Row> = at.iter().copied().filter(|r| r.server_known).collect();
        let new: Vec<&Row> = at.iter().copied().filter(|r| ghosts.contains_key(r.file_id.as_str())).collect();
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
                out.holds.push(Unit::Hold { op_id: op.op_id.clone() });
            }
        }
    }
    out
}

/// The effective parent of a row (spec §3): its parent once D6 applied, or that parent's twin when the plan remaps it.
fn effective_parent<'a>(row: &'a Row, by_path: &ByPath<'a>, fate: &HashMap<&str, Fate<'a>>) -> Option<&'a str> {
    let parent = by_path.of_row(row);
    match parent.and_then(|p| fate.get(p)) {
        Some(Fate::Remapped(twin)) => Some(twin),
        _ => parent,
    }
}

/// The folder an op is sent under (task 2271): its parent once D6 applied, or that parent's twin when the plan remaps
/// it. A create's row may name no parent while the op names one; the server places the file by the op's.
fn sent_to<'a>(op: &'a Op, by_path: &ByPath<'a>, fate: &HashMap<&str, Fate<'a>>) -> Option<&'a str> {
    let parent = by_path.of_op(op);
    match parent.and_then(|p| fate.get(p)) {
        Some(Fate::Remapped(twin)) => Some(twin),
        _ => parent,
    }
}

/// What D10 decided.
struct FileTwins<'a> {
    holds: Vec<Unit>,
    releases: Vec<Unit>,
    /// Every create it holds or keeps held: D8 re-arms none of them.
    held: HashSet<&'a str>,
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
fn file_twins<'a>(
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
    let mut out = FileTwins { holds: Vec::new(), releases: Vec::new(), held: HashSet::new() };
    let ci = opts.case_insensitive;
    let mut server_files: Option<HashMap<Option<&str>, Vec<&Row>>> = None;
    for op in ops.iter().filter(|op| op.create_file && pass_decides(op) && !unresolved.contains(op.op_id.as_str())) {
        let Some(row) = op.file_id.as_deref().and_then(|id| row_of.get(id)).copied() else { continue };
        if row.server_known {
            continue;
        }
        let server_files = server_files.get_or_insert_with(|| {
            let mut by_parent: HashMap<Option<&str>, Vec<&Row>> = HashMap::new();
            for row in rows.iter().filter(|r| !r.is_folder && r.server_known && !busy.contains(r.file_id.as_str())) {
                by_parent.entry(row.parent_id.as_deref()).or_default().push(row);
            }
            by_parent
        });
        let same_name = |f: &&Row| f.file_id != row.file_id && same_leaf(leaf(&f.path), leaf(&row.path), ci);
        let twin_under = |parent: Option<&str>| server_files.get(&parent).is_some_and(|files| files.iter().any(same_name));
        // Under the folder the create is sent to; and under the row's own parent when it names another one. A row that
        // names no parent names no folder: it is never matched at the top level.
        let (sent, named) = (sent_to(op), parent_of(row));
        let twin = twin_under(sent) || (named.is_some() && named != sent && twin_under(named));
        let held = op.paused_reason.as_deref() == Some(NEEDS_REVIEW);
        if twin {
            counts.file_twin += 1;
            out.held.insert(op.op_id.as_str());
            if !held {
                out.holds.push(Unit::Hold { op_id: op.op_id.clone() });
            }
        } else if held {
            counts.released += 1;
            out.releases.push(Unit::Release { op_id: op.op_id.clone() });
        }
    }
    out
}

#[cfg(test)]
mod tests;
