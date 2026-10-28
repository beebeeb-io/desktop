//! Task 2240 slice 2: the Windows tick's placeholder reconcile, decided from directory listings.
//!
//! Spec: `docs/superpowers/specs/2026-10-28-2240-reconcile-from-listing-design.md` (workspace repo).
//!
//! Each file row's attribute word comes from one listing of its parent folder instead of one
//! `GetFileAttributesW` per file (D1). The listing word is only a filter: a row whose listing word
//! asks for a change is read again directly, and only the direct word decides what is written (D2),
//! so a listing that lags can delay a detection but never cause a wrong write. A row the listing
//! does not cover falls back to the direct read (D3), and a rotating sample of rows is read directly
//! too, to measure lag in the field (D4).
//!
//! Platform-independent so the decision is unit-tested everywhere (D7); `windows_cf` supplies the
//! two closures and writes the deltas.

use std::collections::{BTreeMap, HashMap};

use crate::state_db::{FileEntry, FileStatus, PinState, decode_os_state};

/// Rows read directly per tick by the sentinel (D4): ≤ 3.5 ms at ~45 µs per call.
pub(crate) const SENTINEL_SAMPLE: usize = 64;
/// RECALL_ON_DATA_ACCESS | UNPINNED | PINNED: the only bits `decode_os_state` reads.
pub(crate) const OS_STATE_MASK: u32 = 0x0040_0000 | 0x0010_0000 | 0x0008_0000;

/// One row's change, in the shape `StateDb::reconcile_os_state` takes.
pub(crate) type Delta = (String, Option<FileStatus>, Option<PinState>);

/// What one reconcile pass did. Counts only: never a path (zero-knowledge).
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ReconcileCounts {
    pub rows: usize,
    pub dirs_listed: usize,
    pub listing_hits: usize,
    pub fallbacks: usize,
    pub list_errors: usize,
    pub candidates: usize,
    pub confirmed: usize,
    pub stale_candidates: usize,
    pub sentinel_checked: usize,
    pub sentinel_mismatches: usize,
}

pub(crate) struct ReconcilePlan {
    pub deltas: Vec<Delta>,
    pub counts: ReconcileCounts,
    pub next_cursor: usize,
}

/// The status and pin change one attribute word asks of a row, or `(None, None)`.
///
/// Status: only what `decode_os_state` decodes and only when it differs, so engine-owned statuses are
/// never overridden. Pin: the explicit per-file pin the OS carries, compared with the row's EFFECTIVE
/// pin, so an equivalent state never churns.
pub(crate) fn row_delta(attrs: u32, status: &FileStatus, pinned: bool) -> (Option<FileStatus>, Option<PinState>) {
    let (desired_status, desired_pin) = decode_os_state(attrs, status.clone());
    let status_delta = match desired_status {
        Some(s) if s != *status => Some(s),
        _ => None,
    };
    let pin_delta = match desired_pin {
        Some(PinState::Pinned) if !pinned => Some(PinState::Pinned),
        Some(PinState::Unpinned) if pinned => Some(PinState::Unpinned),
        _ => None,
    };
    (status_delta, pin_delta)
}

