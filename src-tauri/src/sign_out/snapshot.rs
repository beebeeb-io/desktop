//! Spec 2026-10-10 §2 "Ask snapshot" and W5 (task 1887): what one ask showed the person, held by Rust, never by the
//! webview, and the gate's comparison of the queue with it.
#![cfg_attr(not(any(target_os = "macos", test)), allow(dead_code))]

use std::collections::BTreeSet;
use std::sync::{Mutex, MutexGuard};

use super::waiting::WaitingChanges;

/// What the webview renders (§5.1, the contract).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AskCounts {
    pub ask_id: String,
    pub files: usize,
    pub copied_files: usize,
    pub others: usize,
    pub missing_files: usize,
    /// §7.3: every interrupted sign-out's folder, oldest first, only when the ask shows a missing write.
    pub earlier_folders: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AskSnapshot {
    pub counts: AskCounts,
    /// The label of the window it was counted for.
    pub owner: String,
    pub files: BTreeSet<String>,
    pub others: BTreeSet<String>,
    pub missing: BTreeSet<String>,
    /// §6.1 condition 5: at least one waiting change is live (the count's `can_send` reads it, W2: no second read).
    pub any_live: bool,
}

impl AskSnapshot {
    pub fn of(waiting: &WaitingChanges, owner: &str, earlier_folders: Vec<String>) -> Self {
        let counts = waiting.counts();
        Self {
            counts: AskCounts {
                ask_id: uuid::Uuid::new_v4().to_string(),
                files: counts.files,
                copied_files: counts.copied_files,
                others: counts.others,
                missing_files: counts.missing_files,
                earlier_folders: if counts.missing_files > 0 {
                    earlier_folders
                } else {
                    Vec::new()
                },
            },
            owner: owner.to_string(),
            files: waiting.file_ids(),
            others: waiting.other_op_ids(),
            missing: waiting.missing_op_ids(),
            any_live: waiting.any_live(),
        }
    }

    pub fn ask_id(&self) -> &str {
        &self.counts.ask_id
    }

