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
/// kept nothing). Looked at again before it means "nothing kept" ([`settle_kept_state`]).
pub const KEPT_MISSING: i32 = 1;
/// The folder exists and is empty. Kept all the same (re-review D1): macOS may not have filled it
/// yet, and an existing folder is shown whether the app could list it or not.
pub const KEPT_EMPTY: i32 = 2;
/// The folder (or a single kept file) holds at least one entry.
pub const KEPT_HAS_ENTRIES: i32 = 3;
/// Something may be there, but the sandbox refused the look (spec §4's fallback).
pub const KEPT_UNCHECKED: i32 = 4;
/// A URL without a path: the domain is removed, and no folder can be named (review M3).
pub const KEPT_NO_PATH: i32 = 5;

/// How often a folder that reads as missing is looked at again before the removal is called
/// "nothing kept" (re-review D1). With [`KEPT_MISSING_RECHECK_INTERVAL`] that is about one second,
/// paid only by a removal that kept nothing.
pub const KEPT_MISSING_RECHECKS: u32 = 4;
/// The wait before each of those looks.
pub const KEPT_MISSING_RECHECK_INTERVAL: std::time::Duration = std::time::Duration::from_millis(250);

/// Re-review D1: a folder that reads as missing is looked at again, a few times over about one
/// second, before the removal counts as "nothing kept". Nothing guarantees that macOS has made or
/// filled the folder when the removal's completion handler runs, and a single early look would
/// hide the person's files for good (no alert, no row, no record, and nothing looks again).
///
/// `check` is the bridge's own folder check on a path (`stat` always answers in the sandbox);
/// `wait` is the sleep. Both are injected, so the tests never sleep. Only a missing folder that
/// has a path is looked at again; any other state is final. It stops at the first look that finds
/// the folder, whatever it holds, and returns what that look found.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub fn settle_kept_state(
    state: i32,
    location: Option<&str>,
    mut check: impl FnMut(&str) -> i32,
    mut wait: impl FnMut(std::time::Duration),
) -> i32 {
    if state != KEPT_MISSING {
        return state;
    }
    let Some(path) = location.filter(|path| !path.trim().is_empty()) else {
        return state;
    };
    for _ in 0..KEPT_MISSING_RECHECKS {
        wait(KEPT_MISSING_RECHECK_INTERVAL);
        let again = check(path);
        if again != KEPT_MISSING {
            return again;
        }
    }
    KEPT_MISSING
}

/// Why a removal kept nothing. Logged (debug), never shown.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NothingKept {
    NotReported,
    /// Still missing after the re-checks ([`settle_kept_state`]).
    Missing,
}

impl NothingKept {
    fn as_str(self) -> &'static str {
        match self {
            Self::NotReported => "none",
            Self::Missing => "missing",
        }
    }
}

