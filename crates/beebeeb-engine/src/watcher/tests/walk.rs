//! `walk` tests: which entries the enumeration walk visits, skips and refuses
//! to follow.

use super::*;

/// Collect every regular file the enumeration scan's [`walk_dir`] visits,
/// as paths relative to `root` with '/'-separators, so assertions read
/// naturally regardless of platform separator.
fn walked_rel(root: &std::path::Path) -> HashSet<String> {
    let mut found: HashSet<String> = HashSet::new();
    walk_dir(root, root, 0, &mut |_dir| {}, &mut |p, _meta| {
        let rel = p.strip_prefix(root).unwrap();
        let joined = rel
            .components()
            .map(|c| c.as_os_str().to_string_lossy().into_owned())
            .collect::<Vec<_>>()
            .join("/");
        found.insert(joined);
    });
    found
}

#[test]
fn walk_visits_nested_files_and_skips_engine_internals() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();

    // User files: one at the root, one nested two levels deep.
    fs::write(root.join("top.txt"), b"hello").unwrap();
    fs::create_dir_all(root.join("docs").join("sub")).unwrap();
    fs::write(root.join("docs").join("a.txt"), b"a").unwrap();
    fs::write(root.join("docs").join("sub").join("b.txt"), b"b").unwrap();

    // Engine-internal: the state dir + lock file must NOT be visited.
    fs::create_dir_all(root.join(".beebeeb")).unwrap();
    fs::write(root.join(".beebeeb").join("state.db"), b"db").unwrap();
    fs::write(root.join(".beebeeb-sync.lock"), b"lock").unwrap();

    let found = walked_rel(root);

    assert!(found.contains("top.txt"), "root-level file should be visited");
    assert!(found.contains("docs/a.txt"), "nested file should be visited");
    assert!(found.contains("docs/sub/b.txt"), "deeply-nested file should be visited");
    assert!(
        !found.iter().any(|p| p.contains(".beebeeb")),
        "no .beebeeb/* path may be visited, got: {found:?}"
    );
    assert!(
        !found.contains(".beebeeb-sync.lock"),
        "the lock file must never be visited"
    );
    assert_eq!(found.len(), 3, "exactly the three user files, got: {found:?}");
}

#[test]
fn walk_skips_empty_root() {
    let dir = tempfile::tempdir().unwrap();
    let found = walked_rel(dir.path());
    assert!(found.is_empty(), "an empty root yields no files");
}

#[cfg(unix)]
#[test]
fn walk_does_not_follow_symlinks() {
    use std::os::unix::fs::symlink;
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();

    // A real file, plus a symlink pointing at an OUTSIDE directory. The walk
    // must not follow the link (no upload-through-link, no cycle risk).
    fs::write(root.join("real.txt"), b"x").unwrap();
    let outside = tempfile::tempdir().unwrap();
    fs::write(outside.path().join("secret.txt"), b"s").unwrap();
    symlink(outside.path(), root.join("link")).unwrap();

    let found = walked_rel(root);
    assert!(found.contains("real.txt"));
    assert!(
        !found.iter().any(|p| p.contains("secret") || p.starts_with("link")),
        "must not descend through a symlink, got: {found:?}"
    );
}
