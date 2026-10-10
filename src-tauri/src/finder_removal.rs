//! Task 1882 (P0): removing Beebeeb's Finder location keeps the files that never reached the
//! server. Spec: `docs/specs/2026-10-09-macos-removal-keeps-unsynced-files.md`.
//!
//! Every `NSFileProviderManager` domain removal passes
//! `NSFileProviderDomainRemovalModePreserveDirtyUserData` (macOS 12+; Beebeeb requires 14). The
//! system then reports the folder where it kept the files that had not synced, or nothing.
//!
//! Cross-platform on purpose: decoding the bridge's reply, the sentence the person reads, and the
//! source pin over the Objective-C and Swift sources are pure, so they run on every OS. Only the
//! FFI calls live in the macOS-only `macos_file_provider` module.

/// The title of the app's alert (spec §5).
pub const PRESERVED_FILES_TITLE: &str = "Files kept on this Mac";

/// The one constant sentence (spec §5). The folder's path follows it on its own line. Mirrored in
/// `src/macSettingsModel.ts`; `tests/finderPreservedFiles.test.ts` pins the two equal.
pub const PRESERVED_FILES_SENTENCE: &str =
    "Files that hadn’t reached your vault yet were kept on this Mac, in this folder:";

/// What one domain removal left behind.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DomainRemoval {
    /// The folder where macOS kept files that had not synced, exactly as the system reported it.
    /// `None` = the system kept nothing.
    pub preserved_location: Option<String>,
}

impl DomainRemoval {
    /// The kept folder to show the person, if any. Logs THAT files were kept, never where: the
    /// path is shown only in the app's UI (spec §5).
    pub fn kept_location(self, context: &'static str) -> Option<String> {
        if self.preserved_location.is_some() {
            tracing::info!(
                context,
                preserved = true,
                "Finder location removed; macOS kept the files that had not reached the server"
            );
        }
        self.preserved_location
    }
}

/// Decodes `beebeeb_fp_remove` / `beebeeb_fp_remove_domain_by_id`: `1` = removed and the system
/// kept files (`location` set), `0` = removed and nothing kept, `-1` = error (`error` set).
// Called from the macOS-only bridge and sign-out paths; tested on every OS.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub fn removal_from_bridge(
    code: i32,
    location: Option<String>,
    error: Option<String>,
) -> Result<DomainRemoval, String> {
    match code {
        0 => Ok(DomainRemoval {
            preserved_location: None,
        }),
        1 => match location {
            Some(path) if !path.trim().is_empty() => Ok(DomainRemoval {
                preserved_location: Some(path),
            }),
            _ => Err("macOS kept files that had not synced but did not report the folder".to_string()),
        },
        code if code < 0 => Err(error
            .filter(|message| !message.trim().is_empty())
            .unwrap_or_else(|| "File Provider domain removal failed".to_string())),
        other => Err(format!("File Provider domain removal returned unexpected code {other}")),
    }
}

/// The alert's body: the sentence, a blank line, then the folder (spec §5).
pub fn preserved_files_message(location: &str) -> String {
    format!("{PRESERVED_FILES_SENTENCE}\n\n{location}")
}

/// Sign-out's removal (spec §3): a failure is logged and never stops the sign-out, as before; a
/// kept folder is returned for the alert.
// Called from the macOS-only bridge and sign-out paths; tested on every OS.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub fn sign_out_kept_location(removal: Result<DomainRemoval, String>) -> Option<String> {
    match removal {
        Ok(removal) => removal.kept_location("sign-out"),
        Err(error) => {
            tracing::warn!(error = %error, "Finder File Provider domain removal on logout failed (best-effort)");
            None
        }
    }
}

/// Repair's removal (spec §3): `(removed, kept folder)`. A failure becomes one warning, as before.
pub fn repair_removal(removal: Result<DomainRemoval, String>, warnings: &mut Vec<String>) -> (bool, Option<String>) {
    match removal {
        Ok(removal) => (true, removal.kept_location("repair")),
        Err(error) => {
            warnings.push(format!("Could not remove Finder File Provider domain: {error}"));
            (false, None)
        }
    }
}

/// One `NSFileProviderManager` domain removal found in a source file.
#[cfg(test)]
#[derive(Debug, Clone, PartialEq, Eq)]
struct RemovalSite {
    line: usize,
    preserving: bool,
}

