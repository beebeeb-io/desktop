//! Config-level tests that run on Linux CI (spec section 2, bullets 3 and 4):
//! what Tauri's merged `app.windows` will be per platform, and whether the
//! capability files cover every window label the macOS registry may create.
//!
//! Tauri merges `tauri.<platform>.conf.json` into `tauri.conf.json` as an RFC
//! 7396 JSON Merge Patch (`tauri_utils::config::parse::read_from` calls
//! `json_patch::merge`), which REPLACES arrays wholesale. That is the whole
//! reason the macOS config can drop `tray`, `windows-onboarding` and `main-app`
//! by declaring its own `app.windows`. Only the bundled result needs a Mac.

use super::registry::MACOS_LABELS;
use serde_json::{Value, json};
use std::path::{Path, PathBuf};

fn manifest_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn read_json(path: &Path) -> Value {
    let text = std::fs::read_to_string(path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
    serde_json::from_str(&text).unwrap_or_else(|e| panic!("parse {}: {e}", path.display()))
}

fn window_labels(config: &Value) -> Vec<String> {
    config
        .pointer("/app/windows")
        .and_then(Value::as_array)
        .map(|ws| {
            ws.iter()
                .filter_map(|w| w.get("label").and_then(Value::as_str).map(str::to_owned))
                .collect()
        })
        .unwrap_or_default()
}

/// `tauri.conf.json` merged with `tauri.<target>.conf.json` when that file exists.
fn merged_config(target: &str) -> Value {
    let mut config = read_json(&manifest_dir().join("tauri.conf.json"));
    let platform_file = manifest_dir().join(format!("tauri.{target}.conf.json"));
    if platform_file.exists() {
        json_patch::merge(&mut config, &read_json(&platform_file));
    }
    config
}

// The base config's window list since slice 6 (the Settings flip): the three
// Windows-shell windows are still declared and created at startup on every
// platform (the P3 defect the registry-mirroring part of slice 6 owns), plus
// `macos-settings` — which is `create: false`, so it is declared and
// capability-covered but built only on demand by the macOS open path.
const BASE_WINDOWS: [&str; 4] = ["tray", "windows-onboarding", "main-app", "macos-settings"];

#[test]
fn the_merge_replaces_arrays_wholesale() {
    // The semantics slice 6 relies on, proven with a synthetic patch so it does
    // not depend on what the real macOS file says today.
    let mut config = read_json(&manifest_dir().join("tauri.conf.json"));
    assert_eq!(window_labels(&config), BASE_WINDOWS);
    let patch = json!({ "app": { "windows": [ { "label": "popover" } ] } });
    json_patch::merge(&mut config, &patch);
    assert_eq!(window_labels(&config), ["popover"]);
    // ...and keeps every sibling key it did not mention.
    assert!(config.pointer("/app/security/csp").is_some());
    assert!(config.pointer("/app/trayIcon/id").is_some());
}

#[test]
fn unmerged_config_lists_the_shell_windows_and_the_ondemand_settings_window() {
    let base = read_json(&manifest_dir().join("tauri.conf.json"));
    assert_eq!(window_labels(&base), BASE_WINDOWS);
    // The Settings window must never be created at startup: it is built on
    // demand by `show_macos_settings_window` (macOS only).
    let settings = base
        .pointer("/app/windows")
        .and_then(Value::as_array)
        .and_then(|ws| ws.iter().find(|w| w.get("label").and_then(Value::as_str) == Some("macos-settings")))
        .and_then(|w| w.get("create"))
        .and_then(Value::as_bool);
    assert_eq!(settings, Some(false), "macos-settings must be create: false");
}

#[test]
fn windows_and_linux_merge_to_the_unmerged_window_list() {
    // There is no tauri.windows.conf.json or tauri.linux.conf.json, so their
    // builds see the base file. If one is added it must not drop these labels.
    for target in ["windows", "linux"] {
        assert_eq!(window_labels(&merged_config(target)), BASE_WINDOWS, "{target}");
    }
}

#[test]
fn only_the_macos_platform_file_exists() {
    let mut names: Vec<String> = std::fs::read_dir(manifest_dir())
        .unwrap()
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| n.starts_with("tauri.") && n.contains(".conf."))
        .collect();
    names.sort();
    assert_eq!(names, ["tauri.conf.json", "tauri.macos.conf.json"]);
}

#[test]
fn macos_merge_still_creates_the_startup_windows_and_merges_the_platform_file() {
    let merged = merged_config("macos");
    // The startup-created Windows-shell windows are still in the merged list
    // on macOS (the P3 defect — the registry-mirroring part of slice 6 owns
    // it). The added `macos-settings` entry is `create: false`, so the flip
    // adds no idle webview; it is only ever built on demand.
    assert_eq!(window_labels(&merged), BASE_WINDOWS);
    // Prove the real macOS file WAS merged and not silently skipped: its
    // template-icon patch is visible in the result, and absent from the base.
    let base = read_json(&manifest_dir().join("tauri.conf.json"));
    assert_eq!(base.pointer("/app/trayIcon/iconAsTemplate"), Some(&Value::Bool(false)));
    assert_eq!(merged.pointer("/app/trayIcon/iconAsTemplate"), Some(&Value::Bool(true)));
}

#[test]
#[ignore = "slice 6 (the flip) gives tauri.macos.conf.json its own app.windows; remove this ignore then"]
fn macos_merge_lists_only_the_popover() {
    assert_eq!(window_labels(&merged_config("macos")), ["popover"]);
}

// -- capabilities -----------------------------------------------------------

