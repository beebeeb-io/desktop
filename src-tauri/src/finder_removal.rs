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

/// What the bridge found at the folder macOS reported after a removal (round 2, spec §4 and §5).
/// The same numbers as `BeebeebKept*` in `src-tauri/macos/FileProviderBridge.m`; a test pins them.
///
/// No URL came back.
pub const KEPT_NONE_REPORTED: i32 = 0;
/// A URL came back, but nothing exists there (device K-F2: macOS reports a folder even when it
/// kept nothing).
pub const KEPT_MISSING: i32 = 1;
/// The folder exists and is empty.
pub const KEPT_EMPTY: i32 = 2;
/// The folder (or a single kept file) holds at least one entry.
pub const KEPT_HAS_ENTRIES: i32 = 3;
/// Something may be there, but the sandbox refused the look (spec §4's fallback).
pub const KEPT_UNCHECKED: i32 = 4;
/// A URL without a path: the domain is removed, and no folder can be named (review M3).
pub const KEPT_NO_PATH: i32 = 5;

/// Why a removal kept nothing. Logged (debug), never shown.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NothingKept {
    NotReported,
    Missing,
    Empty,
}

impl NothingKept {
    fn as_str(self) -> &'static str {
        match self {
            Self::NotReported => "none",
            Self::Missing => "missing",
            Self::Empty => "empty",
        }
    }
}

/// What one removal left on this Mac (spec §5, "When files count as kept").
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KeptFolder {
    /// Nothing to tell the person.
    Nothing(NothingKept),
    /// Files were kept in this folder, exactly as the system reported it. `contents_checked` is
    /// `false` when the folder exists but the app could not look inside (spec §4).
    Kept { path: String, contents_checked: bool },
    /// macOS reported kept files without a folder: removed; folder unknown (review M3).
    Unknown,
}

impl Default for KeptFolder {
    fn default() -> Self {
        Self::Nothing(NothingKept::NotReported)
    }
}

/// Decodes the bridge's folder state. An unknown state with a path never hides that path: it
/// is shown as kept, unchecked.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub fn kept_folder_from_bridge(state: i32, location: Option<String>) -> KeptFolder {
    let path = location.filter(|path| !path.trim().is_empty());
    match (state, path) {
        (KEPT_NONE_REPORTED, _) => KeptFolder::Nothing(NothingKept::NotReported),
        (KEPT_MISSING, _) => KeptFolder::Nothing(NothingKept::Missing),
        (KEPT_EMPTY, _) => KeptFolder::Nothing(NothingKept::Empty),
        (KEPT_HAS_ENTRIES, Some(path)) => KeptFolder::Kept {
            path,
            contents_checked: true,
        },
        (KEPT_NO_PATH, _) | (_, None) => KeptFolder::Unknown,
        // KEPT_UNCHECKED, and any state this build does not know: the folder is shown.
        (_, Some(path)) => KeptFolder::Kept {
            path,
            contents_checked: false,
        },
    }
}

/// What one domain removal left behind.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DomainRemoval {
    pub kept: KeptFolder,
}

impl DomainRemoval {
    /// The kept folder to show the person, if any. Logs THAT files were kept (or why nothing
    /// was), never where: the path is shown only in the app's UI (spec §5).
    pub fn kept_location(self, context: &'static str) -> Option<String> {
        match self.kept {
            KeptFolder::Kept {
                path,
                contents_checked,
            } => {
                tracing::info!(
                    context,
                    preserved = true,
                    contents_checked,
                    "Finder location removed; macOS kept the files that had not reached the server"
                );
                Some(path)
            }
            KeptFolder::Nothing(reason) => {
                tracing::debug!(context, reported = reason.as_str(), "Finder location removed; nothing kept");
                None
            }
            KeptFolder::Unknown => {
                tracing::warn!(
                    context,
                    "Finder location removed; macOS reported kept files without a folder"
                );
                None
            }
        }
    }
}

/// A removal that failed. Review M2 (round 2): macOS may report a kept folder together with an
/// error; the folder is kept with the error, so it still reaches the person.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemovalFailure {
    pub message: String,
    pub kept: KeptFolder,
}

