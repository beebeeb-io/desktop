//! M-N6 (one test per file: the state folder is resolved once per process): `Engine::set_backup_folder` skips the double-backup rule (`switch_on_refusal`) when switching on.

mod common;

use beebeeb_engine::app::backups_api::BackupsError;
use beebeeb_engine::app::bindings::backup_folders::owner_tag;
use beebeeb_engine::backup::sources::ChosenFolder;
use beebeeb_engine::config::DesktopConfig;
use beebeeb_engine::identity::TEST;

fn chosen(name: &str, path: std::path::PathBuf, owner: Option<String>) -> ChosenFolder {
    ChosenFolder {
        name: name.into(),
        path,
        owner,
        folder_id: None,
    }
}

#[test]
fn a_chosen_folder_is_not_switched_on_over_one_that_is_backed_up() {
    let s = common::setup();
    s.sign_in();
    s.unlock();
    let mine = owner_tag(&s.this_account());
    let outer = s.home.join("Papers");
    let inner = outer.join("Letters");
    let apart = s.home.join("Recipes");
    for dir in [&inner, &apart] {
        std::fs::create_dir_all(dir).unwrap();
    }
    // Letters is backed up; Papers (holding it) and Recipes (apart) are stopped.
    DesktopConfig::update_for(&TEST, |config| {
        config.backup_chosen_folders = vec![
            chosen("Letters", inner.clone(), mine.clone()),
            chosen("Papers", outer.clone(), mine.clone()),
            chosen("Recipes", apart.clone(), mine.clone()),
        ];
        config.set_known_folder("custom:Letters", true);
        config.set_known_folder("custom:Papers", false);
        config.set_known_folder("custom:Recipes", false);
        (true, ())
    })
    .unwrap();
    let on = |key: &str| DesktopConfig::load_for(&TEST).unwrap().known_folder_enabled(key);

    assert_eq!(
        s.engine.set_backup_folder("custom:Papers", true),
        Err(BackupsError::Nested),
        "Papers holds Letters, which is backed up: refused"
    );
    assert!(!on("custom:Papers"), "a refused switch writes nothing");

    assert_eq!(s.engine.set_backup_folder("custom:Recipes", true), Ok(()));
    assert!(on("custom:Recipes"), "a folder apart is switched on");
}
