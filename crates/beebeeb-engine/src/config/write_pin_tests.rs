//! The write pins of `desktop.toml` (task 1889), ported from the old shell's `config_write_pin_tests.rs`: three
//! source censuses over the production text of the engine AND the native app, so a writer in either tree is held to
//! the same rule.
//!
//! 1. no load-modify-save span exists outside the one helper (`config/store.rs`);
//! 2. no update closure writes a stale WHOLE copy over the config it was handed;
//! 3. every change of the file is made inside one update: no function loads a copy of its own and then updates, and
//!    the in-memory pause flag is stored only under the config-write lock.
//!
//! The old shell's other guards (its held-copy flows, its `lib.rs` command sites) read code that is not in this
//! tree and are retired with it. Each census proves it reads both trees (`the_census_reads_both_trees`), and each
//! scanner is shown to find the shapes it is for (`the_scanners_find_the_shapes_they_are_for_and_pass_the_real_ones`).

use crate::source_census::{NATIVE_PREFIX, TEST_ITEM_GATES, is_test_file, rust_files_with_native};
use crate::source_pin::{squeeze, without_items};

const STORE: &str = "crates/beebeeb-engine/src/config/store.rs";
const LOCK: &str = "crates/beebeeb-engine/src/config/lock.rs";

/// Every production `.rs` file of the engine and native trees as `(name, text)`: LF line ends, test items dropped,
/// and lines that are comments dropped.
fn production_files() -> Vec<(String, String)> {
    let mut files = Vec::new();
    for (name, path) in rust_files_with_native() {
        if is_test_file(&name) {
            continue;
        }
        let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {name}: {e}"));
        let production = without_items(&text.replace("\r\n", "\n"), &TEST_ITEM_GATES);
        let code = production
            .lines()
            .filter(|line| !line.trim_start().starts_with("//"))
            .collect::<Vec<_>>()
            .join("\n");
        files.push((name, code));
    }
    files.sort();
    files
}

/// The text of the item starting at `start`, up to the next line that closes it at the start's own indentation.
fn item(text: &str, start: &str) -> String {
    let at = text.find(start).unwrap_or_else(|| panic!("{start} exists"));
    let line_start = text[..at].rfind('\n').map_or(0, |n| n + 1);
    let indent: String = text[line_start..].chars().take_while(|c| *c == ' ').collect();
    let end = format!("\n{indent}}}\n");
    let to = text[at..].find(&end).unwrap_or_else(|| panic!("{start} ends"));
    text[at..at + to].to_string()
}

/// The census itself must have read both trees: a pin that scanned nothing passes on anything.
#[test]
fn the_census_reads_the_engine_and_the_native_tree() {
    let files = production_files();
    let names: Vec<&str> = files.iter().map(|(name, _)| name.as_str()).collect();
    for tree in ["crates/beebeeb-engine/src/", NATIVE_PREFIX] {
        let count = names.iter().filter(|name| name.starts_with(tree)).count();
        assert!(count > 20, "{tree}: read only {count}");
    }
    for must in [
        STORE,
        LOCK,
        "crates/beebeeb-engine/src/config.rs",
        "crates/beebeeb-engine/src/keychain.rs",
        "native/beebeeb-drive-rune/src/engine_host/settings/notifications.rs",
    ] {
        assert!(names.contains(&must), "{must} was not read");
    }
    let store = &files.iter().find(|(name, _)| name == STORE).unwrap().1;
    assert!(
        store.contains("fn save_to(") && !store.contains("mod write_pin_tests"),
        "production code is read, test modules are not"
    );
}

/// Native lines that contain `.save()` and are not a config write: the viewers' own `Ui::save`, the text viewer's
/// save command. `(file, the trimmed line, how many)`. `DesktopConfig` has no `save` to call (asserted below) and its
/// `save_to` is `pub(super)`, so no native file can write the file itself; an entry here is a different type. A new
/// `.save()` in a native file stays red until it is named here with its reason.
const SAVE_CALLS_ALLOWED: &[(&str, &str, usize)] = &[
    (
        "native/beebeeb-drive-rune/src/frames/viewers/dialogs.rs",
        "ui.save();",
        1,
    ),
    ("native/beebeeb-drive-rune/src/frames/viewers/mod.rs", "ui.save();", 1),
    (
        "native/beebeeb-drive-rune/src/frames/viewers/mod.rs",
        "\"DW-1524\" | \"DW-1526\" => ui.save(),",
        1,
    ),
    ("native/beebeeb-drive-rune/src/frames/viewers/text.rs", "ui.save();", 1),
    (
        "native/beebeeb-drive-rune/src/frames/viewers/unsaved_guard.rs",
        "ui.save();",
        1,
    ),
];

