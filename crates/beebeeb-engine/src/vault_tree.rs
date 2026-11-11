//! The display name of a folder row. Moved out of `src-tauri/src/vault_tree.rs` unchanged.

/// Derive a folder's display leaf name from its server-relative `path`.
/// `"/Docs/Taxes"` → `"Taxes"`, `"Photos"` → `"Photos"`, `""` → `None`.
pub fn folder_leaf_name(path: &str) -> Option<String> {
    let trimmed = path.trim_matches('/');
    if trimmed.is_empty() {
        return None;
    }
    let leaf = trimmed.rsplit('/').next().unwrap_or(trimmed);
    if leaf.is_empty() { None } else { Some(leaf.to_string()) }
}

/// Stable id-derived fallback label, mirroring the existing top-level path.
pub fn folder_fallback_label(id: &str) -> String {
    let short = id.get(..8).unwrap_or(id);
    format!("Folder {short}…")
}
