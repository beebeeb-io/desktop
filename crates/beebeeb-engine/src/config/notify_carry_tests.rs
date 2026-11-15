//! Tests for the 0.8.x notification carry (`DesktopConfig::load_from`): a file written by 0.8.11 holds
//! `notify_conflicts` and `notify_quota_warnings` but not the 0.9.0 switches `notify_needs_decision` and
//! `notify_local_space`. Tests only.

use super::DesktopConfig;

/// The notification lines of a 0.8.11 `desktop.toml` (the shipped fixture holds both as `true`), with the two values set.
fn old_file(conflicts: bool, quota: bool) -> String {
    format!(
        "sync_root = \"/\"\nnotify_conflicts = {conflicts}\nnotify_sync_complete = false\nnotify_quota_warnings = {quota}\n"
    )
}

fn load(text: &str) -> DesktopConfig {
    let dir = tempfile::tempdir().expect("temp dir");
    let path = dir.path().join("desktop.toml");
    std::fs::write(&path, text).expect("write");
    DesktopConfig::load_from(&path).expect("load")
}

#[test]
fn notify_carry_old_off_choices_become_the_new_switches() {
    let cfg = load(&old_file(false, false));
    assert!(
        !cfg.notify_needs_decision,
        "conflict alerts switched off in 0.8.x stay off"
    );
    assert!(
        !cfg.notify_local_space,
        "local-space alerts switched off in 0.8.x stay off"
    );
}

#[test]
fn notify_carry_each_old_key_moves_only_its_own_switch() {
    let cfg = load(&old_file(false, true));
    assert!(!cfg.notify_needs_decision);
    assert!(cfg.notify_local_space);
    let cfg = load(&old_file(true, false));
    assert!(cfg.notify_needs_decision);
    assert!(!cfg.notify_local_space);
}

#[test]
fn notify_carry_old_on_choices_stay_on() {
    let cfg = load(&old_file(true, true));
    assert!(cfg.notify_needs_decision && cfg.notify_local_space);
}

#[test]
fn notify_carry_a_file_that_holds_the_new_keys_keeps_its_own_choice() {
    let text = format!(
        "{}notify_needs_decision = true\nnotify_local_space = true\n",
        old_file(false, false)
    );
    let cfg = load(&text);
    assert!(
        cfg.notify_needs_decision && cfg.notify_local_space,
        "the new key wins over the old one"
    );
    let text = format!(
        "{}notify_needs_decision = false\nnotify_local_space = false\n",
        old_file(true, true)
    );
    let cfg = load(&text);
    assert!(!cfg.notify_needs_decision && !cfg.notify_local_space);
}

#[test]
fn notify_carry_a_file_with_neither_old_nor_new_key_defaults_on() {
    let cfg = load("sync_root = \"/\"\n");
    assert!(cfg.notify_needs_decision && cfg.notify_local_space);
    assert!(load("").notify_needs_decision);
}
