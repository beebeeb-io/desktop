// Task 2274: the duplicates repair tool's premises, on the app's own apply paths (spec D1, D2; plan T2, T3). A and
// B are two rows at one path, and the file there is A's. The tool forgets B's row before it trashes B, because what
// the app does with B's trash (the `file_trash` echo, or a later snapshot that no longer lists B) acts on B's stored
// path, which is A's file. The tests are hosted in `engine_bridge`'s tests for its private apply functions.
//
// On Windows the apply's own removal runs on the file (a plain file here: every byte on disk, as A's hydrated file
// is to a row with no agreed content). Elsewhere that removal is a no-op, so the tests check which rows the apply
// hands to it, and run the shared removal (`remove_pruned_entries`) with the view Windows reports.
// Included into `engine_bridge`'s tests (`include!`), so these are plain comments.
use super::*;

const A: &str = "a0000000-0000-4000-8000-0000000000a1";
const B: &str = "b0000000-0000-4000-8000-0000000000b1";
const PATH: &str = "dup.txt";
const BYTES: &[u8] = b"A's bytes."; // 10 bytes: the size `seed_bridge_entry` records
const MK: [u8; 32] = [7u8; 32]; // `test_bridge`'s master key

/// This PC with the duplicate: A's row (local), B's row (cloud only), both at `dup.txt`, and A's file on disk.
struct Pc {
    _dir: tempfile::TempDir,
    root: PathBuf,
    bridge: EngineBridge,
    a_before: String,
}

fn pc() -> Pc {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("root");
    std::fs::create_dir_all(&root).unwrap();
    let bridge = test_bridge(&dir.path().join("state.db"));
    seed_bridge_entry(&bridge, A, PATH, None, FileStatus::Local, false, 10);
    seed_bridge_entry(&bridge, B, PATH, None, FileStatus::CloudOnly, false, 10);
    std::fs::write(root.join(PATH), BYTES).unwrap();
    let a_before = a_row(&bridge);
    Pc { _dir: dir, root, bridge, a_before }
}

/// A's row and contract state, every field.
fn a_row(bridge: &EngineBridge) -> String {
    format!("{:?} {:?}", bridge.db().get_file(A).unwrap(), bridge.db().get_file_contract_state(A).unwrap())
}

/// Every file under the root, relative and sorted.
fn listing(root: &Path) -> Vec<String> {
    fn walk(root: &Path, dir: &Path, out: &mut Vec<String>) {
        for entry in std::fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                walk(root, &path, out);
            } else {
                out.push(rel_under(root, &path));
            }
        }
    }
    let mut out = Vec::new();
    walk(root, root, &mut out);
    out.sort();
    out
}

/// B's row as the apply hands it to the removal (`mark_subtree_trashing`, `prune_absent`): its stored path, no agreed
/// content (B's row records none).
#[cfg_attr(target_os = "windows", allow(dead_code))]
fn pruned(b: &FileEntry) -> crate::state_db::PrunedRow {
    assert!(b.content_hash.is_none(), "B's row records no content");
    crate::state_db::PrunedRow {
        file_id: b.file_id.clone(),
        path: b.path.clone(),
        is_dir: false,
        local_hash: None,
        fetched_hash: None,
    }
}

impl Pc {
    /// A's file is where it was, with its bytes, and nothing else is in the folder.
    fn a_untouched(&self) -> bool {
        listing(&self.root) == [PATH] && std::fs::read(self.root.join(PATH)).ok().as_deref() == Some(BYTES)
    }

    /// The app applies the server's `file_trash` op for B (the echo of the tool's soft trash).
    fn echo(&self) -> Vec<String> {
        let op = crate::api_client::SyncOp {
            seq_id: 12,
            op_type: "file_trash".into(),
            payload: serde_json::json!({ "id": B }),
        };
        apply_sync_op(&self.bridge, &self.root, &op, 200, &mut Vec::new()).unwrap()
    }

    /// The app applies a snapshot listing `ids`, all named `dup.txt` at the root.
    fn snapshot(&self, ids: &[&str]) -> Vec<String> {
        let nodes = ids.iter().map(|id| snap_node(&MK, id, PATH, None, false, 10)).collect();
        let snapshot = crate::api_client::SyncSnapshot { seq_id: 13, nodes };
        apply_snapshot(&self.bridge, &self.root, &snapshot, 200, 200, &mut Vec::new()).unwrap()
    }

    /// Off Windows the apply's removal is a no-op: run the shared removal on what the apply handed it, with the view
    /// Windows reports for A's file under a row with no agreed content.
    #[cfg(not(target_os = "windows"))]
    fn remove_as_windows_does(&self, rows: &[crate::state_db::PrunedRow]) {
        remove_pruned_entries(self.bridge.db(), &self.root, rows, &|_, _| LocalEntry::WhollyLocal);
    }
}

