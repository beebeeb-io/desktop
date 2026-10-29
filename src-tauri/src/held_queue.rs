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
    /// A `needs_review` op whose cause is gone: due again.
    Release { op_id: String },
    /// D8 under a ghost kept for replay.
    Rearm { op_id: String },
    /// D6: the parent found by path.
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
    /// A `needs_review` ghost create whose cause is gone, due again.
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
}

/// The plan, in a safe apply order: landed drops, parent resolves, remaps by ascending depth of the ghost's path, then
/// holds, releases and re-arms. `payload_exists` says whether an upload's staged payload is still on disk;
/// `disk_has_file` whether a sync-root-relative directory holds any file (D5).
pub fn plan(
    rows: &[Row],
    ops: &[Op],
    opts: &Options,
    payload_exists: &dyn Fn(&str) -> bool,
    disk_has_file: &dyn Fn(&str) -> bool,
) -> (Vec<Unit>, Counts) {
    // D5 (Task 2) reads `disk_has_file`: true when the directory at that sync-root-relative path holds a file anywhere
    // under it, or cannot be read.
    let _ = disk_has_file;
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

    let mut ghosts: Vec<&Row> =
        rows.iter().filter(|r| r.is_folder && !r.server_known && create_of.contains_key(r.file_id.as_str())).collect();
    ghosts.sort_by_key(|r| depth(&r.path));
    let mut fate: HashMap<&str, Fate> = HashMap::new();
    let mut remaps: Vec<Unit> = Vec::new();
    let mut remap_at: HashMap<&str, usize> = HashMap::new();
    let (mut holds, mut releases, mut rearms) = (Vec::new(), Vec::new(), Vec::new());
    for ghost in ghosts {
        let id = ghost.file_id.as_str();
        let create = create_of[id];
        let parent_fate = ghost.parent_id.as_deref().and_then(|p| fate.get(p).copied());
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
            _ => ghost.parent_id.as_deref(),
        };
        if parent.is_none() && ghost.path.trim_matches('/').contains('/') {
            // A nested folder the scan wrote without a parent: D6 finds its parent by path first, never the top level.
            counts.parent_unresolved += 1;
            fate.insert(id, Fate::Stays);
            continue;
        }
        let twins: Vec<&Row> = server_folders
            .get(&parent)
            .map(|rows| rows.iter().copied().filter(|t| same_leaf(leaf(&t.path), leaf(&ghost.path), opts.case_insensitive)).collect())
            .unwrap_or_default();
        let held = if ops.iter().any(|op| op.file_id.as_deref() == Some(id) && op.op_id != create.op_id) {
            Some(&mut counts.held_acts_on_ghost)
        } else if create.parent_id.is_some() && create.parent_id != ghost.parent_id {
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

    // D8: an op re-pointed off a remapped ghost (its unit carries it) or under a ghost kept for replay, whose last error
    // is a parent 404, given up or still retrying. A park, a pause the pass owns (`auth`, `needs_review`) and any other
    // error stay as they are. D8 amendment (lead ruling 2026-10-29): an op paused for the person gets its retry budget
    // back and keeps its pause, so "Upload to my Beebeeb" (which only clears the pause) sends it; it is never re-labelled,
    // so a superseded one is left as it is.
    let mut rearmed: HashSet<&str> = HashSet::new();
    for op in ops {
        let Some(parent) = op.parent_id.as_deref() else { continue };
        let remap = match fate.get(parent) {
            Some(Fate::Remapped(_)) => Some(remap_at[parent]),
            Some(Fate::Replay) => None,
            _ => continue,
        };
        let person_paused = op.paused_reason.is_some() && !pass_decides(op);
        if (op.paused_reason.is_some() && !person_paused)
            || op.parked
            || landed_ops.contains(op.op_id.as_str())
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
    units.extend(remaps);
    units.extend(holds);
    units.extend(releases);
    units.extend(rearms);
    (units, counts)
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOT_FOUND: &str = "HTTP status client error (404 Not Found) for url (https://api.example/api/v1/uploads/init)";

    fn opts() -> Options {
        Options { device_segment: Some("PC-1".to_string()), case_insensitive: true }
    }

    fn folder(id: &str, parent: Option<&str>, path: &str, server_known: bool) -> Row {
        Row {
            file_id: id.to_string(),
            parent_id: parent.map(str::to_string),
            path: path.to_string(),
            is_folder: true,
            server_known,
            current_version: if server_known { 3 } else { 0 },
        }
    }

    fn file(id: &str, parent: Option<&str>, path: &str, server_known: bool, current_version: i64) -> Row {
        Row {
            file_id: id.to_string(),
            parent_id: parent.map(str::to_string),
            path: path.to_string(),
            is_folder: false,
            server_known,
            current_version,
        }
    }

    fn create_folder(op_id: &str, file_id: &str, parent: Option<&str>) -> Op {
        Op {
            op_id: op_id.to_string(),
            kind: "create_folder".to_string(),
            file_id: Some(file_id.to_string()),
            parent_id: parent.map(str::to_string),
            ..Op::default()
        }
    }

    fn upload(op_id: &str, file_id: &str, parent: Option<&str>) -> Op {
        Op {
            op_id: op_id.to_string(),
            kind: "upload_version".to_string(),
            file_id: Some(file_id.to_string()),
            parent_id: parent.map(str::to_string),
            create_file: true,
            payload_path: Some(format!("staged/{op_id}")),
            ..Op::default()
        }
    }

    fn gave_up(op: Op, error: &str) -> Op {
        Op { given_up: true, last_error: Some(error.to_string()), ..op }
    }

    fn paused(op: Op, reason: &str) -> Op {
        Op { paused_reason: Some(reason.to_string()), ..op }
    }

    fn always(_: &str) -> bool {
        true
    }

    fn run(rows: &[Row], ops: &[Op]) -> (Vec<Unit>, Counts) {
        plan(rows, ops, &opts(), &always, &always)
    }

    fn remap(ghost: &str, twin: &str, ghost_op: &str, rearm: &[&str]) -> Unit {
        remap_holding(ghost, twin, ghost_op, rearm, &[])
    }

    fn remap_holding(ghost: &str, twin: &str, ghost_op: &str, rearm: &[&str], hold: &[&str]) -> Unit {
        let ids = |ids: &[&str]| ids.iter().map(|id| id.to_string()).collect();
        Unit::Remap {
            ghost: ghost.to_string(),
            twin: twin.to_string(),
            ghost_op: ghost_op.to_string(),
            rearm: ids(rearm),
            hold: ids(hold),
        }
    }

    fn hold(op_id: &str) -> Unit {
        Unit::Hold { op_id: op_id.to_string() }
    }

    #[test]
    fn a_ghost_with_one_twin_moves_its_children_and_disappears() {
        let rows = [
            folder("t", None, "Photos", true),
            folder("g", None, "Photos", false),
            file("c", Some("g"), "Photos/a.jpg", false, 0),
        ];
        let ops = [create_folder("op-g", "g", None), upload("op-c", "c", Some("g"))];
        let (units, counts) = run(&rows, &ops);
        assert_eq!(units, vec![remap("g", "t", "op-g", &[])]);
        assert_eq!(counts, Counts { remapped: 1, ..Counts::default() });
    }

    #[test]
    fn nested_ghosts_are_remapped_top_down_in_one_pass() {
        let rows = [
            folder("g2", Some("g1"), "A/B", false), // listed first: the order comes from depth, not from the input
            folder("t1", None, "A", true),
            folder("t2", Some("t1"), "A/B", true),
            folder("g1", None, "A", false),
        ];
        let ops = [create_folder("op-g2", "g2", Some("g1")), create_folder("op-g1", "g1", None)];
        let (units, counts) = run(&rows, &ops);
        assert_eq!(units, vec![remap("g1", "t1", "op-g1", &[]), remap("g2", "t2", "op-g2", &[])]);
        assert_eq!(counts.remapped, 2);
    }

    #[test]
    fn a_ghost_with_no_twin_keeps_its_create_and_its_children_wait() {
        let rows = [folder("g", None, "New", false), file("c", Some("g"), "New/a.txt", false, 0)];
        let ops = [paused(create_folder("op-g", "g", None), "auth"), upload("op-c", "c", Some("g"))];
        let (units, counts) = run(&rows, &ops);
        assert!(units.is_empty(), "{units:?}");
        assert_eq!(counts, Counts { replay: 1, ..Counts::default() });
    }

    #[test]
    fn a_ghost_under_a_new_folder_replays_after_it() {
        let rows = [folder("g1", None, "New", false), folder("g2", Some("g1"), "New/Sub", false)];
        let ops = [create_folder("op-g1", "g1", None), create_folder("op-g2", "g2", Some("g1"))];
        let (units, counts) = run(&rows, &ops);
        assert!(units.is_empty(), "{units:?}");
        assert_eq!(counts, Counts { replay: 2, ..Counts::default() });
    }

    #[test]
    fn two_twins_hold_the_ghost_and_move_nothing() {
        let rows = [
            folder("t1", None, "Photos", true),
            folder("t2", None, "photos", true),
            folder("g", None, "Photos", false),
        ];
        let ops = [paused(create_folder("op-g", "g", None), "auth")];
        let (units, counts) = run(&rows, &ops);
        assert_eq!(units, vec![hold("op-g")]);
        assert_eq!(counts, Counts { held_ambiguous: 1, ..Counts::default() });
    }

    #[test]
    fn an_op_acting_on_the_ghost_itself_holds_it() {
        let rows = [folder("t", None, "Photos", true), folder("g", None, "Photos", false)];
        let rename = Op {
            op_id: "op-rename".to_string(),
            kind: "rename_file".to_string(),
            file_id: Some("g".to_string()),
            ..Op::default()
        };
        let ops = [create_folder("op-g", "g", None), rename];
        let (units, counts) = run(&rows, &ops);
        assert_eq!(units, vec![hold("op-g")], "the rename op is untouched");
        assert_eq!(counts, Counts { held_acts_on_ghost: 1, ..Counts::default() });
    }

    #[test]
    fn a_twin_with_its_own_pending_op_holds_the_ghost() {
        let rows = [folder("t", None, "Photos", true), folder("g", None, "Photos", false)];
        let trash = Op {
            op_id: "op-trash".to_string(),
            kind: "trash_file".to_string(),
            file_id: Some("t".to_string()),
            ..Op::default()
        };
        let ops = [create_folder("op-g", "g", None), trash];
        let (units, counts) = run(&rows, &ops);
        assert_eq!(units, vec![hold("op-g")]);
        assert_eq!(counts, Counts { held_twin_busy: 1, ..Counts::default() });
    }

    #[test]
    fn parents_that_disagree_hold_the_ghost() {
        let rows = [
            folder("s1", None, "X", true),
            folder("s2", None, "Y", true),
            folder("t", Some("s1"), "X/Photos", true),
            folder("g", Some("s1"), "X/Photos", false),
        ];
        let ops = [create_folder("op-g", "g", Some("s2"))];
        let (units, counts) = run(&rows, &ops);
        assert_eq!(units, vec![hold("op-g")]);
        assert_eq!(counts, Counts { held_parents_disagree: 1, ..Counts::default() });
    }

    #[test]
    fn a_landed_create_drops_only_its_op() {
        let rows = [folder("l", None, "Landed", true)];
        let ops = [create_folder("op-l", "l", None)];
        let (units, counts) = run(&rows, &ops);
        assert_eq!(units, vec![Unit::DropLanded { op_id: "op-l".to_string() }]);
        assert_eq!(counts, Counts { landed: 1, ..Counts::default() });
    }

    #[test]
    fn a_twin_seen_only_through_a_delta_is_found() {
        // A delta row: stamped `remote_updated_at`, `current_version` still 0. The reader marks it server-known.
        let twin = Row { current_version: 0, ..folder("t", None, "Photos", true) };
        let rows = [twin, folder("g", None, "Photos", false)];
        let ops = [create_folder("op-g", "g", None)];
        let (units, _) = run(&rows, &ops);
        assert_eq!(units, vec![remap("g", "t", "op-g", &[])]);
    }

    #[test]
    fn letter_case_matches_on_windows_only() {
        assert!(same_leaf("Photos", "photos", true));
        assert!(!same_leaf("Photos", "photos", false));
        assert!(same_leaf("Photos", "Photos", false));
        assert!(!same_leaf("Photos", "Photo", true));
    }

    #[test]
    fn a_parent_404_under_a_ghost_is_rearmed_and_nothing_else_is() {
        let rows = [
            folder("t", None, "Photos", true),
            folder("g", None, "Photos", false),
            file("f1", Some("g"), "Photos/1.jpg", false, 0),
            file("f2", Some("g"), "Photos/2.jpg", false, 0),
            file("f3", Some("g"), "Photos/3.jpg", false, 0),
            file("f4", Some("g"), "Photos/4.jpg", false, 0),
        ];
        let park = Op { parked: true, last_error: Some(NOT_FOUND.to_string()), ..upload("op-park", "f2", Some("g")) };
        let ops = [
            paused(create_folder("op-g", "g", None), "auth"),
            gave_up(upload("op-404", "f1", Some("g")), NOT_FOUND),
            park,
            gave_up(
                upload("op-500", "f3", Some("g")),
                "HTTP status server error (500 Internal Server Error) for url (https://api.example/api/v1/files/404a0000-0000-4000-8000-000000000001)",
            ),
            gave_up(upload("op-401", "f4", Some("g")), "HTTP status client error (401 Unauthorized) for url (https://api.example/x)"),
        ];
        let (units, counts) = run(&rows, &ops);
        assert_eq!(units, vec![remap("g", "t", "op-g", &["op-404"])]);
        assert_eq!(counts, Counts { remapped: 1, rearmed: 1, ..Counts::default() });
    }

    /// D8 amendment (lead ruling 2026-10-29): a change held for the person that gave up on the 404 its ghost caused gets
    /// its retry budget back in the remap, keeps its pause, and is never re-labelled; a park or another error is left.
    #[test]
    fn a_person_paused_upload_on_a_parent_404_under_a_remapped_ghost_gets_its_budget_back_and_keeps_its_pause() {
        let rows = [
            folder("t", None, "Photos", true),
            folder("g", None, "Photos", false),
            file("f1", Some("g"), "Photos/1.jpg", false, 0),
            file("f2", Some("g"), "Photos/2.jpg", false, 0),
            file("f3", Some("g"), "Photos/3.jpg", false, 0),
            file("f4", Some("g"), "Photos/4.jpg", false, 0),
        ];
        let not_found = |op: Op| Op { last_error: Some(NOT_FOUND.to_string()), ..op };
        let ops = [
            paused(create_folder("op-g", "g", None), "auth"),
            paused(not_found(upload("op-held", "f1", Some("g"))), "found_while_paused"),
            paused(not_found(upload("op-quota", "f2", Some("g"))), "quota"),
            paused(Op { parked: true, ..not_found(upload("op-park", "f3", Some("g"))) }, "found_while_paused"),
            paused(
                Op {
                    last_error: Some("HTTP status server error (500 Internal Server Error) for url (https://api.example/x)".to_string()),
                    ..upload("op-500", "f4", Some("g"))
                },
                "found_while_paused",
            ),
        ];
        let (units, counts) = run(&rows, &ops);
        assert_eq!(units, vec![remap("g", "t", "op-g", &["op-held", "op-quota"])]);
        assert_eq!(counts, Counts { remapped: 1, rearmed_paused: 2, ..Counts::default() });
    }

    /// D8 amendment: the same under a ghost kept for replay (no twin).
    #[test]
    fn a_person_paused_upload_on_a_parent_404_under_a_replayed_ghost_gets_its_budget_back() {
        let rows = [folder("g", None, "New", false), file("f1", Some("g"), "New/1.jpg", false, 0)];
        let ops = [
            create_folder("op-g", "g", None),
            paused(Op { last_error: Some(NOT_FOUND.to_string()), ..upload("op-held", "f1", Some("g")) }, "found_while_paused"),
        ];
        let (units, counts) = run(&rows, &ops);
        assert_eq!(units, vec![Unit::Rearm { op_id: "op-held".to_string() }]);
        assert_eq!(counts, Counts { replay: 1, rearmed_paused: 1, ..Counts::default() });
    }

    /// D8 amendment: a held change the server already superseded is left as it is: no budget, no `needs_review`.
    #[test]
    fn a_superseded_person_paused_upload_is_left_as_it_is() {
        let rows = [
            folder("t", None, "Photos", true),
            folder("g", None, "Photos", false),
            file("f1", Some("g"), "Photos/1.jpg", true, 4),
        ];
        let ops = [
            paused(create_folder("op-g", "g", None), "auth"),
            paused(Op { last_error: Some(NOT_FOUND.to_string()), ..upload("op-held", "f1", Some("g")) }, "found_while_paused"),
        ];
        let (units, counts) = run(&rows, &ops);
        assert_eq!(units, vec![remap("g", "t", "op-g", &[])]);
        assert_eq!(counts, Counts { remapped: 1, ..Counts::default() });
    }

    #[test]
    fn a_rearm_needs_its_staged_payload() {
        let rows = [
            folder("t", None, "Photos", true),
            folder("g", None, "Photos", false),
            file("f1", Some("g"), "Photos/1.jpg", false, 0),
        ];
        let ops = [create_folder("op-g", "g", None), gave_up(upload("op-404", "f1", Some("g")), NOT_FOUND)];
        let (units, counts) = plan(&rows, &ops, &opts(), &|_| false, &always);
        assert_eq!(units, vec![remap("g", "t", "op-g", &[])]);
        assert_eq!(counts, Counts { remapped: 1, rearm_no_payload: 1, ..Counts::default() });
    }

    #[test]
    fn an_upload_superseded_by_a_landed_version_is_held_not_rearmed() {
        let rows = [
            folder("t", None, "Photos", true),
            folder("g", None, "Photos", false),
            file("landed", Some("g"), "Photos/new.jpg", true, 1),
            file("newer", Some("g"), "Photos/old.jpg", true, 3),
        ];
        let behind = Op { create_file: false, base_version: Some(2), ..upload("op-ver", "newer", Some("g")) };
        let ops = [
            create_folder("op-g", "g", None),
            gave_up(upload("op-create", "landed", Some("g")), NOT_FOUND),
            gave_up(behind, NOT_FOUND),
        ];
        let (units, counts) = run(&rows, &ops);
        assert_eq!(units, vec![remap_holding("g", "t", "op-g", &[], &["op-create", "op-ver"])]);
        assert_eq!(counts, Counts { remapped: 1, rearm_superseded: 2, ..Counts::default() });
    }

    #[test]
    fn a_given_up_folder_under_a_new_folder_is_rearmed_and_replays() {
        // The 28 Oct shape, inside this device's own backup folder: never dropped.
        let rows = [
            folder("b", None, "Backups", true),
            folder("pc", Some("b"), "Backups/PC-1", true),
            folder("g1", Some("pc"), "Backups/PC-1/Docs", false),
            folder("g2", Some("g1"), "Backups/PC-1/Docs/2026", false),
        ];
        let ops = [
            paused(create_folder("op-g1", "g1", Some("pc")), "auth"),
            gave_up(create_folder("op-g2", "g2", Some("g1")), NOT_FOUND),
        ];
        let (units, counts) = run(&rows, &ops);
        assert_eq!(units, vec![Unit::Rearm { op_id: "op-g2".to_string() }]);
        assert_eq!(counts, Counts { replay: 2, rearmed: 1, ..Counts::default() });
    }

    #[test]
    fn a_held_ghost_is_remapped_once_its_ambiguity_is_gone() {
        let rows = [
            folder("t", None, "Photos", true),
            folder("g", None, "Photos", false),
            folder("t1", None, "Music", true),
            folder("t2", None, "music", true),
            folder("h", None, "Music", false),
        ];
        let ops = [
            paused(create_folder("op-g", "g", None), "needs_review"),
            paused(create_folder("op-h", "h", None), "needs_review"),
        ];
        let (units, counts) = run(&rows, &ops);
        assert_eq!(units, vec![remap("g", "t", "op-g", &[])], "the still-ambiguous one is neither held again nor released");
        assert_eq!(counts, Counts { remapped: 1, held_ambiguous: 1, ..Counts::default() });
    }

    #[test]
    fn a_held_ghost_whose_twins_are_gone_is_released_to_replay() {
        let rows = [folder("g", None, "Photos", false)];
        let ops = [paused(create_folder("op-g", "g", None), "needs_review")];
        let (units, counts) = run(&rows, &ops);
        assert_eq!(units, vec![Unit::Release { op_id: "op-g".to_string() }]);
        assert_eq!(counts, Counts { replay: 1, released: 1, ..Counts::default() });
    }

    #[test]
    fn a_ghost_under_a_held_ghost_waits_with_it() {
        let rows = [
            folder("t1", None, "Photos", true),
            folder("t2", None, "photos", true),
            folder("g", None, "Photos", false),
            folder("g-sub", Some("g"), "Photos/2026", false),
        ];
        let ops = [create_folder("op-g", "g", None), create_folder("op-sub", "g-sub", Some("g"))];
        let (units, counts) = run(&rows, &ops);
        assert_eq!(units, vec![hold("op-g")]);
        assert_eq!(counts, Counts { held_ambiguous: 1, waiting_under_held: 1, ..Counts::default() });
    }

    #[test]
    fn a_held_ghost_paused_for_another_reason_keeps_its_reason() {
        // 2267: a change found while sync was paused waits for the person; the pass never re-labels it.
        let rows = [folder("t1", None, "Photos", true), folder("t2", None, "photos", true), folder("g", None, "Photos", false)];
        let ops = [paused(create_folder("op-g", "g", None), "found_while_paused")];
        let (units, counts) = run(&rows, &ops);
        assert!(units.is_empty(), "{units:?}");
        assert_eq!(counts, Counts { left_paused: 1, ..Counts::default() });
    }

    #[test]
    fn a_person_paused_ghost_with_one_twin_is_left_alone_and_its_sub_ghost_waits() {
        let rows = [
            folder("t", None, "Photos", true),
            folder("g", None, "Photos", false),
            folder("g-sub", Some("g"), "Photos/2026", false),
        ];
        let ops = [
            paused(create_folder("op-g", "g", None), "found_while_paused"),
            create_folder("op-sub", "g-sub", Some("g")),
        ];
        let (units, counts) = run(&rows, &ops);
        assert!(units.is_empty(), "{units:?}");
        assert_eq!(counts, Counts { left_paused: 1, waiting_under_held: 1, ..Counts::default() });
    }

    #[test]
    fn a_nested_ghost_without_a_parent_id_is_never_matched_at_the_top_level() {
        // D6 (task 3) finds its parent by path first; until then it stays as it is.
        let rows = [
            folder("a", None, "A", true),
            folder("t", Some("a"), "A/Photos", true),
            folder("decoy", None, "Photos", true),
            folder("g", None, "A/photos", false),
        ];
        let ops = [create_folder("op-g", "g", None)];
        let (units, counts) = run(&rows, &ops);
        assert!(units.is_empty(), "{units:?}");
        assert_eq!(counts, Counts { parent_unresolved: 1, ..Counts::default() });
    }

    #[test]
    fn a_twin_whose_own_create_landed_is_not_busy() {
        let rows = [folder("t", None, "Photos", true), folder("g", None, "Photos", false)];
        let ops = [create_folder("op-t", "t", None), create_folder("op-g", "g", None)];
        let (units, counts) = run(&rows, &ops);
        assert_eq!(units, vec![Unit::DropLanded { op_id: "op-t".to_string() }, remap("g", "t", "op-g", &[])]);
        assert_eq!(counts, Counts { landed: 1, remapped: 1, ..Counts::default() });
    }

    #[test]
    fn a_replayed_ghost_whose_create_gave_up_on_something_else_is_reported_not_rearmed() {
        let rows = [folder("g", None, "New", false)];
        let ops = [gave_up(create_folder("op-g", "g", None), "HTTP status client error (403 Forbidden) for url (https://api.example/x)")];
        let (units, counts) = run(&rows, &ops);
        assert!(units.is_empty(), "{units:?}");
        assert_eq!(counts, Counts { replay: 1, replay_given_up: 1, ..Counts::default() });
    }

    #[test]
    fn a_parent_404_is_read_before_the_url() {
        assert!(is_parent_404(NOT_FOUND));
        assert!(!is_parent_404("HTTP status server error (500) for url (https://api.example/files/404a0000)"));
        assert!(!is_parent_404("connection reset"));
    }
}
