//! Component ratchet: counts hand-built copies of shared UI and fails when a count rises above the
//! number recorded in `tests/component_ratchet.txt`. Lowering a count: update the table (the test prints
//! the new line). No refactor lives here; see board/briefs/desktop-ui-components-audit-2026-11-21.md.
//! Counts are plain text matches over non-test `.rs` files under `src/`.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(rd) = fs::read_dir(dir) else { return };
    for e in rd.flatten() {
        let p = e.path();
        if p.is_dir() {
            walk(&p, out);
        } else if p.extension().is_some_and(|x| x == "rs") {
            let name = p.file_name().unwrap().to_string_lossy().to_string();
            if name.contains("test") || name.contains("scene") || name.contains("fixture") {
                continue;
            }
            out.push(p);
        }
    }
}

/// Count of `needle` occurrences in files under `src/<sub>` (all of src when `sub` is empty),
/// skipping paths that start with any `skip` prefix (relative to src, `/`-separated).
fn count(files: &[(String, String)], sub: &str, skip: &[&str], needles: &[&str]) -> usize {
    files
        .iter()
        .filter(|(p, _)| p.starts_with(sub) && !skip.iter().any(|s| p.starts_with(s)))
        .map(|(_, body)| needles.iter().map(|n| body.matches(n).count()).sum::<usize>())
        .sum()
}

/// Spacing calls in `frames/` whose first argument is a number literal.
fn spacing_literals(files: &[(String, String)]) -> usize {
    let mut n = 0;
    for (p, body) in files.iter().filter(|(p, _)| p.starts_with("frames/")) {
        let _ = p;
        for call in [".gap(", ".padding(", ".px(", ".py("] {
            for (i, _) in body.match_indices(call) {
                let rest = &body[i + call.len()..];
                if rest.chars().next().is_some_and(|c| c.is_ascii_digit()) {
                    n += 1;
                }
            }
        }
    }
    n
}

/// Button builders that must stop holding `role("button")` (path under src, function name).
const BUTTON_BUILDERS: &[(&str, &str)] = &[
    ("ui/button.rs", "outline_button"),
    ("ui/settings_controls.rs", "button"),
    ("ui/settings_dialog.rs", "button"),
    ("ui/settings_dialog.rs", "button_danger"),
    ("ui/au.rs", "button"),
    ("ui/account_set.rs", "button"),
    ("ui/backups_kit.rs", "ghost_button"),
    ("ui/backups_kit.rs", "button_md"),
    ("ui/setup.rs", "small_button"),
    ("ui/detail_pane.rs", "action_button"),
    ("frames/viewers/chrome.rs", "button"),
    ("frames/viewers/dialogs.rs", "button_keyed"),
    ("frames/viewers/office.rs", "small_button"),
    ("frames/shared/parts/controls.rs", "button_keyed"),
    ("frames/photos/chrome.rs", "button"),
    ("frames/signin/before_files.rs", "button"),
    ("frames/signin/setup/guide.rs", "button"),
    ("frames/signin/create_kit.rs", "small_button"),
    ("frames/files/share.rs", "primary"),
    ("frames/settings/previews.rs", "word_button"),
    ("frames/this_pc/chrome.rs", "outline_button"),
    ("frames/office/states.rs", "text_button"),
    ("feedback/ui.rs", "button"),
    ("feedback/ui.rs", "primary"),
];

fn starts_fn(line: &str) -> bool {
    let t = line.trim_start();
    if t.starts_with("fn ") || t.starts_with("pub fn ") {
        return true;
    }
    // `pub(...) fn `
    t.strip_prefix("pub(").is_some_and(|r| r.split_once(") fn ").is_some_and(|(a, _)| !a.contains(' ')))
}

/// Body of `fn <name>(` up to the next line that starts a function; None when not found.
fn fn_body(body: &str, name: &str) -> Option<String> {
    let needle = format!("fn {name}(");
    let mut out = String::new();
    let mut inside = false;
    for line in body.lines() {
        if inside {
            if starts_fn(line) {
                break;
            }
        } else if line.contains(&needle) {
            inside = true;
        } else {
            continue;
        }
        out.push_str(line);
        out.push('\n');
    }
    inside.then_some(out)
}

/// Entries of `list` whose function body in `files` holds `role("button")`.
fn builders_holding(files: &[(String, String)], list: &[(&str, &str)]) -> usize {
    list.iter()
        .filter(|(path, name)| {
            files
                .iter()
                .find(|(p, _)| p == path)
                .and_then(|(_, b)| fn_body(b, name))
                .is_some_and(|b| b.contains("role(\"button\")"))
        })
        .count()
}

fn family_button_builders(files: &[(String, String)]) -> usize {
    let own = files
        .iter()
        .find(|(p, _)| p == "ui/button.rs")
        .and_then(|(_, b)| fn_body(b, "button"))
        .is_some_and(|b| b.contains("role(\"button\")")) as usize;
    own + builders_holding(files, BUTTON_BUILDERS)
}

