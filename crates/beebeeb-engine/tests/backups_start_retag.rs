//! M-R5: the sync start does not re-tag an earlier build's address-tagged chosen folders by the account's id
//! (`backup_folders::retag_at_start`). On the fake API, settings written to the test's sandboxed file. One test
//! per file: the state folder is resolved once per process. Every name is invented.

// The one test is not run on macOS; its helpers are left unused there.
#![cfg_attr(target_os = "macos", allow(dead_code, unused_imports))]

mod common;

use beebeeb_engine::app::bindings::backup_folders::owner_tag;
use beebeeb_engine::backup::sources::ChosenFolder;
use beebeeb_engine::config::DesktopConfig;
use beebeeb_engine::identity::TEST;
use sha2::{Digest, Sha256};

fn chosen(name: &str, path: std::path::PathBuf, owner: Option<String>) -> ChosenFolder {
    ChosenFolder {
        name: name.into(),
        path,
        owner,
        folder_id: None,
    }
}

fn folders() -> Vec<ChosenFolder> {
    DesktopConfig::load_for(&TEST).unwrap().backup_chosen_folders
}

#[cfg(not(target_os = "macos"))]
#[test]
fn the_sync_start_retags_an_address_tagged_chosen_folder_by_the_accounts_id() {
    use beebeeb_engine::app::SyncStart;

    let s = common::setup();
    let account = s.this_account();
    let by_id = owner_tag(&account);
    assert!(by_id.is_some());
    let address = beebeeb_engine::account_binding::canonical_email(&s.account.email);
    let by_address = Some(beebeeb_engine::hexfmt::encode(&Sha256::digest(
        format!("email:{address}").as_bytes(),
    )));
    DesktopConfig::update_for(&TEST, |config| {
        config.backup_chosen_folders = vec![chosen("Maps", s.home.join("Maps"), by_address.clone())];
        (true, ())
    })
    .unwrap();

    s.sign_in();
    s.unlock();
    assert_eq!(folders()[0].owner, by_address, "nobody's until the account starts");
    assert_eq!(s.engine.start_sync(s.home.join("Beebeeb Test")), Ok(SyncStart::Started));
    assert_eq!(
        folders()[0].owner,
        by_id,
        "the start marks the folder with the account's id"
    );
    s.engine.stop();
}

#[cfg(target_os = "macos")]
#[test]
fn the_test_build_is_refused_on_macos() {
    common::assert_the_test_build_is_refused();
}