/// No load-modify-save span outside the one helper: nothing outside `config/store.rs` writes the file, and the store
/// writes it in exactly one place, which holds the lock across the read, the change and the write.
#[test]
fn no_desktop_config_load_modify_save_span_exists_outside_the_one_helper() {
    let files = production_files();
    let mut unexcused: Vec<(String, String)> = Vec::new();
    for (name, text) in &files {
        if name == STORE {
            continue;
        }
        for banned in [".save()", ".save_to("] {
            for line in text.lines().filter(|line| line.contains(banned)) {
                unexcused.push((name.clone(), line.trim().to_string()));
            }
        }
    }
    for (file, line, count) in SAVE_CALLS_ALLOWED {
        let held = unexcused.iter().filter(|(f, l)| f == file && l == line).count();
        assert_eq!(held, *count, "{file}: `{line}` is listed as {count} non-config save(s)");
        unexcused.retain(|(f, l)| !(f == file && l == line));
    }
    assert!(
        unexcused.is_empty(),
        "a file writes desktop.toml with `.save()` or `.save_to(`; write it through DesktopConfig::update, update_at, update_for or update_held: {unexcused:?}"
    );

    let config = &files.iter().find(|(name, _)| name == STORE).unwrap().1;
    assert!(!config.contains("fn save("), "config/store.rs has no `save` to call");
    assert_eq!(
        config.matches("save_to(").count(),
        2,
        "the store writes in one place: the definition of `save_to` and its one call"
    );
    assert_eq!(
        config.matches("config_write_guard()").count()
            + files
                .iter()
                .find(|(name, _)| name == LOCK)
                .expect("config/lock.rs is production")
                .1
                .matches("config_write_guard()")
                .count(),
        2,
        "the lock is defined once (config/lock.rs) and taken in one place (config/store.rs)"
    );
    let one_place = item(config, "    pub(super) fn try_update_at_then<R>(");
    let lock = one_place
        .find("let _writing = config_write_guard();")
        .expect("it takes the lock");
    let read = one_place.find("Self::load_from(path)").expect("it reads");
    let write = one_place.find("cfg.save_to(path)").expect("it writes");
    let mirror = one_place.find("saved(&cfg);").expect("it runs the mirror step");
    assert!(
        lock < read && read < write && write < mirror,
        "the lock is held before the read and still held at the write and the mirror step:\n{one_place}"
    );
    // Task 1887: the keep step's durable save is the same one place with a durable write step, never a second
    // load-modify-save: `save_to_durable` is defined once and called once, from that place.
    assert_eq!(
        config.matches("save_to_durable(").count(),
        2,
        "the durable write is defined once and called once, from the one place"
    );
    assert!(one_place.contains("cfg.save_to_durable(path, fs_steps)"), "{one_place}");

    // Every other way in is that one place.
    for (start, calls) in [
        ("    pub fn update_at<R>(", "Self::try_update_at("),
        ("    pub fn try_update_at<R>(", "Self::try_update_at_then("),
        ("    pub fn update_at_then<R>(", "Self::try_update_at_then("),
        ("    pub fn update_at_durable_with<R>(", "Self::try_update_at_then("),
        (
            "    pub fn update_for<R>(",
            "Self::update_at(&Self::path_for(identity)?, change)",
        ),
        ("    pub fn update_held_at<R>(", "DesktopConfig::update_at(path,"),
        ("    pub fn ensure_account_id_at(", "Self::try_update_at(path,"),
    ] {
        let body = item(
            if start.contains("update_held_at") {
                &files.iter().find(|(name, _)| name == LOCK).unwrap().1
            } else {
                config
            },
            start,
        );
        assert!(body.contains(calls), "{start} is `{calls}`:\n{body}");
        assert!(!body.contains("load_from(") && !body.contains("save_to("), "{body}");
    }
}

/// The text inside the parentheses of the call whose `(` is at byte `open`.
fn call_text(text: &str, open: usize) -> &str {
    let mut depth = 0usize;
    for (at, c) in text[open..].char_indices() {
        match c {
            '(' => depth += 1,
            ')' => {
                depth -= 1;
                if depth == 0 {
                    return &text[open + 1..open + at];
                }
            }
            _ => {}
        }
    }
    &text[open + 1..]
}