/// The source pin's scanner (spec §6). Finds every domain removal in one Objective-C or Swift
/// source and says whether it passes the preserving mode. Comments are skipped. A remove-all call
/// (`removeAllDomains…`) is always a non-preserving site.
#[cfg(test)]
fn removal_sites(file_name: &str, source: &str) -> Vec<RemovalSite> {
    let _ = file_name; // every pattern is checked in every file, whatever its language
    let code = without_comments(source);
    let line_of = |index: usize| code[..index].matches('\n').count() + 1;
    let squash = |text: &str| text.chars().filter(|c| !c.is_whitespace()).collect::<String>();
    let mut sites: Vec<(usize, bool)> = Vec::new();

    // Objective-C: `removeDomain:` up to its `completionHandler:` must carry the mode.
    for (index, _) in code.match_indices("removeDomain:") {
        let rest = &code[index..];
        let send = rest.find("completionHandler:").map_or(rest, |end| &rest[..end]);
        sites.push((
            index,
            squash(send).contains("mode:NSFileProviderDomainRemovalModePreserveDirtyUserData"),
        ));
    }
    // Swift: the argument list of `NSFileProviderManager.remove(` must carry the mode.
    for (index, matched) in code.match_indices("NSFileProviderManager.remove(") {
        let open = index + matched.len();
        let mut depth = 1usize;
        let mut close = code.len();
        for (offset, ch) in code[open..].char_indices() {
            match ch {
                '(' => depth += 1,
                ')' => {
                    depth -= 1;
                    if depth == 0 {
                        close = open + offset;
                        break;
                    }
                }
                _ => {}
            }
        }
        let args = squash(&code[open..close]);
        sites.push((
            index,
            args.contains("mode:.preserveDirtyUserData")
                || args.contains("mode:NSFileProviderManager.DomainRemovalMode.preserveDirtyUserData"),
        ));
    }
    // Remove-all, in either language, never keeps anything.
    for (index, _) in code.match_indices("removeAllDomains") {
        sites.push((index, false));
    }

    sites.sort();
    sites
        .into_iter()
        .map(|(index, preserving)| RemovalSite {
            line: line_of(index),
            preserving,
        })
        .collect()
}

