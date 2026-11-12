// THROWAWAY (task 2477 spike). Compiles the C shim against the LibreOfficeKit headers of the exact
// LibreOffice version under test. The headers are not vendored: run.ps1 downloads them from the
// release tag, checks their SHA-256 and passes the folder in LOK_INCLUDE_DIR.
fn main() {
    println!("cargo:rerun-if-changed=shim.c");
    println!("cargo:rerun-if-env-changed=LOK_INCLUDE_DIR");
    let inc = std::env::var("LOK_INCLUDE_DIR")
        .expect("LOK_INCLUDE_DIR must hold the folder that contains LibreOfficeKit/*.h");
    cc::Build::new()
        .file("shim.c")
        .include(inc)
        .warnings(false)
        .compile("lokshim");
}
