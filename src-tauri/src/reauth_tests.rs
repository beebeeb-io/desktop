// Split by line count only (the file-size ratchet): the parts are one module, included in order, so
// every helper and `use` is shared exactly as it was in the single file.
include!("reauth_tests/part_1.rs");
include!("reauth_tests/part_2.rs");

#[cfg(target_os = "macos")]
mod on_a_mac {
    include!("reauth_tests/part_3.rs");
    include!("reauth_tests/part_4.rs");
    include!("reauth_tests/part_5.rs");
}
