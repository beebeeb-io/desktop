//! `scaffolding` items split out of `held_queue.rs`.

use super::*;

/// D5: the rows and ops at or under `Backups/unknown`, read once per plan. Only made when D5 applies: this PC's backup
/// folder has a name (D9) and it is not `unknown` under `≡`, so `Backups/unknown` is never this PC's own backup.
pub(super) struct Scaffolding<'a> {
    /// The rows there, and every row whose parent is one of them.
    pub(super) rows: Vec<&'a Row>,
    /// The ops that name one of those rows, as their file or their parent, or a target path there.
    pub(super) ops: Vec<&'a Op>,
    pub(super) case_insensitive: bool,
}

impl<'a> Scaffolding<'a> {
    pub(super) fn read(rows: &'a [Row], ops: &'a [Op], opts: &Options) -> Option<Self> {
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

    pub(super) fn holds(&self, path: &str) -> bool {
        at_or_under(path, BACKUPS_UNKNOWN, self.case_insensitive)
    }

    /// The unit that drops the subtree at `root` (a ghost here with no twin), with the ids of the ghosts and the ops it
    /// drops; `None` when the subtree is more than scaffolding. Scaffolding is ghosts only: every row at or under
    /// `root`, and every row under one of them, is a ghost not decided earlier in this plan (so no server folder is
    /// there, and none of them has a twin); every op on them is a ghost's `create_folder` the pass decides (none waits
    /// for the person); and the directory holds no file on disk.
    pub(super) fn drop_at(
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
