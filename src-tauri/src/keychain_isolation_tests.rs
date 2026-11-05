    use super::finder_setup_command_tests::without_items;
    use super::keychain::{self, AuthSecretStore, SessionToken};

    fn type_of<T>(_: &T) -> &'static str {
        std::any::type_name::<T>()
    }

    #[test]
    fn no_lib_test_can_resolve_the_real_macos_keychain_store() {
        let segmented = keychain::platform_keychain_store_for("keychain-isolation-guard");
        let legacy = keychain::legacy_platform_keychain_store();
        let names = [
            ("platform_keychain_store_for(id)", type_of(&segmented)),
            ("legacy_platform_keychain_store()", type_of(&legacy)),
            (
                "keychain::PlatformKeychainStore",
                std::any::type_name::<keychain::PlatformKeychainStore>(),
            ),
        ];
        for (constructor, name) in names {
            println!("keychain store type a lib test gets from {constructor}: {name}");
            // Asserted BEFORE anything is saved, so a red guard has not touched the real Keychain.
            assert!(
                name.ends_with("::TestKeychainStore"),
                "{constructor} resolves {name}, not the in-memory test store"
            );
            assert!(
                !name.contains("MacOsKeychainStore"),
                "{constructor} resolves the real Keychain store: {name}"
            );
        }
        // It behaves like a store: a save is readable, the legacy layout is a separate namespace, and a
        // delete clears it. All of it in memory.
        let token = SessionToken::new("tok-isolation-guard".to_string()).unwrap();
        segmented.save_session_token(&token).unwrap();
        assert_eq!(
            segmented
                .load_session_token()
                .unwrap()
                .map(|t| t.expose_for_request().to_string()),
            Some("tok-isolation-guard".into())
        );
        assert!(
            legacy.load_session_token().unwrap().is_none(),
            "the legacy flat layout is a separate namespace"
        );
        segmented.delete_session_token().unwrap();
        assert!(segmented.load_session_token().unwrap().is_none());
        assert!(keychain::test_store_touched(Some("keychain-isolation-guard")));
    }

    /// The real store is only reachable through `PlatformKeychainStore`: no file but `keychain.rs`
    /// names it or its FFI module, and every vault in the app's production code is built from one of the
    /// two constructors (so a sign-out or lock test, which can only run production code, gets the
    /// in-memory store).
    #[test]
    fn the_real_store_is_named_nowhere_but_keychain_rs_and_every_vault_uses_the_two_constructors() {
        fn rust_files(dir: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
            for entry in std::fs::read_dir(dir).expect("read the source dir").flatten() {
                let path = entry.path();
                if path.is_dir() {
                    rust_files(&path, out);
                } else if path.extension().is_some_and(|ext| ext == "rs") {
                    out.push(path);
                }
            }
        }
        let src = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let mut files = Vec::new();
        rust_files(&src, &mut files);
        let mut vaults = 0;
        let test_modules = super::finder_setup_command_tests::test_module_files(&src);
        for file in files {
            let name = file.strip_prefix(&src).unwrap().to_string_lossy().replace('\\', "/");
            if test_modules.contains(&name) {
                continue;
            }
            let text = std::fs::read_to_string(&file).unwrap().replace("\r\n", "\n");
            let production = without_items(&text, &["#[cfg(test)]", "#[cfg(all(test"]);
            let code: String = production
                .lines()
                .filter(|line| !line.trim_start().starts_with("//"))
                .collect::<Vec<_>>()
                .join("\n");
            if name != "keychain.rs" {
                for real in ["MacOsKeychainStore", "macos_keychain"] {
                    assert!(!code.contains(real), "{name} names the real Keychain store ({real})");
                }
            }
            if name == "lib.rs" {
                for (at, _) in code.match_indices("AuthVault::new(") {
                    let argument = &code[at + "AuthVault::new(".len()..];
                    assert!(
                        argument.starts_with("platform_keychain_store_for(")
                            || argument.starts_with("keychain::legacy_platform_keychain_store()"),
                        "a vault built from something else: {}",
                        &code[at..(at + 80).min(code.len())]
                    );
                    vaults += 1;
                }
            }
        }
        assert!(vaults >= 8, "the census saw the app's vaults: {vaults}");
    }
