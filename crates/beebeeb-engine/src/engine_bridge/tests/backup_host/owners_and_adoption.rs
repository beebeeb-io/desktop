//! The host's category folder (a known folder continues in its existing folder; a folder the person chose continues
//! only in the folder whose id its settings entry holds, never in one of its name, P3-10) and the per-account switches
//! a stop and its reconcile turn off (P3-8). Every name is invented.

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
        folder_id: None,
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

/// The fixture's account (the local data's owner).
fn me() -> crate::account_binding::Identity {
    crate::account_binding::Identity::new(Some("u-a"), Some("a@beebeeb.io"))
}

/// The account's chosen folder `name` (read from `path`) in the settings file, switched on, holding `folder_id` (what
/// its pass recorded; `None`: a new add).
fn choose(name: &str, path: &Path, folder_id: Option<&str>) {
    crate::config::DesktopConfig::update_for(&crate::identity::REAL, |cfg| {
        cfg.backup_chosen_folders.push(crate::backup::sources::ChosenFolder {
            name: name.into(),
            path: path.to_path_buf(),
            owner: crate::app::bindings::backup_folders::owner_tag(&me()),
            folder_id: folder_id.map(str::to_string),
        });
        (true, ())
    })
    .unwrap();
    switch_on(&[&format!("custom:{name}")]);
}

/// The folder id the settings entry `key` holds now.
fn recorded(key: &str) -> Option<String> {
    settings()
        .backup_chosen_folders
        .into_iter()
        .find(|folder| folder.key() == key)
        .and_then(|folder| folder.folder_id)
}

fn remembered(fx: &Fixture, key: &str) -> Option<String> {
    fx.bridge.db.backup_category_folder(key).unwrap()
}

fn projects() -> Category {
    Category::Custom("Projects".into())
}

/// P3-10, the pass side: a new chosen folder (no id in its entry) never adopts a live folder of its name. Here a kept
/// backup `Projects` (the person stopped a folder and kept its backup) is on the server and the local rows do not hold
/// it yet, so a new folder took the same name: the pass makes a new folder, records its id in the entry and remembers
/// it, and the kept backup is neither listed as this backup's nor written to.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_new_chosen_folder_never_adopts_a_live_one_of_its_name() {
    let config_dir = tempfile::tempdir().unwrap();
    crate::config::CONFIG_DIR_OVERRIDE
        .scope(config_dir.path().to_path_buf(), async {
            let fx = Fixture::new();
            let folders = tempfile::tempdir().unwrap();
            choose("Projects", &folders.path().join("Projects"), None);
            let kept = fx.add(Some(&fx.device.clone()), "Projects", true, 0);
            let kept_file = fx.add(Some(&kept), "plan.txt", false, 4);
            let host = fx.host(vec![projects()]);

            let tree = host.device_tree(&projects()).await.unwrap();
            let made = remembered(&fx, "custom:Projects").unwrap();
            assert_ne!(made, kept, "the kept backup is not adopted");
            assert_eq!(
                recorded("custom:Projects").as_deref(),
                Some(made.as_str()),
                "its id is in its entry"
            );
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
                assert_eq!(nodes[&made].parent.as_deref(), Some(fx.device.as_str()));
            }
            let requests = fx.server.finish();
            assert_eq!(Fixture::creates(&requests), 1, "a new folder");
            assert!(
                requests
                    .iter()
                    .all(|r| !r.path.contains(&kept) && !r.path.contains(&kept_file)),
                "nothing was asked of the kept backup"
            );
        })
        .await;
}

