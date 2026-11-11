//! Pins for `app_identity`: the real app's names never change, a test build shares none of them, and no per-app
//! literal lives outside the module.

use super::*;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// Every per-app name of one identity, by what it names.
fn names(identity: AppIdentity) -> Vec<(&'static str, String)> {
    vec![
        ("bundle identifier", identity.bundle_identifier().to_string()),
        ("product name / autostart entry", identity.product_name().to_string()),
        ("config, data and cache dir", identity.data_dir_name().to_string()),
        ("suggested sync folder", identity.sync_folder_name().to_string()),
        ("credential service", identity.credential_service().to_string()),
        (
            "Cloud Files provider name",
            identity.cloud_files_provider_name().to_string(),
        ),
        (
            "Cloud Files provider GUID",
            format!("{:032x}", identity.cloud_files_provider_guid()),
        ),
        (
            "status column class id",
            format!("{:032x}", identity.status_ui_class_id()),
        ),
        (
            "thumbnail class id",
            format!("{:032x}", identity.thumbnail_provider_class_id()),
        ),
        ("daemon socket", identity.daemon_socket_name().to_string()),
        ("sign-in log", identity.signin_log_name().to_string()),
        ("logs dir", identity.logs_dir_name().to_string()),
        ("checks for updates", identity.checks_for_updates().to_string()),
    ]
}

/// An existing install finds its config, credentials and sync root by these exact bytes.
#[test]
fn the_real_identity_keeps_every_shipped_value() {
    let expected: Vec<(&str, String)> = [
        ("bundle identifier", "io.beebeeb.app"),
        ("product name / autostart entry", "Beebeeb"),
        ("config, data and cache dir", "beebeeb"),
        ("suggested sync folder", "Beebeeb"),
        ("credential service", "io.beebeeb.app"),
        ("Cloud Files provider name", "Beebeeb"),
        ("Cloud Files provider GUID", "b33b33b052174c0a9e3f1f2a7c4d8e90"),
        ("status column class id", "c0b33b3352174c0a9e3f2f3b8d5e9fa1"),
        ("thumbnail class id", "b2e0a7d49c614f3a8d2e7a1c5e9b0f44"),
        ("daemon socket", "beebeeb-daemon.sock"),
        ("sign-in log", "beebeeb-signin.log"),
        ("logs dir", "Beebeeb"),
        ("checks for updates", "true"),
    ]
    .into_iter()
    .map(|(what, value)| (what, value.to_string()))
    .collect();
    assert_eq!(names(AppIdentity::Real), expected);
}

#[test]
fn a_test_build_shares_no_name_with_the_real_app() {
    let real = names(AppIdentity::Real);
    let test = names(AppIdentity::Test);
    assert_eq!(real.len(), 13);
    assert_eq!(real.len(), test.len());
    let mut differing = 0;
    for ((what, real_value), (_, test_value)) in real.iter().zip(&test) {
        assert_ne!(
            real_value, test_value,
            "{what} is shared by the real app and a test build"
        );
        differing += 1;
    }
    assert_eq!(differing, 13);
}

/// `<service>/<account>/<leaf>` and the legacy flat `<service>/<leaf>`: neither identity's names can fall inside
/// the other's namespace, so a legacy read, migration or sweep by a test build never reaches a real entry.
#[test]
fn credential_namespaces_never_overlap() {
    let real = format!("{}/", AppIdentity::Real.credential_service());
    let test = format!("{}/", AppIdentity::Test.credential_service());
    assert!(!real.starts_with(&test) && !test.starts_with(&real));
    for leaf in [
        "session-token",
        "wrapped-master-key",
        "account-email",
        "vault-key-owner",
        "session-owner",
    ] {
        for account in ["", "acct-uuid-0001/"] {
            let real_name = format!("{real}{account}{leaf}");
            let test_name = format!("{test}{account}{leaf}");
            assert_eq!(real_name, format!("io.beebeeb.app/{account}{leaf}"));
            assert_eq!(test_name, format!("io.beebeeb.app.test/{account}{leaf}"));
            assert!(!real_name.starts_with(&test) && !test_name.starts_with(&real));
        }
    }
}

#[test]
fn only_the_exact_real_identifier_is_the_real_app() {
    assert_eq!(AppIdentity::from_bundle_identifier("io.beebeeb.app"), AppIdentity::Real);
    for other in [
        "io.beebeeb.app.test",
        "",
        "io.beebeeb.app ",
        "IO.BEEBEEB.APP",
        "io.beebeeb.ap",
        "com.example",
    ] {
        assert_eq!(
            AppIdentity::from_bundle_identifier(other),
            AppIdentity::Test,
            "{other:?}"
        );
    }
    for identity in AppIdentity::BOTH {
        assert_eq!(
            AppIdentity::from_bundle_identifier(identity.bundle_identifier()),
            identity
        );
    }
}

