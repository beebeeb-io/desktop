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

fn main() {
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
