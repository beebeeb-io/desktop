//! The identity pin: a per-app name exists in ONE file per crate, and every consumer reads it from the identity
//! it was handed.
//!
//! Three rules over the PRODUCTION text (test items and comments removed) of all three Rust trees: the engine
//! (`crates/beebeeb-engine/src`), the Tauri shell (`src-tauri/src`) and the native app
//! (`native/beebeeb-drive-rune/src` and its `build.rs`).
//!
//! 1. **Literals** ([`literal_hits`]): a string or number that IS a per-app name (a bundle identifier or any text
//!    holding `io.beebeeb`, the config folder, the sync folder, the provider name, the daemon socket, the app
//!    group's team prefix, one of the six GUIDs in any digit grouping) may stand only in a file of
//!    [`LITERALS_ALLOWED`], and exactly as often as listed there. The list is by whole file path, so a new
//!    `…/provider_identity.rs` is not on it.
//! 2. **Naming an identity** ([`naming_hits`]): `identity::REAL` and `identity::TEST` (also through a `use` list
//!    or a glob) may stand only where [`NAMING_ALLOWED`] says: the shell's `real_app.rs` and the native
//!    `shell/identity.rs`. Everything else is handed its identity. A third identity cannot be made anywhere
//!    else: the struct's fields are private (`the_identity_type_cannot_be_built_outside_its_file`).
//! 3. **The consumer census** ([`CONSUMERS`]): every read of an identity accessor in the engine, by file and
//!    accessor, with its count. A consumer put back on a literal, or on a name glued from pieces, drops a read
//!    and fails here even when rule 1 cannot see the pieces.
//!
//! `the_pin_goes_red_on_each_known_way_around_it` runs the rules over the ten leaks the review of PR 307 planted,
//! so the pin is seen to fail in every run, not only on the day it was written.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use crate::source_census::{TEST_ITEM_GATES, is_test_file, rust_files};
use crate::source_pin::without_items;

const NATIVE_PREFIX: &str = "native/beebeeb-drive-rune/";

/// A string literal that is exactly one of these is a per-app name.
const EXACT_NAMES: [&str; 5] = ["beebeeb", "beebeeb-test", "Beebeeb", "Beebeeb Test", "BeebeebTest"];
/// A string literal that holds one of these (compared in lower case) is a per-app name.
const NAME_PARTS: [&str; 3] = ["io.beebeeb", "daemon.sock", "r8352wdjjr"];
/// The six GUIDs (provider, status class, thumbnail class; real then test), digits only.
const GUID_DIGITS: [&str; 6] = [
    "b33b33b052174c0a9e3f1f2a7c4d8e90",
    "c0b33b3352174c0a9e3f2f3b8d5e9fa1",
    "b2e0a7d49c614f3a8d2e7a1c5e9b0f44",
    "1876bd5bc73c452aa40ce2501f3de243",
    "ecbe203e9c204ba7babfbfc6dad89f0e",
    "691049a886db450a885afde5b4dc1431",
];

/// Rule 1's allow-list: `(file, literals, why)`. Exact counts: a literal added to one of these files is seen too.
const LITERALS_ALLOWED: [(&str, usize, &str); 12] = [
    (
        "crates/beebeeb-engine/src/identity.rs",
        24,
        "the engine's one identity file: REAL and TEST",
    ),
    (
        "crates/beebeeb-engine/src/finder_setup/error.rs",
        2,
        "two in-process NSError-style domain labels (`io.beebeeb.bridge`, `io.beebeeb.app`): tags an error is \
         classified by, never a name anything is stored or registered under",
    ),
    (
        "native/beebeeb-drive-rune/build.rs",
        2,
        "the build script's two allowed identifiers",
    ),
    (
        "native/beebeeb-drive-rune/src/shell/identity.rs",
        9,
        "the native shell's one identity file",
    ),
    (
        "native/beebeeb-drive-rune/src/ui/kit.rs",
        1,
        "the logo's accessibility label (the brand, for both builds)",
    ),
    (
        "macos_file_provider.rs",
        1,
        "the macOS File Provider domain id; the test build does not run on macOS",
    ),
    ("engine_host.rs", 1, "a notification's title (the brand)"),
    ("native_menu.rs", 3, "menu titles (the brand)"),
    ("tray_window.rs", 2, "window titles (the brand)"),
    (
        "finder_setup/policy.rs",
        0,
        "placeholder row: compares against `APP_DOMAIN`, names nothing",
    ),
    (
        "real_app.rs",
        0,
        "the shell names its identity by `identity::REAL`, not by a literal",
    ),
    (
        "crates/beebeeb-engine/src/test_sandbox.rs",
        0,
        "its three literals are in test-gated items, so none is production text",
    ),
];

