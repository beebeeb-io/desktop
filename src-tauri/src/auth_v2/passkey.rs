//! `auth_passkey_sign_in` (DW-A29, DW-A30). The ceremony and its logic are batch B15's: core's passkey-vault
//! primitive and flows, with only the webauthn.dll call here (crypto review rule 6). Until B15 merges, this command
//! keeps its final name and shape and answers `passkey_unavailable`, so the sign-in screen shows its honest error and
//! the frames render from the web view's fixture. No `navigator.credentials` in the web view, ever.

/// The outcome flags of a passkey sign-in: `prf_unlocked` false means the passkey proved who you are but cannot
/// unlock the vault by itself, so the password is still needed (DW-A30).
#[derive(Clone, Debug, serde::Serialize, PartialEq, Eq)]
pub(crate) struct PasskeySignInOutcome {
    pub(crate) signed_in: bool,
    pub(crate) prf_unlocked: bool,
}

#[tauri::command]
pub(crate) async fn auth_passkey_sign_in() -> Result<PasskeySignInOutcome, String> {
    Err("passkey_unavailable".into())
}
