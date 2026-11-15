//! Tests of the path guards in `path_safety.rs`. Ported from `src-tauri/src/reveal_and_open_file_tests.rs`
//! (the `reject_unsafe_rel_path` and `is_contained` tests; the executable denylist is pinned in `file_ops.rs`).

use super::*;

#[test]
fn safe_paths_are_accepted() {
    assert!(reject_unsafe_rel_path("Documents/report.pdf").is_ok());
    assert!(reject_unsafe_rel_path("a/b/c.txt").is_ok());
    assert!(reject_unsafe_rel_path("Photos/2024/vacation.jpg").is_ok());
    assert!(reject_unsafe_rel_path("file.txt").is_ok());
}

#[test]
fn rejects_dotdot_forward_slash() {
    assert!(reject_unsafe_rel_path("../etc/passwd").is_err());
}

#[test]
fn rejects_dotdot_mid_path() {
    assert!(reject_unsafe_rel_path("a/../../b").is_err());
}

#[test]
fn rejects_current_dir_component() {
    assert!(reject_unsafe_rel_path("a/./b").is_err());
}

#[test]
fn rejects_empty_component() {
    assert!(reject_unsafe_rel_path("a//b").is_err());
}

#[test]
fn rejects_unix_absolute() {
    assert!(reject_unsafe_rel_path("/etc/passwd").is_err());
}

#[test]
fn rejects_unc_path_backslash() {
    assert!(reject_unsafe_rel_path("\\\\server\\share").is_err());
}

#[test]
fn rejects_unc_path_forward_slash() {
    assert!(reject_unsafe_rel_path("//server/share").is_err());
}

#[test]
fn rejects_windows_drive_absolute() {
    assert!(reject_unsafe_rel_path("C:\\Windows\\x").is_err());
}

#[test]
fn rejects_windows_drive_relative() {
    // 'C:foo' is still drive-prefixed, not a plain relative path.
    assert!(reject_unsafe_rel_path("C:foo").is_err());
}

#[test]
fn rejects_empty() {
    assert!(reject_unsafe_rel_path("").is_err());
}

#[test]
fn rejects_whitespace_only() {
    assert!(reject_unsafe_rel_path("   ").is_err());
}

#[test]
fn rejects_dotdot_backslash_separator() {
    assert!(reject_unsafe_rel_path("a\\..\\b").is_err());
}

#[test]
fn file_inside_root_is_contained() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let root = tmp.path();
    let file_path = root.join("subdir").join("file.txt");
    std::fs::create_dir_all(file_path.parent().unwrap()).unwrap();
    std::fs::write(&file_path, b"").unwrap();

    assert!(is_contained(root, &file_path));
}

#[test]
fn file_outside_root_is_not_contained() {
    let tmp_root = tempfile::tempdir().expect("root tempdir");
    let tmp_outside = tempfile::tempdir().expect("outside tempdir");
    let outside_file = tmp_outside.path().join("other.txt");
    std::fs::write(&outside_file, b"").unwrap();

    assert!(!is_contained(tmp_root.path(), &outside_file));
}

#[test]
fn sibling_dir_is_not_contained() {
    // root = <tmp>/root, target = <tmp>/rootEvil: containment is by path component, not by string prefix.
    let tmp = tempfile::tempdir().expect("tempdir");
    let root = tmp.path().join("root");
    let sibling = tmp.path().join("rootEvil");
    std::fs::create_dir_all(&root).unwrap();
    std::fs::create_dir_all(&sibling).unwrap();

    assert!(!is_contained(&root, &sibling));
}

/// Not in the Tauri file: a path that does not exist cannot be proven inside, so it is refused.
#[test]
fn a_missing_path_or_root_is_not_contained() {
    let tmp = tempfile::tempdir().expect("tempdir");
    assert!(!is_contained(tmp.path(), &tmp.path().join("nothing-here")));
    assert!(!is_contained(&tmp.path().join("no-root"), tmp.path()));
}