/// Rule 2's allow-list: `(file, namings, why)`.
const NAMING_ALLOWED: [(&str, usize, &str); 2] = [
    ("real_app.rs", 1, "the Tauri shell is the real app: `APP`"),
    (
        "native/beebeeb-drive-rune/src/shell/identity.rs",
        2,
        "`AppIdentity::engine`: the one bridge from the native identity to the engine's",
    ),
];

/// Rule 3: every production read of an identity accessor in the engine, `(file, accessor, reads)`.
const CONSUMERS: &[(&str, &str, usize)] = &[
    ("app.rs", "refusal_on", 1),
    ("backup/device.rs", "backup_device_name_suffix", 1),
    ("backup/device.rs", "backup_device_salt", 1),
    ("backup/device.rs", "is_real", 1),
    ("config/paths.rs", "config_dir_name", 1),
    ("config/paths.rs", "sync_folder_name", 1),
    ("config/store.rs", "config_dir_name", 1),
    ("engine_bridge/staging_bases.rs", "config_dir_name", 1),
    ("free_up_space.rs", "config_dir_name", 1),
    ("identity.rs", "is_real", 1),
    ("identity.rs", "state_dir_in", 1),
    ("ipc_socket.rs", "daemon_socket_name", 1),
    ("ipc_socket/hydrate.rs", "scratch_dir_in", 1),
    ("ipc_socket/macos_paths.rs", "macos_app_group", 2),
    ("ipc_socket/macos_staging.rs", "macos_app_group", 1),
    ("ipc_socket/thumbnails.rs", "scratch_dir_in", 1),
    ("keychain.rs", "credential_service", 6),
    ("linux_fuse/mod.rs", "config_dir_name", 1),
    ("linux_fuse/mod.rs", "scratch_dir_in", 1),
    ("local_state.rs", "logs_dir_name", 1),
    ("local_state.rs", "scratch_dir_in", 2),
    ("sign_out/kept_names.rs", "bundle_identifier", 1),
    ("sign_out/kept_names.rs", "config_dir_name", 1),
    ("state_paths.rs", "state_dir_in", 1),
    ("windows_cf/registration.rs", "cloud_files_provider_guid", 1),
    ("windows_cf/registration.rs", "cloud_files_provider_name", 2),
    ("windows_cf/status_ui.rs", "status_ui_class_id", 1),
    ("windows_cf/thumbnail_provider.rs", "thumbnail_provider_class_id", 1),
];

const ACCESSORS: [&str; 17] = [
    "bundle_identifier",
    "config_dir_name",
    "state_dir_in",
    "scratch_dir_in",
    "credential_service",
    "sync_folder_name",
    "cloud_files_provider_name",
    "cloud_files_provider_guid",
    "status_ui_class_id",
    "thumbnail_provider_class_id",
    "backup_device_salt",
    "backup_device_name_suffix",
    "daemon_socket_name",
    "macos_app_group",
    "logs_dir_name",
    "refusal_on",
    "is_real",
];

/// A source text split into its code (comments and literal contents removed) and its string literals.
struct Lexed {
    code: String,
    strings: Vec<String>,
}