/// `cargo test` builds with `tauri.conf.json` as it is committed: the real app.
#[test]
fn this_build_is_the_identity_its_tauri_config_names() {
    let config: serde_json::Value =
        serde_json::from_str(include_str!("../tauri.conf.json")).expect("tauri.conf.json is JSON");
    assert_eq!(config["identifier"], "io.beebeeb.app");
    assert_eq!(config["productName"], "Beebeeb");
    assert_eq!(BUILT_BUNDLE_IDENTIFIER, "io.beebeeb.app");
    assert_eq!(AppIdentity::CURRENT, AppIdentity::Real);
}

#[test]
fn a_test_build_never_updates_and_the_real_app_does() {
    assert_eq!(AppIdentity::Real.updater_refusal(), Ok(()));
    assert_eq!(
        AppIdentity::Test.updater_refusal(),
        Err("This is a test build. It does not update itself.".to_string())
    );
}

#[test]
fn a_bundle_that_is_not_the_one_compiled_for_does_not_start() {
    assert_eq!(AppIdentity::Real.check_bundle("io.beebeeb.app", "Beebeeb"), Ok(()));
    assert_eq!(
        AppIdentity::Test.check_bundle("io.beebeeb.app.test", "Beebeeb Test"),
        Ok(())
    );
    // Real names compiled in, test bundle around them (and the reverse): refuse.
    assert!(
        AppIdentity::Real
            .check_bundle("io.beebeeb.app.test", "Beebeeb Test")
            .is_err()
    );
    assert!(AppIdentity::Test.check_bundle("io.beebeeb.app", "Beebeeb").is_err());
    // A test identifier under the real product name would share the autostart entry and the installer's app name.
    assert!(
        AppIdentity::Test
            .check_bundle("io.beebeeb.app.test", "Beebeeb")
            .is_err()
    );
}

// ── Source census ───────────────────────────────────────────────────────────────────────────────────────────

/// A string literal that names something one install owns (or looks like it).
fn is_per_app_literal(literal: &str) -> bool {
    const EXACT: [&str; 6] = [
        "beebeeb",
        "Beebeeb",
        "Beebeeb Test",
        "beebeeb-test",
        "BeebeebTest",
        "Beebeeb Desktop",
    ];
    const PREFIX: [&str; 3] = ["io.beebeeb.", "beebeeb-", ".beebeeb"];
    EXACT.contains(&literal)
        || PREFIX.iter().any(|prefix| literal.starts_with(prefix))
        || literal.contains("io.beebeeb.app")
}

/// The string literals of one line of Rust (escapes are not unfolded; a per-app name has none).
fn literals(line: &str) -> Vec<&str> {
    line.split('"').skip(1).step_by(2).collect()
}

fn rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(dir).expect("read src dir") {
        let path = entry.expect("dir entry").path();
        if path.is_dir() {
            rust_files(&path, out);
        } else if path.extension().is_some_and(|ext| ext == "rs") {
            out.push(path);
        }
    }
}

/// Per-app literals in non-test code outside `app_identity.rs`: `(file, literal) -> count`.
fn census(src: &Path) -> BTreeMap<(String, String), usize> {
    let mut files = Vec::new();
    rust_files(src, &mut files);
    let mut found = BTreeMap::new();
    for file in files {
        let relative = file
            .strip_prefix(src)
            .expect("under src")
            .to_string_lossy()
            .replace('\\', "/");
        if relative.contains("test") || relative == "app_identity.rs" {
            continue;
        }
        let text = std::fs::read_to_string(&file).expect("read source");
        let mut previous = "";
        for line in text.lines() {
            let trimmed = line.trim_start();
            // A file's unit tests are its tail: `#[cfg(test)]` then `mod …`.
            if previous == "#[cfg(test)]" && trimmed.starts_with("mod ") && trimmed.ends_with('{') {
                break;
            }
            previous = trimmed;
            if trimmed.starts_with("//") {
                continue;
            }
            for literal in literals(line) {
                if is_per_app_literal(literal) {
                    *found.entry((relative.clone(), literal.to_string())).or_insert(0) += 1;
                }
            }
        }
    }
    found
}