/// Minimal glob: `*` matches any run of characters, everything else is literal.
/// Enough for Tauri's capability window patterns (`conflict-*`).
fn glob_match(pattern: &str, text: &str) -> bool {
    let parts: Vec<&str> = pattern.split('*').collect();
    if parts.len() == 1 {
        return pattern == text;
    }
    let (first, last) = (parts[0], parts[parts.len() - 1]);
    if !text.starts_with(first) || text.len() < first.len() + last.len() || !text.ends_with(last) {
        return false;
    }
    let mut rest = &text[first.len()..text.len() - last.len()];
    for mid in &parts[1..parts.len() - 1] {
        match rest.find(mid) {
            Some(i) => rest = &rest[i + mid.len()..],
            None => return false,
        }
    }
    true
}

/// The window and webview patterns one capability grants on `platform`
/// (`"macOS"`, `"windows"`, `"linux"`; capability `platforms` is matched
/// case-insensitively, and a capability without `platforms` applies everywhere).
fn patterns_granted_by(cap: &Value, platform: &str) -> Vec<String> {
    if let Some(platforms) = cap.get("platforms").and_then(Value::as_array) {
        if !platforms
            .iter()
            .filter_map(Value::as_str)
            .any(|p| p.eq_ignore_ascii_case(platform))
        {
            return Vec::new();
        }
    }
    ["windows", "webviews"]
        .iter()
        .filter_map(|key| cap.get(*key).and_then(Value::as_array))
        .flat_map(|list| list.iter().filter_map(Value::as_str).map(str::to_owned))
        .collect()
}

/// Every pattern granted by `capabilities/*.json` on `platform`, and how many
/// capability files were read.
fn granted_window_patterns(platform: &str) -> (usize, Vec<String>) {
    let dir = manifest_dir().join("capabilities");
    let mut files = 0;
    let mut patterns = Vec::new();
    for entry in std::fs::read_dir(&dir).unwrap().filter_map(|e| e.ok()) {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }
        files += 1;
        patterns.extend(patterns_granted_by(&read_json(&path), platform));
    }
    (files, patterns)
}

fn uncovered(platform: &str, labels: &[&str]) -> Vec<String> {
    let (files, patterns) = granted_window_patterns(platform);
    // A check that read nothing would call every label covered or uncovered at
    // random; make it prove it read the capability files.
    assert!(
        files >= 1 && !patterns.is_empty(),
        "read {files} capability files, {} patterns",
        patterns.len()
    );
    labels
        .iter()
        .filter(|l| !patterns.iter().any(|p| glob_match(p, l)))
        .map(|l| l.to_string())
        .collect()
}

#[test]
fn a_capability_with_platforms_applies_only_to_those_platforms() {
    // Slice 6 may add a macOS-only capability file; the coverage check must
    // honour `platforms` or it would credit Windows with windows it cannot see.
    let mac_only = json!({ "platforms": ["macOS"], "windows": ["popover", "review"] });
    assert_eq!(patterns_granted_by(&mac_only, "macOS"), ["popover", "review"]);
    assert_eq!(patterns_granted_by(&mac_only, "macos"), ["popover", "review"]);
    assert!(patterns_granted_by(&mac_only, "windows").is_empty());
    assert!(patterns_granted_by(&mac_only, "linux").is_empty());
    let everywhere = json!({ "windows": ["settings"], "webviews": ["w-*"] });
    for platform in ["macOS", "windows", "linux"] {
        assert_eq!(
            patterns_granted_by(&everywhere, platform),
            ["settings", "w-*"],
            "{platform}"
        );
    }
}

#[test]
fn glob_match_behaves() {
    assert!(glob_match("conflict-*", "conflict-7f3a"));
    assert!(glob_match("conflict-*", "conflict-"));
    assert!(!glob_match("conflict-*", "conflict"));
    assert!(!glob_match("conflict-*", "review"));
    assert!(glob_match("settings", "settings"));
    assert!(!glob_match("settings", "settings2"));
    assert!(glob_match("a*b*c", "aXXbYYc"));
    assert!(!glob_match("a*b*c", "aXXcYYb"));
    assert!(glob_match("*", "anything"));
}

#[test]
fn review_does_not_match_the_conflict_glob() {
    // Spec section 2: `review` is not covered by `conflict-*`, so it needs its
    // own capability entry.
    let (_, patterns) = granted_window_patterns("macOS");
    assert!(patterns.iter().any(|p| p == "conflict-*"));
    assert!(!glob_match("conflict-*", "review"));
}

#[test]
fn every_window_the_shipping_config_creates_is_covered_on_every_platform() {
    // tray, windows-onboarding, main-app and the on-demand macos-settings
    // (config) plus the dynamic conflict-<file_id> windows: a window with no
    // capability cannot `invoke` or `listen`, and fails silently.
    // macOS creates the same startup labels today (the P3 defect), so it is
    // in the loop; a capability file that dropped one of them for macOS must
    // fail here.
    for platform in ["macOS", "windows", "linux"] {
        let mut labels: Vec<&str> = BASE_WINDOWS.to_vec();
        labels.extend(["settings", "onboarding", "conflict-7f3a-0001"]);
        assert_eq!(uncovered(platform, &labels), Vec::<String>::new(), "{platform}");
    }
}

#[test]
fn registry_labels_covered_today_are_settings_and_onboarding_only() {
    // The pinned gap. Slice 6 adds `popover` and `review` to the capability
    // file, which makes this assertion fail on purpose: delete it then and
    // un-ignore `every_macos_registry_label_is_covered`.
    assert_eq!(uncovered("macOS", &MACOS_LABELS), ["popover", "review"]);
}

#[test]
#[ignore = "slice 6 adds capability entries for popover and review; remove this ignore then"]
fn every_macos_registry_label_is_covered() {
    assert_eq!(uncovered("macOS", &MACOS_LABELS), Vec::<String>::new());
}