/// A small Rust lexer: enough to tell code from comments from string literals, so a rule cannot be dodged with an
/// attribute line, a trailing comment or a literal split over lines. Handles line and nested block comments,
/// strings with escapes, raw strings (`r"…"`, `r#"…"#`, `br…`), char literals and lifetimes.
fn lex(text: &str) -> Lexed {
    let chars: Vec<char> = text.chars().collect();
    let (mut code, mut strings) = (String::new(), Vec::new());
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        let next = chars.get(i + 1).copied();
        if c == '/' && next == Some('/') {
            while i < chars.len() && chars[i] != '\n' {
                i += 1;
            }
        } else if c == '/' && next == Some('*') {
            let mut depth = 1;
            i += 2;
            while i < chars.len() && depth > 0 {
                if chars[i] == '/' && chars.get(i + 1) == Some(&'*') {
                    depth += 1;
                    i += 2;
                } else if chars[i] == '*' && chars.get(i + 1) == Some(&'/') {
                    depth -= 1;
                    i += 2;
                } else {
                    i += 1;
                }
            }
        } else if c == 'r'
            && !code
                .chars()
                .next_back()
                .is_some_and(|p| p.is_alphanumeric() || p == '_')
            && raw_string_hashes(&chars, i + 1).is_some()
        {
            let hashes = raw_string_hashes(&chars, i + 1).unwrap_or(0);
            let mut j = i + 1 + hashes + 1;
            let mut literal = String::new();
            while j < chars.len() {
                if chars[j] == '"' && (0..hashes).all(|k| chars.get(j + 1 + k) == Some(&'#')) {
                    break;
                }
                literal.push(chars[j]);
                j += 1;
            }
            strings.push(literal);
            code.push_str("\"\"");
            i = j + 1 + hashes;
        } else if c == '"' {
            let mut j = i + 1;
            let mut literal = String::new();
            while j < chars.len() && chars[j] != '"' {
                if chars[j] == '\\' {
                    j += 1;
                    // A line continuation glues the two halves; any other escape keeps its character.
                    if chars.get(j) == Some(&'\n') {
                        while chars.get(j + 1).is_some_and(|ch| ch.is_whitespace()) {
                            j += 1;
                        }
                    } else if let Some(escaped) = chars.get(j) {
                        literal.push(*escaped);
                    }
                } else {
                    literal.push(chars[j]);
                }
                j += 1;
            }
            strings.push(literal);
            code.push_str("\"\"");
            i = j + 1;
        } else if c == '\'' {
            // A char literal (`'x'`, `'\n'`, `'"'`) or a lifetime (`'a`, `'static`).
            if next == Some('\\') {
                let mut j = i + 2;
                while j < chars.len() && chars[j] != '\'' {
                    j += 1;
                }
                i = j + 1;
            } else if chars.get(i + 2) == Some(&'\'') {
                i += 3;
            } else {
                code.push(c);
                i += 1;
            }
        } else {
            code.push(c);
            i += 1;
        }
    }
    Lexed { code, strings }
}

/// `Some(n)` when `chars[at..]` is `#`×n then `"`: the rest of a raw string's opener.
fn raw_string_hashes(chars: &[char], at: usize) -> Option<usize> {
    let hashes = chars[at.min(chars.len())..].iter().take_while(|c| **c == '#').count();
    (chars.get(at + hashes) == Some(&'"')).then_some(hashes)
}

fn digits_only(text: &str) -> String {
    text.chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .collect::<String>()
        .to_ascii_lowercase()
}

/// Rule 1: how many per-app literals `production` holds.
fn literal_hits(production: &str) -> usize {
    let lexed = lex(production);
    let mut hits = 0;
    for literal in &lexed.strings {
        let lower = literal.to_ascii_lowercase();
        if EXACT_NAMES.contains(&literal.as_str()) || NAME_PARTS.iter().any(|part| lower.contains(part)) {
            hits += 1;
        }
    }
    // A GUID in any spelling: a number in the code (`0xb33b33b0_5217_…`) or a string (`"{B33B33B0-5217-…}"`).
    let digits =
        digits_only(&lexed.code) + "|" + &lexed.strings.iter().map(|s| digits_only(s) + "|").collect::<String>();
    hits += GUID_DIGITS
        .iter()
        .map(|guid| digits.matches(guid).count())
        .sum::<usize>();
    hits
}