/// Decides every file row of `rows` (as `file_overview_rows` returns them).
///
/// `list_dir` takes a parent folder's '/'-joined path relative to the sync root (`""` for the root)
/// and returns each entry's exact name with its attribute word. `read_direct` takes a row's
/// '/'-joined relative path and returns its attribute word, or `None` when there is no placeholder.
/// Every delta returned was computed from a `read_direct` word (D2); deltas come back in row order.
pub(crate) fn plan_reconcile(
    rows: Vec<(FileEntry, bool)>,
    cursor: usize,
    mut list_dir: impl FnMut(&str) -> std::io::Result<HashMap<String, u32>>,
    mut read_direct: impl FnMut(&str) -> Option<u32>,
) -> ReconcilePlan {
    // File rows only (folders carry their own pin semantics; an empty path is the root), numbered in
    // input order, each with its '/'-joined path relative to the sync root.
    let files: Vec<(String, FileEntry, bool)> = rows
        .into_iter()
        .filter(|(entry, _)| !entry.is_dir())
        .filter_map(|(entry, pinned)| {
            let rel = entry.path.trim_matches('/').to_string();
            (!rel.is_empty()).then_some((rel, entry, pinned))
        })
        .collect();
    let n = files.len();
    let mut counts = ReconcileCounts {
        rows: n,
        ..ReconcileCounts::default()
    };
    if n == 0 {
        return ReconcilePlan {
            deltas: Vec::new(),
            counts,
            next_cursor: 0,
        };
    }
    let start = cursor % n;
    let window = SENTINEL_SAMPLE.min(n);
    let in_window = |i: usize| (i + n - start) % n < window;

    let mut by_parent: BTreeMap<&str, Vec<usize>> = BTreeMap::new();
    for (i, (rel, _, _)) in files.iter().enumerate() {
        let parent = rel.rsplit_once('/').map_or("", |(parent, _)| parent);
        by_parent.entry(parent).or_default().push(i);
    }

    let mut deltas: Vec<(usize, Delta)> = Vec::new();
    let mut push = |i: usize, attrs: u32| {
        let (_, entry, pinned) = &files[i];
        let (status, pin) = row_delta(attrs, &entry.status, *pinned);
        if status.is_some() || pin.is_some() {
            deltas.push((i, (entry.file_id.clone(), status, pin)));
            true
        } else {
            false
        }
    };

    for (parent, indices) in &by_parent {
        counts.dirs_listed += 1;
        let listing = match list_dir(parent) {
            Ok(listing) => Some(listing),
            Err(_) => {
                counts.list_errors += 1;
                None
            }
        };
        for &i in indices {
            let (rel, entry, pinned) = &files[i];
            let name = rel.rsplit_once('/').map_or(rel.as_str(), |(_, name)| name);
            let Some(&listed) = listing.as_ref().and_then(|listing| listing.get(name)) else {
                // D3: not covered by the listing, so decide it exactly as before.
                counts.fallbacks += 1;
                if let Some(direct) = read_direct(rel) {
                    push(i, direct);
                }
                continue;
            };
            counts.listing_hits += 1;
            let (status, pin) = row_delta(listed, &entry.status, *pinned);
            if status.is_some() || pin.is_some() {
                // D2: the listing only nominates; the direct word decides what is written.
                counts.candidates += 1;
                match read_direct(rel) {
                    Some(direct) if push(i, direct) => counts.confirmed += 1,
                    _ => counts.stale_candidates += 1,
                }
            } else if in_window(i) {
                // D4: measure lag the other way (a change the listing does not show yet).
                counts.sentinel_checked += 1;
                if let Some(direct) = read_direct(rel) {
                    if (direct ^ listed) & OS_STATE_MASK != 0 {
                        counts.sentinel_mismatches += 1;
                        push(i, direct);
                    }
                }
            }
        }
    }

    deltas.sort_by_key(|(i, _)| *i);
    ReconcilePlan {
        deltas: deltas.into_iter().map(|(_, delta)| delta).collect(),
        counts,
        next_cursor: (start + SENTINEL_SAMPLE) % n,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state_db::ItemKind;
    use std::cell::RefCell;

    const RECALL: u32 = 0x0040_0000;
    const PINNED: u32 = 0x0008_0000;
    const UNPINNED: u32 = 0x0010_0000;

    fn row(path: &str, status: FileStatus, pinned: bool) -> (FileEntry, bool) {
        (
            FileEntry {
                file_id: format!("id:{path}"),
                path: path.into(),
                status,
                size_bytes: 0,
                modified_at: 0,
                content_hash: None,
                remote_updated_at: 0,
                parent_id: None,
                item_kind: ItemKind::File,
            },
            pinned,
        )
    }

    fn local(path: &str) -> (FileEntry, bool) {
        row(path, FileStatus::Local, false)
    }

    /// A fake filesystem: listings per parent, direct words per row path, and a record of each call.
    #[derive(Default)]
    struct Fs {
        listings: HashMap<String, std::io::Result<HashMap<String, u32>>>,
        direct: HashMap<String, u32>,
        listed: RefCell<Vec<String>>,
        read: RefCell<Vec<String>>,
    }

    impl Fs {
        /// Every row listed in its parent with `word`, and read directly with the same `word`.
        fn fresh(paths: &[&str], word: u32) -> Self {
            let mut fs = Fs::default();
            for p in paths {
                fs.put(p, word, word);
            }
            fs
        }

        fn put(&mut self, path: &str, listing: u32, direct: u32) {
            let (parent, name) = path.rsplit_once('/').unwrap_or(("", path));
            let entry = self
                .listings
                .entry(parent.to_string())
                .or_insert_with(|| Ok(HashMap::new()));
            entry.as_mut().unwrap().insert(name.to_string(), listing);
            self.direct.insert(path.to_string(), direct);
        }

        fn run(&self, rows: Vec<(FileEntry, bool)>, cursor: usize) -> ReconcilePlan {
            plan_reconcile(
                rows,
                cursor,
                |parent| {
                    self.listed.borrow_mut().push(parent.to_string());
                    match self.listings.get(parent) {
                        Some(Ok(map)) => Ok(map.clone()),
                        Some(Err(e)) => Err(std::io::Error::new(e.kind(), "fake")),
                        None => Err(std::io::ErrorKind::NotFound.into()),
                    }
                },
                |path| {
                    self.read.borrow_mut().push(path.to_string());
                    self.direct.get(path).copied()
                },
            )
        }
    }

    fn many(n: usize) -> Vec<String> {
        (0..n).map(|i| format!("d/{i:03}")).collect()
    }

    #[test]
    fn t1_one_listing_per_dir_and_no_direct_reads_outside_the_sentinel() {
        let paths = ["a/1", "a/2", "b/3", "c/4", "c/5"];
        let fs = Fs::fresh(&paths, 0);
        let plan = fs.run(paths.iter().map(|p| local(p)).collect(), 0);
        let c = plan.counts;
        assert_eq!((c.rows, c.dirs_listed, c.listing_hits, c.fallbacks), (5, 3, 5, 0));
        assert_eq!(c.candidates, 0);
        assert!(plan.deltas.is_empty());
        assert_eq!(
            c.sentinel_checked, 5,
            "5 rows < SENTINEL_SAMPLE, so the window holds all of them"
        );
        assert_eq!(fs.read.borrow().len(), c.sentinel_checked);
        assert_eq!(fs.listed.borrow().len(), 3);
    }

    #[test]
    fn t2_case_variant_name_falls_back() {
        let paths = ["a/1", "a/b.txt", "b/3", "c/4", "c/5"];
        let mut fs = Fs::fresh(&["a/1", "b/3", "c/4", "c/5"], 0);
        fs.listings
            .get_mut("a")
            .unwrap()
            .as_mut()
            .unwrap()
            .insert("B.txt".into(), 0);
        fs.direct.insert("a/b.txt".into(), 0);
        let plan = fs.run(paths.iter().map(|p| local(p)).collect(), 0);
        assert_eq!((plan.counts.listing_hits, plan.counts.fallbacks), (4, 1));
        assert!(fs.read.borrow().iter().any(|p| p == "a/b.txt"));
    }

    #[test]
    fn t2b_two_rows_one_file_differing_in_case() {
        let mut fs = Fs::default();
        fs.put("a/x.txt", UNPINNED, UNPINNED);
        fs.direct.insert("a/X.txt".into(), UNPINNED);
        let rows = vec![
            row("a/x.txt", FileStatus::Local, true),
            row("a/X.txt", FileStatus::Local, true),
        ];
        let plan = fs.run(rows, 0);
        assert_eq!((plan.counts.listing_hits, plan.counts.fallbacks), (1, 1));
        assert_eq!(
            plan.deltas,
            vec![
                ("id:a/x.txt".to_string(), None, Some(PinState::Unpinned)),
                ("id:a/X.txt".to_string(), None, Some(PinState::Unpinned)),
            ]
        );
    }

    #[test]
    fn t3_listing_error_falls_back_for_that_dir_only() {
        let paths = ["a/1", "b/2", "c/3"];
        let mut fs = Fs::fresh(&paths, 0);
        fs.listings
            .insert("b".into(), Err(std::io::ErrorKind::PermissionDenied.into()));
        let plan = fs.run(paths.iter().map(|p| local(p)).collect(), 0);
        let c = plan.counts;
        assert_eq!((c.list_errors, c.fallbacks, c.listing_hits), (1, 1, 2));
        assert!(fs.read.borrow().iter().any(|p| p == "b/2"));
    }

    #[test]
    fn listing_error_in_one_dir_does_not_stop_the_next() {
        let mut fs = Fs::fresh(&["a/1", "c/3"], RECALL);
        fs.listings
            .insert("b".into(), Err(std::io::ErrorKind::PermissionDenied.into()));
        fs.direct.insert("b/2".into(), 0);
        let plan = fs.run(vec![local("a/1"), local("b/2"), local("c/3")], 0);
        assert_eq!(fs.listed.borrow().as_slice(), ["a", "b", "c"]);
        assert_eq!(
            plan.deltas,
            vec![
                ("id:a/1".to_string(), Some(FileStatus::CloudOnly), None),
                ("id:c/3".to_string(), Some(FileStatus::CloudOnly), None),
            ]
        );
    }

    #[test]
    fn t4_missing_placeholder_is_skipped() {
        let fs = Fs::default();
        let plan = fs.run(vec![local("gone/file")], 0);
        assert!(plan.deltas.is_empty());
        assert_eq!(
            (plan.counts.rows, plan.counts.fallbacks, plan.counts.listing_hits),
            (1, 1, 0)
        );
    }

    #[test]
    fn t5_stale_listing_never_unpins_an_app_pin() {
        let paths = many(70);
        let mut fs = Fs::fresh(&paths.iter().map(String::as_str).collect::<Vec<_>>(), 0);
        let mut rows: Vec<_> = paths.iter().map(|p| local(p)).collect();
        fs.put("d/069", UNPINNED, PINNED);
        rows[69] = row("d/069", FileStatus::Local, true);
        let plan = fs.run(rows, 0);
        let c = plan.counts;
        assert!(
            plan.deltas.is_empty(),
            "a delta came from the stale listing: {:?}",
            plan.deltas
        );
        assert_eq!((c.candidates, c.stale_candidates, c.confirmed), (1, 1, 0));
    }

    #[test]
    fn t5b_candidate_whose_file_vanished_is_not_written() {
        let mut fs = Fs::default();
        fs.put("a/f", UNPINNED, 0);
        fs.direct.remove("a/f");
        let plan = fs.run(vec![row("a/f", FileStatus::Local, true)], 0);
        assert!(plan.deltas.is_empty());
        assert_eq!((plan.counts.candidates, plan.counts.stale_candidates), (1, 1));
    }

    #[test]
    fn t6_stale_listing_free_up_found_only_by_the_sentinel() {
        let paths = many(65);
        let mut fs = Fs::fresh(&paths.iter().map(String::as_str).collect::<Vec<_>>(), 0);
        fs.put("d/064", 0, RECALL);
        let rows = || paths.iter().map(|p| local(p)).collect::<Vec<_>>();

        let outside = fs.run(rows(), 0);
        assert!(outside.deltas.is_empty());
        assert!(
            !fs.read.borrow().iter().any(|p| p == "d/064"),
            "row 64 is outside the window at cursor 0"
        );
        assert_eq!(outside.counts.sentinel_mismatches, 0);

        fs.read.borrow_mut().clear();
        let inside = fs.run(rows(), 64);
        assert_eq!(
            inside.deltas,
            vec![("id:d/064".to_string(), Some(FileStatus::CloudOnly), None)]
        );
        assert_eq!(inside.counts.sentinel_mismatches, 1);
    }

    #[test]
    fn t7_sentinel_window_rotates_and_wraps() {
        let paths = many(70);
        let fs = Fs::fresh(&paths.iter().map(String::as_str).collect::<Vec<_>>(), 0);
        let rows = || paths.iter().map(|p| local(p)).collect::<Vec<_>>();

        let tick1 = fs.run(rows(), 0);
        assert_eq!(tick1.counts.sentinel_checked, 64);
        assert_eq!(*fs.read.borrow(), paths[0..64].to_vec());
        assert_eq!(tick1.next_cursor, 64);

        fs.read.borrow_mut().clear();
        let tick2 = fs.run(rows(), 64);
        let mut want: Vec<String> = paths[0..58].to_vec();
        want.extend_from_slice(&paths[64..70]);
        let mut got = fs.read.borrow().clone();
        got.sort();
        want.sort();
        assert_eq!(got, want);
        assert_eq!(tick2.counts.sentinel_checked, 64);
        assert_eq!(tick2.next_cursor, 58);
    }

    #[test]
    fn t7b_cursor_beyond_row_count_wraps() {
        let paths = ["a/1", "a/2", "a/3", "a/4", "a/5"];
        let fs = Fs::fresh(&paths, 0);
        let plan = fs.run(paths.iter().map(|p| local(p)).collect(), 1_000);
        assert_eq!(plan.counts.sentinel_checked, 5);
        assert_eq!(plan.next_cursor, (1_000 % 5 + SENTINEL_SAMPLE) % 5);

        let empty = Fs::default();
        let plan = empty.run(Vec::new(), 1_000);
        assert_eq!(plan.next_cursor, 0);
        assert_eq!(plan.counts, ReconcileCounts::default());
        assert!(empty.listed.borrow().is_empty() && empty.read.borrow().is_empty());
    }

    /// Today's rule, written out (windows_cf/mod.rs:1297-1316 at 60bd1eb), as the reference.
    fn reference_delta(attrs: u32, status: &FileStatus, pinned: bool) -> (Option<FileStatus>, Option<PinState>) {
        let (desired_status, desired_pin) = decode_os_state(attrs, status.clone());
        let status_delta = match desired_status {
            Some(s) if s != *status => Some(s),
            _ => None,
        };
        let pin_delta = match desired_pin {
            Some(PinState::Pinned) if !pinned => Some(PinState::Pinned),
            Some(PinState::Unpinned) if pinned => Some(PinState::Unpinned),
            _ => None,
        };
        (status_delta, pin_delta)
    }

    #[test]
    fn t8_equivalent_to_todays_rule_when_fresh() {
        let statuses = [
            FileStatus::Local,
            FileStatus::CloudOnly,
            FileStatus::Uploading,
            FileStatus::Error,
            FileStatus::Trashing,
        ];
        let mut fs = Fs::default();
        let mut rows = Vec::new();
        let mut want = Vec::new();
        for bits in 0..8u32 {
            let attrs = (if bits & 1 != 0 { RECALL } else { 0 })
                | (if bits & 2 != 0 { PINNED } else { 0 })
                | (if bits & 4 != 0 { UNPINNED } else { 0 });
            for (s, status) in statuses.iter().enumerate() {
                for pinned in [false, true] {
                    let path = format!("d{bits}/s{s}p{}", pinned as u8);
                    fs.put(&path, attrs, attrs);
                    let (st, pin) = reference_delta(attrs, status, pinned);
                    if st.is_some() || pin.is_some() {
                        want.push((format!("id:{path}"), st, pin));
                    }
                    rows.push(row(&path, status.clone(), pinned));
                }
            }
        }
        assert!(!want.is_empty());
        let plan = fs.run(rows, 0);
        assert_eq!(plan.deltas, want);
        assert_eq!(plan.counts.sentinel_mismatches, 0);
        assert_eq!(plan.counts.stale_candidates, 0);
    }

    #[test]
    fn t9_confirmed_delta_is_written() {
        let mut fs = Fs::default();
        fs.put("a/f", UNPINNED, UNPINNED);
        let plan = fs.run(vec![row("a/f", FileStatus::Local, true)], 0);
        assert_eq!(
            plan.deltas,
            vec![("id:a/f".to_string(), None, Some(PinState::Unpinned))]
        );
        assert_eq!(plan.counts.confirmed, 1);
    }

    #[test]
    fn counts_partition_the_rows() {
        let paths = ["a/1", "b/2", "c/3", "c/4"];
        let mut fs = Fs::fresh(&paths, 0);
        fs.listings
            .insert("b".into(), Err(std::io::ErrorKind::PermissionDenied.into()));
        let mut rows: Vec<_> = paths.iter().map(|p| local(p)).collect();
        let mut folder = local("a");
        folder.0.item_kind = ItemKind::Folder;
        rows.push(folder);
        rows.push(local("/"));
        let plan = fs.run(rows, 0);
        let c = plan.counts;
        assert_eq!(c.rows, 4, "folders and empty paths are not file rows");
        assert_eq!(c.listing_hits + c.fallbacks, c.rows);
    }
}
