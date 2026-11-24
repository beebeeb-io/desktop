//! The engine's calls for the plan40 batch-1 slices, one file per slice, so no two slices share a file and
//! `app.rs` holds ONE line for all of them. Each file is an `impl Engine` of its own; a slice fills only its file.
//!
//! Every call here is session-bound like its neighbours in `app/` (`*_for(session)`): asked FOR the session the host
//! read at its press, answered only while that session is still the one in memory.

pub mod account_deletion;
pub mod approve_sign_in;
pub mod backup_folders;
pub mod device_request;
pub mod file_request;
mod link_session;
pub mod media_stream;
pub mod needs_attention;
pub mod photos_folders;
pub mod recovery;
pub mod search_recent;
pub mod share_folder_link;
pub mod share_safety;
pub mod shared_offline;
pub mod status_on_files;
