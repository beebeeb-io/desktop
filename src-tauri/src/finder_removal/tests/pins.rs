//! The source pins: every domain removal in this repo keeps unsynced files, and the pins
//! read a Windows checkout (CRLF text, `\` paths) the same as any other.
use super::*;

    // ── the source pin (spec §6) ─────────────────────────────────────────────

    #[test]
    fn test_1882_scanner_flags_objc_removals_without_the_mode() {
        let src = "\
// [NSFileProviderManager removeDomain:old completionHandler:nil]; a comment, not a call
/* [NSFileProviderManager removeDomain:x completionHandler:nil]; */
[NSFileProviderManager removeDomain:domain completionHandler:^(NSError *error) {}];
[NSFileProviderManager removeDomain:domain
                               mode:NSFileProviderDomainRemovalModePreserveDirtyUserData
                  completionHandler:^(NSURL *url, NSError *error) {}];
[NSFileProviderManager removeDomain:domain mode:NSFileProviderDomainRemovalModeRemoveAll completionHandler:nil];
[NSFileProviderManager removeAllDomainsWithCompletionHandler:^(NSError *error) {}];
";
        assert_eq!(
            removal_sites("Bridge.m", src),
            vec![
                RemovalSite {
                    line: 3,
                    preserving: false
                },
                RemovalSite {
                    line: 4,
                    preserving: true
                },
                RemovalSite {
                    line: 7,
                    preserving: false
                },
                RemovalSite {
                    line: 8,
                    preserving: false
                },
            ]
        );
    }

    #[test]
    fn test_1882_scanner_flags_swift_removals_without_the_mode() {
        let src = "\
// NSFileProviderManager.remove(domain) in a comment is not a call
NSFileProviderManager.remove(domain) { error in }
NSFileProviderManager.remove(domain, mode: .preserveDirtyUserData) { url, error in }
try await NSFileProviderManager.remove(
    domain,
    mode: .preserveDirtyUserData
)
NSFileProviderManager.remove(domain, mode: .removeAll) { url, error in }
NSFileProviderManager.removeAllDomains { error in }
";
        assert_eq!(
            removal_sites("Tool.swift", src),
            vec![
                RemovalSite {
                    line: 2,
                    preserving: false
                },
                RemovalSite {
                    line: 3,
                    preserving: true
                },
                RemovalSite {
                    line: 4,
                    preserving: true
                },
                RemovalSite {
                    line: 8,
                    preserving: false
                },
                RemovalSite {
                    line: 9,
                    preserving: false
                },
            ]
        );
    }

    /// One scanned source as the pins see it, from the path and text the OS gave: the path with
    /// `/` separators and the text with `\n` line ends, so a Windows checkout reads like the rest.
    fn scanned_source(rel: &str, raw: &str) -> (String, String) {
        (slash(rel), lf(raw))
    }

    /// Every Objective-C and Swift source in this repo. `target`, `node_modules`, `.git`, `dist`,
    /// `graphify-out` and `docs` are skipped (build output, dependencies, and prose that quotes
    /// the old calls on purpose).
    fn repo_native_sources() -> Vec<(String, String)> {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
        let mut found = Vec::new();
        let mut stack = vec![root.clone()];
        while let Some(dir) = stack.pop() {
            let entries = std::fs::read_dir(&dir).unwrap_or_else(|e| panic!("read {}: {e}", dir.display()));
            for entry in entries {
                let path = entry.expect("dir entry").path();
                let name = path
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or_default()
                    .to_string();
                if path.is_dir() {
                    if !matches!(
                        name.as_str(),
                        "target" | "node_modules" | ".git" | "dist" | "graphify-out" | "docs"
                    ) {
                        stack.push(path);
                    }
                } else if matches!(path.extension().and_then(|e| e.to_str()), Some("m" | "mm" | "swift")) {
                    let rel = path.strip_prefix(&root).unwrap_or(&path).display().to_string();
                    let source = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {rel}: {e}"));
                    found.push(scanned_source(&rel, &source));
                }
            }
        }
        found.sort();
        found
    }

    #[test]
    fn test_1882_every_domain_removal_in_the_repo_keeps_unsynced_files() {
        every_domain_removal_keeps_unsynced_files(&repo_native_sources());
    }

    fn every_domain_removal_keeps_unsynced_files(sources: &[(String, String)]) {
        let mut sites = Vec::new();
        for (file, source) in sources {
            for site in removal_sites(file, source) {
                sites.push((file.clone(), site));
            }
        }
        // The scan must prove it looked: the bridge's one removal and the helper's.
        let in_file = |suffix: &str| sites.iter().filter(|(file, _)| file.ends_with(suffix)).count();
        assert_eq!(in_file("src-tauri/macos/FileProviderBridge.m"), 1, "sites: {sites:?}");
        assert_eq!(
            in_file("BeebeebFileProviderTools/DomainControlTool.swift"),
            1,
            "sites: {sites:?}"
        );
        assert!(
            sites.len() >= 2,
            "a scan that found fewer than 2 removals scanned the wrong tree: {sites:?}"
        );

        // The bridge's one removal serves both exported entry points (sign-out / Repair /
        // rollback, and the sweep): one definition plus two calls, comments excluded.
        let (_, bridge) = sources
            .iter()
            .find(|(file, _)| file.ends_with("src-tauri/macos/FileProviderBridge.m"))
            .expect("the bridge was scanned");
        let bridge_code = without_comments(bridge);
        assert_eq!(bridge_code.matches("BeebeebRemoveDomainKeepingUnsynced(").count(), 3);
        for entry in ["int beebeeb_fp_remove(", "int beebeeb_fp_remove_domain_by_id("] {
            let start = bridge_code.find(entry).unwrap_or_else(|| panic!("{entry} exists"));
            let body = &bridge_code[start..start + bridge_code[start..].find("\n}\n").expect("function ends")];
            assert!(
                body.contains("BeebeebRemoveDomainKeepingUnsynced("),
                "{entry} must remove through the keeping helper"
            );
        }

        let offenders: Vec<String> = sites
            .iter()
            .filter(|(_, site)| !site.preserving)
            .map(|(file, site)| format!("{file}:{}", site.line))
            .collect();
        assert!(
            offenders.is_empty(),
            "every File Provider domain removal must pass the preserve-dirty-user-data mode \
             (spec 2026-10-09 §3); without it macOS deletes files that never reached the server: {offenders:?}"
        );
    }

    // ── r5: the pins read a Windows checkout (CRLF text, `\` paths) like any other ──

    #[test]
    fn test_1882_r5_the_reader_gives_a_windows_source_the_same_path_and_text() {
        let bridge = include_str!("../../../macos/FileProviderBridge.m");
        let windows = crlf(bridge);
        assert!(windows.contains("\r\n"), "the copy really has Windows line ends");
        assert_eq!(
            scanned_source("src-tauri\\macos\\FileProviderBridge.m", &windows),
            ("src-tauri/macos/FileProviderBridge.m".to_string(), lf(bridge)),
            "the `\\` path and the CRLF text read as the `/` path and the LF text"
        );
        assert_eq!(
            scanned_source("src-tauri/macos/FileProviderBridge.m", bridge),
            ("src-tauri/macos/FileProviderBridge.m".to_string(), lf(bridge)),
            "and a Unix source is unchanged"
        );
    }

    #[test]
    fn test_1882_r5_the_settle_pin_passes_on_a_windows_checkout() {
        every_removal_settles_a_missing_folder(&crlf(include_str!("../../macos_file_provider.rs")));
    }

    #[test]
    fn test_1882_r5_the_bridge_pins_pass_on_a_windows_checkout() {
        let bridge = crlf(include_str!("../../../macos/FileProviderBridge.m"));
        the_bridge_checks_the_folder_before_the_error(&bridge);
        the_folder_states_match_the_bridge(&bridge);
    }

    #[test]
    fn test_1882_r5_the_repo_scan_passes_on_a_windows_checkout() {
        let windows: Vec<(String, String)> = repo_native_sources()
            .iter()
            .map(|(file, source)| scanned_source(&file.replace('/', "\\"), &crlf(source)))
            .collect();
        assert_eq!(windows, repo_native_sources(), "the reader undoes the Windows form");
        every_domain_removal_keeps_unsynced_files(&windows);
    }

    /// The pins stay meaningful in the Windows form: a real violation still fails them there, and
    /// fails them for the reason it should (not because the Windows text defeated an anchor).
    #[test]
    fn test_1882_r5_the_pins_still_fail_a_real_violation_in_the_windows_form() {
        fn panic_message(check: impl FnOnce()) -> Option<String> {
            let payload = std::panic::catch_unwind(std::panic::AssertUnwindSafe(check)).err()?;
            Some(
                payload
                    .downcast_ref::<String>()
                    .cloned()
                    .or_else(|| payload.downcast_ref::<&str>().map(|text| (*text).to_string()))
                    .unwrap_or_default(),
            )
        }

        // The bridge no longer checks the folder.
        let bridge = crlf(include_str!("../../../macos/FileProviderBridge.m")).replace(
            "BeebeebKeptFolderState(found_location)",
            "BeebeebNotTheCheck(found_location)",
        );
        let message = panic_message(|| the_bridge_checks_the_folder_before_the_error(&bridge));
        assert_eq!(message.as_deref(), Some("the folder is checked"));

        // A decode that no longer settles a missing folder first.
        let provider = crlf(include_str!("../../macos_file_provider.rs")).replace(
            "crate::finder_removal::settle_kept_state(",
            "crate::finder_removal::skip_the_settle(",
        );
        let message = panic_message(|| every_removal_settles_a_missing_folder(&provider));
        assert!(
            message
                .as_deref()
                .is_some_and(|m| m.starts_with("it settles the state first")),
            "{message:?}"
        );

        // The helper stops looking again at a missing folder.
        let helper = crlf(include_str!("../../../../BeebeebFileProviderTools/DomainControlTool.swift"))
            .replace("Thread.sleep(forTimeInterval: keptMissingRecheckInterval)", "");
        let message = panic_message(|| the_helper_prints_preserved_for_any_existing_folder(&helper));
        assert!(
            message
                .as_deref()
                .is_some_and(|m| m.starts_with("the helper looks again at a missing folder")),
            "{message:?}"
        );

        // A removal without the mode, in a CRLF Swift source with a `\` path, is an offender.
        let mut sources = repo_native_sources();
        sources.push(scanned_source(
            "BeebeebFileProviderTools\\Other.swift",
            "NSFileProviderManager.remove(domain) { error in }\r\n",
        ));
        let message = panic_message(|| every_domain_removal_keeps_unsynced_files(&sources));
        assert!(
            message
                .as_deref()
                .is_some_and(|m| m.contains("BeebeebFileProviderTools/Other.swift:1")),
            "{message:?}"
        );
    }
