//! `judge` items split out of `own_data_proof.rs`.

use super::*;

/// What the proof concluded about the rows and the queue (pure: no database, no network).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Judgement {
    pub tally: Tally,
    /// The queued changes that act on proven data.
    pub proven: HashSet<String>,
}

/// Judge every server-named My Files row and every queued change against this account's tree.
pub fn judge(master_key: &[u8; 32], rows: &[ProofRow], ops: &[ProofOp], nodes: &[serde_json::Value]) -> Judgement {
    let tree: HashMap<&str, &serde_json::Value> = nodes
        .iter()
        .filter_map(|node| node.get("id").and_then(serde_json::Value::as_str).map(|id| (id, node)))
        .collect();
    let local: HashMap<&str, &ProofRow> = rows.iter().map(|row| (row.file_id.as_str(), row)).collect();
    // Where two rows hold one path (a folder made here and its server twin, task 1888), the server's row wins.
    let by_path: HashMap<&str, &ProofRow> = rows
        .iter()
        .filter(|row| !row.server_named)
        .chain(rows.iter().filter(|row| row.server_named))
        .map(|row| (row.path.trim_matches('/'), row))
        .collect();
    let judge = Judge {
        master_key,
        tree,
        local,
        by_path,
        probes: std::cell::RefCell::new(HashMap::new()),
    };

    let mut tally = Tally::default();
    for row in rows.iter().filter(|row| row.my_files && row.server_named) {
        tally.checked += 1;
        match judge.row(row) {
            Probe::Own => tally.own += 1,
            Probe::Neutral => tally.neutral += 1,
            Probe::Gone => tally.gone += 1,
            Probe::Unproven => tally.unproven += 1,
            Probe::Missing => tally.missing += 1,
        }
    }
    // Lead review of round 2, P1: a path proves a folder made here whose own path is an own folder's (its twin: its
    // create sends no content, and task 1888 folds it into that folder), never anything else (self-review of round 3,
    // F2: not a new folder inside it). Every other change needs one anchor tied by real parent links.
    let proven: HashSet<String> = ops
        .iter()
        .filter(|op| {
            let own = op.file_id.as_deref().map(|anchor| judge.anchor(anchor));
            let ties: Vec<Tie> = own.into_iter().chain(op.parent_id.as_deref().map(|anchor| judge.anchor(anchor))).collect();
            !ties.is_empty()
                && !ties.contains(&Tie::Not)
                && (ties.contains(&Tie::Linked) || (op.kind == "create_folder" && own == Some(Tie::Twin)))
        })
        .map(|op| op.op_id.clone())
        .collect();
    tally.proven_changes = proven.len();
    Judgement { tally, proven }
}

/// How a queued change's anchor is tied to this account's proven data.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Tie {
    /// Up real parent links to an own row (or one gone from an own folder).
    Linked,
    /// The anchor is a row made here at the path of an own row of the server's (its twin), and nothing else ties it.
    Twin,
    /// Only through a path otherwise: a twin further up, or the folder at a path (a row with no parent link, its own or
    /// one up the walk).
    ByPath,
    Not,
}

/// The tree and the local rows, indexed, with each present node's probe computed once.
pub(super) struct Judge<'a> {
    pub(super) master_key: &'a [u8; 32],
    pub(super) tree: HashMap<&'a str, &'a serde_json::Value>,
    pub(super) local: HashMap<&'a str, &'a ProofRow>,
    pub(super) by_path: HashMap<&'a str, &'a ProofRow>,
    pub(super) probes: std::cell::RefCell<HashMap<&'a str, Probe>>,
}

impl<'a> Judge<'a> {
    /// The probe of the node `id` of this account's tree, or `None` when the tree has no such node.
    pub(super) fn present(&self, id: &'a str) -> Option<Probe> {
        let node = self.tree.get(id)?;
        Some(*self.probes.borrow_mut().entry(id).or_insert_with(|| probe(self.master_key, node)))
    }

