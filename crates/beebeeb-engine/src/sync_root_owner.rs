//! Whose sync folder is this? An engine never starts in a folder that Windows has registered to another sync
//! provider: not the test build in the real app's folder (the incident of 2026-11-11: two engines on one root),
//! not the real app in the test build's, not either in a third provider's.
//!
//! [`refusal`] is the whole decision, as a pure function. [`check`] reads the folder's registration (Windows
//! Cloud Files; no other OS has one) and asks it. Callers: the engine handle before it starts a sync, the runner
//! before it connects the root, `windows_cf::register_sync_root` before it registers anything and
//! `windows_cf::unregister_sync_root` before it removes a registration.
//!
//! It fails closed: a registration that could not be read is not "nobody's". Only a folder that does not exist,
//! or one Windows answers with one of its two "this is not a sync root" codes, is free ([`registration_of`]).

use std::path::Path;

use crate::identity::EngineIdentity;

/// What the OS says about a folder's sync-root registration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Registration {
    /// Not a sync root of any provider.
    Nobody,
    /// Registered to the provider of this name.
    Provider(String),
    /// The OS could not say. Never read as "nobody".
    Unreadable,
}

/// `HRESULT_FROM_WIN32(ERROR_NOT_A_CLOUD_FILE)`: the folder is not a cloud file at all.
pub const NOT_A_CLOUD_FILE: i32 = 0x8007_0178_u32 as i32;
/// `HRESULT_FROM_WIN32(ERROR_CLOUD_FILE_NOT_UNDER_SYNC_ROOT)`: the folder is under no sync root.
pub const NOT_UNDER_SYNC_ROOT: i32 = 0x8007_0186_u32 as i32;

/// What one read of the registration means: the provider's name, or the failure's HRESULT. Only the two "this is
/// not a sync root" codes mean nobody registered the folder; every other failure is [`Registration::Unreadable`].
pub fn registration_of(read: Result<String, i32>) -> Registration {
    match read {
        Ok(name) => Registration::Provider(name),
        Err(NOT_A_CLOUD_FILE | NOT_UNDER_SYNC_ROOT) => Registration::Nobody,
        Err(_) => Registration::Unreadable,
    }
}

/// Why `identity` must not use `root`, given what the OS says about the folder's registration, or `None` when it
/// may. A folder is this app's to use when nobody registered it or this app's own provider did; Windows compares
/// provider names without case, so this does too. The sentence names the folder, never its path.
pub fn refusal(identity: &EngineIdentity, registered: &Registration, root: &Path) -> Option<String> {
    let folder = root
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    match registered {
        Registration::Nobody => None,
        Registration::Provider(provider) if provider.eq_ignore_ascii_case(identity.cloud_files_provider_name()) => None,
        Registration::Provider(provider) => {
            let owner = if provider.is_empty() { "another app" } else { provider };
            Some(format!(
                "The folder \u{201c}{folder}\u{201d} is already the sync folder of {owner}. Nothing was changed. Choose another folder."
            ))
        }
        Registration::Unreadable => Some(format!(
            "Windows could not say whose sync folder \u{201c}{folder}\u{201d} is. Nothing was changed. Try again, or choose another folder."
        )),
    }
}

/// What the OS says about `root` now. A folder that does not exist yet is nobody's (the first start makes it);
/// a folder whose existence cannot be read is unreadable.
fn registered(root: &Path) -> Registration {
    match std::fs::symlink_metadata(root) {
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Registration::Nobody,
        Err(_) => return Registration::Unreadable,
    }
    #[cfg(target_os = "windows")]
    {
        registration_of(crate::windows_cf::registered_provider_name(root))
    }
    #[cfg(not(target_os = "windows"))]
    {
        Registration::Nobody
    }
}

