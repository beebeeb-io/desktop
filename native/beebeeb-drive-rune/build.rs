//! Stamps the build's commit for the `feedback` feature's notes (`note.json` → `app.commit`).
//! `BEEBEEB_BUILD_COMMIT` set by the packaging wins; otherwise `git rev-parse HEAD`, with
//! `-dirty` when the tree has changes. Without git the notes say "unknown".

use std::process::Command;

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

fn main() {
    emit_app_identifier();
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