/// A sign-out empties the local state (the remembered folder, the records, the layout); the settings file keeps the
/// chosen folder and its id. The same account signs in again: the pass continues in that folder, rebuilds its records
/// from what it holds, uploads nothing again and makes no folder.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn signed_in_again_the_pass_continues_in_its_folder_and_uploads_nothing_again() {
    let config_dir = tempfile::tempdir().unwrap();
    crate::config::CONFIG_DIR_OVERRIDE
        .scope(config_dir.path().to_path_buf(), async {
            let fx = Fixture::new();
            *fx.server.date_ahead_s.lock().unwrap() = Some(180);
            // Whole seconds an hour ago, so every file system keeps the mtime exactly.
            let now_s = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_secs();
            let changed = std::time::UNIX_EPOCH + Duration::from_secs(now_s - 3600);
            let changed_ms = i64::try_from((now_s - 3600) * 1000).unwrap();
            let source = tempfile::tempdir().unwrap();
            let folder = fx.add(Some(&fx.device.clone()), "Projects", true, 0);
            // Each file's copy was made 30 s after it changed (the server's clock runs 180 s ahead).
            let copied = chrono::DateTime::from_timestamp_millis(changed_ms + 210_000).unwrap();
            for name in ["plan.txt", "notes.txt"] {
                let path = source.path().join(name);
                std::fs::write(&path, b"7 bytes").unwrap();
                std::fs::File::options()
                    .write(true)
                    .open(&path)
                    .unwrap()
                    .set_modified(changed)
                    .unwrap();
                let stamp = Some(copied.to_rfc3339_opts(chrono::SecondsFormat::Millis, true));
                fx.add_stamped(Some(&folder), name, false, 7, stamp);
            }
            choose("Projects", source.path(), Some(&folder));
            fx.bridge
                .db
                .set_backup_category_folder("custom:Projects", Some(&folder))
                .unwrap();

            fx.bridge.db.clear_account_data(false).unwrap();
            fx.bridge.db.set_owner(&me()).unwrap();
            assert_eq!(
                remembered(&fx, "custom:Projects"),
                None,
                "the sign-out emptied the local state"
            );

            let sync_root = tempfile::tempdir().unwrap();
            let report = run(&fx, &[(projects(), source.path())], sync_root.path()).await;
            assert!(matches!(report.passes.as_slice(), [(_, Ok(_))]), "{:?}", report.passes);
            assert_eq!(remembered(&fx, "custom:Projects").as_deref(), Some(folder.as_str()));
            assert_eq!(recorded("custom:Projects").as_deref(), Some(folder.as_str()));
            assert_eq!(
                fx.bridge.db.backup_records("custom:Projects").unwrap().len(),
                2,
                "the records are rebuilt from its folder"
            );
            assert_eq!(count_backup_ops(&fx.bridge), 0, "nothing is uploaded again");
            let requests = fx.server.finish();
            assert_eq!(Fixture::creates(&requests), 0, "no new folder");
        })
        .await;
}

/// "Back up again" after a stop (which forgets the remembered folder) continues in the folder whose id the entry
/// holds while it is there: no new folder.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn back_up_again_after_a_stop_continues_in_the_same_folder() {
    let config_dir = tempfile::tempdir().unwrap();
    crate::config::CONFIG_DIR_OVERRIDE
        .scope(config_dir.path().to_path_buf(), async {
            let fx = Fixture::new();
            let folders = tempfile::tempdir().unwrap();
            let folder = fx.add(Some(&fx.device.clone()), "Projects", true, 0);
            choose("Projects", &folders.path().join("Projects"), Some(&folder));
            fx.bridge
                .db
                .set_backup_category_folder("custom:Projects", Some(&folder))
                .unwrap();
            let host = fx.host(vec![projects()]);
            host.stop_backups(
                &projects(),
                beebeeb_types::backup::BackupStopReason::Unknown,
                false,
                false,
                None,
            )
            .await
            .unwrap();
            assert_eq!(remembered(&fx, "custom:Projects"), None, "the stop forgot the folder");

            crate::backup::stop::restart(&fx.bridge.db, "custom:Projects").unwrap();
            switch_on(&["custom:Projects"]);
            let tree = host.device_tree(&projects()).await.unwrap();
            assert_eq!(tree.folders[""], uuid(&folder));
            assert_eq!(remembered(&fx, "custom:Projects").as_deref(), Some(folder.as_str()));
            assert_eq!(recorded("custom:Projects").as_deref(), Some(folder.as_str()));
            let requests = fx.server.finish();
            assert_eq!(Fixture::creates(&requests), 0, "no new folder");
        })
        .await;
}

