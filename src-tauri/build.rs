fn main() {
    let target = std::env::var("TARGET").unwrap_or_default();
    if target.ends_with("apple-darwin") {
        build_macos_file_provider_bridge(&target);
        // spike(2176), never merged: only with `--features spike-2176`. Without the feature this
        // branch emits nothing, so the build script's output is the same as on main.
        if std::env::var_os("CARGO_FEATURE_SPIKE_2176").is_some() {
            build_spike_2176_swap_probe_client(&target);
        }
    }
    tauri_build::build()
}

/// spike(2176): compile the XPC swap-probe client (Objective-C, a load-time constructor) into a
/// static library and force the linker to keep it: no Rust code references it, so `src/` stays
/// untouched. See spikes/2176-xpc-swap-proof/README.md.
fn build_spike_2176_swap_probe_client(target: &str) {
    use std::path::PathBuf;
    use std::process::Command;

    let source = "../spikes/2176-xpc-swap-proof/client/Spike2176SwapProbeClient.m";
    let out_dir = PathBuf::from(std::env::var("OUT_DIR").expect("OUT_DIR"));
    let object = out_dir.join("Spike2176SwapProbeClient.o");
    let archive = out_dir.join("libbeebeeb_spike_2176_client.a");
    let clang_target = match target {
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
            "-Wall",
            "-Werror",
            "-c",
            source,
            "-o",
        ])
        .arg(&object)
        .status()
        .expect("run clang for Spike2176SwapProbeClient.m");
    assert!(status.success(), "clang failed for Spike2176SwapProbeClient.m");
    let status = Command::new("ar")
        .arg("crs")
        .arg(&archive)
        .arg(&object)
        .status()
        .expect("archive Spike2176SwapProbeClient.o");
    assert!(status.success(), "ar failed for Spike2176SwapProbeClient.o");

    println!("cargo:rerun-if-changed={source}");
    // OUT_DIR is already on the search path (build_macos_file_provider_bridge).
    println!("cargo:rustc-link-lib=static=beebeeb_spike_2176_client");
    // Keep the object (and its constructor) although nothing calls into it.
    println!("cargo:rustc-link-arg-bins=-Wl,-u,_beebeeb_spike_2176_anchor");
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