/// `source` with `//` and `/* */` comments blanked to spaces (newlines kept, so line numbers hold).
/// String literals are skipped as written, so a `//` inside one is not a comment.
#[cfg(test)]
fn without_comments(source: &str) -> String {
    let mut out = String::with_capacity(source.len());
    let mut chars = source.chars().peekable();
    let blank = |ch: char| if ch == '\n' { '\n' } else { ' ' };
    while let Some(ch) = chars.next() {
        match ch {
            '"' => {
                out.push(ch);
                while let Some(inner) = chars.next() {
                    out.push(inner);
                    match inner {
                        '\\' => {
                            if let Some(escaped) = chars.next() {
                                out.push(escaped);
                            }
                        }
                        '"' | '\n' => break,
                        _ => {}
                    }
                }
            }
            '/' if chars.peek() == Some(&'/') => {
                out.push(' ');
                for inner in chars.by_ref() {
                    out.push(blank(inner));
                    if inner == '\n' {
                        break;
                    }
                }
            }
            '/' if chars.peek() == Some(&'*') => {
                out.push(' ');
                out.push(blank(chars.next().unwrap_or(' ')));
                let mut previous = ' ';
                for inner in chars.by_ref() {
                    out.push(blank(inner));
                    if previous == '*' && inner == '/' {
                        break;
                    }
                    previous = inner;
                }
            }
            _ => out.push(ch),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── the bridge's reply ───────────────────────────────────────────────────

    #[test]
    fn test_1882_bridge_reply_with_a_kept_folder_reports_that_exact_folder() {
        let folder = "/Users/someone/Library/CloudStorage/Beebeeb (kept) /Notes ";
        assert_eq!(
            removal_from_bridge(1, Some(folder.to_string()), None),
            Ok(DomainRemoval {
                preserved_location: Some(folder.to_string())
            }),
            "the folder is reported byte for byte, trailing space included"
        );
    }

    #[test]
    fn test_1882_bridge_reply_with_nothing_kept_reports_no_folder() {
        assert_eq!(
            removal_from_bridge(0, None, None),
            Ok(DomainRemoval {
                preserved_location: None
            })
        );
        // A stale buffer never turns "nothing kept" into a folder.
        assert_eq!(
            removal_from_bridge(0, Some("/leftover".to_string()), None),
            Ok(DomainRemoval {
                preserved_location: None
            })
        );
    }

    #[test]
    fn test_1882_bridge_errors_never_invent_a_folder() {
        assert_eq!(
            removal_from_bridge(
                -1,
                Some("/leftover".to_string()),
                Some("no provider (NSFileProviderErrorDomain -2001)".to_string())
            ),
            Err("no provider (NSFileProviderErrorDomain -2001)".to_string())
        );
        assert!(removal_from_bridge(-1, None, None).is_err());
        assert!(
            removal_from_bridge(1, None, None).is_err(),
            "kept, but no folder: an error, not a silent 'nothing kept'"
        );
        assert!(removal_from_bridge(1, Some("   ".to_string()), None).is_err());
        assert!(
            removal_from_bridge(7, None, None).is_err(),
            "an unknown reply is an error"
        );
    }

    // ── the sentence and the alert ───────────────────────────────────────────

    #[test]
    fn test_1882_alert_text_is_the_sentence_a_blank_line_then_the_folder() {
        let folder = "/Users/someone/Library/CloudStorage/Kept";
        assert_eq!(
            preserved_files_message(folder),
            format!("{PRESERVED_FILES_SENTENCE}\n\n{folder}")
        );
        assert_eq!(PRESERVED_FILES_TITLE, "Files kept on this Mac");
        assert_eq!(
            PRESERVED_FILES_SENTENCE,
            "Files that hadn’t reached your vault yet were kept on this Mac, in this folder:"
        );
    }

    #[test]
    fn test_1882_the_sentence_names_no_provider_and_holds_no_path() {
        for text in [PRESERVED_FILES_SENTENCE, PRESERVED_FILES_TITLE] {
            let lower = text.to_lowercase();
            for word in [
                "apple",
                "icloud",
                "hetzner",
                "file provider",
                "fileprovider",
                "cloudstorage",
                "/",
            ] {
                assert!(!lower.contains(word), "{text:?} must not contain {word:?}");
            }
        }
    }

    // ── where the kept folder goes ───────────────────────────────────────────

    #[test]
    fn test_1882_kept_location_returns_the_folder_and_nothing_when_nothing_was_kept() {
        let folder = "/Users/someone/Kept".to_string();
        assert_eq!(
            DomainRemoval {
                preserved_location: Some(folder.clone())
            }
            .kept_location("sign-out"),
            Some(folder)
        );
        assert_eq!(
            DomainRemoval {
                preserved_location: None
            }
            .kept_location("sign-out"),
            None
        );
    }

    #[test]
    fn test_1882_sign_out_carries_a_kept_folder_and_survives_a_failed_removal() {
        let folder = "/Users/someone/Kept".to_string();
        assert_eq!(
            sign_out_kept_location(Ok(DomainRemoval {
                preserved_location: Some(folder.clone())
            })),
            Some(folder)
        );
        assert_eq!(sign_out_kept_location(Ok(DomainRemoval::default())), None);
        assert_eq!(sign_out_kept_location(Err("not registered".to_string())), None);
    }

    #[test]
    fn test_1882_repair_carries_a_kept_folder_and_turns_a_failure_into_one_warning() {
        let folder = "/Users/someone/Kept".to_string();

        let mut warnings = Vec::new();
        assert_eq!(
            repair_removal(
                Ok(DomainRemoval {
                    preserved_location: Some(folder.clone())
                }),
                &mut warnings
            ),
            (true, Some(folder))
        );
        assert!(warnings.is_empty());

        let mut warnings = Vec::new();
        assert_eq!(
            repair_removal(Ok(DomainRemoval::default()), &mut warnings),
            (true, None)
        );
        assert!(warnings.is_empty());

        let mut warnings = Vec::new();
        assert_eq!(repair_removal(Err("busy".to_string()), &mut warnings), (false, None));
        assert_eq!(
            warnings,
            vec!["Could not remove Finder File Provider domain: busy".to_string()]
        );
    }

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
                    found.push((rel, source));
                }
            }
        }
        found.sort();
        found
    }

    #[test]
    fn test_1882_every_domain_removal_in_the_repo_keeps_unsynced_files() {
        let sources = repo_native_sources();
        let mut sites = Vec::new();
        for (file, source) in &sources {
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
}