/// The calls whose closure gets the config to change. `update_for` is the one the engine and the native app call
/// with an identity (it is `update_at` on that identity's path).
const UPDATE_CALLS: [&str; 8] = [
    "DesktopConfig::update(",
    "DesktopConfig::update_at(",
    "DesktopConfig::update_for(",
    "DesktopConfig::try_update_at(",
    "DesktopConfig::update_at_then(",
    "DesktopConfig::update_at_durable_with(",
    ".update_held(",
    ".update_held_at(",
];

/// Shape 1: a stale WHOLE copy written through an update closure. The closure's argument is the file as it is now;
/// assigning over all of it (`*cfg = ...`), swapping or replacing or taking it, or `clone_from` puts back a copy
/// loaded earlier, which is the bug in a new shape. Returns what it found, one line each.
fn whole_copy_writes(text: &str) -> Vec<String> {
    let mut found = Vec::new();
    for needle in UPDATE_CALLS {
        for (at, _) in text.match_indices(needle) {
            let call = call_text(text, at + needle.len() - 1);
            let Some(open) = call.find('|') else { continue };
            let Some(close) = call[open + 1..].find('|') else {
                continue;
            };
            let params = call[open + 1..open + 1 + close].trim();
            let name: String = params
                .trim_start_matches("mut ")
                .trim_start_matches("&mut ")
                .chars()
                .take_while(|c| c.is_alphanumeric() || *c == '_')
                .collect();
            if name.is_empty() || name == "_" {
                continue;
            }
            let body = squeeze(&call[open + 1 + close + 1..]);
            let assign = format!("*{name}=");
            let assigns = body.matches(&assign).count() - body.matches(&format!("{assign}=")).count();
            if assigns > 0 {
                found.push(format!(
                    "{needle} closure `{name}` assigns over the whole config: `{assign}`"
                ));
            }
            for shape in [
                format!("mem::replace({name},"),
                format!("mem::swap({name},"),
                format!("mem::take({name})"),
                format!("{name}.clone_from("),
            ] {
                if body.contains(&shape) {
                    found.push(format!(
                        "{needle} closure `{name}` overwrites the whole config: `{shape}`"
                    ));
                }
            }
        }
    }
    found
}

/// Shape 3: a function that loads a copy of the config of its own and also changes the file through an update. Load,
/// change and write are one update; a copy loaded first is what a concurrent writer's change is lost under. Returns
/// the names of the functions found. A function starts at a line that opens with `fn`, `pub fn`, `async fn` (any
/// visibility) and ends at the line that closes it at the same indentation.
fn own_copy_then_update(text: &str) -> Vec<String> {
    let mut found = Vec::new();
    let mut offset = 0;
    for line in text.split_inclusive('\n') {
        let trimmed = line.trim_start();
        let after_vis = if let Some(rest) = trimmed.strip_prefix("pub(") {
            rest.split_once(") ").map_or(trimmed, |(_, r)| r)
        } else {
            trimmed.strip_prefix("pub ").unwrap_or(trimmed)
        };
        let after_vis = after_vis.strip_prefix("async ").unwrap_or(after_vis);
        if let Some(rest) = after_vis.strip_prefix("fn ") {
            let name: String = rest.chars().take_while(|c| c.is_alphanumeric() || *c == '_').collect();
            let start = offset + (line.len() - trimmed.len());
            let indent = &line[..line.len() - trimmed.len()];
            let end = format!("\n{indent}}}\n");
            if let Some(to) = text[start..].find(&end) {
                let body = &text[start..start + to];
                if body.contains("DesktopConfig::load(") && UPDATE_CALLS.iter().any(|call| body.contains(call)) {
                    found.push(name);
                }
            }
        }
        offset += line.len();
    }
    found
}

