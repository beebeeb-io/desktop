//! Spec 2026-10-10 §11 (task 1887): the strings Rust shows or returns. Typographic apostrophes, the one-character
//! ellipsis, no emoji. `src/signOutCopy.ts` mirrors them; `tests/signOutCopy.test.ts` pins the two equal.
#![cfg_attr(not(any(target_os = "macos", test)), allow(dead_code))]

pub const UNSENT_CHANGES_CODE: &str = "unsent_changes";
pub const UNSENT_COPY_FAILED_CODE: &str = "unsent_copy_failed";
pub const UNSENT_COUNT_FAILED_CODE: &str = "unsent_count_failed";

pub const E1_UNSENT_CHANGES: &str =
    "Some changes haven’t reached the server yet. Sign out from Settings › Account to send or keep them first.";
pub const E2_UNSENT_COPY_FAILED: &str = "Beebeeb couldn’t copy the changes that haven’t reached the server, so it didn’t sign out. They’re still waiting to upload. Try again; if it keeps happening, restart Beebeeb.";
pub const E3_UNSENT_COUNT_FAILED: &str = "Beebeeb couldn’t check for changes that haven’t reached the server, so it changed nothing. Try again. If it keeps happening, restart Beebeeb.";

/// §11: `<code>: <sentence>`. A caller matches the code, never the sentence.
pub fn coded(code: &str, sentence: &str) -> String {
    format!("{code}: {sentence}")
}
