//! The support bundle (Settings > General > Diagnostics > Export, DW-0790b): a small JSON file the person can read
//! before they send it, saved in their Downloads folder.
//!
//! It holds the signed-in account's queue counts, an error code and the last error message, as
//! [`crate::state_db::StateDb::queue_diagnostics_with_paths`] hands them out: every string has already gone through
//! the engine's export redaction (`crate::diagnostic_redaction`), with the sync folder's own names added to the names
//! it removes. The file on disk is the redacted one; there is no unredacted copy. Never: a file or folder name, a
//! path, the account's email, a device name, a key, a token or the recovery phrase. Nothing is sent anywhere.
//!
//! For one session only (the press's): the bundle is not written once the session moved on. BLOCKS: a worker's.

use std::io::Write;
use std::path::PathBuf;

use serde::Serialize;

use super::{CredentialStores, Engine};
use crate::account::SessionGeneration;
use crate::state_db::QueueDiagnostics;

/// The file as written: a format number and the redacted queue facts, nothing else.
#[derive(Serialize)]
struct Bundle<'a> {
    format: u32,
    queue: &'a QueueDiagnostics,
}

/// Where a bundle was saved.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SavedBundle {
    pub path: PathBuf,
    /// The file's own name, which holds no name of the person's.
    pub file_name: String,
    /// The folder as a page may name it: an environment name, never the person's own path.
    pub folder: &'static str,
}

/// Why no bundle was saved. Fixed sentences only: no OS error, no path.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BundleRefused {
    NotSignedIn,
    /// Nothing has synced on this PC yet (or the state is another account's): there are no queue facts to export.
    NoQueue,
    /// This PC names no Downloads folder.
    NoFolder,
    WriteFailed,
}

impl BundleRefused {
    pub fn words(self) -> &'static str {
        match self {
            Self::NotSignedIn => "Unlock Beebeeb to export a support bundle.",
            Self::NoQueue => "Nothing has synced on this PC yet, so there is nothing to put in a bundle.",
            Self::NoFolder => "This PC has no Downloads folder to save the bundle in.",
            Self::WriteFailed => "The bundle could not be saved in your Downloads folder.",
        }
    }
}

/// The bundle's bytes for `queue`. Pure, so a test reads exactly what is written.
pub fn bundle_json(queue: &QueueDiagnostics) -> Vec<u8> {
    serde_json::to_vec_pretty(&Bundle { format: 1, queue }).unwrap_or_default()
}

impl<C: CredentialStores> Engine<C> {
    /// Save the support bundle of the signed-in account's local state, under `session`.
    pub fn export_support_bundle_for(&self, session: SessionGeneration) -> Result<SavedBundle, BundleRefused> {
        let folder = dirs::download_dir().ok_or(BundleRefused::NoFolder)?;
        self.export_support_bundle_into(session, &folder)
    }

    /// [`Self::export_support_bundle_for`] into `folder` (a test's own).
    pub(crate) fn export_support_bundle_into(
        &self,
        session: SessionGeneration,
        folder: &std::path::Path,
    ) -> Result<SavedBundle, BundleRefused> {
        if !self.acct.session_unchanged_since(session) {
            return Err(BundleRefused::NotSignedIn);
        }
        // The sync folder's own names are scrubbed too, as the state database's known names are.
        let extra: Vec<String> = self
            .configured_sync_root()
            .ok()
            .flatten()
            .map(|root| root.to_string_lossy().into_owned())
            .into_iter()
            .collect();
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_secs() as i64);
        let queue = self
            .local_state()
            .and_then(|state| state.with(|db| db.queue_diagnostics_with_paths(now, &extra)))
            .map_err(|_| BundleRefused::NoQueue)?
            .map_err(|_| BundleRefused::NoQueue)?;
        // Written only for the session that asked.
        if !self.acct.session_unchanged_since(session) {
            return Err(BundleRefused::NotSignedIn);
        }
        let file_name = format!("beebeeb-support-bundle-{now}.json");
        let path = folder.join(&file_name);
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
            .map_err(|_| BundleRefused::WriteFailed)?;
        file.write_all(&bundle_json(&queue))
            .and_then(|()| file.flush())
            .map_err(|_| BundleRefused::WriteFailed)?;
        Ok(SavedBundle {
            path,
            file_name,
            folder: "Downloads",
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    fn queue() -> QueueDiagnostics {
        QueueDiagnostics {
            queued: 3,
            due: 1,
            paused: 2,
            by_kind: BTreeMap::from([("upload_file".to_string(), 3)]),
            paused_by_reason: BTreeMap::from([("quota".to_string(), 2)]),
            last_error: Some("failed to [name] [path]".into()),
            last_error_code: Some(crate::diagnostic_redaction::DiagnosticErrorCode::Quota),
            last_error_redactions: 2,
            last_error_class: Some("quota".into()),
        }
    }

    #[test]
    fn the_file_holds_the_counts_the_code_and_the_redacted_message_and_nothing_else() {
        let text = String::from_utf8(bundle_json(&queue())).unwrap();
        let value: serde_json::Value = serde_json::from_str(&text).unwrap();
        let mut keys: Vec<_> = value.as_object().unwrap().keys().cloned().collect();
        keys.sort();
        assert_eq!(keys, ["format", "queue"]);
        let mut queue_keys: Vec<_> = value["queue"].as_object().unwrap().keys().cloned().collect();
        queue_keys.sort();
        assert_eq!(
            queue_keys,
            [
                "by_kind",
                "due",
                "last_error",
                "last_error_class",
                "last_error_code",
                "last_error_redactions",
                "paused",
                "paused_by_reason",
                "queued"
            ]
        );
        assert_eq!(value["queue"]["last_error"], "failed to [name] [path]");
        assert_eq!(value["queue"]["queued"], 3);
    }
}
