//! Tasks 2270 / 2280 (desktop 0.8.20): what to do with one upload finalization journal row, decided from the row,
//! the server row and the platform's view of the file, before any file is opened for its data (spec
//! `docs/superpowers/specs/2026-10-29-2270-2280-desktop-0820-read-only-finalization-design.md`, §4 F0-F7).
//! Pure, so the table is tested on every platform; Windows applies it (`windows_cf::upload_finalization`).
use crate::state_db::UploadFinalization;
use crate::unknown_check::PlatformView;

/// Why a finalization is held for review (spec R2). The label is what the journal stores and the only thing logged
/// with the file id: never a path or a name.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ReviewReason {
    RowGone,
    Moved,
    Unreadable,
    OtherFile,
    PartlyOnDisk,
    PlatformError,
}

impl ReviewReason {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            ReviewReason::RowGone => "row_gone",
            ReviewReason::Moved => "moved",
            ReviewReason::Unreadable => "unreadable",
            ReviewReason::OtherFile => "other_file",
            ReviewReason::PartlyOnDisk => "partly_on_disk",
            ReviewReason::PlatformError => "platform_error",
        }
    }
}

/// The first step for one row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Step {
    /// Nothing is left to stamp: the row ends (marked stamped, staged copy removed, forgotten).
    Done,
    Review(ReviewReason),
    /// F5: a cloud-only placeholder of this upload. Its identity is re-stamped without opening its data.
    ReStampCloudOnly,
    /// F7: a plain file, or a placeholder of this upload wholly on disk. Its bytes are hashed and judged.
    JudgeBytes,
}

/// F0-F7 in order. `server_path` is the path of the server row `server_file_id` names, `None` when that row is gone.
pub(crate) fn first_step(row: &UploadFinalization, server_path: Option<&str>, view: &PlatformView) -> Step {
    use crate::unknown_check::OnDisk;
    match (server_path, view) {
        (None, PlatformView::Missing) => return Step::Done,
        (None, _) => return Step::Review(ReviewReason::RowGone),
        (Some(path), _) if path != row.target_path => return Step::Review(ReviewReason::Moved),
        _ => {}
    }
    match view {
        PlatformView::Missing => Step::Done,
        PlatformView::Unreadable => Step::Review(ReviewReason::Unreadable),
        PlatformView::PlainFile => Step::JudgeBytes,
        PlatformView::Placeholder { identity: Some(id), .. } if *id != row.local_file_id && *id != row.server_file_id => {
            Step::Review(ReviewReason::OtherFile)
        }
        PlatformView::Placeholder { on_disk: OnDisk::Nothing, .. } => Step::ReStampCloudOnly,
        PlatformView::Placeholder { on_disk: OnDisk::Partly, .. } => Step::Review(ReviewReason::PartlyOnDisk),
        PlatformView::Placeholder { on_disk: OnDisk::Wholly, .. } => Step::JudgeBytes,
    }
}

/// What the bytes allow (F7a-F7c).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Judged {
    /// The server identity, marked in sync.
    MarkInSync,
    /// The server identity alone; the stamp pass marks it in sync once its bytes are the agreed content.
    IdentityOnly,
}

