//! The one walker the source censuses share: every `.rs` file of the desktop's Rust, in BOTH trees, the shell's
//! (`src-tauri/src`) and the engine crate's (`crates/beebeeb-engine/src`). A census that read only the shell's tree
//! stopped seeing a module the day it moved to the engine crate; a rule about production code holds in both.
//!
//! A shell file keeps the name the censuses always used (`runner.rs`, `finder_setup/core.rs`). An engine file is
//! named with [`ENGINE_PREFIX`], so no allow-list entry written for a shell file can match it.

use std::path::{Path, PathBuf};

/// What an engine file's name starts with.
pub(crate) const ENGINE_PREFIX: &str = "crates/beebeeb-engine/src/";

/// The attributes that mark an item as test-only, as `without_items` takes them: `#[cfg(test)]` forms, and the
/// engine crate's form for a helper another crate's tests call (the dev-only `test-support` feature). An item
/// gated `any(target_os = .., test, ..)` is production code on that OS and is not in this list.
pub(crate) const TEST_ITEM_GATES: [&str; 4] = [
    "#[cfg(test)]",
    "#[cfg(all(test",
    "#[cfg(any(test, feature = \"test-support\"))]",
    "#[cfg(all(any(test, feature = \"test-support\")",
];

fn shell_src() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("src")
}

fn engine_src() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../crates/beebeeb-engine/src")
}

/// A test-only file by its name: `tests.rs`, `*_tests.rs`, or anything under a `tests` or `*_tests` directory.
pub(crate) fn is_test_file(name: &str) -> bool {
    let mut parts: Vec<&str> = name.split('/').collect();
    let file = parts.pop().unwrap_or_default();
    file == "tests.rs"
        || file.ends_with("_tests.rs")
        || parts.iter().any(|dir| *dir == "tests" || dir.ends_with("_tests"))
}

/// Is `name` an engine file?
pub(crate) fn is_engine_file(name: &str) -> bool {
    name.starts_with(ENGINE_PREFIX)
}

fn walk(dir: &Path, root: &Path, prefix: &str, out: &mut Vec<(String, PathBuf)>) {
    for entry in std::fs::read_dir(dir).unwrap_or_else(|e| panic!("read {}: {e}", dir.display())) {
        let path = entry.expect("dir entry").path();
        if path.is_dir() {
            walk(&path, root, prefix, out);
        } else if path.extension().is_some_and(|ext| ext == "rs") {
            let rel = path.strip_prefix(root).unwrap().to_string_lossy().replace('\\', "/");
            out.push((format!("{prefix}{rel}"), path));
        }
    }
}

/// Every `.rs` file of both trees as `(name, path)`, sorted by name. Panics when either tree is missing or
/// near-empty: a census that reads nothing passes for the wrong reason.
pub(crate) fn rust_files() -> Vec<(String, PathBuf)> {
    let mut files = Vec::new();
    let shell = shell_src();
    walk(&shell, &shell, "", &mut files);
    let shell_count = files.len();
    let engine = engine_src();
    walk(&engine, &engine, ENGINE_PREFIX, &mut files);
    let engine_count = files.len() - shell_count;
    assert!(
        shell_count > 20 && engine_count > 20,
        "the census reads both trees: {shell_count} shell files, {engine_count} engine files"
    );
    files.sort();
    files
}

/// The production files of a top-level module, wherever it lives: `<module>.rs` and everything under `<module>/`
/// that is not a test file, shell first. The text is read with LF line ends.
pub(crate) fn production_tree_parts(module: &str) -> Vec<String> {
    let own = format!("{module}.rs");
    let under = format!("{module}/");
    let parts: Vec<String> = rust_files()
        .into_iter()
        .filter(|(name, _)| {
            let rel = name.strip_prefix(ENGINE_PREFIX).unwrap_or(name);
            (rel == own || rel.starts_with(&under)) && !is_test_file(rel)
        })
        .map(|(_, path)| {
            std::fs::read_to_string(&path)
                .unwrap_or_else(|e| panic!("{}: {e}", path.display()))
                .replace("\r\n", "\n")
        })
        .collect();
    assert!(!parts.is_empty(), "no production file of `{module}` in either tree");
    parts
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn both_trees_are_read_and_an_engine_file_is_named_apart() {
        let files = rust_files();
        assert!(files.iter().any(|(name, _)| name == "runner.rs"));
        assert!(
            files
                .iter()
                .any(|(name, _)| name == "crates/beebeeb-engine/src/state_db.rs")
        );
        assert!(
            !files.iter().any(|(name, _)| name == "state_db.rs"),
            "the engine's file keeps its prefix"
        );
        assert!(is_test_file(
            "crates/beebeeb-engine/src/state_db/reconcile_tests/pass.rs"
        ));
        assert!(
            is_test_file("held_queue/tests/mod.rs") && is_test_file("a_tests.rs") && !is_test_file("state_db/types.rs")
        );
        assert!(production_tree_parts("state_db").len() > 20 && production_tree_parts("engine_bridge").len() > 20);
    }

    /// The gate the engine crate uses for a test helper is stripped like `#[cfg(test)]`; an OS-or-test gate is not.
    #[test]
    fn a_test_support_item_is_not_production_and_an_os_gated_item_is() {
        let source = "fn a() {}\n#[cfg(any(test, feature = \"test-support\"))]\nfn helper() {\n    planted();\n}\n\
                      #[cfg(any(target_os = \"windows\", test, feature = \"test-support\"))]\nfn windows() {\n    real();\n}\n";
        let production = crate::finder_setup_command_tests::without_items(source, &TEST_ITEM_GATES);
        assert!(
            !production.contains("planted()") && production.contains("real()"),
            "{production}"
        );
    }
}