/// Rule 2: how often `production` names an identity itself instead of being handed one.
fn naming_hits(production: &str) -> usize {
    let code: String = lex(production).code.chars().filter(|c| !c.is_whitespace()).collect();
    let mut hits = code.matches("identity::REAL").count()
        + code.matches("identity::TEST").count()
        + code.matches("identity::*").count();
    // `use …identity::{EngineIdentity, REAL};` and `…identity::{self, TEST}`.
    let mut rest = code.as_str();
    while let Some(at) = rest.find("identity::{") {
        let list = &rest[at + "identity::{".len()..];
        let end = list.find('}').unwrap_or(list.len());
        hits += list[..end]
            .split(',')
            .filter(|name| matches!(name.split("as").next(), Some("REAL" | "TEST")))
            .count();
        rest = &list[end..];
    }
    hits
}

/// Rule 3: the reads of each accessor in `production`, as `.name(` on any receiver.
fn accessor_reads(production: &str) -> BTreeMap<&'static str, usize> {
    let code: String = lex(production).code.chars().filter(|c| !c.is_whitespace()).collect();
    ACCESSORS
        .iter()
        .map(|name| (*name, code.matches(&format!(".{name}(")).count()))
        .filter(|(_, reads)| *reads > 0)
        .collect()
}

fn production_text(path: &Path) -> String {
    let text = std::fs::read_to_string(path)
        .unwrap_or_else(|e| panic!("{}: {e}", path.display()))
        .replace("\r\n", "\n");
    without_items(&text, &TEST_ITEM_GATES)
}

fn native_files() -> Vec<(String, PathBuf)> {
    fn walk(dir: &Path, root: &Path, out: &mut Vec<(String, PathBuf)>) {
        for entry in std::fs::read_dir(dir).unwrap_or_else(|e| panic!("read {}: {e}", dir.display())) {
            let path = entry.expect("dir entry").path();
            if path.is_dir() {
                walk(&path, root, out);
            } else if path.extension().is_some_and(|ext| ext == "rs") {
                let rel = path.strip_prefix(root).unwrap().to_string_lossy().replace('\\', "/");
                out.push((format!("{NATIVE_PREFIX}{rel}"), path));
            }
        }
    }
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../native/beebeeb-drive-rune");
    let mut files = Vec::new();
    walk(&root.join("src"), &root, &mut files);
    files.push((format!("{NATIVE_PREFIX}build.rs"), root.join("build.rs")));
    assert!(files.len() > 20, "the native tree is read: {} files", files.len());
    files
}

/// Every production file of the three trees as `(name, production text)`.
fn production_files() -> Vec<(String, String)> {
    let mut all = rust_files();
    all.extend(native_files());
    let files: Vec<(String, String)> = all
        .into_iter()
        .filter(|(name, _)| !is_test_file(name))
        .map(|(name, path)| (name, production_text(&path)))
        .collect();
    for tree in ["crates/beebeeb-engine/src/", NATIVE_PREFIX] {
        assert!(
            files.iter().filter(|(name, _)| name.starts_with(tree)).count() > 20,
            "{tree}"
        );
    }
    files
}

fn found(files: &[(String, String)], rule: fn(&str) -> usize) -> BTreeMap<String, usize> {
    files
        .iter()
        .map(|(name, text)| (name.clone(), rule(text)))
        .filter(|(_, hits)| *hits > 0)
        .collect()
}

fn allowed(list: &[(&str, usize, &str)]) -> BTreeMap<String, usize> {
    list.iter()
        .filter(|(_, hits, _)| *hits > 0)
        .map(|(name, hits, _)| (name.to_string(), *hits))
        .collect()
}

#[test]
fn a_per_app_literal_stands_only_in_an_identity_file() {
    let files = production_files();
    assert_eq!(
        found(&files, literal_hits),
        allowed(&LITERALS_ALLOWED),
        "a per-app name (identifier, folder, provider name, socket, GUID) outside the files that may hold one, or a \
         changed count inside one: take the name from the identity the code is handed (left: found, right: allowed)"
    );
}