/// A folder create whose answer is lost (the server made it): its id was recorded in the entry before the create, so
/// the retry finds that folder by its id and continues there. One folder, never two.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_create_whose_answer_was_lost_is_found_by_its_id_and_never_made_twice() {
    let config_dir = tempfile::tempdir().unwrap();
    crate::config::CONFIG_DIR_OVERRIDE
        .scope(config_dir.path().to_path_buf(), async {
            let fx = Fixture::new();
            let folders = tempfile::tempdir().unwrap();
            choose("Projects", &folders.path().join("Projects"), None);
            fx.server.lose_create_answer.store(true, Ordering::SeqCst);
            let host = fx.host(vec![projects()]);

            assert!(
                host.device_tree(&projects()).await.is_err(),
                "the create's answer was lost"
            );
            let made = recorded("custom:Projects").expect("the id is recorded before the create");
            assert!(
                fx.server.nodes.lock().unwrap().contains_key(&made),
                "the server made it"
            );
            assert_eq!(remembered(&fx, "custom:Projects"), None);

            let tree = host.device_tree(&projects()).await.unwrap();
            assert_eq!(tree.folders[""], uuid(&made));
            assert_eq!(remembered(&fx, "custom:Projects").as_deref(), Some(made.as_str()));
            let requests = fx.server.finish();
            assert_eq!(Fixture::creates(&requests), 1, "one folder, never two");
        })
        .await;
}

/// A settings file from before the folder id (its entry has no `folder_id`) reads as "no folder yet": no error, and
/// never an adoption, by name or by the folder the local state remembers from the old adoption by name. A new folder
/// is made and recorded; the kept one is untouched.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_settings_file_from_before_the_folder_id_reads_as_no_folder_and_never_adopts() {
    let config_dir = tempfile::tempdir().unwrap();
    crate::config::CONFIG_DIR_OVERRIDE
        .scope(config_dir.path().to_path_buf(), async {
            let fx = Fixture::new();
            let folders = tempfile::tempdir().unwrap();
            let kept = fx.add(Some(&fx.device.clone()), "Projects", true, 0);
            fx.bridge
                .db
                .set_backup_category_folder("custom:Projects", Some(&kept))
                .unwrap();
            let tag = crate::app::bindings::backup_folders::owner_tag(&me()).unwrap();
            let file = crate::config::DesktopConfig::path_for(&crate::identity::REAL).unwrap();
            let path = folders.path().join("Projects");
            let text = format!(
                "known_folder_backup = [\"custom:Projects\"]\n\n[[backup_chosen_folders]]\n\
                 name = \"Projects\"\npath = {path:?}\nowner = \"{tag}\"\n"
            );
            std::fs::write(&file, text).unwrap();
            let entries = settings().backup_chosen_folders;
            assert_eq!(entries.len(), 1, "the old file reads");
            assert_eq!(entries[0].folder_id, None);

            let host = fx.host(vec![projects()]);
            let tree = host.device_tree(&projects()).await.unwrap();
            let made = remembered(&fx, "custom:Projects").unwrap();
            assert_ne!(made, kept, "the remembered folder of the old adoption is not continued");
            assert_eq!(recorded("custom:Projects").as_deref(), Some(made.as_str()));
            assert_eq!(tree.folders[""], uuid(&made));
            assert!(!fx.server.nodes.lock().unwrap()[&kept].trashed);
            let requests = fx.server.finish();
            assert_eq!(Fixture::creates(&requests), 1, "a new folder");
        })
        .await;
}

/// The folder whose id the entry holds is continued only inside this device's current folder: a live folder of that
/// id under another device folder (an earlier one of this device) is never written to; a new folder is made here.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_recorded_folder_outside_this_devices_folder_is_never_continued() {
    let config_dir = tempfile::tempdir().unwrap();
    crate::config::CONFIG_DIR_OVERRIDE
        .scope(config_dir.path().to_path_buf(), async {
            let fx = Fixture::new();
            let folders = tempfile::tempdir().unwrap();
            let earlier_device = fx.add(Some(&fx.root.clone()), "TestPC (earlier)", true, 0);
            let elsewhere = fx.add(Some(&earlier_device), "Projects", true, 0);
            choose("Projects", &folders.path().join("Projects"), Some(&elsewhere));
            let host = fx.host(vec![projects()]);

            let tree = host.device_tree(&projects()).await.unwrap();
            let made = remembered(&fx, "custom:Projects").unwrap();
            assert_ne!(made, elsewhere, "not continued outside this device's folder");
            assert_eq!(tree.folders[""], uuid(&made));
            assert_eq!(recorded("custom:Projects").as_deref(), Some(made.as_str()));
            assert_eq!(
                fx.server.nodes.lock().unwrap()[&made].parent.as_deref(),
                Some(fx.device.as_str())
            );
            let requests = fx.server.finish();
            assert_eq!(Fixture::creates(&requests), 1);
        })
        .await;
}