/// F7a-F7c. `bytes` is the file's SHA-256 (hex); `payload` the staged payload's, `None` when it is gone; `agreed` the
/// row's agreed content (`local_hash`), passed as `None` while an upload of the file is queued.
pub(crate) fn judge(bytes: &str, payload: Option<&str>, agreed: Option<&str>) -> Judged {
    if payload == Some(bytes) || agreed == Some(bytes) {
        Judged::MarkInSync
    } else {
        Judged::IdentityOnly
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::unknown_check::OnDisk;

    fn row() -> UploadFinalization {
        UploadFinalization {
            op_id: "op".into(),
            local_file_id: "local-1".into(),
            server_file_id: "server-1".into(),
            target_path: "Backups/pc/Documents/a.txt".into(),
            payload_path: "proof".into(),
            stamped: false,
            review: None,
        }
    }

    fn placeholder(on_disk: OnDisk, identity: Option<&str>) -> PlatformView {
        PlatformView::Placeholder { on_disk, in_sync: false, identity: identity.map(str::to_string) }
    }

    const HERE: Option<&str> = Some("Backups/pc/Documents/a.txt");

    /// F0: the server row is gone while the file is still here: held for review, the staged copy kept (lead P2).
    #[test]
    fn a_gone_server_row_with_a_file_goes_to_review() {
        for view in [PlatformView::PlainFile, placeholder(OnDisk::Wholly, Some("local-1")), PlatformView::Unreadable] {
            assert_eq!(first_step(&row(), None, &view), Step::Review(ReviewReason::RowGone), "{view:?}");
        }
    }

    /// F0': the server row and the file are both gone: nothing is left to stamp.
    #[test]
    fn a_gone_server_row_without_a_file_is_done() {
        assert_eq!(first_step(&row(), None, &PlatformView::Missing), Step::Done);
    }

    /// F1: the server row has another path (moved since): held for review, whatever is at the old path.
    #[test]
    fn a_moved_file_goes_to_review() {
        for view in [PlatformView::Missing, PlatformView::PlainFile, placeholder(OnDisk::Nothing, Some("server-1"))] {
            assert_eq!(
                first_step(&row(), Some("Backups/pc/Documents/b.txt"), &view),
                Step::Review(ReviewReason::Moved),
                "{view:?}"
            );
        }
    }

    /// F2: nothing at the target: the row ends.
    #[test]
    fn a_missing_target_finishes_the_row() {
        assert_eq!(first_step(&row(), HERE, &PlatformView::Missing), Step::Done);
    }

    /// F3: something is there but it cannot be read: held for review.
    #[test]
    fn an_unreadable_target_goes_to_review() {
        assert_eq!(first_step(&row(), HERE, &PlatformView::Unreadable), Step::Review(ReviewReason::Unreadable));
    }

    /// F4: a placeholder naming another file is never stamped, however much of it is on disk.
    #[test]
    fn a_placeholder_of_another_file_goes_to_review() {
        for on_disk in [OnDisk::Nothing, OnDisk::Partly, OnDisk::Wholly] {
            assert_eq!(
                first_step(&row(), HERE, &placeholder(on_disk, Some("server-2"))),
                Step::Review(ReviewReason::OtherFile),
                "{on_disk:?}"
            );
        }
    }

    /// F5: a cloud-only placeholder of this upload (its throwaway local id, the server id, or none) is re-stamped,
    /// never judged: judging would open its data, and that open fetches it.
    #[test]
    fn a_cloud_only_placeholder_is_re_stamped_not_read() {
        for identity in [Some("local-1"), Some("server-1"), None] {
            assert_eq!(
                first_step(&row(), HERE, &placeholder(OnDisk::Nothing, identity)),
                Step::ReStampCloudOnly,
                "{identity:?}"
            );
        }
    }

    /// F6: part of it is on disk: never read (the rest would be fetched), held for review.
    #[test]
    fn a_partly_downloaded_placeholder_is_never_read() {
        for identity in [Some("local-1"), Some("server-1"), None] {
            assert_eq!(
                first_step(&row(), HERE, &placeholder(OnDisk::Partly, identity)),
                Step::Review(ReviewReason::PartlyOnDisk),
                "{identity:?}"
            );
        }
    }

    /// F7: a plain file (the 82 read-only copies), or a placeholder of this upload wholly on disk, in sync or not.
    #[test]
    fn a_plain_or_wholly_local_file_is_judged() {
        assert_eq!(first_step(&row(), HERE, &PlatformView::PlainFile), Step::JudgeBytes);
        for identity in [Some("local-1"), Some("server-1"), None] {
            for in_sync in [false, true] {
                let view = PlatformView::Placeholder {
                    on_disk: OnDisk::Wholly,
                    in_sync,
                    identity: identity.map(str::to_string),
                };
                assert_eq!(first_step(&row(), HERE, &view), Step::JudgeBytes, "{view:?}");
            }
        }
    }

    /// F7a-F7c and anti-goal 3: in sync only on the payload's bytes or the agreed bytes; the agreed bytes count only
    /// when no upload is queued (the caller then passes `None`).
    #[test]
    fn judge_marks_the_payload_or_the_agreed_bytes_and_nothing_else() {
        assert_eq!(judge("h1", Some("h1"), Some("h0")), Judged::MarkInSync, "the payload's bytes");
        assert_eq!(judge("h2", Some("h1"), Some("h2")), Judged::MarkInSync, "a later edit that landed");
        assert_eq!(judge("h2", None, Some("h2")), Judged::MarkInSync, "the staged copy is gone, the bytes are agreed");
        assert_eq!(judge("h3", Some("h1"), Some("h2")), Judged::IdentityOnly, "neither");
        assert_eq!(judge("h3", None, None), Judged::IdentityOnly, "nothing to compare with");
        assert_eq!(judge("h2", Some("h1"), None), Judged::IdentityOnly, "agreed, but an upload is queued");
    }
}
