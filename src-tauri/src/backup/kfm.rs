//! LD-0113 Amendment 9 (Guus, 2026-11-06, decision 2338: "Ask once"): known folders that OneDrive's Known Folder Move
//! has moved.
//!
//! Desktop resolves both locations of each known folder: the default one (`KF_FLAG_DEFAULT_PATH`) and the current one
//! (OneDrive's, when Known Folder Move redirected it). When they differ the default folder is usually empty, so the
//! person is asked once, per moved folder, which one to back up: OneDrive's folder, where the files are, or the default
//! one. The answer is kept in the folder's backup setting (`known_folder_source`). Until it is answered that folder is
//! not backed up, and the Backups screen says it is waiting for the answer.
//!
//! Unchanged: one way. Nothing is ever written into OneDrive's folder, and an online-only file there is never read
//! (core's walk reports it `NotLocal`, rule 8), so nothing is downloaded.
//!
//! The words are desktop's until core's screen model has them (the amendment puts the question and its follow-up
//! note in core; core v0.2.1 has neither): [`question`] and [`not_local_note`].

use std::path::{Path, PathBuf};

use crate::config::KnownFolderSource;

/// Both locations of one known folder on this device.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct KnownFolderPaths {
    /// The default location (`KF_FLAG_DEFAULT_PATH`).
    pub(crate) default: Option<PathBuf>,
    /// The current location (Known Folder Move's redirection included).
    pub(crate) current: Option<PathBuf>,
}

/// Where a known folder is backed up from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum SourceResolution {
    Read(PathBuf),
    /// OneDrive moved it and the person has not answered yet: it is not backed up.
    Waiting { default: PathBuf, moved: PathBuf },
    /// This device has no such folder.
    Unavailable,
}

/// Whether OneDrive (or anything else) moved the folder: its current location is not its default one.
pub(crate) fn is_moved(paths: &KnownFolderPaths) -> bool {
    match (&paths.default, &paths.current) {
        (Some(default), Some(current)) => !super::sources::same_folder(default, current),
        _ => false,
    }
}

/// Where to back the folder up from, given the person's answer (if any).
pub(crate) fn resolve(paths: &KnownFolderPaths, choice: Option<KnownFolderSource>) -> SourceResolution {
    let Some(default) = paths.default.clone() else {
        return SourceResolution::Unavailable;
    };
    if !is_moved(paths) {
        return SourceResolution::Read(default);
    }
    let moved = paths.current.clone().unwrap_or_else(|| default.clone());
    match choice {
        Some(KnownFolderSource::OneDrive) => SourceResolution::Read(moved),
        Some(KnownFolderSource::Default) => SourceResolution::Read(default),
        None => SourceResolution::Waiting { default, moved },
    }
}

/// Both locations of the known folder `key` on this device. Windows only: no other system has known folders.
pub(crate) fn known_folder_paths(key: &str) -> KnownFolderPaths {
    #[cfg(target_os = "windows")]
    {
        KnownFolderPaths {
            default: crate::known_folder::resolve_known_folder(key),
            current: crate::known_folder::resolve_known_folder_current(key),
        }
    }
    #[cfg(not(target_os = "windows"))]
    {
        let _ = key;
        KnownFolderPaths::default()
    }
}

/// What the Backups card shows for one known folder.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct FolderView {
    /// Where it is backed up from (the default location while the question is open).
    pub(crate) source_path: Option<PathBuf>,
    /// `source_path` is the default location (counted for the card; OneDrive's folder never is).
    pub(crate) reads_default: bool,
    pub(crate) moved: MovedFolder,
}

/// The card's fields for a folder OneDrive moved (all `None` for one it did not).
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize)]
pub(crate) struct MovedFolder {
    /// Where the folder is now.
    pub(crate) moved_path: Option<String>,
    /// It is under one of OneDrive's folders (Known Folder Move); else the person or a policy moved it.
    pub(crate) moved_by_onedrive: bool,
    pub(crate) source_choice: Option<KnownFolderSource>,
    /// While the folder is on and unanswered: it is not backed up until then.
    pub(crate) question: Option<String>,
    /// After a pass of OneDrive's folder that left online-only items out.
    pub(crate) note: Option<String>,
}