/// Files holding `needle`, `skip` prefixes left out.
fn files_holding(files: &[(String, String)], skip: &[&str], needle: &str) -> usize {
    files
        .iter()
        .filter(|(p, b)| !skip.iter().any(|s| p.starts_with(s)) && b.contains(needle))
        .count()
}

fn measure() -> BTreeMap<&'static str, usize> {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut paths = Vec::new();
    walk(&src, &mut paths);
    let files: Vec<(String, String)> = paths
        .iter()
        .map(|p| {
            let rel = p.strip_prefix(&src).unwrap().to_string_lossy().replace('\\', "/");
            (rel, fs::read_to_string(p).unwrap_or_default())
        })
        .collect();
    let mut m = BTreeMap::new();
    m.insert("sidebar_assemblies", count(&files, "", &[], &["fn sidebar("]));
    m.insert("button_builders_role_button_outside_ui", count(&files, "", &["ui/"], &["role(\"button\")"]));
    m.insert("status_bars_key_outside_ui", count(&files, "", &["ui/"], &["key(\"status-bar\")"]));
    m.insert("dialog_cards_role_dialog_outside_ui", count(&files, "", &["ui/"], &["role(\"dialog\")"]));
    m.insert("empty_states", count(&files, "", &[], &["fn empty_state(", "fn empty("]));
    m.insert("toasts", count(&files, "", &[], &["fn toast("]));
    m.insert("local_textstyle_new_outside_theme", count(&files, "", &["theme/"], &["TextStyle::new("]));
    m.insert("spacing_literals_in_frames", spacing_literals(&files));
    m.insert("family_button_builders", family_button_builders(&files));
    m.insert("family_status_bars", files_holding(&files, &[], "key(\"status-bar\")"));
    m.insert("family_dialog_cards", files_holding(&files, &[], "kit::drop(24.0, 60.0") + files_holding(&files, &[], "kit::drop(18.0, 50.0"));
    m.insert("family_drive_sidebars", files_holding(&files, &["theme/"], "\"THIS PC\""));
    m
}

#[test]
fn component_ratchet() {
    let table_path = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/component_ratchet.txt");
    let table = fs::read_to_string(&table_path).unwrap_or_default();
    let mut limits: BTreeMap<String, usize> = BTreeMap::new();
    for line in table.lines().map(str::trim).filter(|l| !l.is_empty() && !l.starts_with('#')) {
        if let Some((k, v)) = line.split_once(char::is_whitespace) {
            limits.insert(k.to_string(), v.trim().parse().expect("bad count in component_ratchet.txt"));
        }
    }
    let now = measure();
    if std::env::var_os("RATCHET_WRITE").is_some_and(|v| v == "1")
        && !now.iter().any(|(k, v)| limits.get(*k).is_some_and(|max| v > max))
    {
        let header: Vec<&str> = table.lines().filter(|l| l.trim_start().starts_with('#')).collect();
        let mut out = header.join("\n");
        if !out.is_empty() {
            out.push('\n');
        }
        for (k, v) in &now {
            out.push_str(&format!("{k} {v}\n"));
        }
        fs::write(&table_path, out).expect("write component_ratchet.txt");
        return;
    }
    let mut bad = Vec::new();
    for (k, v) in &now {
        match limits.get(*k) {
            None => bad.push(format!("{k}: no recorded count (now {v}); add `{k} {v}`")),
            Some(max) if v > max => bad.push(format!(
                "{k}: {v} is above the recorded {max}; use the shared component in src/ui instead of a new copy"
            )),
            Some(max) if v < max => {
                eprintln!("component_ratchet: {k} dropped {max} -> {v}; lower it in tests/component_ratchet.txt")
            }
            _ => {}
        }
    }
    assert!(bad.is_empty(), "component ratchet failed:\n{}", bad.join("\n"));
}

#[cfg(test)]
mod tests {
    use super::*;

    const B: &str = "role(\"button\")";

    #[test]
    fn body_ends_at_pub_crate_fn() {
        let src = format!("fn a() {{\n  {B}\n}}\npub(crate) fn b() {{}}\n");
        let body = fn_body(&src, "a").unwrap();
        assert!(body.contains(B) && !body.contains("fn b"));
        assert!(!fn_body(&src, "b").unwrap().contains(B));
    }

    #[test]
    fn body_ends_at_fn_and_end_of_file() {
        let src = format!("fn a() {{}}\n    fn b() {{\n {B}\n}}\n");
        assert!(!fn_body(&src, "a").unwrap().contains(B));
        assert!(fn_body(&src, "b").unwrap().contains(B));
        assert!(fn_body(&src, "c").is_none());
    }

    #[test]
    fn missing_file_counts_zero() {
        let files = vec![("ui/x.rs".to_string(), format!("fn a() {{ {B} }}"))];
        assert_eq!(builders_holding(&files, &[("ui/x.rs", "a"), ("ui/none.rs", "a"), ("ui/x.rs", "z")]), 1);
    }
}
