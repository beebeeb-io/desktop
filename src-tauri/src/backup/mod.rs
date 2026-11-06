//! LD-0113: backups from source (spec 2026-11-06). The person's own folders are read in place and uploaded into
//! `Backups/<device>/<category>` on the server; nothing is copied into the sync root.
//!
//! - `sources`: what a backup may read from (never the sync root) and what its walk leaves out.

pub(crate) mod sources;
