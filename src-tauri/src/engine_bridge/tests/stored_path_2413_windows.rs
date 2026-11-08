//! Task 2413, Windows: against the real file system, nothing outside the sync folder is touched. A remote delete of a
//! stored row, the placeholder pass and the hydrate writer each meet a path that would resolve outside the root (an
//! already-stored row, a junction made locally inside the folder); each test first proves its fixture reaches outside.
//! The store refuses Windows-only names with this build's own rules (no test override).

use super::*;

/// A scratch folder holding `root` (the sync folder) and `outside` (beside it), with `outside/precious.txt`.
struct Folders {
    _dir: tempfile::TempDir,
    root: PathBuf,
    outside: PathBuf,
}

impl Folders {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("root");
        let outside = dir.path().join("outside");
        std::fs::create_dir_all(root.join("docs")).unwrap();
        std::fs::create_dir_all(&outside).unwrap();
        std::fs::write(outside.join("precious.txt"), b"outside bytes").unwrap();
        std::fs::write(outside.join("~$ecious.txt"), b"owner file").unwrap();
        Self { _dir: dir, root, outside }
    }

    fn outside_untouched(&self) {
        assert_eq!(std::fs::read(self.outside.join("precious.txt")).unwrap(), b"outside bytes");
        assert_eq!(std::fs::read(self.outside.join("~$ecious.txt")).unwrap(), b"owner file");
    }

    /// A junction at `link` (inside the root) to `outside`; needs no special rights.
    fn junction(&self, link: &Path) {
        let output = std::process::Command::new("cmd.exe")
            .args(["/C", "mklink", "/J"])
            .arg(link)
            .arg(&self.outside)
            .output()
            .unwrap();
        assert!(output.status.success(), "junction fixture failed: {output:?}");
        assert_eq!(
            std::fs::read(link.join("precious.txt")).unwrap(),
            b"outside bytes",
            "the fixture must reach outside"
        );
    }
}

/// A row stored before this check, whose path resolves outside the root (`docs\..\..\outside\precious.txt`).
const OUTSIDE_REL: &str = "docs\\..\\..\\outside\\precious.txt";

#[test]
fn task_2413_windows_a_remote_delete_never_removes_a_file_outside_the_sync_folder() {
    let f = Folders::new();
    assert!(f.root.join(OUTSIDE_REL).exists(), "the fixture path must resolve outside");
    let bridge = test_bridge(&f.root.parent().unwrap().join("state.db"));
    let id = "a1300000-0000-4000-8000-000000000001";
    seed_bridge_entry(&bridge, id, OUTSIDE_REL, None, FileStatus::Local, false, 10);
    let rows = bridge.db().mark_subtree_trashing(id).unwrap();
    assert_eq!(rows.len(), 1);
    remove_pruned_entries(bridge.db(), &f.root, &rows, &|_, _| LocalEntry::InSync);
    f.outside_untouched();
}

#[test]
fn task_2413_windows_the_placeholder_pass_never_makes_a_folder_outside_the_sync_folder() {
    let f = Folders::new();
    let made = f.outside.parent().unwrap().join("made-outside");
    let bridge = test_bridge(&f.root.parent().unwrap().join("state.db"));
    let id = "a1300000-0000-4000-8000-000000000002";
    seed_bridge_entry(&bridge, id, "docs\\..\\..\\made-outside\\f.txt", None, FileStatus::CloudOnly, false, 10);
    crate::windows_cf::restore_placeholders(bridge.db(), &f.root);
    assert!(!made.exists(), "no folder outside the sync folder");
}

/// A junction made inside the sync folder under a server folder's name: `root\Plans` reaches `outside`.
fn plans_junction(f: &Folders) -> PathBuf {
    let link = f.root.join("Plans");
    f.junction(&link);
    link
}

/// A remote delete of a server folder `Plans` and its file, where `Plans` on disk is a junction to `outside`: nothing
/// is removed through it (not the target's file, not its Office owner file), and the junction stays as it is.
#[test]
fn task_2413_windows_a_remote_delete_through_a_junction_leaves_its_target() {
    let f = Folders::new();
    let link = plans_junction(&f);
    let bridge = test_bridge(&f.root.parent().unwrap().join("state.db"));
    let folder = "a1320000-0000-4000-8000-000000000001";
    let file = "a1320000-0000-4000-8000-000000000002";
    seed_bridge_entry(&bridge, folder, "Plans", None, FileStatus::CloudOnly, true, 0);
    seed_bridge_entry(&bridge, file, "Plans/precious.txt", Some(folder), FileStatus::Local, false, 13);
    let rows = bridge.db().mark_subtree_trashing(folder).unwrap();
    assert_eq!(rows.len(), 2);
    for local in [LocalEntry::InSync, LocalEntry::WhollyLocal] {
        remove_pruned_entries(bridge.db(), &f.root, &rows, &|_, _| local);
        f.outside_untouched();
    }
    assert!(std::fs::symlink_metadata(&link).unwrap().file_type().is_symlink(), "the junction is left as it is");
    std::fs::remove_dir(&link).unwrap();
}

