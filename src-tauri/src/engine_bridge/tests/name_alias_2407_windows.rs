//! Task 2407 (core 2404), Windows: the sites against real 8.3 short names. Each test first proves that its fixture
//! aliases (the short name opens the other entry), so a pass is never a check that could not fail. A volume that makes
//! no 8.3 names is skipped with the reason printed; with `CI` set, the test fails instead.

use super::*;
use crate::name_alias::{NameConflict, conflict_below};

/// The 8.3 short name the volume gave `path`'s last component, if it has one other than its long name.
fn short_name_of(path: &Path) -> Option<String> {
    use std::os::windows::ffi::OsStrExt as _;
    use windows::Win32::Storage::FileSystem::GetShortPathNameW;
    let wide: Vec<u16> = path.as_os_str().encode_wide().chain([0]).collect();
    let mut buffer = vec![0u16; 1024];
    // SAFETY: `wide` is NUL-terminated; `buffer` is writable for its whole length.
    let written = unsafe { GetShortPathNameW(windows::core::PCWSTR(wide.as_ptr()), Some(&mut buffer)) } as usize;
    assert!(
        written != 0 && written < buffer.len(),
        "GetShortPathNameW failed for {}",
        path.display()
    );
    let short = String::from_utf16(&buffer[..written]).expect("a short path is UTF-16");
    let last = short.rsplit('\\').next().unwrap_or(&short).to_owned();
    (last != path.file_name().expect("a file name").to_string_lossy()).then_some(last)
}

/// A scratch folder holding `long` (a file with `contents`, or an empty folder for `None`), and the 8.3 short name the
/// volume gave it. Tries the temp folder, then the local-app-data temp folder (often the system drive, which makes 8.3
/// names by default). `None` after printing why when neither volume makes 8.3 names; with `CI` set that fails.
fn aliased(long: &str, contents: Option<&[u8]>) -> Option<(tempfile::TempDir, String)> {
    let mut roots = vec![std::env::temp_dir()];
    if let Some(local) = std::env::var_os("LOCALAPPDATA") {
        roots.push(PathBuf::from(local).join("Temp"));
    }
    for root in &roots {
        let Ok(dir) = tempfile::Builder::new().prefix("bb-2407-").tempdir_in(root) else {
            continue;
        };
        let path = dir.path().join(long);
        match contents {
            Some(bytes) => std::fs::write(&path, bytes).unwrap(),
            None => std::fs::create_dir(&path).unwrap(),
        }
        if let Some(short) = short_name_of(&path) {
            if let Some(bytes) = contents {
                assert_eq!(
                    std::fs::read(dir.path().join(&short)).unwrap(),
                    bytes,
                    "the fixture must alias"
                );
            } else {
                assert!(dir.path().join(&short).is_dir(), "the fixture must alias");
            }
            return Some((dir, short));
        }
    }
    assert!(
        std::env::var_os("CI").is_none(),
        "no volume among {roots:?} made an 8.3 short name, so this test proves nothing here"
    );
    eprintln!("skipped: no volume among {roots:?} makes 8.3 short names");
    None
}

#[test]
fn the_check_finds_a_real_short_name_and_passes_the_long_one() {
    let Some((dir, short)) = aliased("Report for 2026.pdf", Some(b"precious")) else {
        return;
    };
    assert_eq!(
        conflict_below(dir.path(), &dir.path().join(&short)).unwrap(),
        Some(NameConflict)
    );
    assert_eq!(
        conflict_below(dir.path(), &dir.path().join("Report for 2026.pdf")).unwrap(),
        None
    );
    assert_eq!(conflict_below(dir.path(), &dir.path().join("new.pdf")).unwrap(), None);
}

#[test]
fn a_hydrate_write_never_truncates_the_file_a_short_name_reaches() {
    let Some((dir, short)) = aliased("Report for 2026.pdf", Some(b"precious")) else {
        return;
    };
    let error =
        write_hydrated_from_reader(&dir.path().join(&short), &[dir.path()], &mut &b"server bytes"[..]).unwrap_err();
    assert_eq!(error.kind(), std::io::ErrorKind::AlreadyExists, "{error}");
    assert_eq!(
        std::fs::read(dir.path().join("Report for 2026.pdf")).unwrap(),
        b"precious"
    );
}

