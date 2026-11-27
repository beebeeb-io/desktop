//! Folder digests (v1) on the desktop: read the server's digest of a folder and its child folders, and find which
//! folders of a tree changed since the digests a caller trusts, walking down only where they differ.
//!
//! The definition is core's (`beebeeb_core::folder_digest`, `docs/folder-digest.md`); the transport is
//! `api_client::digests`. This module only compares hex strings the server sent.
//!
//! ## The root
//!
//! A folder id is the UUID string the listing carries. The empty string `""` is the account's root (the request
//! without `folder_id`); a [`Known`] keys the root's digest under `""` too.
//!
//! ## The after-read rule
//!
//! A digest read for folder F describes F's subtree at that read; the walker then reads F's children one request at a
//! time, and a write between those reads can land in a part already read. So a caller (the backup pass, the sync
//! drift check) trusts what it read under F's `digest` (stores it in its [`Known`]) only when [`digest_now`] of F,
//! asked after the caller finished F's whole subtree, is still that digest. When it moved, the caller keeps its old
//! entry (or none) for F, and the next walk enters F again.
//!
//! ## Logging
//!
//! No folder id, file id or name in a log line: counts and outcomes only.

mod walk;

pub use walk::{
    FolderDigests, Known, MAX_RESTARTS, PAGE_LIMIT, RequestCount, WalkError, changed_folders, digest_now, read_folder,
};

#[cfg(test)]
mod tests;
