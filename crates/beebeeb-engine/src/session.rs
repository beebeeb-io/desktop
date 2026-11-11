//! The signed-in session as the engine holds it: the token and the vault key, wiped when dropped.
//! Must not: print, log or serialize either secret (`Debug` shows neither).

use std::fmt;

/// Authenticated session pushed in from the WebView after login.
///
/// `master_key` is the user's 32-byte vault root key, derived from the
/// OPAQUE export_key (or, for legacy accounts, from Argon2id over the
/// password). It is the input to per-file `derive_file_key()` calls
/// inside `beebeeb-sync` — the engine cannot do anything without it.
///
/// The struct is intentionally NOT `Clone`: there should be exactly one
/// authoritative copy of the master key in this process at any moment.
/// `take()` removes it from the registry on logout; everything else
/// reads it through a borrow under the `Mutex`.
///
/// `ZeroizeOnDrop` wipes the secret bytes when the `Session` is dropped — on
/// `lock_vault` / `clear_session` (both `take()` the `Option<Session>`, dropping
/// it) and on process teardown — so the master key and the session token never
/// linger in freed memory. All three fields are `Zeroize`: `[u8; 32]`,
/// `String` (token), and `Option<String>` (email) each implement it in
/// `zeroize` 1.x, so no field needs `#[zeroize(skip)]`.
#[derive(zeroize::ZeroizeOnDrop)]
pub struct Session {
    pub token: String,
    pub master_key: [u8; 32],
    pub email: Option<String>,
}

impl fmt::Debug for Session {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Session")
            .field("token", &"<redacted>")
            .field("master_key", &"<redacted>")
            .field("email", &self.email.as_deref().map(|_| "<set>").unwrap_or("<unset>"))
            .finish()
    }
}
