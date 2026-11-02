//! `secrets` tests split out of `keychain.rs`.
//!
//! Source-text tests pinning secret zeroization and key borrowing,
//! split out of `keychain.rs`.

use super::*;


    /// Lead ruling 13 (Task 12): the vault key is BORROWED by the constructor that copies it into the wiped buffer, so
    /// no by-value copy of the key is left behind in a frame that nobody wipes.
    #[test]
    fn the_master_key_constructor_borrows_the_key() {
        let source = include_str!("../../keychain.rs").replace("\r\n", "\n");
        let at = source
            .find("pub fn new_master_key(bytes: &[u8; MASTER_KEY_BYTES]) -> Self {")
            .expect("the constructor borrows");
        let body = &source[at..];
        let body = &body[body.find('{').unwrap() + 1..body.find("\n    }").unwrap()];
        assert_eq!(
            body.trim(),
            "Self(bytes.to_vec())",
            "the borrowed key goes straight into the wiped buffer, through no local copy"
        );
        // Read without whitespace: a by-value signature with more parameters is wrapped by rustfmt, which puts
        // `bytes:` on a line of its own. (This file is also compiled into `tests/keychain.rs`, where the crate's
        // `source_pin` helper does not exist.)
        let squeezed: String = source.split_whitespace().collect();
        assert!(
            !squeezed.contains(concat!("new_master_key(bytes:[u8", ";")),
            "no by-value constructor"
        );
    }

    /// The session token wipes itself: `zeroize` empties its text, and its `Drop` calls it.
    #[test]
    fn a_session_token_is_wiped_when_it_goes() {
        use zeroize::Zeroize as _;
        let mut token = SessionToken::new("a-secret-session-token").unwrap();
        token.zeroize();
        assert_eq!(token.expose_for_request(), "", "the text is gone");
        let source = include_str!("../../keychain.rs").replace("\r\n", "\n");
        let drop_impl = &source[source
            .find("impl Drop for SessionToken {")
            .expect("the token wipes on drop")..];
        let drop_impl = &drop_impl[..drop_impl.find("\n}\n").unwrap()];
        assert!(drop_impl.contains("zeroize::Zeroize::zeroize(self)"), "{drop_impl}");
    }

    /// A secret is wiped with the `zeroize` crate (volatile writes the optimizer may not drop), never with a plain fill
    /// before the free, which a compiler may remove as a dead store: `SecretBytes` wipes itself on drop, the macOS read
    /// that fails to free, and the Windows read that fails to decode. No plain fill is left in this file.
    #[test]
    fn secrets_are_wiped_with_zeroize_never_a_plain_fill() {
        use zeroize::Zeroize as _;
        let mut key = SecretBytes::new_master_key(&[7u8; MASTER_KEY_BYTES]);
        key.zeroize();
        assert!(key.expose_for_crypto().is_empty(), "the bytes are gone");
        let source = include_str!("../../keychain.rs").replace("\r\n", "\n");
        let plain_fill = concat!(".fill", "(0)");
        assert_eq!(
            source.matches(plain_fill).count(),
            0,
            "a plain fill is still used to wipe a secret"
        );
        let secret_bytes_drop = &source[source
            .find("impl Drop for SecretBytes {")
            .expect("SecretBytes wipes on drop")..];
        let secret_bytes_drop = &secret_bytes_drop[..secret_bytes_drop.find("\n}\n").unwrap()];
        assert!(
            secret_bytes_drop.contains("zeroize::Zeroize::zeroize(self)"),
            "{secret_bytes_drop}"
        );
        let windows = &source[source
            .find("impl AuthSecretStore for WindowsCredentialStore {")
            .expect("the Windows store")..];
        let windows = &windows[..windows.find("\n}\n").unwrap()];
        assert!(
            windows.contains("zeroize::Zeroize::zeroize(&mut raw)"),
            "the Windows read that fails to decode wipes with zeroize:\n{windows}"
        );
    }