/// Every per-app literal that may stay outside `app_identity.rs`, with the reason. A new one fails the census:
/// derive it from `AppIdentity`, or add it here with why both identities may share it.
const SHARED: &[(&str, &str, usize, &str)] = &[
    // The brand name in words a person reads (window titles, menus, a notification's title).
    (
        "backup_protect.rs",
        "Beebeeb",
        1,
        "notification title: the brand in person-facing words",
    ),
    (
        "native_menu.rs",
        "Beebeeb",
        3,
        "menu titles: the brand in person-facing words",
    ),
    (
        "tray_window.rs",
        "Beebeeb",
        2,
        "window titles: the brand in person-facing words",
    ),
    // Names INSIDE a location that is already per identity (the sync folder, the state dir, a chosen folder).
    (
        "engine_bridge/fs.rs",
        ".beebeeb",
        1,
        "legacy state dir name inside the sync folder",
    ),
    (
        "engine_bridge/fs.rs",
        ".beebeeb-sync.lock",
        1,
        "lock file name inside the sync folder",
    ),
    (
        "engine_bridge/fs.rs",
        ".beebeeb-write-test-{}",
        1,
        "probe file name inside the sync folder",
    ),
    (
        "state_paths.rs",
        ".beebeeb",
        1,
        "legacy state dir name inside the sync folder",
    ),
    (
        "state_paths.rs",
        ".beebeeb-sync.lock",
        1,
        "lock file name inside the per-identity state dir",
    ),
    (
        "linux_thumbnail.rs",
        ".beebeeb-{}-{hash}.tmp",
        1,
        "temp file name next to the thumbnail it becomes",
    ),
    (
        "settings_commands.rs",
        "beebeeb-diagnostics-{}.json",
        1,
        "export file name in the folder the person picks",
    ),
    (
        "settings_commands.rs",
        "beebeeb-diagnostics.json",
        1,
        "export file name in the folder the person picks",
    ),
    // What the app tells the server or a file it is (the client's name, not an install's).
    (
        "linux_thumbnail.rs",
        "Beebeeb Desktop",
        1,
        "PNG Software tag: the client's name",
    ),
    (
        "runner/telemetry.rs",
        "Beebeeb Desktop",
        1,
        "telemetry client name sent to the API",
    ),
    // Thread names: per process, never registered with the OS.
    (
        "windows_cf/mod.rs",
        "beebeeb-thumbnail-provider",
        1,
        "thread name inside this process",
    ),
    (
        "windows_cf/mod.rs",
        "beebeeb-status-ui",
        1,
        "thread name inside this process",
    ),
    // macOS: bound to the signed bundle and its provisioning profile. No macOS test build exists
    // (`testbuild.ps1` is Windows); one would need its own profile, app group and extension first.
    (
        "finder_setup/error.rs",
        "io.beebeeb.app",
        1,
        "NSError domain the ObjC bridge reports",
    ),
    (
        "finder_setup/error.rs",
        "io.beebeeb.bridge",
        1,
        "NSError domain the ObjC bridge reports",
    ),
    (
        "ipc_socket.rs",
        "R8352WDJJR.io.beebeeb.app.fileprovider",
        1,
        "macOS app group: fixed by the profile",
    ),
    (
        "macos_file_provider.rs",
        "io.beebeeb.app.domain",
        1,
        "File Provider domain id, also in the Swift extension",
    ),
    // Linux FUSE prototype (unmounted): the filesystem's name in the mount table.
    (
        "linux_fuse/mod.rs",
        "beebeeb",
        1,
        "FUSE FSName of the unmounted prototype",
    ),
];

#[test]
fn source_census_no_per_app_literal_outside_the_identity_module() {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let found = census(&src);
    let allowed: BTreeMap<(String, String), usize> = SHARED
        .iter()
        .map(|(file, literal, count, _why)| ((file.to_string(), literal.to_string()), *count))
        .collect();
    assert_eq!(allowed.len(), SHARED.len(), "a (file, literal) pair is listed twice");
    let mut problems = Vec::new();
    for (key, count) in &found {
        match allowed.get(key) {
            Some(expected) if expected == count => {}
            Some(expected) => problems.push(format!("{} {:?}: {count} found, {expected} listed", key.0, key.1)),
            None => problems.push(format!("{} {:?}: {count} found, not listed", key.0, key.1)),
        }
    }
    for key in allowed.keys() {
        if !found.contains_key(key) {
            problems.push(format!("{} {:?}: listed, but gone from the source", key.0, key.1));
        }
    }
    assert!(
        problems.is_empty(),
        "per-app literals outside app_identity.rs ({} problems):\n{}",
        problems.len(),
        problems.join("\n")
    );
    // The scan itself must be seeing the tree: the listed literals are all found.
    assert_eq!(found.len(), SHARED.len());
    assert!(
        found.len() >= 10,
        "the census found only {} literals: is it scanning src?",
        found.len()
    );
}

/// The census rule catches each kind of per-app literal, and skips comments and a file's test tail.
#[test]
fn source_census_sees_a_planted_literal() {
    let dir = tempfile::tempdir().expect("temp dir");
    std::fs::write(
        dir.path().join("planted.rs"),
        "// a comment may say \"beebeeb\"\nconst A: &str = \"beebeeb\";\nlet b = x.join(\"Beebeeb\").join(\"io.beebeeb.app/session-token\");\nlet c = \"Open Beebeeb\";\n#[cfg(test)]\nmod tests {\n    const D: &str = \"beebeeb\";\n}\n",
    )
    .expect("write");
    std::fs::write(dir.path().join("some_tests.rs"), "const E: &str = \"beebeeb\";\n").expect("write");
    let found = census(dir.path());
    let keys: Vec<(String, usize)> = found
        .iter()
        .map(|((_, literal), count)| (literal.clone(), *count))
        .collect();
    assert_eq!(
        keys,
        vec![
            ("Beebeeb".to_string(), 1),
            ("beebeeb".to_string(), 1),
            ("io.beebeeb.app/session-token".to_string(), 1)
        ]
    );
}