    /// The folder `row` is in: its parent link, or, for a row without one, the row at its path's folder; and whether
    /// it was found by the path.
    pub(super) fn parent_link(&self, row: &'a ProofRow) -> Option<(&'a str, bool)> {
        if let Some(parent) = row.parent_id.as_deref() {
            return Some((parent, false));
        }
        let (folder, _) = row.path.trim_matches('/').rsplit_once('/')?;
        self.by_path.get(folder).map(|parent| (parent.file_id.as_str(), true))
    }

    /// A server-named row: its node's probe, or, when this account's tree does not hold it, `Gone` if its nearest
    /// ancestor the tree holds is this account's own (deleted since), else `Missing`. The walk follows the local
    /// parents and ends as `Missing` at the top, at a parent with no row, or on a cycle.
    pub(super) fn row(&self, row: &'a ProofRow) -> Probe {
        self.row_tie(row).0
    }

    /// [`Self::row`], and whether the walk up to the ancestor that explains a row gone from the tree went through a
    /// folder found at a path (a row with no parent link): such a row is tied to that folder by its path only
    /// (self-review of round 3, F1).
    pub(super) fn row_tie(&self, row: &'a ProofRow) -> (Probe, bool) {
        if let Some(probe) = self.present(&row.file_id) {
            return (probe, false);
        }
        let mut seen: HashSet<&str> = HashSet::from([row.file_id.as_str()]);
        let mut current = row;
        let mut via_path = false;
        while let Some((id, found_by_path)) = self.parent_link(current) {
            if !seen.insert(id) {
                break;
            }
            via_path |= found_by_path;
            if let Some(probe) = self.present(id) {
                return (if probe == Probe::Own { Probe::Gone } else { Probe::Missing }, via_path);
            }
            match self.local.get(id) {
                Some(&parent) => current = parent,
                None => break,
            }
        }
        (Probe::Missing, via_path)
    }

    /// How a queued change's anchor (the item it acts on, or the folder it acts in) is tied to this account's proven
    /// data: the first judged row up its local parents (or at a row made here's own path, its twin) is in My Files and
    /// proven its own (or gone from its own folder). An item made on this PC at the top level, or in a folder shared
    /// with this account, or in one whose name proves nothing, is not. The tie is `Twin` when the anchor's own row
    /// is a twin's, and `ByPath` when the walk went through a twin further up or through the folder at a path (a row
    /// with no parent link), the walk up from a row gone from the tree included.
    pub(super) fn anchor(&self, anchor: &'a str) -> Tie {
        let mut seen: HashSet<&str> = HashSet::new();
        let mut id = anchor;
        let mut by_path = false;
        let tied = |proven: bool, by_path: bool| match (proven, by_path) {
            (false, _) => Tie::Not,
            (true, false) => Tie::Linked,
            (true, true) => Tie::ByPath,
        };
        let proves = |row: &'a ProofRow| {
            let (probe, via_path) = self.row_tie(row);
            (row.my_files && matches!(probe, Probe::Own | Probe::Gone), via_path)
        };
        loop {
            if !seen.insert(id) {
                return Tie::Not;
            }
            let Some(&row) = self.local.get(id) else {
                // No local row: a node of this account's tree this PC has not written yet.
                return tied(self.present(id) == Some(Probe::Own), by_path);
            };
            if row.server_named {
                let (proven, via_path) = proves(row);
                return tied(proven, by_path || via_path);
            }
            // A folder made here whose create never reached the server shares its path with the server's folder
            // (its twin, task 1888): it is that folder.
            if let Some(&twin) = self.by_path.get(row.path.trim_matches('/')).filter(|twin| twin.server_named) {
                let (proven, _) = proves(twin);
                return match tied(proven, true) {
                    Tie::ByPath if id == anchor => Tie::Twin,
                    tie => tie,
                };
            }
            match self.parent_link(row) {
                Some((parent, found_by_path)) => {
                    by_path |= found_by_path;
                    id = parent;
                }
                None => return Tie::Not,
            }
        }
    }
}
