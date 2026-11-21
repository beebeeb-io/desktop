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
