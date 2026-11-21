//! Walks every registered scene's accessibility_json(): per area folder, counts the nodes with a click
//! action and an empty name or no role. `tests/a11y_ratchet.txt` records the counts; they may only fall.
//! `RATCHET_WRITE=1 cargo test a11y_walk` rewrites the file (as `component_ratchet` lowers a line).

use std::collections::BTreeMap;

use rune::{Env, Runtime};
use serde_json::Value;

use super::SCENES;
use crate::ui::gate::Scene;
use crate::{State, init, view};
use crate::frames as f;

/// The area folders: (name, its scene tables). `shell` is the shell's own table.
fn areas() -> Vec<(&'static str, Vec<&'static [Scene<State>]>)> {
    vec![
        ("shell", vec![SCENES]),
        ("files", vec![f::files::SCENES, f::files::scenes_waveb::SCENES, f::files::protected::SCENES,
            f::files::long_list::SCENES, f::files::session::SCENES, f::files::browse_states::SCENES]),
        ("search", vec![f::search::FRAMES]),
        ("global", vec![f::global::FRAMES]),
        ("settings", vec![f::settings::FRAMES]),
        ("account", vec![f::account::FRAMES, f::account::app_lock_password::SCENES]),
        ("signin", vec![f::signin::FRAMES, f::signin::create::FRAMES, f::signin::setup::FRAMES,
            f::signin::setup::RENDER_SCENES, f::signin::before_files::FRAMES, f::signin::lock::SCENES,
            f::signin::setup::guide::FRAMES]),
        ("item", vec![f::item::scenes::RENDER_SCENES]),
        ("this_pc", vec![f::this_pc::scenes::FRAMES, f::this_pc::scenes::RENDER_SCENES]),
        ("backups", vec![f::backups::FRAMES]),
        ("tray", vec![f::tray::FRAMES]),
        ("viewers", vec![f::viewers::FRAMES]),
        ("shared", vec![f::shared::scenes::FRAMES]),
        ("office", vec![f::office::sheets::FRAMES, f::office::slides::FRAMES, f::office::dialogs::FRAMES,
            f::office::docs::FRAMES, f::office::states::FRAMES]),
    ]
}

/// Nodes with a click action, in one accessibility tree.
fn clickables(node: &Value) -> usize {
    let own = usize::from(node["actions"].as_array().is_some_and(|a| a.iter().any(|x| x == "click")));
    own + node["children"].as_array().map_or(0, |c| c.iter().map(clickables).sum())
}

/// Nodes with a click action and an empty name or no role, in one accessibility tree.
fn unlabelled_clickables(node: &Value) -> usize {
    let clicks = node["actions"].as_array().is_some_and(|a| a.iter().any(|x| x == "click"));
    let named = node["name"].as_str().is_some_and(|n| !n.trim().is_empty());
    let role = node["role"].as_str().is_some_and(|r| !r.is_empty() && r != "unknown");
    let mine = usize::from(clicks && !(named && role));
    mine + node["children"].as_array().map_or(0, |c| c.iter().map(unlabelled_clickables).sum())
}

fn count_scene(apply: fn(&mut State)) -> usize {
    let mut state = init();
    apply(&mut state);
    let mut rt = Runtime::new(state, view, (1278.0, 798.0), Env { system_dark: false, scale: 2.0 });
    rt.frame(0.0);
    unlabelled_clickables(&rt.accessibility_json())
}

fn counts() -> BTreeMap<&'static str, usize> {
    let mut out = BTreeMap::new();
    for (area, tables) in areas() {
        let mut seen = std::collections::HashSet::new();
        let mut n = 0;
        for (name, apply) in tables.into_iter().flatten() {
            if seen.insert(*name) {
                n += count_scene(*apply);
            }
        }
        out.insert(area, n);
    }
    out
}

const FILE: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/a11y_ratchet.txt");
const HEAD: &str = "# Recorded counts for shell/a11y_walk_tests.rs: clickable nodes with an empty name or no role, per area folder,\n# summed over its scenes (light). Counts may only go down; RATCHET_WRITE=1 rewrites this file.\n";

fn recorded() -> BTreeMap<String, usize> {
    std::fs::read_to_string(FILE)
        .unwrap_or_default()
        .lines()
        .filter(|l| !l.starts_with('#'))
        .filter_map(|l| l.split_once(' '))
        .filter_map(|(k, v)| Some((k.to_string(), v.trim().parse().ok()?)))
        .collect()
}

/// The walk reads real trees: the Files window has clickable nodes (a zero count means a broken walk).
#[test]
fn a11y_walk_sees_clickable_nodes() {
    let mut rt = Runtime::new(init(), view, (1278.0, 798.0), Env { system_dark: false, scale: 2.0 });
    rt.frame(0.0);
    assert!(clickables(&rt.accessibility_json()) > 5);
}

#[test]
fn a11y_walk_unlabelled_clickables_never_rise() {
    let now = counts();
    if std::env::var_os("RATCHET_WRITE").is_some() {
        let body: String = now.iter().map(|(k, v)| format!("{k} {v}\n")).collect();
        std::fs::write(FILE, format!("{HEAD}{body}")).expect("write a11y_ratchet.txt");
        return;
    }
    let rec = recorded();
    let mut bad = Vec::new();
    for (area, n) in &now {
        match rec.get(*area) {
            None => bad.push(format!("{area}: no recorded count (now {n}); run with RATCHET_WRITE=1")),
            Some(r) if n > r => bad.push(format!("{area}: {n} unlabelled clickables, recorded {r}")),
            Some(r) if n < r => bad.push(format!("{area}: fell to {n} (recorded {r}); lower the line: `{area} {n}`")),
            _ => {}
        }
    }
    assert!(bad.is_empty(), "{}", bad.join("\n"));
}