/// T2, kept as a guard on the premise (D1, 2283): with B's row still present, the echo of B's trash removes A's file
/// (Windows keeps a copy of bytes it cannot match, under another name). If this test ever fails, the app no longer
/// acts on B's path, and the order the tool depends on can be revisited.
#[test]
fn trashing_b_before_forgetting_its_row_touches_a() {
    let pc = pc();
    let applied = pc.echo();
    assert!(applied.iter().any(|id| id == B), "the echo applied B's trash: {applied:?}");
    let b = pc.bridge.db().get_file(B).unwrap().expect("B's row stays, in the trash view");
    assert_eq!((&b.status, b.path.as_str()), (&FileStatus::Trashing, PATH), "B's trash is applied at A's path");
    #[cfg(not(target_os = "windows"))]
    pc.remove_as_windows_does(&[pruned(&b)]);
    assert!(!pc.a_untouched(), "A's file was left alone: {:?}", listing(&pc.root));
    assert!(std::fs::read(pc.root.join(PATH)).is_err(), "A's file is no longer at its path: {:?}", listing(&pc.root));
}

/// T1's echo (D2): once B's row is forgotten, the echo of B's trash finds no row and hands nothing to the removal; A's
/// row and A's file stay exactly as they were.
#[test]
fn forgetting_b_first_keeps_the_echo_off_a() {
    let pc = pc();
    pc.bridge.db().delete_file(B).unwrap();
    let applied = pc.echo();
    assert!(applied.is_empty(), "the echo removed nothing: {applied:?}");
    assert!(pc.bridge.db().get_file(B).unwrap().is_none(), "B's row stays forgotten");
    assert_eq!(a_row(&pc.bridge), pc.a_before, "A's row is unchanged in every field");
    assert!(pc.a_untouched(), "A's file is unchanged: {:?}", listing(&pc.root));
}

/// T3: the tool stops right after forgetting B's row (no trash sent). A snapshot that still lists B brings B's row
/// back, as the app had it, and leaves A's file alone: stopping there loses nothing.
#[test]
fn a_snapshot_after_the_forget_brings_b_back_and_leaves_a() {
    let pc = pc();
    pc.bridge.db().delete_file(B).unwrap();
    let applied = pc.snapshot(&[A, B]);
    assert!(applied.iter().any(|id| id == B), "the snapshot ingested B again: {applied:?}");
    let b = pc.bridge.db().get_file(B).unwrap().expect("B's row is back");
    assert_eq!(b.path, PATH, "B's row is back at the path it shared with A");
    let a = pc.bridge.db().get_file(A).unwrap().expect("A's row stays");
    assert_eq!((a.path.as_str(), a.status), (PATH, FileStatus::Local), "A's row keeps its path and state");
    assert!(pc.a_untouched(), "A's file is unchanged: {:?}", listing(&pc.root));
}

/// The same shape through the snapshot (found while tracing 0.8.20, 2026-10-29): B already in the server's trash (the
/// L6 pairs) and its row still present here. The next snapshot does not list B, so its prune hands B's path, A's
/// file, to the removal.
#[test]
fn a_snapshot_without_b_prunes_b_at_a_path_while_its_row_is_present() {
    let pc = pc();
    let b = pc.bridge.db().get_file(B).unwrap().unwrap();
    assert_eq!(b.path, PATH, "B's row is stored at A's path");
    let applied = pc.snapshot(&[A]);
    assert!(applied.iter().any(|id| id == B), "the snapshot pruned B: {applied:?}");
    assert!(pc.bridge.db().get_file(B).unwrap().is_none(), "B's row is gone");
    #[cfg(not(target_os = "windows"))]
    pc.remove_as_windows_does(&[pruned(&b)]);
    assert!(std::fs::read(pc.root.join(PATH)).is_err(), "A's file is no longer at its path: {:?}", listing(&pc.root));
}

/// And forgetting B first leaves that snapshot nothing to prune.
#[test]
fn forgetting_b_first_leaves_a_snapshot_without_b_nothing_to_prune() {
    let pc = pc();
    pc.bridge.db().delete_file(B).unwrap();
    let applied = pc.snapshot(&[A]);
    assert!(!applied.iter().any(|id| id == B), "nothing of B's was pruned: {applied:?}");
    let a = pc.bridge.db().get_file(A).unwrap().expect("A's row stays");
    assert_eq!((a.path.as_str(), a.status), (PATH, FileStatus::Local), "A's row keeps its path and state");
    assert!(pc.a_untouched(), "A's file is unchanged: {:?}", listing(&pc.root));
}