    pub fn is_empty(&self) -> bool {
        self.files.is_empty() && self.others.is_empty()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GateRefusal;

/// The one newest snapshot (§2): a newer count replaces it; the gate takes it out; a close of its owner drops it; a
/// completed sign-out empties the store.
#[derive(Default)]
pub struct SnapshotStore {
    newest: Mutex<Option<AskSnapshot>>,
}

impl SnapshotStore {
    fn lock(&self) -> MutexGuard<'_, Option<AskSnapshot>> {
        self.newest.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    pub fn record(&self, snapshot: AskSnapshot) {
        *self.lock() = Some(snapshot);
    }

    pub fn newest_ask_id(&self) -> Option<String> {
        self.lock().as_ref().map(|snapshot| snapshot.ask_id().to_string())
    }

    /// A close or reload of the window `label`. Another window's snapshot is left alone.
    pub fn drop_owned_by(&self, label: &str) -> bool {
        let mut newest = self.lock();
        if newest.as_ref().is_some_and(|snapshot| snapshot.owner == label) {
            *newest = None;
            return true;
        }
        false
    }

    pub fn clear(&self) {
        *self.lock() = None;
    }

    /// W5. Refuses when changes wait and no snapshot is named; when the named one is not the newest; or when the queue
    /// holds a waiting file, an other change or a missing write it did not hold. A newer write to a held file does
    /// not refuse (the autosave case). Accepting takes the snapshot out of the store and hands it over by value; a
    /// refusal leaves the store as it was.
    pub fn gate(
        &self,
        waiting: &WaitingChanges,
        keep_unsent: Option<&str>,
    ) -> Result<Option<AskSnapshot>, GateRefusal> {
        let mut newest = self.lock();
        let Some(ask_id) = keep_unsent else {
            return if waiting.is_empty() { Ok(None) } else { Err(GateRefusal) };
        };
        let Some(snapshot) = newest.as_ref().filter(|snapshot| snapshot.ask_id() == ask_id) else {
            return Err(GateRefusal);
        };
        let held = waiting.file_ids().is_subset(&snapshot.files)
            && waiting.other_op_ids().is_subset(&snapshot.others)
            && waiting.missing_op_ids().is_subset(&snapshot.missing);
        if !held {
            return Err(GateRefusal);
        }
        Ok(newest.take())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sign_out::waiting::{OtherChange, WaitingChanges, WaitingFile, WaitingWrite, WriteState};
    use crate::state_db::WriteOrigin;

    fn waiting(files: &[(&str, &[(&str, WriteState)])], others: &[&str]) -> WaitingChanges {
        WaitingChanges {
            files: files
                .iter()
                .map(|(file_id, writes)| WaitingFile {
                    file_id: file_id.to_string(),
                    path: None,
                    writes: writes
                        .iter()
                        .enumerate()
                        .map(|(i, (op_id, state))| WaitingWrite {
                            op_id: op_id.to_string(),
                            payload_path: None,
                            origin: WriteOrigin::Minted,
                            queue_order: i as i64,
                            live: true,
                            state: *state,
                        })
                        .collect(),
                })
                .collect(),
            others: others
                .iter()
                .map(|op_id| OtherChange {
                    op_id: op_id.to_string(),
                    live: true,
                })
                .collect(),
        }
    }

    /// U37, the gate's comparison (the sign-out tests in `lib.rs` add the keep step's snapshot).
    #[test]
    fn the_gate_compares_the_queue_with_the_snapshot() {
        use WriteState::*;
        let asked = waiting(&[("f1", &[("w1", Copied)])], &["r1"]);
        let store = SnapshotStore::default();
        assert_eq!(
            store.gate(&asked, None),
            Err(GateRefusal),
            "changes wait and no keepUnsent"
        );
        let snapshot = AskSnapshot::of(&asked, "macos-settings", Vec::new());
        let id = snapshot.ask_id().to_string();
        store.record(snapshot);
        assert_eq!(store.gate(&asked, Some("stale")), Err(GateRefusal));
        assert_eq!(
            store.newest_ask_id().as_deref(),
            Some(id.as_str()),
            "a refusal leaves the store as it was"
        );
        let new_file = waiting(&[("f1", &[("w1", Copied)]), ("f2", &[("w2", Copied)])], &["r1"]);
        assert_eq!(
            store.gate(&new_file, Some(&id)),
            Err(GateRefusal),
            "a waiting file it did not hold"
        );
        let new_other = waiting(&[("f1", &[("w1", Copied)])], &["r1", "r2"]);
        assert_eq!(
            store.gate(&new_other, Some(&id)),
            Err(GateRefusal),
            "an other change it did not hold"
        );
        let new_missing = waiting(&[("f1", &[("w1", Copied), ("w9", Missing)])], &["r1"]);
        assert_eq!(
            store.gate(&new_missing, Some(&id)),
            Err(GateRefusal),
            "a missing write it did not hold"
        );
        let autosave = waiting(&[("f1", &[("w1", Covered), ("w2", Copied)])], &["r1"]);
        let accepted = store
            .gate(&autosave, Some(&id))
            .expect("a newer write to a held file never refuses");
        assert_eq!(accepted.map(|s| s.ask_id().to_string()), Some(id));
        assert_eq!(
            store.newest_ask_id(),
            None,
            "the gate takes the snapshot out of the store"
        );
        assert_eq!(
            store.gate(&WaitingChanges::default(), None),
            Ok(None),
            "nothing waits, no keepUnsent: no refusal"
        );
    }

    #[test]
    fn earlier_folders_ride_only_an_ask_that_shows_a_missing_write() {
        use WriteState::*;
        let folders = vec!["/kept/one".to_string()];
        let none_missing = AskSnapshot::of(&waiting(&[("f1", &[("w1", Copied)])], &[]), "o", folders.clone());
        assert!(none_missing.counts.earlier_folders.is_empty());
        let missing = AskSnapshot::of(&waiting(&[("f1", &[("w1", Missing)])], &[]), "o", folders.clone());
        assert_eq!(missing.counts.earlier_folders, folders);
        let json = serde_json::to_value(&missing.counts).unwrap();
        for key in [
            "askId",
            "files",
            "copiedFiles",
            "others",
            "missingFiles",
            "earlierFolders",
        ] {
            assert!(json.get(key).is_some(), "{key} in {json}");
        }
    }
}
