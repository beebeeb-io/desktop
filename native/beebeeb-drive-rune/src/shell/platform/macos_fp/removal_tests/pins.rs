//! Source pins: no plain domain removal anywhere in this crate, the one decode site settles first (read through CRLF
//! text too), and the folder-state numbers are the bridge's. The pins over the Objective-C and Swift sources
//! (`removeDomain:` carries the mode, the bridge checks the folder before the error) run in
//! `crates/beebeeb-engine/tests/file_provider_source_pins.rs`.
use super::*;

fn lf(raw: &str) -> String {
    raw.replace("\r\n", "\n")
}

fn crlf(text: &str) -> String {
    lf(text).replace('\n', "\r\n")
}

/// `source` without `//` comments (every line ending in a comment loses it; none of this file's strings holds `//`).
fn without_comments(source: &str) -> String {
    lf(source)
        .lines()
        .map(|line| line.find("//").map_or(line, |at| &line[..at]))
        .collect::<Vec<_>>()
        .join("\n")
}

fn squash(text: &str) -> String {
    text.chars().filter(|c| !c.is_whitespace()).collect()
}

/// The non-test part of `removal.rs`, comments removed.
fn production(raw: &str) -> String {
    let full = lf(raw);
    let end = full.find("\n#[cfg(test)]\n#[path").expect("the tests module");
    without_comments(&full[..end])
}

fn function(source: &str, name: &str) -> String {
    let from = &source[source.find(name).unwrap_or_else(|| panic!("{name}"))..];
    squash(&from[..from.find("\n}\n").expect("function ends")])
}

fn every_removal_settles_a_missing_folder(raw: &str) {
    let source = production(raw);
    assert_eq!(
        source.matches("removal_from_bridge(").count(),
        2,
        "its definition and one decode site, so no removal can skip the settle"
    );
    let decode = function(&source, "fn decode_removal(");
    assert!(
        decode.contains("decode_removal_with(code,kept_state,location_buffer,error_message(code,error),|path|kept_state_number(bridge::kept_state_for_path(path)),std::thread::sleep,)"),
        "the real decode looks with the bridge's check and really waits: {decode}"
    );
    let with = function(&source, "fn decode_removal_with(");
    assert!(
        with.contains("letkept_state=settle_kept_state(kept_state,location.as_deref(),check,wait);"),
        "it settles the state first: {with}"
    );
    assert!(
        with.contains("removal_from_bridge(code,kept_state,location,error)"),
        "and decodes the settled state, not the first look: {with}"
    );
    assert_eq!(
        source.matches("decode_removal(").count(),
        3,
        "the definition, `remove_keeping_unsynced()` and the sweep's `remove_domain`"
    );
}

#[test]
fn test_1882_r3_every_removal_settles_a_missing_folder_before_it_decides() {
    every_removal_settles_a_missing_folder(include_str!("../removal.rs"));
}

#[test]
fn test_1882_r5_the_settle_pin_passes_on_a_windows_checkout() {
    every_removal_settles_a_missing_folder(&crlf(include_str!("../removal.rs")));
}

#[test]
fn test_1882_r5_the_settle_pin_still_fails_a_decode_that_skips_the_settle() {
    let broken = include_str!("../removal.rs").replace("= settle_kept_state(", "= skip_the_settle(");
    let failed = std::panic::catch_unwind(|| every_removal_settles_a_missing_folder(&crlf(&broken)));
    assert!(failed.is_err(), "a decode without the settle must fail the pin");
}

/// Every Rust source of this crate, with its path.
fn crate_sources() -> Vec<(std::path::PathBuf, String)> {
    let mut found = Vec::new();
    let mut stack = vec![std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src")];
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir).expect("read dir") {
            let path = entry.expect("entry").path();
            if path.is_dir() {
                stack.push(path);
            } else if path.extension().is_some_and(|e| e == "rs") {
                let text = std::fs::read_to_string(&path).expect("read source");
                found.push((path, text));
            }
        }
    }
    found
}

/// Rule 1: no removal in this crate but the bridge's preserving one. A plain `removeDomain` or any `removeAllDomains`
/// in Rust (an `msg_send`, a string handed to the runtime) would keep nothing. The needles are built so this file does
/// not match itself.
#[test]
fn test_1882_no_plain_remove_domain_or_remove_all_domains_exists_in_the_crate() {
    let needles = [["remove", "Domain"].concat(), ["remove", "AllDomains"].concat()];
    let scanned = crate_sources();
    assert!(scanned.len() > 50, "the scan looked at the crate");
    let offenders: Vec<String> = scanned
        .iter()
        .filter(|(path, _)| !path.ends_with("macos_fp/removal_tests/pins.rs"))
        .flat_map(|(path, text)| {
            let code = without_comments(text);
            needles
                .iter()
                .filter(move |needle| code.contains(needle.as_str()))
                .map(move |needle| format!("{}: {needle}", path.display()))
                .collect::<Vec<_>>()
        })
        .collect();
    assert!(
        offenders.is_empty(),
        "a removal that does not keep unsynced files: {offenders:?}"
    );
}

/// The folder-state numbers are the bridge's (`BeebeebKept* = n,`), read through CRLF text too.
#[test]
fn test_1882_r2_the_folder_states_match_the_bridge() {
    let bridge = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../packaging/macos/FileProviderBridge.m");
    let raw = std::fs::read_to_string(&bridge).expect("the bridge source");
    for text in [lf(&raw), crlf(&raw).replace("\r\n", "\n")] {
        for (name, value) in [
            ("BeebeebKeptNoneReported", KEPT_NONE_REPORTED),
            ("BeebeebKeptMissing", KEPT_MISSING),
            ("BeebeebKeptEmpty", KEPT_EMPTY),
            ("BeebeebKeptHasEntries", KEPT_HAS_ENTRIES),
            ("BeebeebKeptUnchecked", KEPT_UNCHECKED),
            ("BeebeebKeptNoPath", KEPT_NO_PATH),
        ] {
            assert!(
                text.contains(&format!("{name} = {value},")),
                "FileProviderBridge.m must define {name} = {value}"
            );
        }
    }
}