/// The scanners are seen to find the shapes they are for, and to pass the real ones.
#[test]
fn the_scanners_find_the_shapes_they_are_for_and_pass_the_real_ones() {
    // Shape 1.
    for bad in [
        "DesktopConfig::update(|cfg| { *cfg = stale.clone(); (true, ()) })",
        "DesktopConfig::update_at(&path, |cfg| { *cfg=stale; (true, ()) })",
        "DesktopConfig::update_for(identity, |config| { *config = stale; (true, ()) })",
        "held.update_held(|fresh| { std::mem::replace(fresh, stale); (true, ()) })",
        "DesktopConfig::try_update_at(&path, |cfg| { std::mem::swap(cfg, &mut stale); (true, ()) })",
        "DesktopConfig::update(|cfg| { cfg.clone_from(&stale); (true, ()) })",
        "DesktopConfig::update(|c| { let _old = std::mem::take(c); (true, ()) })",
        "DesktopConfig::update_at_then(&path, |cfg| { *cfg = stale.clone(); (true, ()) }, |_| {})",
        "DesktopConfig::update_at_durable_with(&path, |cfg| { *cfg = stale; (true, ()) }, &RealConfigFs)",
    ] {
        assert!(!whole_copy_writes(bad).is_empty(), "not found: {bad}");
    }
    for good in [
        "DesktopConfig::update(|cfg| { cfg.sync_mode = Some(mode); (true, ()) })",
        "DesktopConfig::update_for(identity, |config| { config.notify_conflicts = on; (true, ()) })",
        "DesktopConfig::update(|cfg| { if *cfg == other { return (false, ()); } (true, ()) })",
        "held.update_held(|held| (clear_finder_install_failure(held), ()))",
        "DesktopConfig::update_at_then(path, |cfg| { cfg.pause_sync = paused; (true, ()) }, |cfg| flag.store(cfg.pause_sync, Ordering::Relaxed))",
        "DesktopConfig::update_at_durable_with(path, |cfg| (true, record_kept_unsent(cfg, folder)), fs_steps)",
    ] {
        assert_eq!(whole_copy_writes(good), Vec::<String>::new(), "found in: {good}");
    }
    // Shape 3.
    let bad = "fn toggle(identity: Identity) {\n    let mut cfg = DesktopConfig::load().unwrap();\n    \
               DesktopConfig::update_for(identity, |c| { c.pause_sync = !cfg.pause_sync; (true, ()) });\n}\n";
    assert_eq!(
        own_copy_then_update(bad),
        vec!["toggle".to_string()],
        "not found: {bad}"
    );
    for good in [
        "fn read_only() {\n    let cfg = DesktopConfig::load().unwrap();\n    use_it(cfg.pause_sync);\n}\n",
        "pub(crate) async fn change(identity: Identity) {\n    DesktopConfig::update_for(identity, |c| { c.pause_sync = true; (true, ()) });\n}\n",
    ] {
        assert_eq!(own_copy_then_update(good), Vec::<String>::new(), "found in: {good}");
    }
}

/// Shape 1 on the real source: no update closure anywhere in production code writes over the whole config.
/// (`config/store.rs` and `config/lock.rs` are the places that hold the whole copy on purpose, in
/// `HeldConfig::update_held_at`.)
#[test]
fn no_stale_whole_copy_is_written_through_an_update_closure() {
    let mut found = Vec::new();
    for (name, text) in production_files() {
        if name == STORE || name == LOCK {
            continue;
        }
        for line in whole_copy_writes(&text) {
            found.push(format!("{name}: {line}"));
        }
    }
    assert!(found.is_empty(), "{found:#?}");
}

/// Every change of `desktop.toml` is made inside one update, in both trees: no function outside the config module
/// loads a copy of its own and then updates, and the in-memory pause flag is stored from the file only under the
/// config-write lock, through the helpers, never by a bare store next to a load.
#[test]
fn every_writer_of_desktop_toml_changes_it_inside_one_update() {
    let files = production_files();
    let mut found = Vec::new();
    let mut updates = 0;
    for (name, text) in &files {
        if name.starts_with("crates/beebeeb-engine/src/config/") || name == "crates/beebeeb-engine/src/config.rs" {
            continue;
        }
        updates += UPDATE_CALLS
            .iter()
            .map(|call| text.matches(call).count())
            .sum::<usize>();
        for function in own_copy_then_update(text) {
            found.push(format!("{name}: `{function}` loads a copy of its own and then updates"));
        }
        assert!(
            !text.contains(".sync_paused.store("),
            "{name} stores the pause flag outside the locked helpers"
        );
    }
    assert!(found.is_empty(), "{found:#?}");
    // The census saw the writers it is about, in both trees: the engine's bindings and the native settings.
    for tree in [
        "crates/beebeeb-engine/src/app/",
        "native/beebeeb-drive-rune/src/engine_host/settings",
    ] {
        assert!(
            files
                .iter()
                .any(|(name, text)| name.starts_with(tree) && text.contains("DesktopConfig::update_for(")),
            "{tree} has no writer the census can see"
        );
    }
    assert!(updates >= 10, "the census read only {updates} update calls");
}