impl RemovalFailure {
    /// Like [`DomainRemoval::kept_location`]: the folder to show, if any; logs never carry it.
    pub fn kept_location(self, context: &'static str) -> Option<String> {
        DomainRemoval { kept: self.kept }.kept_location(context)
    }
}

impl From<String> for RemovalFailure {
    fn from(message: String) -> Self {
        Self {
            message,
            kept: KeptFolder::default(),
        }
    }
}

impl std::fmt::Display for RemovalFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

/// Decodes `beebeeb_fp_remove` / `beebeeb_fp_remove_domain_by_id`: `0` = removed (`kept_state`
/// says what the reported folder holds), `-1` = error (`error` set; `kept_state` still says what
/// the reported folder holds, review M2).
// Called from the macOS-only bridge and sign-out paths; tested on every OS.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub fn removal_from_bridge(
    code: i32,
    kept_state: i32,
    location: Option<String>,
    error: Option<String>,
) -> Result<DomainRemoval, RemovalFailure> {
    match code {
        0 => Ok(DomainRemoval {
            kept: kept_folder_from_bridge(kept_state, location),
        }),
        code if code < 0 => Err(RemovalFailure {
            message: error
                .filter(|message| !message.trim().is_empty())
                .unwrap_or_else(|| "File Provider domain removal failed".to_string()),
            kept: kept_folder_from_bridge(kept_state, location),
        }),
        other => Err(RemovalFailure {
            message: format!("File Provider domain removal returned unexpected code {other}"),
            kept: kept_folder_from_bridge(kept_state, location),
        }),
    }
}

/// Add to Finder's own cleanup failed the install (review M1): the error as before, and the folder
/// the cleanup's removal kept, if any.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstallFailure {
    pub message: String,
    pub kept_folder: Option<String>,
}

impl From<String> for InstallFailure {
    fn from(message: String) -> Self {
        Self {
            message,
            kept_folder: None,
        }
    }
}

/// Review M1 (round 2): the cleanup inside Add to Finder removes the domain this attempt added
/// when it does not come up in time. The install still fails with the setup error (and the
/// cleanup's error, if that failed too, as before), and the folder that removal kept rides along.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub fn install_cleanup_failure(setup_error: String, cleanup: Result<DomainRemoval, RemovalFailure>) -> InstallFailure {
    match cleanup {
        Ok(removal) => InstallFailure {
            message: setup_error,
            kept_folder: removal.kept_location("install cleanup"),
        },
        Err(failure) => InstallFailure {
            message: format!("{setup_error}; cleanup failed: {}", failure.message),
            kept_folder: failure.kept_location("install cleanup"),
        },
    }
}

/// Saves `location` as the latest kept folder, for the Settings › Sync row (spec §5, lead ruling
/// I2). A newer folder replaces an older one. Returns whether the config changed (the caller
/// saves it).
pub fn record_kept_folder(cfg: &mut crate::config::DesktopConfig, location: &str) -> bool {
    if cfg.kept_unsynced_folder.as_deref() == Some(location) {
        return false;
    }
    cfg.kept_unsynced_folder = Some(location.to_string());
    true
}

/// Dismisses the row: clears the saved folder only if it is still the one the row showed, so a
/// folder kept in the meantime is never dismissed unseen. Returns whether the config changed.
pub fn dismiss_kept_folder(cfg: &mut crate::config::DesktopConfig, shown: &str) -> bool {
    if cfg.kept_unsynced_folder.as_deref() != Some(shown) {
        return false;
    }
    cfg.kept_unsynced_folder = None;
    true
}

/// The alert after a removal that kept files: `(title, body)`, or nothing at all when nothing was
/// kept (spec §5). The one decision `show_preserved_files_alert` makes, so it can be tested.
pub fn kept_folder_alert(preserved_location: Option<&str>) -> Option<(&'static str, String)> {
    preserved_location.map(|location| (PRESERVED_FILES_TITLE, preserved_files_message(location)))
}

/// The alert's body: the sentence, a blank line, then the folder (spec §5).
pub fn preserved_files_message(location: &str) -> String {
    format!("{PRESERVED_FILES_SENTENCE}\n\n{location}")
}

