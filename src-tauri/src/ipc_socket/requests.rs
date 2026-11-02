//! The typed request boundary.
//!
//! The wire variants keep their `String`/`bool` fields (the JSON shape is the
//! Swift extension's contract), so the connection types and parses each field
//! once, here, and the write arms take a request struct instead of a nine-field
//! parameter list.

use super::*;

/// Whether a create arrived because the system is resolving a deletion
/// conflict (spec 2026-10-09 §7.2). The two arms call different engine
/// methods, so the flag is an enum: no call site has to remember what `true`
/// meant.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum DeletionConflict {
    /// An ordinary create.
    None,
    /// `deletion_conflicted` on the wire: the system is replacing an item it
    /// could not delete on disk.
    Conflicted,
}

impl DeletionConflict {
    pub(super) fn from_wire(deletion_conflicted: bool) -> Self {
        if deletion_conflicted {
            Self::Conflicted
        } else {
            Self::None
        }
    }

    pub(super) fn is_conflicted(self) -> bool {
        matches!(self, Self::Conflicted)
    }
}

/// A `QueueFinderCreate` request: its wire fields, typed and parsed once.
pub(super) struct CreateRequest {
    pub(super) parent_id: Option<ParentId>,
    pub(super) filename: String,
    pub(super) kind: crate::engine_bridge::FinderWriteItemKind,
    pub(super) contents_path: Option<String>,
    pub(super) content_type: Option<String>,
    pub(super) request_id: Option<RequestId>,
    pub(super) deletion_conflict: DeletionConflict,
    pub(super) template_identifier: Option<String>,
    pub(super) template_content_version: Option<String>,
}

/// A `QueueFinderModify` request: its wire fields, typed and parsed once.
pub(super) struct ModifyRequest {
    pub(super) file_id: ItemId,
    pub(super) parent_id: Option<ParentId>,
    pub(super) filename: String,
    pub(super) kind: crate::engine_bridge::FinderWriteItemKind,
    pub(super) contents_path: Option<String>,
    pub(super) content_type: Option<String>,
    pub(super) base_version_identifier: Option<String>,
    pub(super) request_id: Option<RequestId>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deletion_conflict_is_the_wire_flag() {
        assert_eq!(DeletionConflict::from_wire(false), DeletionConflict::None);
        assert_eq!(DeletionConflict::from_wire(true), DeletionConflict::Conflicted);
        assert!(!DeletionConflict::None.is_conflicted());
        assert!(DeletionConflict::Conflicted.is_conflicted());
    }

    #[test]
    fn create_request_keeps_the_wire_ids() {
        let request = CreateRequest {
            parent_id: Some(ParentId::new("folder-7")),
            filename: "notes.txt".into(),
            kind: crate::engine_bridge::FinderWriteItemKind::File,
            contents_path: Some("/tmp/notes.txt".into()),
            content_type: Some("text/plain".into()),
            request_id: Some(RequestId::new("req-1")),
            deletion_conflict: DeletionConflict::Conflicted,
            template_identifier: None,
            template_content_version: None,
        };
        assert_eq!(request.parent_id.as_ref().map(ParentId::as_str), Some("folder-7"));
        assert_eq!(request.request_id.as_ref().map(RequestId::as_str), Some("req-1"));
        assert!(request.deletion_conflict.is_conflicted());
    }

    #[test]
    fn modify_request_keeps_the_wire_ids() {
        let request = ModifyRequest {
            file_id: ItemId::new("file-42"),
            parent_id: None,
            filename: "notes.txt".into(),
            kind: crate::engine_bridge::FinderWriteItemKind::File,
            contents_path: Some("/tmp/notes.txt".into()),
            content_type: Some("text/plain".into()),
            base_version_identifier: Some("v1".into()),
            request_id: None,
        };
        assert_eq!(request.file_id.as_str(), "file-42");
        assert!(request.parent_id.is_none());
        assert!(request.request_id.is_none());
    }
}