/// [`refusal`] for `root` as the OS has it now.
pub fn check(identity: &EngineIdentity, root: &Path) -> Result<(), String> {
    refusal(identity, &registered(root), root).map_or(Ok(()), Err)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::identity::{REAL, TEST};

    fn named(provider: &str) -> Registration {
        Registration::Provider(provider.to_string())
    }

    fn root() -> &'static Path {
        Path::new(if cfg!(windows) {
            r"C:\home\a\Beebeeb"
        } else {
            "/data/a/Beebeeb"
        })
    }

    #[test]
    fn a_folder_nobody_registered_or_this_app_registered_is_used() {
        for identity in [&REAL, &TEST] {
            assert_eq!(refusal(identity, &Registration::Nobody, root()), None);
        }
        assert_eq!(refusal(&REAL, &named("Beebeeb"), root()), None);
        assert_eq!(
            refusal(&REAL, &named("BEEBEEB"), root()),
            None,
            "Windows compares these names without case"
        );
        assert_eq!(refusal(&TEST, &named("BeebeebTest"), root()), None);
    }

    /// The incident: the test build handed the real app's folder. And the reverse, and a stranger's folder.
    #[test]
    fn a_folder_registered_to_another_provider_is_refused() {
        let words = refusal(&TEST, &named("Beebeeb"), root()).expect("the test build in the real app's folder");
        assert!(
            words.contains("\u{201c}Beebeeb\u{201d}") && words.contains("of Beebeeb."),
            "{words}"
        );
        assert!(words.contains("Nothing was changed"), "{words}");
        assert!(
            !words.contains("Users") && !words.contains("home"),
            "the folder's name, never its path: {words}"
        );
        assert!(
            refusal(&REAL, &named("BeebeebTest"), root()).is_some(),
            "the real app in the test build's folder"
        );
        for identity in [&REAL, &TEST] {
            assert!(refusal(identity, &named("OneDrive"), root()).is_some());
            assert!(refusal(identity, &named("BeebeebQA"), root()).is_some());
            let unnamed = refusal(identity, &named(""), root()).expect("a registration without a name is not ours");
            assert!(unnamed.contains("of another app."), "{unnamed}");
        }
        // A prefix is not the same provider.
        assert!(refusal(&REAL, &named("BeebeebTest"), root()).is_some());
        assert!(refusal(&TEST, &named("Beebeeb"), root()).is_some());
    }

    /// Fail closed (review of #309, P2-4): a registration that could not be read refuses, for both apps; only the
    /// two "not a sync root" codes, and a folder that does not exist, mean nobody.
    #[test]
    fn a_registration_that_cannot_be_read_is_not_nobodys() {
        assert_eq!(registration_of(Err(NOT_A_CLOUD_FILE)), Registration::Nobody);
        assert_eq!(registration_of(Err(NOT_UNDER_SYNC_ROOT)), Registration::Nobody);
        assert_eq!(
            (NOT_A_CLOUD_FILE as u32, NOT_UNDER_SYNC_ROOT as u32),
            (0x8007_0178, 0x8007_0186)
        );
        // Access denied, a sharing violation, a timeout, a path that is not there, "no error" as an error.
        for other in [
            0x8007_0005_u32,
            0x8007_0020,
            0x8007_05B4,
            0x8007_0002,
            0x8007_0003,
            0,
            1,
        ] {
            assert_eq!(
                registration_of(Err(other as i32)),
                Registration::Unreadable,
                "{other:#x}"
            );
        }
        assert_eq!(registration_of(Ok("Beebeeb".into())), named("Beebeeb"));
        for identity in [&REAL, &TEST] {
            let words = refusal(identity, &Registration::Unreadable, root()).expect("unreadable refuses");
            assert!(
                words.contains("could not say") && words.contains("Nothing was changed"),
                "{words}"
            );
            assert!(!words.contains("Users") && !words.contains("home"), "{words}");
        }
        let gone = tempfile::tempdir().unwrap().path().join("not-made-yet");
        assert_eq!(
            registered(&gone),
            Registration::Nobody,
            "the first start makes the folder"
        );
    }

    /// The four callers ask before they act: the handle before it makes an engine, the runner before it connects
    /// the root, the Windows registration before it registers, the Windows unregistration before it unregisters.
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
            (
                "windows_cf/registration.rs",
                "pub fn unregister_sync_root(",
                "crate::sync_root_owner::check(identity, root).map_err(|refusal| anyhow::anyhow!(refusal))?;",
                "unregister_any_sync_root(root)",
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
        // No registration or unregistration of a caller's own choosing leaves the crate (review of #309, P2-3):
        // the forms that take a provider, and the unregistration that asks nobody, are the crate's and its tests'.
        let cf = read("windows_cf/registration.rs") + &read("windows_cf/mod.rs");
        for private in [
            "fn register_sync_root_with(",
            "fn register_shell_sync_root_as(",
            "fn unregister_any_sync_root(",
        ] {
            assert!(
                cf.contains(&format!("pub(crate) {private}")),
                "{private} must be pub(crate)"
            );
            assert!(!cf.contains(&format!("pub {private}")), "{private} is public");
        }
        // By the function's NAME (a read like `registered_provider_name` changes nothing and is not one).
        fn changes_a_root(line: &str) -> bool {
            let name = line.strip_prefix("pub fn ").and_then(|rest| rest.split('(').next());
            name.is_some_and(|name| {
                (name.contains("register") && !name.starts_with("registered")) || name == "connect_root"
            })
        }
        let public: Vec<&str> = cf
            .lines()
            .filter(|line| changes_a_root(line))
            .map(|line| &line[..line.find('(').unwrap_or(line.len())])
            .collect();
        assert_eq!(
            public,
            [
                "pub fn unregister_sync_root",
                "pub fn register_sync_root",
                "pub fn register_shell_sync_root",
                "pub fn unregister_shell_sync_root",
                "pub fn register_thumbnail_provider_for_sync_root",
                "pub fn register_status_ui_for_sync_root",
                "pub fn connect_root",
            ],
            "a new public way to register, connect or unregister a sync root: it takes the identity and asks \
             sync_root_owner first, then it is listed here"
        );
        // Each of them takes the identity.
        for line in cf.lines().filter(|line| changes_a_root(line)) {
            let from = cf.find(line).unwrap();
            let signature = &cf[from..from + cf[from..].find(')').unwrap()];
            assert!(signature.contains("identity: &"), "{line} takes no identity");
        }
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