/// Sign-out's removal (spec §3): a failure is logged and never stops the sign-out, as before; a
/// kept folder is returned for the alert.
// Called from the macOS-only bridge and sign-out paths; tested on every OS.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub fn sign_out_kept_location(removal: Result<DomainRemoval, RemovalFailure>) -> Option<String> {
    match removal {
        Ok(removal) => removal.kept_location("sign-out"),
        Err(failure) => {
            tracing::warn!(error = %failure, "Finder File Provider domain removal on logout failed (best-effort)");
            // Review M2: a folder kept with the error still reaches the person.
            failure.kept_location("sign-out")
        }
    }
}

/// Repair's removal (spec §3): `(removed, kept folder)`. A failure becomes one warning, as before.
pub fn repair_removal(
    removal: Result<DomainRemoval, RemovalFailure>,
    warnings: &mut Vec<String>,
) -> (bool, Option<String>) {
    match removal {
        Ok(removal) => (true, removal.kept_location("repair")),
        Err(failure) => {
            warnings.push(format!("Could not remove Finder File Provider domain: {}", failure.message));
            // Review M2: a folder kept with the error still reaches the person.
            (false, failure.kept_location("repair"))
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

    fn kept(path: &str, contents_checked: bool) -> KeptFolder {
        KeptFolder::Kept {
            path: path.to_string(),
            contents_checked,
        }
    }

    fn removal(kept: KeptFolder) -> DomainRemoval {
        DomainRemoval { kept }
    }

    #[test]
    fn test_1882_bridge_reply_with_a_kept_folder_reports_that_exact_folder() {
        let folder = "/Users/someone/Library/CloudStorage/Beebeeb (kept) /Notes ";
        assert_eq!(
            removal_from_bridge(0, KEPT_HAS_ENTRIES, Some(folder.to_string()), None),
            Ok(removal(kept(folder, true))),
            "the folder is reported byte for byte, trailing space included"
        );
    }

    #[test]
    fn test_1882_bridge_reply_with_nothing_kept_reports_no_folder() {
        assert_eq!(
            removal_from_bridge(0, KEPT_NONE_REPORTED, None, None),
            Ok(removal(KeptFolder::Nothing(NothingKept::NotReported)))
        );
        // A stale buffer never turns "nothing kept" into a folder.
        assert_eq!(
            removal_from_bridge(0, KEPT_NONE_REPORTED, Some("/leftover".to_string()), None),
            Ok(removal(KeptFolder::Nothing(NothingKept::NotReported)))
        );
    }

    #[test]
    fn test_1882_bridge_errors_never_invent_a_folder() {
        assert_eq!(
            removal_from_bridge(
                -1,
                KEPT_NONE_REPORTED,
                Some("/leftover".to_string()),
                Some("no provider (NSFileProviderErrorDomain -2001)".to_string())
            ),
            Err(RemovalFailure::from("no provider (NSFileProviderErrorDomain -2001)".to_string()))
        );
        assert!(removal_from_bridge(-1, KEPT_NONE_REPORTED, None, None).is_err());
        assert!(
            removal_from_bridge(1, KEPT_HAS_ENTRIES, Some("/x".to_string()), None).is_err(),
            "round 1's reply 1 is gone: the bridge answers 0 or -1"
        );
        assert!(
            removal_from_bridge(7, KEPT_NONE_REPORTED, None, None).is_err(),
            "an unknown reply is an error"
        );
    }

    // ── round 2: is anything actually there? (device K-F2, review I3, M3) ───

    #[test]
    fn test_1882_r2_a_reported_folder_that_is_missing_or_empty_keeps_nothing() {
        let folder = "/Users/someone/Library/CloudStorage/Beebeeb-Drive (10-10-2026 10:52)";
        for (state, reason) in [
            (KEPT_NONE_REPORTED, NothingKept::NotReported),
            (KEPT_MISSING, NothingKept::Missing),
            (KEPT_EMPTY, NothingKept::Empty),
        ] {
            let decoded = kept_folder_from_bridge(state, Some(folder.to_string()));
            assert_eq!(decoded, KeptFolder::Nothing(reason), "state {state}");
            assert_eq!(removal(decoded).kept_location("sign-out"), None, "state {state}: no alert, no row");
        }
        let mut warnings = Vec::new();
        assert_eq!(
            repair_removal(Ok(removal(kept_folder_from_bridge(KEPT_MISSING, Some(folder.to_string())))), &mut warnings),
            (true, None)
        );
        assert!(warnings.is_empty());
    }

    #[test]
    fn test_1882_r2_a_folder_with_an_entry_or_that_cannot_be_checked_is_kept() {
        let folder = "/Users/someone/Library/CloudStorage/Beebeeb-Beebeeb (10-10-2026 10:50)";
        assert_eq!(kept_folder_from_bridge(KEPT_HAS_ENTRIES, Some(folder.to_string())), kept(folder, true));
        assert_eq!(
            kept_folder_from_bridge(KEPT_UNCHECKED, Some(folder.to_string())),
            kept(folder, false),
            "spec §4's fallback: an existing folder the sandbox would not list is shown"
        );
        for state in [KEPT_HAS_ENTRIES, KEPT_UNCHECKED] {
            assert_eq!(
                removal(kept_folder_from_bridge(state, Some(folder.to_string()))).kept_location("sign-out"),
                Some(folder.to_string())
            );
        }
    }

    #[test]
    fn test_1882_r2_a_url_without_a_path_is_removed_with_the_folder_unknown() {
        assert_eq!(kept_folder_from_bridge(KEPT_NO_PATH, None), KeptFolder::Unknown);
        for blank in [None, Some(String::new()), Some("   ".to_string())] {
            assert_eq!(kept_folder_from_bridge(KEPT_HAS_ENTRIES, blank.clone()), KeptFolder::Unknown);
            assert_eq!(kept_folder_from_bridge(KEPT_UNCHECKED, blank), KeptFolder::Unknown);
        }
        // Its own outcome: the domain IS removed, so Repair says so, with no warning and no folder.
        let mut warnings = Vec::new();
        assert_eq!(repair_removal(Ok(removal(KeptFolder::Unknown)), &mut warnings), (true, None));
        assert!(warnings.is_empty(), "removed; folder unknown is not a failed removal: {warnings:?}");
        assert_eq!(sign_out_kept_location(Ok(removal(KeptFolder::Unknown))), None);
    }

    #[test]
    fn test_1882_r2_an_unknown_state_never_hides_a_path() {
        assert_eq!(kept_folder_from_bridge(9, Some("/Users/someone/Kept".to_string())), kept("/Users/someone/Kept", false));
        assert_eq!(kept_folder_from_bridge(-4, Some("/Users/someone/Kept".to_string())), kept("/Users/someone/Kept", false));
        assert_eq!(kept_folder_from_bridge(9, None), KeptFolder::Unknown);
    }

    /// The developer helper follows the same rule (spec §5): `preserved:` only when the folder
    /// holds something or could not be checked, and a plain "nothing kept" line otherwise. The
    /// helper has no test harness of its own, so its source is pinned here.
    #[test]
    fn test_1882_r2_the_helper_prints_preserved_only_when_something_was_kept() {
        let helper = without_comments(include_str!("../../BeebeebFileProviderTools/DomainControlTool.swift"));
        let remove = &helper[helper.find("static func remove()").expect("remove()")..];
        let remove = &remove[..remove.find("static func signalRoot()").expect("next function")];
        let squashed: String = remove.chars().filter(|c| !c.is_whitespace()).collect();
        assert!(
            squashed.contains("switchkeptFolder(preservedLocation){case.hasEntries,.unchecked:print(\"preserved:\\(path)\")"),
            "`preserved:` is printed only for a folder that holds something or could not be checked:\n{remove}"
        );
        assert_eq!(remove.matches("print(\"preserved:").count(), 1, "one `preserved:` line, in the kept arm");
        for line in ["which is missing", "which is empty"] {
            assert!(remove.contains(line), "the helper says {line:?} when nothing was kept");
        }
        let check = &helper[helper.find("static func keptFolder(").expect("keptFolder()")..];
        assert!(check.contains("startAccessingSecurityScopedResource()"), "it looks through the URL's own scope");
    }

    // ── round 2: an error that carries a folder (M2), the install cleanup (M1) ─

    #[test]
    fn test_1882_r2_a_reply_with_an_error_and_a_folder_keeps_the_folder() {
        let folder = "/Users/someone/Library/CloudStorage/Beebeeb-Beebeeb (10-10-2026 10:50)";
        let failure = removal_from_bridge(-1, KEPT_HAS_ENTRIES, Some(folder.to_string()), Some("busy (NSFileProviderErrorDomain -1001)".to_string()))
            .expect_err("the removal reported an error");
        assert_eq!(failure.message, "busy (NSFileProviderErrorDomain -1001)");
        assert_eq!(failure.kept, kept(folder, true), "the folder rides with the error");

        // Sign-out: the failure is still only logged, but the folder reaches the alert.
        assert_eq!(sign_out_kept_location(Err(failure.clone())), Some(folder.to_string()));
        // Repair: one warning, as before, and the folder for the row.
        let mut warnings = Vec::new();
        assert_eq!(repair_removal(Err(failure), &mut warnings), (false, Some(folder.to_string())));
        assert_eq!(warnings, vec!["Could not remove Finder File Provider domain: busy (NSFileProviderErrorDomain -1001)".to_string()]);

        // An error with a missing or empty folder is still "nothing kept".
        let failure = removal_from_bridge(-1, KEPT_MISSING, Some(folder.to_string()), Some("busy".to_string())).unwrap_err();
        assert_eq!(failure.kept, KeptFolder::Nothing(NothingKept::Missing));
    }

    /// Review M2: the bridge checks the reported folder BEFORE it looks at the error, so a reply
    /// that carries both reaches Rust with the folder's state.
    #[test]
    fn test_1882_r2_the_bridge_checks_the_folder_before_the_error() {
        let bridge = without_comments(include_str!("../macos/FileProviderBridge.m"));
        let start = bridge.find("static int BeebeebRemoveDomainKeepingUnsynced(").expect("the shared removal");
        let body = &bridge[start..start + bridge[start..].find("\n}\n").expect("function ends")];
        let checked = body.find("BeebeebKeptFolderState(found_location)").expect("the folder is checked");
        let copied = body.find("BeebeebCopyMessage(path, location_buffer").expect("its path is copied");
        let error = body.find("if (found_error != nil)").expect("the error is handled");
        assert!(checked < error && copied < error, "folder state and path come before the error return:\n{body}");
    }

    #[test]
    fn test_1882_r2_the_install_cleanup_reports_the_folder_it_kept() {
        let folder = "/Users/someone/Library/CloudStorage/Beebeeb-Beebeeb (10-10-2026 11:00)";
        let setup = "Timed out waiting for the Beebeeb File Provider domain to become available".to_string();

        let kept_by_cleanup = install_cleanup_failure(setup.clone(), Ok(removal(kept(folder, true))));
        assert_eq!(
            kept_by_cleanup,
            InstallFailure {
                message: setup.clone(),
                kept_folder: Some(folder.to_string())
            }
        );

        let nothing = install_cleanup_failure(setup.clone(), Ok(removal(KeptFolder::Nothing(NothingKept::Missing))));
        assert_eq!(nothing, InstallFailure::from(setup.clone()));

        let failed_cleanup = install_cleanup_failure(
            setup.clone(),
            Err(RemovalFailure {
                message: "busy".to_string(),
                kept: kept(folder, false),
            }),
        );
        assert_eq!(
            failed_cleanup,
            InstallFailure {
                message: format!("{setup}; cleanup failed: busy"),
                kept_folder: Some(folder.to_string())
            },
            "the error text is as before, and a folder kept with the error still rides along"
        );
    }

    // ── round 2: the saved kept folder (review I2, lead ruling) ──────────────

    #[test]
    fn test_1882_r2_the_latest_kept_folder_is_saved_and_replaces_an_older_one() {
        let mut cfg = crate::config::DesktopConfig::default();
        assert!(record_kept_folder(&mut cfg, "/Users/someone/Kept (1)"));
        assert_eq!(cfg.kept_unsynced_folder.as_deref(), Some("/Users/someone/Kept (1)"));
        assert!(record_kept_folder(&mut cfg, "/Users/someone/Kept (2)"));
        assert_eq!(cfg.kept_unsynced_folder.as_deref(), Some("/Users/someone/Kept (2)"));
        assert!(!record_kept_folder(&mut cfg, "/Users/someone/Kept (2)"), "the same folder again changes nothing");
    }

    #[test]
    fn test_1882_r2_dismissing_clears_only_the_folder_the_row_showed() {
        let mut cfg = crate::config::DesktopConfig::default();
        record_kept_folder(&mut cfg, "/Users/someone/Kept (2)");
        assert!(
            !dismiss_kept_folder(&mut cfg, "/Users/someone/Kept (1)"),
            "a row showing an older folder cannot dismiss a newer one"
        );
        assert_eq!(cfg.kept_unsynced_folder.as_deref(), Some("/Users/someone/Kept (2)"));
        assert!(dismiss_kept_folder(&mut cfg, "/Users/someone/Kept (2)"));
        assert_eq!(cfg.kept_unsynced_folder, None);
        assert!(!dismiss_kept_folder(&mut cfg, "/Users/someone/Kept (2)"), "nothing left to dismiss");
    }

    #[test]
    fn test_1882_r2_the_saved_folder_survives_a_settings_save_and_a_restart() {
        let mut cfg = crate::config::DesktopConfig::default();
        record_kept_folder(&mut cfg, "/Users/someone/Library/CloudStorage/Beebeeb-Beebeeb (10-10-2026 10:50)");
        let settings = crate::config::DesktopSettings::from(&crate::config::DesktopConfig::default());
        cfg.apply_settings(settings);
        assert_eq!(
            cfg.kept_unsynced_folder.as_deref(),
            Some("/Users/someone/Library/CloudStorage/Beebeeb-Beebeeb (10-10-2026 10:50)"),
            "a settings save never clears it"
        );
        let toml = toml::to_string(&cfg).expect("serialize");
        let back: crate::config::DesktopConfig = toml::from_str(&toml).expect("parse");
        assert_eq!(back.kept_unsynced_folder, cfg.kept_unsynced_folder, "it survives a restart");
        let fresh = toml::to_string(&crate::config::DesktopConfig::default()).expect("serialize");
        assert!(!fresh.contains("kept_unsynced_folder"), "absent until something was kept");
    }

    #[test]
    fn test_1882_r2_the_folder_states_match_the_bridge() {
        let bridge = include_str!("../macos/FileProviderBridge.m");
        for (name, value) in [
            ("BeebeebKeptNoneReported", KEPT_NONE_REPORTED),
            ("BeebeebKeptMissing", KEPT_MISSING),
            ("BeebeebKeptEmpty", KEPT_EMPTY),
            ("BeebeebKeptHasEntries", KEPT_HAS_ENTRIES),
            ("BeebeebKeptUnchecked", KEPT_UNCHECKED),
            ("BeebeebKeptNoPath", KEPT_NO_PATH),
        ] {
            assert!(
                bridge.contains(&format!("{name} = {value},")),
                "FileProviderBridge.m must define {name} = {value}"
            );
        }
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

    /// Review M4: the nothing-kept side of the alert, as a decision a mutation can turn red.
    #[test]
    fn test_1882_r2_nothing_kept_raises_no_alert() {
        assert_eq!(kept_folder_alert(None), None);
        let folder = "/Users/someone/Library/CloudStorage/Beebeeb-Beebeeb (10-10-2026 10:50)";
        assert_eq!(
            kept_folder_alert(Some(folder)),
            Some((PRESERVED_FILES_TITLE, format!("{PRESERVED_FILES_SENTENCE}\n\n{folder}")))
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
        assert_eq!(removal(kept(&folder, true)).kept_location("sign-out"), Some(folder));
        assert_eq!(removal(KeptFolder::Nothing(NothingKept::NotReported)).kept_location("sign-out"), None);
    }

    #[test]
    fn test_1882_sign_out_carries_a_kept_folder_and_survives_a_failed_removal() {
        let folder = "/Users/someone/Kept".to_string();
        assert_eq!(
            sign_out_kept_location(Ok(removal(kept(&folder, true)))),
            Some(folder)
        );
        assert_eq!(sign_out_kept_location(Ok(DomainRemoval::default())), None);
        assert_eq!(sign_out_kept_location(Err("not registered".to_string().into())), None);
    }

    #[test]
    fn test_1882_repair_carries_a_kept_folder_and_turns_a_failure_into_one_warning() {
        let folder = "/Users/someone/Kept".to_string();

        let mut warnings = Vec::new();
        assert_eq!(
            repair_removal(Ok(removal(kept(&folder, true))), &mut warnings),
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
        assert_eq!(repair_removal(Err("busy".to_string().into()), &mut warnings), (false, None));
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
