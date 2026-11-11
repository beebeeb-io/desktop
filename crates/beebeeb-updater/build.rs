//! Checks the build time a release build bakes in, and leaves it in the binary as a mark the release workflow reads.
//!
//! `BEEBEEB_BUILD_NOT_BEFORE` (unix seconds; the release workflow sets it to the commit's time) becomes
//! `host::BUILD_NOT_BEFORE`: the build never installs from a manifest signed before it. A value that is set but is
//! not plain digits, or is earlier than the day the v2 key was made, stops the build here: a release must not
//! carry a rule that is silently off. Unset is a development build; at run time it refuses every manifest
//! (`Refusal::NoBuildTime`), visibly.

/// `policy::V2_KEY_CREATED` (2026-11-11T00:00:00Z). A test holds the two to the same number.
const EARLIEST: u64 = 1_794_355_200;

fn main() {
    println!("cargo:rerun-if-env-changed=BEEBEEB_BUILD_NOT_BEFORE");
    let mark = match std::env::var("BEEBEEB_BUILD_NOT_BEFORE") {
        Err(_) => "unset".to_string(),
        Ok(text) => {
            let plain = !text.is_empty() && text.len() <= 12 && text.bytes().all(|b| b.is_ascii_digit());
            match text.parse::<u64>() {
                Ok(time) if plain && time >= EARLIEST => text,
                _ => panic!("BEEBEEB_BUILD_NOT_BEFORE={text:?} is not a build time: unix seconds, at least {EARLIEST}"),
            }
        }
    };
    println!("cargo:rustc-env=BEEBEEB_UPDATER_BUILD_MARK=beebeeb-updater-not-before={mark};");
}
