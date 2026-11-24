//! Tests of `vault_tree.rs`. Ported from `src-tauri/src/selective_sync_wave2_tests.rs`
//! (`folder_leaf_name_takes_last_path_segment`).

use super::*;

#[test]
fn folder_leaf_name_takes_last_path_segment() {
    assert_eq!(folder_leaf_name("/Docs/Taxes").as_deref(), Some("Taxes"));
    assert_eq!(folder_leaf_name("Photos").as_deref(), Some("Photos"));
    assert_eq!(folder_leaf_name("/Photos/").as_deref(), Some("Photos"));
    assert_eq!(folder_leaf_name(""), None);
    assert_eq!(folder_leaf_name("/"), None);
}

/// Not in the Tauri file: the fallback label uses the first 8 characters, and a short id as it is.
#[test]
fn the_fallback_label_is_the_first_eight_characters_of_the_id() {
    assert_eq!(folder_fallback_label("0123456789abcdef"), "Folder 01234567…");
    assert_eq!(folder_fallback_label("abc"), "Folder abc…");
}
