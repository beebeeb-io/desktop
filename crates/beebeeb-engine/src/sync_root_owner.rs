//! Whose sync folder is this? An engine never starts in a folder that Windows has registered to another sync
//! provider: not the test build in the real app's folder (the incident of 2026-11-11: two engines on one root),
//! not the real app in the test build's, not either in a third provider's.
//!
//! [`refusal`] is the whole decision, as a pure function. [`check`] reads the folder's registration (Windows
//! Cloud Files; no other OS has one) and asks it. Callers: the engine handle before it starts a sync, the runner
//! before it connects the root, and `windows_cf::register_sync_root` before it registers anything.

use std::path::Path;

use crate::identity::EngineIdentity;

/// Why `identity` must not use `root`, given the name of the provider the OS has the folder registered to
/// (`None`: registered to nobody), or `None` when it may. A folder is this app's to use when nobody registered it
/// or this app's own provider did; Windows compares provider names without case, so this does too.
pub fn refusal(identity: &EngineIdentity, registered_provider: Option<&str>, root: &Path) -> Option<String> {
    let provider = registered_provider?;
    if provider.eq_ignore_ascii_case(identity.cloud_files_provider_name()) {
        return None;
    }
    let folder = root
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    let owner = if provider.is_empty() { "another app" } else { provider };
    Some(format!(
        "The folder \u{201c}{folder}\u{201d} is already the sync folder of {owner}. Sync was not started and nothing was changed. Choose another folder."
    ))
}

/// The name of the provider the OS has `root` registered to, if any.
fn registered_provider(root: &Path) -> Option<String> {
    #[cfg(target_os = "windows")]
    {
        crate::windows_cf::registered_provider_name(root)
    }
    #[cfg(not(target_os = "windows"))]
    {
        let _ = root;
        None
    }
}

/// [`refusal`] for `root` as the OS has it now.
pub fn check(identity: &EngineIdentity, root: &Path) -> Result<(), String> {
    refusal(identity, registered_provider(root).as_deref(), root).map_or(Ok(()), Err)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::identity::{REAL, TEST};

    fn root() -> &'static Path {
        Path::new(if cfg!(windows) {
            r"C:\Users\a\Beebeeb"
        } else {
            "/home/a/Beebeeb"
        })
    }

    #[test]
    fn a_folder_nobody_registered_or_this_app_registered_is_used() {
        for identity in [&REAL, &TEST] {
            assert_eq!(refusal(identity, None, root()), None);
        }
        assert_eq!(refusal(&REAL, Some("Beebeeb"), root()), None);
        assert_eq!(
            refusal(&REAL, Some("BEEBEEB"), root()),
            None,
            "Windows compares these names without case"
        );
        assert_eq!(refusal(&TEST, Some("BeebeebTest"), root()), None);
    }

    /// The incident: the test build handed the real app's folder. And the reverse, and a stranger's folder.
    #[test]
    fn a_folder_registered_to_another_provider_is_refused() {
        let words = refusal(&TEST, Some("Beebeeb"), root()).expect("the test build in the real app's folder");
        assert!(
            words.contains("\u{201c}Beebeeb\u{201d}") && words.contains("of Beebeeb."),
            "{words}"
        );
        assert!(words.contains("nothing was changed"), "{words}");
        assert!(
            !words.contains("Users") && !words.contains("home"),
            "the folder's name, never its path: {words}"
        );
        assert!(
            refusal(&REAL, Some("BeebeebTest"), root()).is_some(),
            "the real app in the test build's folder"
        );
        for identity in [&REAL, &TEST] {
            assert!(refusal(identity, Some("OneDrive"), root()).is_some());
            assert!(refusal(identity, Some("BeebeebQA"), root()).is_some());
            let unnamed = refusal(identity, Some(""), root()).expect("a registration without a name is not ours");
            assert!(unnamed.contains("of another app."), "{unnamed}");
        }
        // A prefix is not the same provider.
        assert!(refusal(&REAL, Some("BeebeebTest"), root()).is_some());
        assert!(refusal(&TEST, Some("Beebeeb"), root()).is_some());
    }

    /// The three callers ask before they act: the handle before it makes an engine, the runner before it connects
    /// the root, the Windows registration before it registers.
    #[test]
    fn every_way_into_a_sync_folder_asks_first() {
        let read = |file: &str| {
            let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src").join(file);
            std::fs::read_to_string(path).unwrap().replace("\r\n", "\n")
        };
        for (file, from, ask, act) in [
            (
                "app.rs",
                "pub fn start_sync(",
                "crate::sync_root_owner::check(self.identity, &sync_root)?;",
                "EngineRunner::spawn(",
            ),
            (
                "runner.rs",
                "async fn run(",
                "if let Err(msg) = crate::sync_root_owner::check(identity, &sync_root) {",
                "crate::windows_cf::connect_root(identity, &sync_root);",
            ),
            (
                "windows_cf/mod.rs",
                "pub fn register_sync_root(",
                "crate::sync_root_owner::check(identity, sync_root_path).map_err(|refusal| anyhow::anyhow!(refusal))?;",
                "register_sync_root_with(sync_root_path, &app_provider(identity), HYDRATION_POLICY)",
            ),
        ] {
            let source = read(file);
            let body = &source[source.find(from).unwrap_or_else(|| panic!("{file}: {from}"))..];
            let asked = body.find(ask).unwrap_or_else(|| panic!("{file} does not ask"));
            let acted = body.find(act).unwrap_or_else(|| panic!("{file}: {act}"));
            assert!(asked < acted, "{file} acts before it asks");
        }
        // The runner's refusal ends the run.
        let runner = read("runner.rs");
        let asked = &runner[runner
            .find("crate::sync_root_owner::check(identity, &sync_root)")
            .unwrap()..];
        assert!(asked[..asked.find("\n    }\n").unwrap()].contains("return;"));
    }

    /// Off Windows no folder has a registration, so nothing is refused here (the pure rule above is the test).
    #[cfg(not(target_os = "windows"))]
    #[test]
    fn without_cloud_files_there_is_no_registration_to_read() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(check(&TEST, dir.path()), Ok(()));
        assert_eq!(check(&REAL, dir.path()), Ok(()));
    }
}
