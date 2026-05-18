fn main() {
    let target = std::env::var("TARGET").unwrap_or_default();
    if target.ends_with("apple-darwin") {
        build_macos_file_provider_bridge(&target);
    }
    tauri_build::build()
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
    println!("cargo:rustc-link-lib=framework=FileProvider");
}
