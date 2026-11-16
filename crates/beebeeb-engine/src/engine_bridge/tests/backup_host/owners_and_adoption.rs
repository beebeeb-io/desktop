//! The host's category folder (a known folder continues in its existing folder; a folder the person chose never adopts
//! one of its name, P3-10) and the per-account switches a stop and its reconcile turn off (P3-8). Every name is
//! invented.

use super::*;

/// A first backup adopts the device folder's existing `Pictures` (the old mirror's) instead of creating a second
/// one, remembers it, and creates a category folder only where there is none.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn host_adopts_the_existing_category_folder_and_remembers_it() {
    let fx = Fixture::new();
    let legacy = fx.add(Some(&fx.device.clone()), "Pictures", true, 0);
    fx.add(Some(&legacy), "old.jpg", false, 3);
    let host = fx.host(vec![Category::Pictures, Category::Documents]);

    let tree = host.device_tree(&Category::Pictures).await.unwrap();
    assert_eq!(tree.category_folder_live, None, "a first backup on this device");
    assert_eq!(tree.folders[""], uuid(&legacy));
    assert_eq!(tree.files.len(), 1);
    assert_eq!(
        fx.bridge.db.backup_category_folder("pictures").unwrap().as_deref(),
        Some(legacy.as_str())
    );

    let documents = host.device_tree(&Category::Documents).await.unwrap();
    let requests = fx.server.finish();
    assert_eq!(Fixture::creates(&requests), 1, "only Documents had no folder");
    assert!(documents.files.is_empty() && documents.complete);
    let made = fx.bridge.db.backup_category_folder("documents").unwrap().unwrap();
    assert_eq!(documents.folders[""], uuid(&made));
}

/// The settings file's backup folders for the per-account tests: `Letters` is the local data's owner's (A, the
/// fixture's), `Recipes` another account's (B) on this PC, `Drafts` untagged (nobody's, P3-8); every switch on, with
/// Pictures.
fn three_accounts_folders(folders: &Path) {
    use crate::account_binding::Identity;
    use crate::app::bindings::backup_folders::owner_tag;
    let a = Identity::new(Some("u-a"), Some("a@beebeeb.io"));
    let b = Identity::new(Some("u-b"), Some("b@beebeeb.io"));
    let chosen = |name: &str, owner: Option<&Identity>| crate::backup::sources::ChosenFolder {
        name: name.into(),
        path: folders.join(name),
        owner: owner.and_then(owner_tag),
    };
    crate::config::DesktopConfig::update_for(&crate::identity::REAL, |cfg| {
        cfg.backup_chosen_folders = vec![
            chosen("Letters", Some(&a)),
            chosen("Recipes", Some(&b)),
            chosen("Drafts", None),
        ];
        (true, ())
    })
    .unwrap();
    switch_on(&["pictures", "custom:Letters", "custom:Recipes", "custom:Drafts"]);
}

/// The per-account rule of P3-8 (data safety): when every backup of this device stops (here its
/// device folder is gone), the switches that turn off are the local data's owner's alone. Another account's chosen
/// folder and an untagged one keep their switch on, and no stop row is written for them; the owner's own chosen
/// folder, which this pass did not run, turns off with Pictures.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_stop_of_every_backup_turns_off_the_bound_accounts_switches_alone() {
    let config_dir = tempfile::tempdir().unwrap();
    crate::config::CONFIG_DIR_OVERRIDE
        .scope(config_dir.path().to_path_buf(), async {
            let fx = Fixture::new();
            let folders = tempfile::tempdir().unwrap();
            three_accounts_folders(folders.path());
            let host = fx.host(vec![Category::Pictures]);
            host.stop_backups(
                &Category::Pictures,
                beebeeb_types::backup::BackupStopReason::Unknown,
                true,
                true,
                None,
            )
            .await
            .unwrap();
            assert_eq!(
                settings().known_folder_backup,
                ["custom:Recipes", "custom:Drafts"],
                "only the owner's switches turned off"
            );
            let stopped: Vec<String> = stops(&fx.bridge).into_iter().map(|(key, _)| key).collect();
            assert_eq!(stopped, ["custom:Letters", "pictures"]);
        })
        .await;
}

/// The per-account rule of P3-8, the other writer of the switches: the reconcile of a stopped backup whose switch
/// stayed on (`stop::reconcile_stopped_switches`) turns off the owner's own keys alone. Here the owner's state holds an
/// older stop row for `custom:Recipes` (its own folder of that name, stopped, then removed), and another account has
/// since chosen a folder named `Recipes`: that account's switch stays on.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_reconcile_turns_off_the_bound_accounts_stopped_switches_alone() {
    let config_dir = tempfile::tempdir().unwrap();
    crate::config::CONFIG_DIR_OVERRIDE
        .scope(config_dir.path().to_path_buf(), async {
            let fx = Fixture::new();
            let folders = tempfile::tempdir().unwrap();
            three_accounts_folders(folders.path());
            let keys = ["custom:Letters".to_string(), "custom:Recipes".to_string()];
            fx.bridge
                .db
                .record_backup_stops(&crate::state_db::BackupStopWrite {
                    reason: "unknown",
                    at_ms: now_ms(),
                    categories: &keys,
                    clear_layout: false,
                    marks: &[],
                })
                .unwrap();
            let stopped =
                crate::backup::stop::reconcile_stopped_switches(&crate::identity::REAL, &fx.bridge.db).unwrap();
            assert_eq!(stopped, keys, "the owner's stop rows, read as they are");
            assert_eq!(
                settings().known_folder_backup,
                ["pictures", "custom:Recipes", "custom:Drafts"],
                "the owner's stopped switch turned off, the other account's kept"
            );
        })
        .await;
}

/// P3-10, the pass side: a chosen folder's backup with no folder remembered never adopts a live
/// folder of its name. Here a kept backup `Projects` (the person stopped a folder and kept its backup) is on the
/// server and the local rows do not hold it yet, so a new folder took the same name: the pass makes a new folder and
/// remembers it, and the kept backup is neither listed as this backup's nor written to. A known folder still adopts
/// its folder (`host_adopts_the_existing_category_folder_and_remembers_it`).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_chosen_folder_with_no_remembered_folder_never_adopts_a_live_one_of_its_name() {
    let fx = Fixture::new();
    let kept = fx.add(Some(&fx.device.clone()), "Projects", true, 0);
    let kept_file = fx.add(Some(&kept), "plan.txt", false, 4);
    let projects = Category::Custom("Projects".into());
    let host = fx.host(vec![projects.clone()]);

    let tree = host.device_tree(&projects).await.unwrap();
    let made = fx.bridge.db.backup_category_folder("custom:Projects").unwrap().unwrap();
    assert_ne!(made, kept, "the kept backup is not adopted");
    assert_eq!(tree.folders[""], uuid(&made));
    assert!(
        tree.files.is_empty(),
        "the kept backup's files are not this backup's: {:?}",
        tree.files
    );
    {
        let nodes = fx.server.nodes.lock().unwrap();
        assert!(!nodes[&kept].trashed && nodes[&kept].parent.as_deref() == Some(fx.device.as_str()));
        assert_eq!(nodes[&kept_file].parent.as_deref(), Some(kept.as_str()));
    }
    let requests = fx.server.finish();
    assert_eq!(Fixture::creates(&requests), 1, "a new folder");
    assert!(
        requests
            .iter()
            .all(|r| !r.path.contains(&kept) && !r.path.contains(&kept_file)),
        "nothing was asked of the kept backup"
    );
}