#[test]
fn only_a_host_names_an_identity() {
    let files = production_files();
    assert_eq!(
        found(&files, naming_hits),
        allowed(&NAMING_ALLOWED),
        "`identity::REAL` / `identity::TEST` outside the files that may name one: engine code is handed its \
         identity (left: found, right: allowed)"
    );
}

/// No field of the identity is public and the type has no constructor: REAL and TEST are the only values there
/// can be, so "exactly two" is the compiler's rule, not a convention.
#[test]
fn the_identity_type_cannot_be_built_outside_its_file() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/identity.rs");
    let text = production_text(&path);
    let body = crate::source_pin::body_between(&text, "pub struct EngineIdentity {", "\n}");
    let fields: Vec<&str> = body
        .lines()
        .skip(1)
        .map(str::trim)
        .filter(|line| line.contains(':'))
        .collect();
    assert_eq!(fields.len(), 14, "{fields:?}");
    for field in fields {
        assert!(
            !field.starts_with("pub"),
            "a public field lets any crate build a third identity: {field}"
        );
    }
    let code: String = lex(&text).code.chars().filter(|c| !c.is_whitespace()).collect();
    assert_eq!(
        code.matches("EngineIdentity=EngineIdentity{").count(),
        2,
        "REAL and TEST, nothing else"
    );
    assert_eq!(
        code.matches("->Self").count() + code.matches("->EngineIdentity").count(),
        0,
        "no constructor"
    );
    assert!(
        !code.contains("#[derive(Clone") && !code.contains("Default"),
        "no way to copy or default one"
    );
}