/// What one removal left on this Mac (spec §5, "When files count as kept").
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KeptFolder {
    /// Nothing to tell the person.
    Nothing(NothingKept),
    /// Files were kept in this folder, exactly as the system reported it. `contents_checked` is
    /// `false` when the folder exists but the app could not look inside (spec §4). `empty` is
    /// `true` when the app looked and found nothing yet: the folder is still shown, because an
    /// existing folder may be one macOS has not filled (re-review D1).
    Kept {
        path: String,
        contents_checked: bool,
        empty: bool,
    },
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
        // Re-review D1: an existing folder is never hidden, empty or not.
        (KEPT_EMPTY, Some(path)) => KeptFolder::Kept {
            path,
            contents_checked: true,
            empty: true,
        },
        (KEPT_HAS_ENTRIES, Some(path)) => KeptFolder::Kept {
            path,
            contents_checked: true,
            empty: false,
        },
        (KEPT_UNCHECKED, Some(path)) => KeptFolder::Kept {
            path,
            contents_checked: false,
            empty: false,
        },
        (KEPT_NO_PATH, _) | (_, None) => KeptFolder::Unknown,
        // A state this build does not know: the folder is shown, never hidden.
        (_, Some(path)) => KeptFolder::Kept {
            path,
            contents_checked: false,
            empty: false,
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
                empty,
            } => {
                tracing::info!(
                    context,
                    preserved = true,
                    contents_checked,
                    empty,
                    "Finder location removed; macOS kept the files that had not reached the server"
                );
                Some(path)
            }
            KeptFolder::Nothing(reason) => {
                tracing::debug!(
                    context,
                    reported = reason.as_str(),
                    "Finder location removed; nothing kept"
                );
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
/// the cleanup's removal kept, if any. Since spec 2026-10-06 only Windows and Linux build it (macOS has
/// no install path; the reconciler adds Beebeeb and never removes a domain it added).
#[cfg_attr(target_os = "macos", allow(dead_code))]
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

/// Saves `location` as the latest kept folder in `desktop.toml`, as one load-change-save under the
/// config-write lock (r5: the app-start sweep and a Settings save can run at the same time, and
/// both go through the one `desktop.toml.tmp`). Returns whether the config changed.
pub fn remember_kept_folder(location: &str) -> Result<bool, String> {
    remember_kept_folder_at(&crate::config::DesktopConfig::path_for(crate::APP)?, location)
}

/// [`remember_kept_folder`] on the config file at `path` (a seam for the tests).
pub(crate) fn remember_kept_folder_at(path: &std::path::Path, location: &str) -> Result<bool, String> {
    crate::config::DesktopConfig::update_at(path, |cfg| {
        let changed = record_kept_folder(cfg, location);
        (changed, changed)
    })
}

/// Windows and Linux (task 1889): [`record_kept_folder`] for a flow that keeps its own copy of the config to read across
/// slow work (Add to Finder, Repair). The folder is written onto the file as it is now, as one load-change-save under the
/// config-write lock, and `cfg` becomes what was saved. Recording into `cfg` and saving it later would put back whatever
/// the copy loaded at the start held, such as a kept folder the app-start sweep saved meanwhile.
///
/// A write that failed is logged without the path, and the record is queued on the copy (fix round 1, M4): the flow's
/// next write applies it in the same write as its own result, so the row's record and the result land together, and if
/// that write fails too the flow returns its error. The person is told either way: the alert that follows this call names
/// the folder, and a failed save is the command's error, which is the live 1882 rule ("A failed save ... Repair then
/// returns an error and the app's alert names the folder", spec Frontend bullets, re-review D5, P1; the r4 sentence "the
/// alert, and saved for the row if the save allows" is struck there, but `remember_kept_folder` keeps its rule: a failure
/// is logged without the path and the alert still names the folder).
#[cfg_attr(target_os = "macos", allow(dead_code))]
pub fn remember_kept_folder_held(cfg: &mut crate::config::HeldConfig, location: &str) {
    let saved = crate::config::DesktopConfig::path_for(crate::APP)
        .and_then(|path| remember_kept_folder_held_at(cfg, &path, location));
    if let Err(error) = saved {
        cfg.queue_for_next_write(record_kept_folder, location);
        tracing::warn!(error = %error, "could not save the kept folder for Settings › Sync; it rides the next write");
    }
}

/// [`remember_kept_folder_held`] on the config file at `path` (a seam for the tests). Returns whether the config changed;
/// a write that failed queues the record on `cfg` and returns the error.
#[cfg_attr(target_os = "macos", allow(dead_code))]
pub(crate) fn remember_kept_folder_held_at(
    cfg: &mut crate::config::HeldConfig,
    path: &std::path::Path,
    location: &str,
) -> Result<bool, String> {
    let saved = cfg.update_held_at(path, |held| {
        let changed = record_kept_folder(held, location);
        (changed, changed)
    });
    if saved.is_err() {
        cfg.queue_for_next_write(record_kept_folder, location);
    }
    saved
}

/// What the row's "Dismiss" did, as the frontend reads it (r5, review thread on the dismiss
/// command): `cleared` says whether the saved folder was the one the row showed and is now gone;
/// `current` is the folder saved after the command, which is `None` when nothing is saved. A
/// dismiss that carries an older folder than the saved one clears nothing and reports the newer
/// path, so the row can show it instead of vanishing. The field names are the command's JSON.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct DismissOutcome {
    pub cleared: bool,
    pub current: Option<String>,
}

/// The row's "Dismiss" in `desktop.toml`, as one load-change-save under the config-write lock.
/// See [`DismissOutcome`] and [`dismiss_kept_folder`].
pub fn dismiss_saved_kept_folder(shown: &str) -> Result<DismissOutcome, String> {
    dismiss_saved_kept_folder_at(&crate::config::DesktopConfig::path_for(crate::APP)?, shown)
}

/// [`dismiss_saved_kept_folder`] on the config file at `path` (a seam for the tests).
pub(crate) fn dismiss_saved_kept_folder_at(path: &std::path::Path, shown: &str) -> Result<DismissOutcome, String> {
    crate::config::DesktopConfig::update_at(path, |cfg| {
        let cleared = dismiss_kept_folder(cfg, shown);
        (
            cleared,
            DismissOutcome {
                cleared,
                current: cfg.kept_unsynced_folder.clone(),
            },
        )
    })
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
            warnings.push(format!(
                "Could not remove Finder File Provider domain: {}",
                failure.message
            ));
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

    // ── portable source pins (r5) ────────────────────────────────────────────
    //
    // The pins below read Rust, Objective-C and Swift sources with `include_str!` or from disk.
    // A Windows checkout turns line endings into CRLF and path separators into `\`, so a pin that
    // looks for `"\n}\n"` or compares a path with `/` finds nothing there (PR #119, round 5).
    // Every pin reads its source through these two functions and takes the raw text as an
    // argument, so a test can hand it the Windows form on any OS.

    /// A source's text with `\r\n` turned into `\n`: what every pin anchors on.
    fn lf(raw: &str) -> String {
        raw.replace("\r\n", "\n")
    }

    /// The Windows form of a source: every line ends in `\r\n`.
    fn crlf(text: &str) -> String {
        lf(text).replace('\n', "\r\n")
    }

    /// A path relative to the repo root with `/` separators, whatever the OS wrote.
    fn slash(path: &str) -> String {
        path.replace('\\', "/")
    }

    // ── the bridge's reply ───────────────────────────────────────────────────

    fn kept(path: &str, contents_checked: bool) -> KeptFolder {
        KeptFolder::Kept {
            path: path.to_string(),
            contents_checked,
            empty: false,
        }
    }

    fn kept_empty(path: &str) -> KeptFolder {
        KeptFolder::Kept {
            path: path.to_string(),
            contents_checked: true,
            empty: true,
        }
    }

    fn removal(kept: KeptFolder) -> DomainRemoval {
        DomainRemoval { kept }
    }

    fn every_removal_settles_a_missing_folder(raw: &str) {
        let full = lf(raw);
        let source = without_comments(&full[..full.find("\n#[cfg(test)]\nmod tests {").expect("tests module")]);
        let squash = |text: &str| text.chars().filter(|c| !c.is_whitespace()).collect::<String>();
        assert_eq!(
            source.matches("removal_from_bridge(").count(),
            1,
            "one decode site, so no removal can skip the settle:\n{source}"
        );
        let function = |name: &str| {
            let from = &source[source.find(name).unwrap_or_else(|| panic!("{name}"))..];
            squash(&from[..from.find("\n}\n").expect("function ends")])
        };
        let decode = function("fn decode_removal(");
        assert!(
            decode.contains("decode_removal_with(code,kept_state,location_buffer,error_buffer,kept_state_for_path,std::thread::sleep,)"),
            "the real decode looks with the bridge's check and really waits: {decode}"
        );
        let with = function("fn decode_removal_with(");
        assert!(
            with.contains(
                "letkept_state=crate::finder_removal::settle_kept_state(kept_state,location.as_deref(),check,wait);"
            ),
            "it settles the state first: {with}"
        );
        assert!(
            with.contains("crate::finder_removal::removal_from_bridge(code,kept_state,location,"),
            "and decodes the settled state, not the first look: {with}"
        );
        assert_eq!(
            source.matches("decode_removal(").count(),
            3,
            "the definition, `remove()` and the sweep's `remove_domain`"
        );
    }

    fn the_helper_prints_preserved_for_any_existing_folder(raw: &str) {
        let helper = without_comments(&lf(raw));
        let remove = &helper[helper.find("static func remove()").expect("remove()")..];
        let remove = &remove[..remove.find("static func signalRoot()").expect("next function")];
        let squashed: String = remove.chars().filter(|c| !c.is_whitespace()).collect();
        assert!(
            squashed.contains(
                "switchsettledKeptFolder(preservedLocation){case.hasEntries,.empty,.unchecked:print(\"preserved:\\(path)\")"
            ),
            "`preserved:` is printed for any folder that exists, empty or not, after the re-checks (re-review D1):\n{remove}"
        );
        assert_eq!(
            remove.matches("print(\"preserved:").count(),
            1,
            "one `preserved:` line, in the kept arm"
        );
        assert!(
            remove.contains("which is missing"),
            "the helper says so when the folder is still missing after the re-checks"
        );
        assert!(
            !remove.contains("which is empty"),
            "an empty folder is kept now, so the helper never calls it 'nothing kept'"
        );
        // The same bounded re-check as the app (`settle_kept_state`): a few looks, about a second.
        let settle = &helper[helper
            .find("static func settledKeptFolder(")
            .expect("settledKeptFolder()")..];
        let settle = &settle[..settle.find("\n    }\n").expect("function ends")];
        let settle: String = settle.chars().filter(|c| !c.is_whitespace()).collect();
        assert!(
            settle.contains("for_in0..<keptMissingRechecks{")
                && settle.contains("Thread.sleep(forTimeInterval:keptMissingRecheckInterval)")
                && settle.contains("keptFolder(location)"),
            "the helper looks again at a missing folder, on the same URL:\n{settle}"
        );
        let looks = format!("static let keptMissingRechecks = {KEPT_MISSING_RECHECKS}");
        let seconds = format!(
            "static let keptMissingRecheckInterval: TimeInterval = {}",
            KEPT_MISSING_RECHECK_INTERVAL.as_secs_f64()
        );
        assert!(
            helper.contains(&looks) && helper.contains(&seconds),
            "the helper's constants are the app's ({looks}; {seconds})"
        );
        let check = &helper[helper.find("static func keptFolder(").expect("keptFolder()")..];
        assert!(
            check.contains("startAccessingSecurityScopedResource()"),
            "it looks through the URL's own scope"
        );
    }

    fn the_bridge_checks_the_folder_before_the_error(raw: &str) {
        let bridge = without_comments(&lf(raw));
        let start = bridge
            .find("static int BeebeebRemoveDomainKeepingUnsynced(")
            .expect("the shared removal");
        let body = &bridge[start..start + bridge[start..].find("\n}\n").expect("function ends")];
        let checked = body
            .find("BeebeebKeptFolderState(found_location)")
            .expect("the folder is checked");
        // Spec 2026-10-06 renamed the bridge's bounded copy to `BeebeebCopyString` (rebase, 2026-10-10).
        let copied = body
            .find("BeebeebCopyString(path, location_buffer")
            .expect("its path is copied");
        let error = body.find("if (found_error != nil)").expect("the error is handled");
        assert!(
            checked < error && copied < error,
            "folder state and path come before the error return:\n{body}"
        );
    }

    fn the_folder_states_match_the_bridge(raw: &str) {
        let bridge = lf(raw);
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

    /// The developer helper follows the same rule (spec §5): `preserved:` only when the folder
    /// holds something or could not be checked, and a plain "nothing kept" line otherwise. The
    /// helper has no test harness of its own, so its source is pinned here.
    #[test]
    fn test_1882_r3_the_helper_prints_preserved_for_any_existing_folder() {
        the_helper_prints_preserved_for_any_existing_folder(include_str!(
            "../../BeebeebFileProviderTools/DomainControlTool.swift"
        ));
    }

    #[test]
    fn test_1882_r5_the_helper_pin_passes_on_a_windows_checkout() {
        the_helper_prints_preserved_for_any_existing_folder(&crlf(include_str!(
            "../../BeebeebFileProviderTools/DomainControlTool.swift"
        )));
    }

    mod bridge;
    mod pins;
    mod saved;
    mod writes;
}