/// The same writer still writes a file's own name, and a new one, whole.
#[test]
fn a_hydrate_write_replaces_its_own_name_and_creates_a_new_one() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("own.txt"), b"old").unwrap();
    for name in ["own.txt", "new.txt"] {
        let written =
            write_hydrated_from_reader(&dir.path().join(name), &[dir.path()], &mut &b"server bytes"[..]).unwrap();
        assert_eq!(written, 12);
        assert_eq!(std::fs::read(dir.path().join(name)).unwrap(), b"server bytes", "{name}");
    }
    let mut names: Vec<String> = std::fs::read_dir(dir.path())
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    assert_eq!(names, ["new.txt", "own.txt"], "no temp file is left");
}

/// A destination that is not built on an allowed root is refused, as the Unix writer refuses it.
#[test]
fn a_hydrate_write_outside_every_allowed_root_is_refused() {
    let root = tempfile::tempdir().unwrap();
    let elsewhere = tempfile::tempdir().unwrap();
    let dest = elsewhere.path().join("new.txt");
    let error = write_hydrated_from_reader(&dest, &[root.path()], &mut &b"server bytes"[..]).unwrap_err();
    assert_eq!(error.kind(), std::io::ErrorKind::PermissionDenied, "{error}");
    assert!(!dest.exists());
}

#[test]
fn a_remote_deletion_never_deletes_the_file_a_short_name_reaches() {
    let Some((dir, short)) = aliased("Report for 2026.pdf", Some(b"precious")) else {
        return;
    };
    let state = tempfile::tempdir().unwrap();
    let db = StateDb::open(&state.path().join("state.db")).unwrap();
    let file_id = "aaaa0000-0000-4000-8000-000000002407";
    db.upsert_file_with_contract(
        &FileEntry {
            file_id: file_id.into(),
            path: short.clone(),
            status: FileStatus::Local,
            size_bytes: 8,
            modified_at: 1_700_000_000,
            content_hash: None,
            remote_updated_at: 1_700_000_000,
            parent_id: None,
            item_kind: ItemKind::File,
        },
        |contract| contract.item_kind = ItemKind::File,
    )
    .unwrap();
    let rows = db.mark_subtree_trashing(file_id).unwrap();
    assert_eq!(rows.len(), 1);
    remove_pruned_entries(&db, dir.path(), &rows, &|_, _| LocalEntry::InSync);
    assert_eq!(
        std::fs::read(dir.path().join("Report for 2026.pdf")).unwrap(),
        b"precious"
    );
}

#[test]
fn an_office_owner_file_a_short_name_reaches_is_never_removed() {
    let Some((dir, short)) = aliased("~$Report for 2026.docx", Some(b"another document's owner file")) else {
        return;
    };
    let Some(original) = short.strip_prefix("~$") else {
        eprintln!("skipped: the volume named the owner file {short}, which no document's owner name can be");
        return;
    };
    landing::remove_office_owner_files(&dir.path().join(original));
    assert!(
        dir.path().join("~$Report for 2026.docx").is_file(),
        "the owner file the name reaches is untouched"
    );
}

#[test]
fn a_scaffold_folder_a_short_name_reaches_is_never_removed() {
    let Some((dir, short)) = aliased("Photos 2026", None) else {
        return;
    };
    landing::remove_scaffold_dirs(dir.path(), &[short]);
    assert!(
        dir.path().join("Photos 2026").is_dir(),
        "the empty folder the name reaches is untouched"
    );
}

#[tokio::test]
async fn keep_both_never_renames_the_file_a_short_name_reaches() {
    let Some((dir, short)) = aliased("Conflict notes 2026.txt", Some(b"a different file")) else {
        return;
    };
    let state = tempfile::tempdir().unwrap();
    let bridge = test_bridge_with_api(&state.path().join("state.db"), "http://127.0.0.1:9".into(), [15u8; 32]);
    let file_id = "bbbb0000-0000-4000-8000-000000002409";
    bridge
        .db
        .upsert_file(&FileEntry {
            file_id: file_id.into(),
            path: short.clone(),
            status: FileStatus::Conflict,
            size_bytes: 16,
            modified_at: 100,
            content_hash: Some("local-conflict-hash".into()),
            remote_updated_at: 90,
            parent_id: None,
            item_kind: ItemKind::File,
        })
        .unwrap();
    let entry = bridge.db.get_file(file_id).unwrap().unwrap();

    let _ = bridge.auto_resolve_keep_both(dir.path(), &entry).await.unwrap_err();

    assert_eq!(
        std::fs::read(dir.path().join("Conflict notes 2026.txt")).unwrap(),
        b"a different file"
    );
    assert_eq!(
        std::fs::read_dir(dir.path()).unwrap().count(),
        1,
        "nothing renamed, nothing written"
    );
}
