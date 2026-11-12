//! Native rung of `sync_root_owner`: Windows names the provider a folder is registered to, and neither app takes,
//! or re-registers, a folder that is another provider's (here the tests' own QA provider). A plain folder has no
//! provider and is free.

use crate::identity::{REAL, TEST};
use crate::windows_cf::{HYDRATION_POLICY, QA_PROVIDER, register_sync_root, register_sync_root_with};
use crate::windows_cf::{registered_provider_name, unregister_sync_root};

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
        let _ = unregister_sync_root(&self.root);
    }
}

#[test]
fn a_root_registered_to_another_provider_is_read_and_refused() {
    let qa = QaFolder::new();
    assert_eq!(registered_provider_name(&qa.root).as_deref(), Some("BeebeebQA"));
    for identity in [&REAL, &TEST] {
        let refusal = crate::sync_root_owner::check(identity, &qa.root).expect_err("another provider's folder");
        assert!(refusal.contains("BeebeebQA"), "{refusal}");
        assert!(register_sync_root(identity, &qa.root).is_err());
    }
    assert_eq!(
        registered_provider_name(&qa.root).as_deref(),
        Some("BeebeebQA"),
        "the refused registration changed nothing"
    );
}

#[test]
fn a_plain_folder_has_no_provider_and_is_free() {
    let plain = tempfile::tempdir().unwrap();
    assert_eq!(registered_provider_name(plain.path()), None);
    for identity in [&REAL, &TEST] {
        assert_eq!(crate::sync_root_owner::check(identity, plain.path()), Ok(()));
    }
}