/// [`FolderView`] of the known folder `key` on this device.
pub(crate) fn folder_view(key: &str, display_name: &str, enabled: bool, choice: Option<KnownFolderSource>) -> FolderView {
    let paths = known_folder_paths(key);
    let moved = is_moved(&paths);
    let by_onedrive = paths.current.as_deref().is_some_and(moved_by_onedrive);
    let (source_path, question) = match resolve(&paths, choice) {
        SourceResolution::Read(path) => (Some(path), None),
        SourceResolution::Waiting { default, moved } => {
            (Some(default.clone()), enabled.then(|| question(display_name, &default, &moved, by_onedrive)))
        }
        SourceResolution::Unavailable => (None, None),
    };
    let note = (moved && choice == Some(KnownFolderSource::OneDrive))
        .then(|| super::scheduler::not_local_count(key))
        .flatten()
        .filter(|count| *count > 0)
        .map(not_local_note);
    FolderView {
        reads_default: source_path.is_some() && source_path == paths.default,
        source_path,
        moved: MovedFolder {
            moved_path: moved.then(|| paths.current.as_ref().map(|p| p.to_string_lossy().into_owned())).flatten(),
            moved_by_onedrive: moved && by_onedrive,
            source_choice: moved.then_some(choice).flatten(),
            question,
            note,
        },
    }
}

/// Whether `current` lies in one of OneDrive's folders (`%OneDrive%`, `%OneDriveConsumer%`, `%OneDriveCommercial%`):
/// Known Folder Move. A folder moved anywhere else was moved by the person or a policy, and the words say so (review
/// 2026-11-07); the question is the same.
pub(crate) fn moved_by_onedrive(current: &Path) -> bool {
    ["OneDrive", "OneDriveConsumer", "OneDriveCommercial"]
        .iter()
        .filter_map(std::env::var_os)
        .map(PathBuf::from)
        .filter(|root| root.is_absolute())
        .any(|root| super::sources::check_source(current, &root) == Err(super::sources::SourceRefusal::InsideSyncRoot))
}

/// The question, naming the folder and both locations.
pub(crate) fn question(display_name: &str, default: &Path, moved: &Path, by_onedrive: bool) -> String {
    if by_onedrive {
        format!(
            "OneDrive has moved your {display_name} folder. Back up OneDrive's folder ({}), where your files are, or the default one ({})?",
            moved.display(),
            default.display()
        )
    } else {
        format!(
            "Your {display_name} folder has moved. Back up the folder where your files are now ({}), or the default one ({})?",
            moved.display(),
            default.display()
        )
    }
}

