fn main() {
    let target = std::env::var("TARGET").unwrap_or_default();
    if target.ends_with("apple-darwin") {
        build_macos_file_provider_bridge(&target);
    }
    emit_app_identifier();
    tauri_build::build()
}

/// The bundle identifier this binary is built with, for `src/app_identity.rs` (`BEEBEEB_APP_IDENTIFIER`).
///
/// It is the SAME value Tauri bundles with: `tauri.conf.json`'s `identifier`, overridden by the merged `--config`
/// the Tauri CLI hands every build script in `TAURI_CONFIG` (how a test build sets `io.beebeeb.app.test`). An
/// explicit `BEEBEEB_APP_IDENTIFIER` in the environment wins (the native app's build sets it; it has no Tauri
/// config). A config that cannot be read stops the build: guessing here would let a test build act as the real app.
fn emit_app_identifier() {
    println!("cargo:rerun-if-env-changed=TAURI_CONFIG");
    println!("cargo:rerun-if-env-changed=BEEBEEB_APP_IDENTIFIER");
    println!("cargo:rerun-if-changed=tauri.conf.json");

    fn identifier_of(json: &str, what: &str) -> Option<String> {
        let value: serde_json::Value = serde_json::from_str(json).unwrap_or_else(|e| panic!("{what} is not JSON: {e}"));
        value.get("identifier").and_then(|id| id.as_str()).map(str::to_owned)
    }

    let base = std::fs::read_to_string("tauri.conf.json").expect("read tauri.conf.json");
    let mut identifier = identifier_of(&base, "tauri.conf.json").expect("tauri.conf.json has an identifier");
    if let Ok(merged) = std::env::var("TAURI_CONFIG") {
        if let Some(overridden) = identifier_of(&merged, "TAURI_CONFIG") {
            identifier = overridden;
        }
    }
    if let Ok(explicit) = std::env::var("BEEBEEB_APP_IDENTIFIER") {
        if !explicit.trim().is_empty() {
            identifier = explicit.trim().to_owned();
        }
    }
    println!("cargo:rustc-env=BEEBEEB_APP_IDENTIFIER={identifier}");
}

fn build_macos_file_provider_bridge(target: &str) {
    use std::path::PathBuf;
    use std::process::Command;

    let out_dir = PathBuf::from(std::env::var("OUT_DIR").expect("OUT_DIR"));
    let object = out_dir.join("FileProviderBridge.o");
    let archive = out_dir.join("libbeebeeb_file_provider_bridge.a");
    let clang_target = match target {
        "aarch64-apple-darwin" => "arm64-apple-macos14.0",
        "x86_64-apple-darwin" | "x86_64h-apple-darwin" => "x86_64-apple-macos14.0",
        _ => "arm64-apple-macos14.0",
    };

    let status = Command::new("xcrun")
        .args([
            "clang",
            "-target",
            clang_target,
            "-fobjc-arc",
            "-fmodules",
            "-mmacosx-version-min=14.0",
            "-c",
            "macos/FileProviderBridge.m",
            "-o",
        ])
        .arg(&object)
        .status()
        .expect("run clang for FileProviderBridge.m");
    assert!(status.success(), "clang failed for FileProviderBridge.m");

    let status = Command::new("ar")
        .arg("crs")
        .arg(&archive)
        .arg(&object)
        .status()
        .expect("archive FileProviderBridge.o");
    assert!(status.success(), "ar failed for FileProviderBridge.o");

    println!("cargo:rerun-if-changed=macos/FileProviderBridge.m");
    println!("cargo:rustc-link-search=native={}", out_dir.display());
    println!("cargo:rustc-link-lib=static=beebeeb_file_provider_bridge");
    println!("cargo:rustc-link-lib=framework=Foundation");
    println!("cargo:rustc-link-lib=framework=AppKit");
    println!("cargo:rustc-link-lib=framework=FileProvider");
}
