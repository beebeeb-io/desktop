//! Native rung of `sync_root_owner`: Windows names the provider a folder is registered to, and neither app takes,
//! or re-registers, a folder that is another provider's (here the tests' own QA provider). A plain folder has no
//! provider and is free; so is a folder that does not exist yet. Neither app unregisters another provider's folder.

use crate::identity::{REAL, TEST};
use crate::windows_cf::{HYDRATION_POLICY, QA_PROVIDER, register_sync_root, register_sync_root_with};
use crate::windows_cf::{registered_provider_name, unregister_any_sync_root, unregister_sync_root};

/// A folder registered to the QA provider; unregistered on drop, whatever the test did.
struct QaFolder {
    _dir: tempfile::TempDir,
    root: std::path::PathBuf,
}

impl QaFolder {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("qa-root");
        std::fs::create_dir(&root).unwrap();
        register_sync_root_with(&root, &QA_PROVIDER, HYDRATION_POLICY).unwrap();
        Self { _dir: dir, root }
    }
}

impl Drop for QaFolder {
    fn drop(&mut self) {
        let _ = unregister_any_sync_root(&self.root);
    }
}

#[test]
fn a_root_registered_to_another_provider_is_read_and_refused() {
    let qa = QaFolder::new();
    assert_eq!(registered_provider_name(&qa.root).as_deref(), Ok("BeebeebQA"));
    for identity in [&REAL, &TEST] {
        let refusal = crate::sync_root_owner::check(identity, &qa.root).expect_err("another provider's folder");
        assert!(refusal.contains("BeebeebQA"), "{refusal}");
        assert!(register_sync_root(identity, &qa.root).is_err());
    }
    assert_eq!(
        registered_provider_name(&qa.root).as_deref(),
        Ok("BeebeebQA"),
        "the refused registration changed nothing"
    );
}

/// Review of #309, P2-3: a sign-out of one app must not unregister the folder another provider registered (an
/// unregister deletes every cloud-only placeholder in it).
#[test]
fn neither_app_unregisters_another_providers_root() {
    let qa = QaFolder::new();
    for identity in [&REAL, &TEST] {
        let refusal = unregister_sync_root(identity, &qa.root).expect_err("another provider's folder");
        assert!(refusal.to_string().contains("BeebeebQA"), "{refusal}");
        assert_eq!(
            registered_provider_name(&qa.root).as_deref(),
            Ok("BeebeebQA"),
            "the refused unregistration left the registration"
        );
    }
}

/// An app unregisters its own root, and a second unregistration of the then plain folder is not an error.
#[test]
fn an_app_unregisters_its_own_root() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("own-root");
    std::fs::create_dir(&root).unwrap();
    struct Unregister(std::path::PathBuf);
    impl Drop for Unregister {
        fn drop(&mut self) {
            let _ = unregister_any_sync_root(&self.0);
        }
    }
    let _cleanup = Unregister(root.clone());
    register_sync_root(&TEST, &root).unwrap();
    assert_eq!(registered_provider_name(&root).as_deref(), Ok("BeebeebTest"));
    assert!(
        unregister_sync_root(&REAL, &root).is_err(),
        "the real app leaves the test build's root"
    );
    assert_eq!(registered_provider_name(&root).as_deref(), Ok("BeebeebTest"));
    unregister_sync_root(&TEST, &root).unwrap();
    assert!(registered_provider_name(&root).is_err());
    unregister_sync_root(&TEST, &root).unwrap();
}

#[test]
fn a_plain_folder_has_no_provider_and_is_free() {
    let plain = tempfile::tempdir().unwrap();
    let code = registered_provider_name(plain.path()).expect_err("a plain folder has no provider");
    assert!(
        [
            crate::sync_root_owner::NOT_A_CLOUD_FILE,
            crate::sync_root_owner::NOT_UNDER_SYNC_ROOT
        ]
        .contains(&code),
        "Windows answered a plain folder with {code:#x}"
    );
    use windows::Win32::Foundation::{ERROR_CLOUD_FILE_NOT_UNDER_SYNC_ROOT, ERROR_NOT_A_CLOUD_FILE};
    assert_eq!(
        (
            windows::core::HRESULT::from_win32(ERROR_NOT_A_CLOUD_FILE.0).0,
            windows::core::HRESULT::from_win32(ERROR_CLOUD_FILE_NOT_UNDER_SYNC_ROOT.0).0
        ),
        (
            crate::sync_root_owner::NOT_A_CLOUD_FILE,
            crate::sync_root_owner::NOT_UNDER_SYNC_ROOT
        ),
        "the two codes are Windows' own"
    );
    for identity in [&REAL, &TEST] {
        assert_eq!(crate::sync_root_owner::check(identity, plain.path()), Ok(()));
    }
}

/// The first start: the sync folder is not there yet. Windows answers "not found", which is no answer about a
/// registration; the check does not ask Windows about a folder that does not exist.
#[test]
fn a_folder_that_does_not_exist_yet_is_free_and_its_read_is_not_an_answer() {
    let dir = tempfile::tempdir().unwrap();
    let gone = dir.path().join("not-made-yet");
    let code = registered_provider_name(&gone).expect_err("nothing there");
    assert_eq!(
        crate::sync_root_owner::registration_of(Err(code)),
        crate::sync_root_owner::Registration::Unreadable,
        "{code:#x} is not one of the two codes"
    );
    for identity in [&REAL, &TEST] {
        assert_eq!(crate::sync_root_owner::check(identity, &gone), Ok(()));
    }
}
