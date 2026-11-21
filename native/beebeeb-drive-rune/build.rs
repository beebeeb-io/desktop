//! Stamps the build's commit for the `feedback` feature's notes (`note.json` → `app.commit`).
//! `BEEBEEB_BUILD_COMMIT` set by the packaging wins; otherwise `git rev-parse HEAD`, with
//! `-dirty` when the tree has changes. Without git the notes say "unknown".

use std::process::Command;

mod build_version;

fn git(args: &[&str]) -> Option<String> {
    let out = Command::new("git").args(args).output().ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).trim().to_string())
}

/// Which app this binary is, decided here and nowhere else (`src/shell/identity.rs` reads the result).
///
/// `BEEBEEB_APP_IDENTIFIER` must be one of the two identifiers; any other value stops the build. Unset, the build
/// is the TEST app: a build nobody named must never open the real app's credentials, settings or sync root. A
/// release (`BEEBEEB_RELEASE_VERSION` set) must name the real identifier itself, and is refused with the test one.
/// Nothing here looks at the profile (`debug`/`release`): `cargo build --release` of a test build stays a test
/// build.
fn emit_app_identifier() {
    const REAL: &str = "io.beebeeb.app";
    const TEST: &str = "io.beebeeb.app.test";
    println!("cargo:rerun-if-env-changed=BEEBEEB_APP_IDENTIFIER");
    println!("cargo:rerun-if-env-changed=BEEBEEB_RELEASE_VERSION");
    let named = std::env::var("BEEBEEB_APP_IDENTIFIER").ok().filter(|v| !v.is_empty());
    let release = std::env::var("BEEBEEB_RELEASE_VERSION").is_ok_and(|v| !v.is_empty());
    let identifier = match (named.as_deref(), release) {
        (Some(REAL), _) => REAL,
        (Some(TEST), false) => TEST,
        (Some(TEST), true) => panic!(
            "BEEBEEB_RELEASE_VERSION is set with BEEBEEB_APP_IDENTIFIER={TEST}: a release is the real app ({REAL})"
        ),
        (Some(other), _) => panic!("BEEBEEB_APP_IDENTIFIER={other:?} is not an app: use {REAL} or {TEST}"),
        (None, true) => panic!(
            "BEEBEEB_RELEASE_VERSION is set without BEEBEEB_APP_IDENTIFIER: a release names its identity ({REAL})"
        ),
        (None, false) => TEST,
    };
    println!("cargo:rustc-env=BEEBEEB_APP_IDENTIFIER={identifier}");
    // For the test that pins the default (`an_unnamed_build_is_the_test_build`).
    let source = if named.is_some() { "named" } else { "default" };
    println!("cargo:rustc-env=BEEBEEB_APP_IDENTIFIER_SOURCE={source}");
    // What a packager and the release workflow read out of the built binary (`identity.rs`, `BUILD_MARK`): the
    // identifier alone cannot be searched for, because both apps' names are in every binary (the engine names both).
    println!("cargo:rustc-env=BEEBEEB_APP_IDENTITY_MARK=beebeeb-app-identity={identifier};");
}

/// The revision of core the engine links, read from the engine's own manifest (the one place it is pinned), so
/// About says what this build contains and cannot drift from it.
fn emit_core_rev() {
    let manifest = "../../crates/beebeeb-engine/Cargo.toml";
    println!("cargo:rerun-if-changed={manifest}");
    let Ok(text) = std::fs::read_to_string(manifest) else {
        return;
    };
    let rev = text
        .lines()
        .find(|l| l.trim_start().starts_with("beebeeb-core"))
        .and_then(|l| l.split("rev = \"").nth(1))
        .and_then(|r| r.split('"').next());
    if let Some(rev) = rev.filter(|r| r.len() >= 7 && r.bytes().all(|b| b.is_ascii_hexdigit())) {
        println!("cargo:rustc-env=BEEBEEB_CORE_REV={rev}");
    }
}