/// The placeholder pass over rows below `Plans`, where `Plans` on disk is a junction to `outside`: no folder is made
/// through it, and nothing is created there.
#[test]
fn task_2413_windows_the_placeholder_pass_makes_no_folder_through_a_junction() {
    let f = Folders::new();
    let link = plans_junction(&f);
    let bridge = test_bridge(&f.root.parent().unwrap().join("state.db"));
    seed_bridge_entry(&bridge, "a1330000-0000-4000-8000-000000000001", "Plans/sub/f.txt", None, FileStatus::CloudOnly, false, 10);
    seed_bridge_entry(&bridge, "a1330000-0000-4000-8000-000000000002", "Plans/g.txt", None, FileStatus::CloudOnly, false, 10);
    crate::windows_cf::restore_placeholders(bridge.db(), &f.root);
    assert!(!f.outside.join("sub").exists(), "no folder made through the junction");
    let mut there: Vec<String> =
        std::fs::read_dir(&f.outside).unwrap().map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned()).collect();
    there.sort();
    assert_eq!(there, ["precious.txt", "~$ecious.txt"], "nothing created through the junction");
    f.outside_untouched();
    std::fs::remove_dir(&link).unwrap();
}

#[test]
fn task_2413_windows_the_hydrate_writer_never_writes_through_a_junction() {
    let f = Folders::new();
    f.junction(&f.root.join("link"));
    f.junction(&f.root.join("docs").join("deeper"));
    for dest in [f.root.join("link").join("precious.txt"), f.root.join("docs").join("deeper").join("precious.txt")] {
        let error = write_hydrated_from_reader(&dest, &[f.root.as_path()], &mut &b"server bytes"[..]).unwrap_err();
        assert_eq!(error.kind(), std::io::ErrorKind::PermissionDenied, "{}: {error}", dest.display());
    }
    // A new file through the junction is refused too (nothing is created outside).
    let error = write_hydrated_from_reader(&f.root.join("link").join("new.txt"), &[f.root.as_path()], &mut &b"x"[..])
        .unwrap_err();
    assert_eq!(error.kind(), std::io::ErrorKind::PermissionDenied, "{error}");
    f.outside_untouched();
    assert!(!f.outside.join("new.txt").exists());
    // A plain folder below the root still takes the write.
    let own = f.root.join("docs").join("own.txt");
    assert_eq!(write_hydrated_from_reader(&own, &[f.root.as_path()], &mut &b"server bytes"[..]).unwrap(), 12);
    assert_eq!(std::fs::read(&own).unwrap(), b"server bytes");
    std::fs::remove_dir(f.root.join("link")).unwrap();
    std::fs::remove_dir(f.root.join("docs").join("deeper")).unwrap();
}

/// The store, with this build's Windows rules: a reserved name, a `:`, a trailing dot and a `\` are never stored; a
/// long name Windows holds is (100 CJK characters; a CJK title broken by spaces; 80 emoji; exactly 255 UTF-16 units),
/// and 256 units is not.
#[test]
fn task_2413_windows_the_store_refuses_names_windows_cannot_hold() {
    let dir = tempfile::tempdir().unwrap();
    let db = StateDb::open(dir.path().join("state.db")).unwrap();
    for (n, name) in ["CON", "nul.txt", "report.txt:stream", "trailing.", "Q1: plan", "a\\..\\..\\x.txt"].iter().enumerate() {
        let id = format!("a1310000-0000-4000-8000-{n:012}");
        let row = serde_json::json!({ "id": id, "name_encrypted": name, "size_bytes": 1, "updated_at": 5 });
        let entry = apply_metadata_file_row(
            &db,
            &row,
            Namespace::MyFiles,
            None,
            None,
            PERMISSION_READ | PERMISSION_WRITE | PERMISSION_OWNER,
            100,
            &[7u8; 32],
            "docs",
        )
        .unwrap();
        assert!(entry.is_none(), "{name:?}");
        assert!(db.get_file(&id).unwrap().is_none(), "{name:?}");
    }
    let held = [
        "漢".repeat(100),
        format!("{}報告", "第三四半期 計画 ".repeat(11)),
        "😀".repeat(80),
        format!("{}a", "😀".repeat(127)),
    ];
    let mut wrong = Vec::new();
    for (n, long) in held.iter().enumerate() {
        let id = format!("a1310000-0000-4000-8000-0000000001{n:02}");
        let row = serde_json::json!({ "id": id, "name_encrypted": long, "size_bytes": 1 });
        let entry = apply_metadata_file_row(&db, &row, Namespace::MyFiles, None, None, PERMISSION_READ, 100, &[7u8; 32], "docs");
        let mapped = crate::engine_bridge::local_file_path_under_sync_root(dir.path(), &format!("docs/{long}")).is_ok();
        if entry.unwrap().map(|e| e.path) != Some(format!("docs/{long}")) || !mapped {
            wrong.push((long.encode_utf16().count(), long.len()));
        }
    }
    assert!(wrong.is_empty(), "not stored, as (UTF-16 units, bytes): {wrong:?}");
    let row = serde_json::json!({ "id": "a1310000-0000-4000-8000-0000000002ff", "name_encrypted": "😀".repeat(128), "size_bytes": 1 });
    let entry = apply_metadata_file_row(&db, &row, Namespace::MyFiles, None, None, PERMISSION_READ, 100, &[7u8; 32], "docs");
    assert!(entry.unwrap().is_none(), "256 UTF-16 units");
}