/// The follow-up note once OneDrive's folder is backed up and some of it is only in OneDrive.
pub(crate) fn not_local_note(count: u64) -> String {
    match count {
        1 => "1 file is online-only in OneDrive and wasn't backed up.".to_string(),
        n => format!("{n} files are online-only in OneDrive and weren't backed up."),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backup::sources::backup_sources_with;
    use crate::state_db::StateDb;
    use std::collections::BTreeMap;

    fn db(dir: &Path) -> StateDb {
        let db = StateDb::open(dir.join("state.db")).unwrap();
        db.set_owner(&crate::account_binding::Identity::new(Some("u-a"), None)).unwrap();
        db
    }

    /// The default Pictures and OneDrive's, as Known Folder Move leaves them: the default one empty.
    fn moved_pictures(dir: &Path) -> (PathBuf, PathBuf) {
        let default = dir.join("Users").join("guus").join("Pictures");
        let onedrive = dir.join("OneDrive").join("Pictures");
        std::fs::create_dir_all(&default).unwrap();
        std::fs::create_dir_all(&onedrive).unwrap();
        std::fs::write(onedrive.join("beach.jpg"), b"photo").unwrap();
        (default, onedrive)
    }

    fn pictures_on() -> Vec<String> {
        vec!["pictures".to_string()]
    }

    /// Amendment 9: two passes over a moved folder give one question, and the folder is not backed up meanwhile. Once
    /// answered, the answer is used on every pass and nothing is asked again.
    #[test]
    fn kfm_moved_folder_asks_once_and_remembers() {
        let dir = tempfile::tempdir().unwrap();
        let db = db(dir.path());
        let (default, onedrive) = moved_pictures(dir.path());
        let paths = |key: &str| match key {
            "pictures" => KnownFolderPaths { default: Some(default.clone()), current: Some(onedrive.clone()) },
            _ => KnownFolderPaths::default(),
        };
        let mut choices = BTreeMap::new();

        let first = backup_sources_with(Some(&db), &pictures_on(), &choices, paths);
        let second = backup_sources_with(Some(&db), &pictures_on(), &choices, paths);
        assert!(first.sources.is_empty() && second.sources.is_empty(), "not backed up until answered");
        assert_eq!(first.newly_asked, ["pictures"], "asked on the first pass");
        assert!(second.newly_asked.is_empty(), "two passes, one question");
        let open = db.backup_source_questions().unwrap();
        assert_eq!(open.len(), 1);
        assert_eq!(open[0].1, default.to_string_lossy());
        assert_eq!(open[0].2, onedrive.to_string_lossy());
        assert!(question("Pictures", &default, &onedrive, true).starts_with("OneDrive has moved your Pictures folder."));
        assert!(question("Pictures", &default, &onedrive, false).starts_with("Your Pictures folder has moved."));
        assert!(question("Pictures", &default, &onedrive, false).contains(&*onedrive.to_string_lossy()));

        choices.insert("pictures".to_string(), KnownFolderSource::OneDrive);
        for pass in 0..2 {
            let read = backup_sources_with(Some(&db), &pictures_on(), &choices, paths);
            assert_eq!(read.sources.len(), 1, "pass {pass}");
            assert_eq!(read.sources[0].path, onedrive, "the answer is remembered");
            assert!(read.newly_asked.is_empty(), "never asked again");
            assert!(db.backup_source_questions().unwrap().is_empty(), "the question is settled");
        }
    }

    /// A folder OneDrive did not move is backed up from its default location as before, whatever is stored, and a
    /// moved folder whose answer is "the default one" is backed up from exactly that location; nothing is asked.
    #[test]
    fn kfm_default_choice_unchanged() {
        let dir = tempfile::tempdir().unwrap();
        let db = db(dir.path());
        let (default, onedrive) = moved_pictures(dir.path());
        let not_moved = |_: &str| KnownFolderPaths { default: Some(default.clone()), current: Some(default.clone()) };
        let unknown_current = |_: &str| KnownFolderPaths { default: Some(default.clone()), current: None };
        let moved = |_: &str| KnownFolderPaths { default: Some(default.clone()), current: Some(onedrive.clone()) };
        let none = BTreeMap::new();
        let default_chosen = BTreeMap::from([("pictures".to_string(), KnownFolderSource::Default)]);
        let onedrive_chosen = BTreeMap::from([("pictures".to_string(), KnownFolderSource::OneDrive)]);

        for (label, read) in [
            ("not moved", backup_sources_with(Some(&db), &pictures_on(), &none, not_moved)),
            ("not moved, an old answer", backup_sources_with(Some(&db), &pictures_on(), &onedrive_chosen, not_moved)),
            ("no current location", backup_sources_with(Some(&db), &pictures_on(), &none, unknown_current)),
            ("moved, the default chosen", backup_sources_with(Some(&db), &pictures_on(), &default_chosen, moved)),
        ] {
            let paths: Vec<&Path> = read.sources.iter().map(|s| s.path.as_path()).collect();
            assert_eq!(paths, [default.as_path()], "{label}");
            assert!(read.newly_asked.is_empty(), "{label}: nothing asked");
        }
        assert!(db.backup_source_questions().unwrap().is_empty());
        let off = backup_sources_with(Some(&db), &[], &none, moved);
        assert!(off.sources.is_empty() && off.newly_asked.is_empty(), "a folder that is off is never asked about");
    }
}
