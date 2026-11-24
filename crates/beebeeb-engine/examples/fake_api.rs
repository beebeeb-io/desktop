//! A fake Beebeeb API on a loopback port, for the native rungs of the sync-start contract (S1, rung R1.1): it serves
//! the invented account of `test_api::fake_account::test_account` (OPAQUE sign-in, the recovery check, 7 encrypted
//! items) until it is stopped.
//!
//! It prints, one per line: the base URL, then the account's email, password and 12 words. They are fixtures that
//! open nothing but this process, not secrets. After that it prints the route of every request it answers
//! (`request: POST /api/v1/auth/verify-recovery-check`), never a header or a body, so a rung counts requests from
//! its output.
//!
//! Built only with the test helpers (`required-features = ["test-support"]`), so never by a release build:
//! `cargo run --example fake_api --features test-support`. It refuses to serve on anything but loopback.

use std::io::Write;
use std::time::Duration;

use beebeeb_engine::test_api::fake_account::{require_loopback, test_account};

/// The invented account's password (`test_api::fake_account::test_account` registers it; the fake keeps only its
/// OPAQUE record).
const PASSWORD: &str = "correct horse battery";

fn main() {
    let (account, phrase, fake) = test_account();
    if let Err(refusal) = require_loopback(&fake.base) {
        eprintln!("fake_api: {refusal}");
        std::process::exit(2);
    }
    let mut out = std::io::stdout();
    let _ = writeln!(out, "base: {}", fake.base);
    let _ = writeln!(out, "email: {}", account.email);
    let _ = writeln!(out, "password: {PASSWORD}");
    let _ = writeln!(out, "phrase: {phrase}");
    let _ = out.flush();
    let mut printed = 0;
    loop {
        std::thread::sleep(Duration::from_millis(200));
        let routes = fake.routes();
        for route in &routes[printed..] {
            let _ = writeln!(out, "request: {route}");
        }
        if routes.len() > printed {
            printed = routes.len();
            let _ = out.flush();
        }
    }
}
