//! The one walker the source censuses share: every `.rs` file of the engine crate (`crates/beebeeb-engine/src`),
//! and, for the rules that hold for every writer in the desktop, the native app's tree too
//! ([`rust_files_with_native`]).
//!
//! An engine file is named with [`ENGINE_PREFIX`] (`crates/beebeeb-engine/src/state_db.rs`); a native file with
//! [`NATIVE_PREFIX`], so no allow-list entry written for an engine file can match a native one.

use std::path::{Path, PathBuf};

/// What an engine file's name starts with.
pub const ENGINE_PREFIX: &str = "crates/beebeeb-engine/src/";

/// The attributes that mark an item as test-only, as `without_items` takes them: `#[cfg(test)]` forms, and the
/// engine crate's form for a helper another crate's tests call (the dev-only `test-support` feature). An item
/// gated `any(target_os = .., test, ..)` is production code on that OS and is not in this list.
pub const TEST_ITEM_GATES: [&str; 4] = [
    "#[cfg(test)]",
    "#[cfg(all(test",
    "#[cfg(any(test, feature = \"test-support\"))]",
    "#[cfg(all(any(test, feature = \"test-support\")",
];

/// What a native file's name starts with.
pub const NATIVE_PREFIX: &str = "native/beebeeb-drive-rune/";

fn engine_src() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("src")
}

/// A test-only file by its name: `tests.rs`, `*_tests.rs`, or anything under a `tests` or `*_tests` directory.
pub fn is_test_file(name: &str) -> bool {
    let mut parts: Vec<&str> = name.split('/').collect();
    let file = parts.pop().unwrap_or_default();
    file == "tests.rs"
        || file.ends_with("_tests.rs")
        || parts.iter().any(|dir| *dir == "tests" || dir.ends_with("_tests"))
}

/// Is `name` an engine file?
pub fn is_engine_file(name: &str) -> bool {
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

/// Every `.rs` file of the engine tree as `(name, path)`, sorted by name. Panics when the tree is missing or
/// near-empty: a census that reads nothing passes for the wrong reason.
pub fn rust_files() -> Vec<(String, PathBuf)> {
    let mut files = Vec::new();
    let engine = engine_src();
    walk(&engine, &engine, ENGINE_PREFIX, &mut files);
    assert!(
        files.len() > 20,
        "the census reads the engine tree: {} engine files",
        files.len()
    );
    files.sort();
    files
}

/// The engine tree and the native app's tree (`native/beebeeb-drive-rune/src` and its `build.rs`), as
/// `(name, path)`, sorted by name. For a rule about every production writer in the desktop. Panics when either
/// tree is near-empty.
pub fn rust_files_with_native() -> Vec<(String, PathBuf)> {
    let mut files = rust_files();
    let engine_count = files.len();
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../native/beebeeb-drive-rune");
    walk(&root.join("src"), &root, NATIVE_PREFIX, &mut files);
    files.push((format!("{NATIVE_PREFIX}build.rs"), root.join("build.rs")));
    let native_count = files.len() - engine_count;
    assert!(
        engine_count > 20 && native_count > 20,
        "the census reads both trees: {engine_count} engine files, {native_count} native files"
    );
    files.sort();
    files
}

/// The production files of a top-level module, in the engine tree: `<module>.rs` and everything under `<module>/`
/// that is not a test file. The text is read with LF line ends.
pub fn production_tree_parts(module: &str) -> Vec<String> {
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
    assert!(!parts.is_empty(), "no production file of `{module}` in the engine tree");
    parts
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn both_trees_are_read_and_an_engine_file_is_named_apart() {
        let files = rust_files_with_native();
        assert!(
            files
                .iter()
                .any(|(name, _)| name == "native/beebeeb-drive-rune/src/engine_host/mod.rs"),
            "a native file is read, under its own prefix"
        );
        assert!(
            files
                .iter()
                .any(|(name, _)| name == "native/beebeeb-drive-rune/build.rs")
        );
        assert!(
            files
                .iter()
                .any(|(name, _)| name == "crates/beebeeb-engine/src/state_db.rs")
        );
        assert!(
            !files.iter().any(|(name, _)| name == "state_db.rs"),
            "the engine's file keeps its prefix"
        );
        assert!(
            rust_files().iter().all(|(name, _)| name.starts_with(ENGINE_PREFIX)),
            "the engine census reads the engine tree alone"
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
        let production = crate::source_pin::without_items(source, &TEST_ITEM_GATES);
        assert!(
            !production.contains("planted()") && production.contains("real()"),
            "{production}"
        );
    }
}
