//! On Windows (MSVC) the COM libraries the `windows` crate's helpers reach (`PropVariantClear`, `GetErrorInfo`,
//! `SetErrorInfo`, `CoCreateFreeThreadedMarshaler`) are delay-loaded: not in the executable's load-time import table,
//! so Windows' loader does not initialise ole32, combase and oleaut32 before `main` (on the VM they failed with
//! 0xc0000142 and the helper never started). They load on the first call, after `mf::api` has loaded them from the
//! system folder, only when the system-decoder route runs. `tests/load_time_imports.rs` holds it.
fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    let windows = std::env::var("CARGO_CFG_TARGET_OS").is_ok_and(|os| os == "windows");
    let msvc = std::env::var("CARGO_CFG_TARGET_ENV").is_ok_and(|env| env == "msvc");
    if windows && msvc {
        for library in ["ole32.dll", "combase.dll", "oleaut32.dll"] {
            println!("cargo:rustc-link-arg=/DELAYLOAD:{library}");
        }
        println!("cargo:rustc-link-arg=delayimp.lib");
    }
}
