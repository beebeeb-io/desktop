//! LD-0113: backups from source (spec 2026-11-06). The person's own folders are read in place and uploaded into
//! `Backups/<device>/<category>` on the server; nothing is copied into the sync root.
//!
//! - `host`: `DesktopBackupHost`, desktop's `BackupHost` for core's pass.
//! - `scheduler`: when the passes run (the 10-minute pass and the debounced one after a change); it replaces the
//!   old mirror.
//! - `device`: this device for core's identity rule (its name from today's source, its machine-derived id).
//! - `sources`: what a backup may read from (never the sync root) and what its walk leaves out.
//! - `kfm`: a known folder OneDrive's Known Folder Move has moved, asked about once (Amendment 9).
//! - `layout`: where the backup lives on the server: the Backups root (found, claimed or created, legacy duplicates
//!   merged) and this device's folder (core's identity rule, the device manifest written on adoption and creation).

pub mod device;
pub mod host;
pub mod kfm;
pub mod layout;
pub mod scheduler;
pub mod sources;
pub mod stop;

use crate::backup_protect::{OwnBackupFolder, OwnBackups};
use crate::state_db::StateDb;

/// What this device backs up now, for the drive protection: the device folder's name as the resolved layout has it
/// (core's `find_device_folder`, e.g. `GLegion (a3f2)` after a name collision), never re-derived from the
/// hostname, and the folders that are on with where each is read from. No network. `None` when the settings
/// cannot be read: the caller keeps its last answer (a read error never lifts a protection). Before the layout is
/// resolved the device folder is `None`.
pub fn own_backups(db: &StateDb) -> Option<OwnBackups> {
    let cfg = crate::config::DesktopConfig::load().ok()?;
    Some(own_backups_for(
        db,
        &sources::backup_sources_quiet(&cfg.known_folder_backup, &cfg.known_folder_source),
    ))
}

pub fn own_backups_for(db: &StateDb, sources: &[scheduler::BackupSource]) -> OwnBackups {
    OwnBackups {
        device_folder: db
            .backup_layout()
            .ok()
            .flatten()
            .map(|layout| layout.device_folder_name),
        folders: sources
            .iter()
            .map(|source| OwnBackupFolder {
                display_name: source.category.display_name().to_string(),
                source_path: source.path.clone(),
            })
            .collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The device folder protection guards is the one the layout resolved (`GLegion (a3f2)` after a collision),
    /// never `sanitize_device_segment` of the hostname; before the layout exists there is none.
    #[test]
    fn own_backups_names_the_resolved_device_folder() {
        let dir = tempfile::tempdir().unwrap();
        let db = StateDb::open(&dir.path().join("state.db")).unwrap();
        db.set_owner(&crate::account_binding::Identity::new(Some("u-a"), None))
            .unwrap();
        let sources = [scheduler::BackupSource {
            category: beebeeb_types::backup::Category::Pictures,
            path: dir.path().join("Pictures"),
        }];
        assert_eq!(
            own_backups_for(&db, &sources).device_folder,
            None,
            "no layout, no device folder"
        );

        let layout = crate::state_db::BackupLayout {
            root_id: "0193b1a6-0000-7000-8000-000000000001".into(),
            device_folder_id: "0193b1a6-0000-7000-8000-000000000002".into(),
            device_folder_name: "GLegion (a3f2)".into(),
        };
        assert!(db.set_backup_layout(&layout).unwrap());
        let own = own_backups_for(&db, &sources);
        assert_eq!(own.device_folder.as_deref(), Some("GLegion (a3f2)"));
        assert_eq!(
            own.folders,
            vec![OwnBackupFolder {
                display_name: "Pictures".into(),
                source_path: dir.path().join("Pictures")
            }]
        );
    }
}