/// Windows targets only: the exe's version resource and icon (what Explorer, the uninstall entry and the installer's
/// release check read). `BEEBEEB_RELEASE_VERSION` when set, else the crate version (a test build). The resource
/// compiler is `embed-resource`, a build dependency of a Windows host (the release and CI builds are Windows hosts).
fn emit_windows_resource() {
    println!("cargo:rerun-if-changed=build_version.rs");
    println!("cargo:rerun-if-changed=../../packaging/windows/icon.ico");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    embed_windows_resource();
}

#[cfg(windows)]
fn embed_windows_resource() {
    let release = std::env::var("BEEBEEB_RELEASE_VERSION").ok();
    let crate_version = std::env::var("CARGO_PKG_VERSION").expect("cargo sets CARGO_PKG_VERSION");
    let version = build_version::version_string(release.as_deref(), &crate_version);
    let icon = std::fs::canonicalize("../../packaging/windows/icon.ico")
        .expect("packaging/windows/icon.ico is missing: the exe's icon");
    let icon = icon.to_string_lossy().trim_start_matches(r"\\?\").to_string();
    let rc = build_version::render_rc(&version, "beebeeb-drive.exe", &icon);
    let out = std::path::PathBuf::from(std::env::var_os("OUT_DIR").expect("cargo sets OUT_DIR")).join("beebeeb.rc");
    std::fs::write(&out, rc).expect("writing the version resource source");
    embed_resource::compile(&out, embed_resource::NONE)
        .manifest_required()
        .expect("embedding the version resource");
}

#[cfg(not(windows))]
fn embed_windows_resource() {
    println!(
        "cargo:warning=building for Windows from a non-Windows host: the exe carries no version resource, so a release installer would refuse it"
    );
}

/// The source offer's two addresses, derived here so no source link is typed by hand: the commit of desktop-office
/// that `../beebeeb-office-render/Cargo.lock` pins (`BEEBEEB_OFFICE_REV`) and the version the desktop release tag
/// carries (`BEEBEEB_SOURCE_VERSION`: the release version, else the crate version).
fn emit_source_offer() {
    let lock = "../beebeeb-office-render/Cargo.lock";
    println!("cargo:rerun-if-changed={lock}");
    println!("cargo:rerun-if-env-changed=BEEBEEB_RELEASE_VERSION");
    let text = std::fs::read_to_string(lock).expect("reading the office-render lock");
    let marker = "desktop-office?rev=";
    let at = text.find(marker).expect("the office-render lock has a desktop-office git source") + marker.len();
    let rev: String = text[at..].chars().take_while(char::is_ascii_hexdigit).collect();
    assert!(rev.len() == 40, "the desktop-office rev in the lock is not a full commit: {rev:?}");
    println!("cargo:rustc-env=BEEBEEB_OFFICE_REV={rev}");
    let release = std::env::var("BEEBEEB_RELEASE_VERSION").ok().filter(|v| !v.trim().is_empty());
    let version = release.unwrap_or_else(|| std::env::var("CARGO_PKG_VERSION").expect("cargo sets CARGO_PKG_VERSION"));
    println!("cargo:rustc-env=BEEBEEB_SOURCE_VERSION={}", version.trim());
}

fn main() {
    emit_source_offer();
    emit_app_identifier();
    emit_windows_resource();
    emit_core_rev();
    println!("cargo:rerun-if-env-changed=BEEBEEB_BUILD_COMMIT");
    println!("cargo:rerun-if-env-changed=BEEBEEB_BUILD_KIND");
    if let Some(log) = git(&["rev-parse", "--git-path", "logs/HEAD"]) {
        println!("cargo:rerun-if-changed={log}");
    }
    if std::env::var_os("BEEBEEB_BUILD_COMMIT").is_some() {
        return;
    }
    if let Some(sha) = git(&["rev-parse", "HEAD"]).filter(|s| !s.is_empty()) {
        let dirty = git(&["status", "--porcelain", "--untracked-files=no"]).is_some_and(|s| !s.is_empty());
        println!(
            "cargo:rustc-env=BEEBEEB_BUILD_COMMIT={sha}{}",
            if dirty { "-dirty" } else { "" }
        );
    }
}