#[test]
fn every_engine_consumer_reads_its_name_from_the_identity() {
    let mut found: Vec<(String, &'static str, usize)> = Vec::new();
    for (name, text) in production_files() {
        let Some(rel) = name.strip_prefix(crate::source_census::ENGINE_PREFIX) else {
            continue;
        };
        for (accessor, reads) in accessor_reads(&text) {
            found.push((rel.to_string(), accessor, reads));
        }
    }
    found.sort();
    let expected: Vec<(String, &'static str, usize)> = CONSUMERS
        .iter()
        .map(|(file, accessor, reads)| (file.to_string(), *accessor, *reads))
        .collect();
    assert_eq!(
        found, expected,
        "the engine's reads of an identity accessor changed (left: found, right: the census): a consumer was put \
         back on a literal, or a new one was added and belongs in `CONSUMERS`"
    );
    // Every accessor has a consumer: a name the identity owns and nothing reads is a name something else spells.
    for accessor in ACCESSORS {
        assert!(
            expected.iter().any(|(_, name, _)| *name == accessor),
            "{accessor} has no consumer"
        );
    }
}

#[test]
fn the_lexer_tells_code_from_comments_from_literals() {
    let lexed = lex(
        "let a = \"one\"; // \"not this\"\n/* \"nor /* this */ one\" */ let b = r#\"two \"2\"\"#;\nlet c = '\"'; let d: &'static str = \"th\\\n    ree\";\n#[allow(dead_code)] const X: &str = \"four\";",
    );
    assert_eq!(lexed.strings, ["one", "two \"2\"", "three", "four"]);
    assert!(lexed.code.contains("#[allow(dead_code)]") && !lexed.code.contains("not this"));
    assert!(lexed.code.contains("&'static str"));
}

/// The ten ways around the previous pin that the review of PR 307 planted (8 it missed, 2 it caught), as text:
/// each is seen by a rule. Mutants 2, 7 and 10 are also run against the real files (the census must move).
#[test]
fn the_pin_goes_red_on_each_known_way_around_it() {
    let literal = |text: &str| literal_hits(&without_items(text, &TEST_ITEM_GATES));
    let naming = |text: &str| naming_hits(&without_items(text, &TEST_ITEM_GATES));

    // 1. The Windows target name spelled out.
    assert_eq!(literal("let combined = format!(\"io.beebeeb.app/{account}\");"), 1);
    // 2. The free-up-space cache root back on its literal (a line holding `beebeeb_` used to be skipped).
    assert_eq!(literal("let beebeeb_cache = cache_dir.join(\"beebeeb\");"), 1);
    // 3. The identifier glued with `concat!`.
    assert_eq!(literal("const S: &str = concat!(\"io.beebeeb\", \".app\");"), 1);
    // 4. Behind an attribute (a line starting with `#` used to be skipped).
    assert_eq!(literal("#[allow(dead_code)] const X: &str = \"io.beebeeb.app\";"), 1);
    assert_eq!(literal("#[allow(dead_code)]\nconst X: &str = \"io.beebeeb.app\";"), 1);
    // 5. A new file holding the provider's name and GUID: the allow-list is by whole path, and both are seen.
    let provider =
        "pub const NAME: &str = \"Beebeeb\";\npub const GUID: u128 = 0xb33b_33b0_5217_4c0a_9e3f_1f2a_7c4d_8e90;";
    assert_eq!(literal(provider), 2);
    assert!(
        !LITERALS_ALLOWED
            .iter()
            .any(|(file, _, _)| *file == "crates/beebeeb-engine/src/windows_cf/provider_identity.rs")
    );
    assert_eq!(literal("let clsid = \"{C0B33B33-5217-4C0A-9E3F-2F3B8D5E9FA1}\";"), 1);
    // 6. The kept root back on a bundle-id constant.
    assert_eq!(literal("pub const APP_BUNDLE_ID: &str = \"io.beebeeb.app\";"), 1);
    // 8. The real identity read directly instead of the one handed in, however it is imported.
    assert_eq!(
        naming("let dir = base.join(crate::identity::REAL.config_dir_name());"),
        1
    );
    assert_eq!(naming("use crate::identity::{EngineIdentity, REAL};"), 1);
    assert_eq!(naming("use beebeeb_engine::identity::{self, TEST as T};"), 1);
    assert_eq!(naming("use crate::identity::*;"), 1);
    // 9. A plain constant in keychain.rs.
    assert_eq!(literal("const X: &str = \"io.beebeeb.app\";"), 1);
    // The socket, the app group, the sync folder, the test build's names.
    assert_eq!(
        literal("p.join(\"beebeeb-daemon.sock\"); q.join(\"R8352WDJJR.x\"); h.join(\"Beebeeb Test\");"),
        3
    );
    // Not a per-app name: a sentence, a link, a header, a test-gated item, a comment.
    assert_eq!(
        literal(
            "let a = \"Restart Beebeeb.\"; let b = \"beebeeb://open\"; let c = \"X-Beebeeb-Client\"; // \"beebeeb\"\n\
             #[cfg(test)]\nconst T: &str = \"io.beebeeb.app\";\n"
        ),
        0
    );

    // 7 and 10 (and 2 again): a consumer rebuilt from pieces, or put back on a literal, loses its read. Rule 1
    // cannot see `\"bee\"` + `\"beeb\"`; the census does.
    let reads_after = |file: &str, from: &str, to: &str| {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("src").join(file);
        let text = production_text(&path);
        assert_eq!(text.matches(from).count(), 1, "{file} holds `{from}` once");
        (accessor_reads(&text), accessor_reads(&text.replace(from, to)))
    };
    for (file, from, to) in [
        (
            "config/store.rs",
            "base.join(identity.config_dir_name())",
            "base.join([\"bee\", \"beeb\"].concat())",
        ),
        (
            "engine_bridge/staging_bases.rs",
            "folder: identity.config_dir_name(),\n            data: dirs::data_dir(),",
            "folder: \"beebeeb\",\n            data: dirs::data_dir(),",
        ),
        (
            "free_up_space.rs",
            "cache_dir.join(identity.config_dir_name())",
            "cache_dir.join(\"beebeeb\")",
        ),
    ] {
        let (before, after) = reads_after(file, from, to);
        assert_ne!(before, after, "{file}: the census does not see `{to}`");
    }
}
