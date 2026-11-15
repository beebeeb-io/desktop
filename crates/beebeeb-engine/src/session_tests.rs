//! The engine's signed-in `Session` (the token and the vault key) wipes itself when dropped and prints neither.
//! Ported from the old shell's `session_zeroizes_master_key_on_drop`; that shell re-exported this very type.

use super::Session;

/// Port of `session_zeroizes_master_key_on_drop`, the master-key half the keychain's token test does not cover.
/// `Session` must stay `ZeroizeOnDrop` (a compile-time bound: remove the derive and this file stops compiling), every
/// field it wipes is wiped by `zeroize`, and a filled session drops cleanly. Mutant: replace the derive with a plain
/// `impl Drop` that does nothing; the bound no longer holds and this test does not build.
#[test]
fn session_zeroizes_master_key_on_drop() {
    use zeroize::Zeroize;

    fn assert_zeroize_on_drop<T: zeroize::ZeroizeOnDrop>() {}
    assert_zeroize_on_drop::<Session>();

    // The derive wipes each field with its own `Zeroize`: the key array, the token text, the optional email.
    let mut key = [7u8; 32];
    key.zeroize();
    assert_eq!(key, [0u8; 32]);
    let mut token = String::from("super-secret-session-token");
    token.zeroize();
    assert!(token.is_empty());
    let mut email = Some(String::from("someone@example.invalid"));
    email.zeroize();
    assert!(email.is_none());

    // The real drop path on a filled session (the wipe runs inside `Drop`, where the bytes can no longer be read).
    let session = Session {
        token: "tok".to_string(),
        master_key: [9u8; 32],
        email: Some("someone@example.invalid".to_string()),
    };
    drop(session);
}

/// The session's `Debug` names neither secret and not the email, only whether one is set. Mutant: print
/// `master_key` (or the token) in the impl; the asserts go red.
#[test]
fn session_debug_shows_no_token_key_or_email() {
    let session = Session {
        token: "super-secret-session-token".to_string(),
        master_key: [9u8; 32],
        email: Some("someone@example.invalid".to_string()),
    };
    for shown in [format!("{session:?}"), format!("{session:#?}")] {
        assert!(!shown.contains("super-secret"), "no token in {shown}");
        assert!(!shown.contains('9'), "no key byte in {shown}");
        assert!(!shown.contains("someone"), "no email in {shown}");
        assert!(shown.contains("<set>"), "says an email is set: {shown}");
    }
    let without_email = Session {
        token: "tok".to_string(),
        master_key: [9u8; 32],
        email: None,
    };
    assert!(format!("{without_email:?}").contains("<unset>"));
}
