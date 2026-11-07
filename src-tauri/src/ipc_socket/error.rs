//! The IPC socket's own typed error.
//!
//! `anyhow` reaches this module only from the engine bridge, and only through
//! the two boundary functions whose signatures the engine's own tests pin
//! (`write_outcome_response`, `hydrate_failure_reply`). Everything inside the
//! module carries [`IpcError`]: the write outcome the engine refused,
//! classified once here, and the thumbnail staging refusals. The reply text is
//! the engine's, unchanged — the extension and the person see exactly what they
//! saw before.

use std::fmt;
use std::path::PathBuf;

use super::write_refusal_category;

/// Why the socket could not do what a request asked.
pub(crate) enum IpcError {
    /// A thumbnail staging destination outside every allowed root. Nothing was
    /// written.
    DestinationNotAllowed { dest: PathBuf },
    /// The engine's fd-anchored writer refused the plaintext. Nothing landed at
    /// the destination.
    Stage { dest: PathBuf, source: anyhow::Error },
    /// The write was not queued and the system must hand it over again: the
    /// extension reports a transient error, so it keeps the change (spec §8.4,
    /// §8.7 S6, 2026-10-10 §15 question 3, and a full disk/database).
    RetryLater { category: &'static str, message: String },
    /// The write named an item the state holds no row and no alias for.
    UnknownItem { requested_id: String, message: String },
    /// Any other engine refusal: a definitive Finder error.
    Failed { category: &'static str, message: String },
}

impl IpcError {
    /// Classify an engine write error exactly as the reply used to: the same
    /// variants, the same fixed texts, the same log categories. `op` decides
    /// whether a full disk is retryable (a create/modify queued nothing) or a
    /// definitive error (a delete's reply item is never read).
    pub(super) fn from_engine(op: &'static str, error: anyhow::Error) -> Self {
        if crate::engine_bridge::engine_cause::<crate::engine_bridge::FinderStagingUnavailable>(&error).is_some() {
            return Self::RetryLater {
                category: "staging_root_unavailable",
                message: error.to_string(),
            };
        }
        if crate::engine_bridge::engine_cause::<crate::engine_bridge::StagedCopyGone>(&error).is_some() {
            return Self::RetryLater {
                category: "staged_copy_gone",
                message: error.to_string(),
            };
        }
        if crate::engine_bridge::engine_cause::<crate::engine_bridge::AcceptsClosed>(&error).is_some() {
            return Self::RetryLater {
                category: "accepts_closed",
                message: error.to_string(),
            };
        }
        if matches!(op, "create" | "modify")
            && matches!(
                crate::engine_bridge::error_category(error.as_ref()),
                "disk_full" | "database_full"
            )
        {
            return Self::RetryLater {
                category: write_refusal_category(&error),
                message: "the disk or Beebeeb's local database is full".into(),
            };
        }
        // LD-0113: core's words go to Finder in the reply; the log keeps the category only.
        if let Some(refused) = crate::engine_bridge::engine_cause::<crate::backup_protect::DriveChangeRefused>(&error) {
            return Self::Failed {
                category: "backup_protected",
                message: refused.message.clone(),
            };
        }
        match crate::engine_bridge::engine_cause::<crate::engine_bridge::UnknownItem>(&error) {
            Some(unknown) => Self::UnknownItem {
                requested_id: unknown.requested_id.clone(),
                message: error.to_string(),
            },
            None => Self::Failed {
                category: write_refusal_category(&error),
                message: error.to_string(),
            },
        }
    }

    /// The fixed log category for [`log_refused_write`].
    pub(super) fn category(&self) -> &'static str {
        match self {
            Self::DestinationNotAllowed { .. } => "destination_not_allowed",
            Self::Stage { .. } => "stage_failed",
            Self::RetryLater { category, .. } | Self::Failed { category, .. } => *category,
            Self::UnknownItem { .. } => "unknown_item",
        }
    }

    /// The id a refusal named, when it named one ([`log_refused_unknown_item`]).
    pub(super) fn unknown_item_id(&self) -> Option<&str> {
        match self {
            Self::UnknownItem { requested_id, .. } => Some(requested_id),
            _ => None,
        }
    }

    /// Whether the system must hand the write over again (a transient error)
    /// rather than show a definitive Finder error.
    pub(super) fn is_retry_later(&self) -> bool {
        matches!(self, Self::RetryLater { .. })
    }

    /// The reply text: the engine's own for an engine error, a fixed reason for
    /// a staging refusal. Never a path the person cannot act on, except the
    /// destination the caller itself named.
    pub(super) fn message(&self) -> String {
        self.to_string()
    }
}

impl fmt::Display for IpcError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DestinationNotAllowed { dest } => write!(
                f,
                "thumbnail destination {} is not under an allowed root",
                dest.display()
            ),
            Self::Stage { dest, source } => {
                write!(f, "could not stage thumbnail at {}: {source}", dest.display())
            }
            Self::RetryLater { message, .. } | Self::Failed { message, .. } | Self::UnknownItem { message, .. } => {
                f.write_str(message)
            }
        }
    }
}

impl fmt::Debug for IpcError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // The reply text can name a file; the debug form is for tests only and
        // stays the Display form so a failing assertion never adds a path.
        fmt::Display::fmt(self, f)
    }
}

impl std::error::Error for IpcError {}

#[cfg(test)]
mod tests {
    use super::*;

    /// The macOS staging refusal reaches the write path boxed in `EngineError` (a `?` into
    /// `anyhow::Result`), not bare: a top-level downcast missed it and the extension got a
    /// definitive error instead of `WriteRetryLater`. This is that shape, and the bare one.
    #[test]
    fn the_write_classes_see_an_engine_error_variant_behind_the_anyhow_boundary() {
        let wrapped = anyhow::Error::new(crate::engine_bridge::EngineError::from(
            crate::engine_bridge::StagedCopyGone,
        ));
        assert_eq!(write_refusal_category(&wrapped), "staged_copy_gone");
        assert!(IpcError::from_engine("modify", wrapped).is_retry_later());

        let bare = anyhow::Error::new(crate::engine_bridge::StagedCopyGone);
        assert_eq!(write_refusal_category(&bare), "staged_copy_gone");
        assert!(IpcError::from_engine("modify", bare).is_retry_later());
    }

    #[test]
    fn an_unknown_item_behind_the_anyhow_boundary_keeps_its_refusal() {
        let wrapped = anyhow::Error::new(crate::engine_bridge::EngineError::from(
            crate::engine_bridge::UnknownItem {
                requested_id: "id".into(),
            },
        ));
        assert_eq!(write_refusal_category(&wrapped), "unknown_item");
        assert!(matches!(
            IpcError::from_engine("modify", wrapped),
            IpcError::UnknownItem { .. }
        ));
    }

    #[test]
    fn destination_refusal_keeps_the_engine_message() {
        let error = IpcError::DestinationNotAllowed {
            dest: std::path::PathBuf::from("/tmp/elsewhere/thumb.bin"),
        };
        assert_eq!(
            error.to_string(),
            "thumbnail destination /tmp/elsewhere/thumb.bin is not under an allowed root"
        );
    }
}
