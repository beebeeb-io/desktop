//! Signatures: minisign, in the exact form Tauri's signer writes and 0.8.x verifies.
//!
//! A published `.sig` (and a manifest's `signature` field) is base64 of the minisign signature text. Its trusted
//! comment is `timestamp:<unix seconds><TAB>file:<file name>`, and minisign's global signature covers that comment,
//! so the time and the file name are signed too. Tauri's updater checks only the bytes; this module also hands out
//! the signed time and name, which the policy uses to tie a signature to one file and one moment.
//!
//! Two keys, one format (Guus, 2026-11-11). The native line (0.9.0 and later: installers and v2 manifests) is
//! signed by the v2 key, [`UPDATER_V2_PUBKEY`], and a native build trusts only that key. The 0.8.x line keeps the
//! key every 0.8.x build bakes in ([`LEGACY_0_8_PUBKEY`], `src-tauri/tauri.conf.json`); a client on 0.8.24 or
//! older therefore cannot verify a native installer. Only the bridge release 0.8.25 trusts both.

use std::io::Read;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;

/// The public key of the native line, in the form `tauri signer generate` writes its `.pub` file (base64 of the
/// minisign public key text). The file is the one source: this crate, `scripts/native-channel-manifest.mjs` and
/// the release workflow all read `crates/beebeeb-updater/keys/updater-v2.pub`.
///
/// Key id `FED25848C41F2348`. The same file is `src-tauri/updater-v2.pub` in the bridge release 0.8.25, so the two
/// lines can be diffed. `tests::a_release_build_has_the_v2_key` holds it to: a key, not a placeholder, not the
/// 0.8.x key.
pub const UPDATER_V2_PUBKEY: &str = include_str!("../keys/updater-v2.pub");

/// The updater public key every 0.8.x release since 0.3.0 bakes in (minisign key id `C6FADFD59D732197`), copied
/// from `src-tauri/tauri.conf.json`. A native build does not trust it. `tests::baked_key_matches_tauri_config`
/// fails if the two drift.
pub const LEGACY_0_8_PUBKEY: &str = "dW50cnVzdGVkIGNvbW1lbnQ6IG1pbmlzaWduIHB1YmxpYyBrZXk6IEM2RkFERkQ1OUQ3MzIxOTcKUldTWElYT2QxZC82eG1ocjBUK0ZUZUZqU25JbmFNdmJoZ0w1dnhidDUxREZMd2wycUlDOGpVNloK";

/// Why a key or signature was refused. Never carries key or signature bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SignatureError {
    /// Not base64, not text, or not a minisign key or signature.
    Malformed,
    /// The trusted comment is not `timestamp:<n><TAB>file:<name>`.
    NoTimestampOrFile,
    /// Made by another key than the one this build trusts.
    WrongKey,
    /// A legacy (not pre-hashed) signature: Tauri's signer never writes one, so it is not ours.
    Legacy,
    /// The bytes are not what was signed.
    Mismatch,
    /// The bytes could not be read.
    Unreadable,
}

/// The key a build trusts.
#[derive(Clone)]
pub struct PublicKey(minisign_verify::PublicKey);

impl PublicKey {
    /// From the form in `tauri.conf.json`: base64 of the minisign public key text.
    pub fn from_tauri_base64(encoded: &str) -> Result<Self, SignatureError> {
        let text = decode_text(encoded)?;
        minisign_verify::PublicKey::decode(&text)
            .map(Self)
            .map_err(|_| SignatureError::Malformed)
    }

    /// The key of the native line. `Err` only if `keys/updater-v2.pub` is not a minisign public key.
    pub fn updater_v2() -> Result<Self, SignatureError> {
        Self::from_tauri_base64(UPDATER_V2_PUBKEY)
    }

    /// The key of the 0.8.x line: for reading what 0.8.x published, never trusted by a native build.
    pub fn legacy_0_8() -> Self {
        Self::from_tauri_base64(LEGACY_0_8_PUBKEY).expect("the baked 0.8.x updater public key decodes")
    }
}

/// A decoded signature with its signed time and file name. Decoding proves nothing: call a `verify_*` method.
pub struct Signature {
    inner: minisign_verify::Signature,
    /// Unix seconds, from the trusted comment. Trust it only after a successful verify.
    pub timestamp: u64,
    /// The signed file's name, from the trusted comment. Trust it only after a successful verify.
    pub file: String,
}

impl Signature {
    /// From a `.sig` file's content or a manifest's `signature` field.
    pub fn from_base64(encoded: &str) -> Result<Self, SignatureError> {
        let text = decode_text(encoded)?;
        let inner = minisign_verify::Signature::decode(&text).map_err(|_| SignatureError::Malformed)?;
        let (timestamp, file) =
            parse_trusted_comment(inner.trusted_comment()).ok_or(SignatureError::NoTimestampOrFile)?;
        Ok(Self { inner, timestamp, file })
    }

    /// Whether `key` made this signature (its key id, and that it is pre-hashed), before any byte is downloaded.
    /// It proves nothing about the signed bytes or the trusted comment.
    pub fn made_by(&self, key: &PublicKey) -> Result<(), SignatureError> {
        key.0.verify_stream(&self.inner).map(|_| ()).map_err(map_error)
    }

    /// Verifies bytes held in memory (a manifest).
    pub fn verify_bytes(&self, key: &PublicKey, bytes: &[u8]) -> Result<(), SignatureError> {
        self.verify_reader(key, bytes)
    }

    /// Verifies a stream (an installer on disk) without holding it in memory.
    pub fn verify_reader(&self, key: &PublicKey, mut reader: impl Read) -> Result<(), SignatureError> {
        let mut verifier = key.0.verify_stream(&self.inner).map_err(map_error)?;
        let mut buffer = vec![0u8; 64 * 1024];
        loop {
            match reader.read(&mut buffer) {
                Ok(0) => break,
                Ok(read) => verifier.update(&buffer[..read]),
                Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(_) => return Err(SignatureError::Unreadable),
            }
        }
        verifier.finalize().map_err(map_error)
    }
}

fn map_error(error: minisign_verify::Error) -> SignatureError {
    match error {
        minisign_verify::Error::UnexpectedKeyId => SignatureError::WrongKey,
        minisign_verify::Error::UnsupportedLegacyMode | minisign_verify::Error::UnexpectedAlgorithm => {
            SignatureError::Legacy
        }
        minisign_verify::Error::InvalidSignature => SignatureError::Mismatch,
        _ => SignatureError::Malformed,
    }
}

fn decode_text(encoded: &str) -> Result<String, SignatureError> {
    let bytes = BASE64.decode(encoded.trim()).map_err(|_| SignatureError::Malformed)?;
    String::from_utf8(bytes).map_err(|_| SignatureError::Malformed)
}

/// `timestamp:<n><TAB>file:<name>`: tab-separated `key:value` fields, both required, each at most once.
fn parse_trusted_comment(comment: &str) -> Option<(u64, String)> {
    let (mut timestamp, mut file) = (None, None);
    for field in comment.split('\t') {
        if let Some(value) = field.strip_prefix("timestamp:") {
            if timestamp.replace(value.parse::<u64>().ok()?).is_some() {
                return None;
            }
        } else if let Some(value) = field.strip_prefix("file:")
            && (value.is_empty() || file.replace(value.to_string()).is_some())
        {
            return None;
        }
    }
    Some((timestamp?, file?))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testkit::TestKey;

    #[test]
    fn trusted_comment_needs_one_timestamp_and_one_file() {
        assert_eq!(
            parse_trusted_comment("timestamp:1793959838\tfile:Beebeeb_0.8.22_x64-setup.exe"),
            Some((1793959838, "Beebeeb_0.8.22_x64-setup.exe".to_string()))
        );
        assert_eq!(
            parse_trusted_comment("timestamp:5\tfile:a\thashed"),
            Some((5, "a".to_string()))
        );
        for bad in [
            "",
            "timestamp:5",
            "file:a",
            "timestamp:x\tfile:a",
            "timestamp:5\tfile:",
            "timestamp:5\ttimestamp:6\tfile:a",
            "timestamp:5\tfile:a\tfile:b",
        ] {
            assert_eq!(parse_trusted_comment(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn verifies_what_was_signed_and_reads_its_time_and_name() {
        let key = TestKey::generate();
        let signature =
            Signature::from_base64(&key.sign(b"installer bytes", 1_800_000_000, "Beebeeb_0.9.0_x64-setup.exe"))
                .expect("decodes");
        assert_eq!(
            (signature.timestamp, signature.file.as_str()),
            (1_800_000_000, "Beebeeb_0.9.0_x64-setup.exe")
        );
        assert_eq!(signature.verify_bytes(&key.public(), b"installer bytes"), Ok(()));
    }

    #[test]
    fn refuses_changed_bytes() {
        let key = TestKey::generate();
        let signature = Signature::from_base64(&key.sign(b"installer bytes", 1, "a.exe")).expect("decodes");
        assert_eq!(
            signature.verify_bytes(&key.public(), b"installer bytez"),
            Err(SignatureError::Mismatch)
        );
        assert_eq!(
            signature.verify_bytes(&key.public(), b""),
            Err(SignatureError::Mismatch)
        );
    }

    #[test]
    fn refuses_another_key() {
        let (ours, theirs) = (TestKey::generate(), TestKey::generate());
        let signature = Signature::from_base64(&theirs.sign(b"installer bytes", 1, "a.exe")).expect("decodes");
        assert_eq!(
            signature.verify_bytes(&ours.public(), b"installer bytes"),
            Err(SignatureError::WrongKey)
        );
    }

    /// The trusted comment is signed: a signature whose time or file name was rewritten no longer verifies.
    #[test]
    fn refuses_a_rewritten_trusted_comment() {
        let key = TestKey::generate();
        let honest = key.sign(b"installer bytes", 1_700_000_000, "Beebeeb_0.8.1_x64-setup.exe");
        let text = String::from_utf8(BASE64.decode(honest).expect("base64")).expect("text");
        let forged = BASE64.encode(text.replace("Beebeeb_0.8.1_", "Beebeeb_9.9.9_"));
        let signature = Signature::from_base64(&forged).expect("still decodes");
        assert_eq!(signature.file, "Beebeeb_9.9.9_x64-setup.exe");
        assert_eq!(
            signature.verify_bytes(&key.public(), b"installer bytes"),
            Err(SignatureError::Mismatch)
        );
    }

    #[test]
    fn refuses_what_is_not_a_signature() {
        assert!(matches!(
            Signature::from_base64("not base64 !"),
            Err(SignatureError::Malformed)
        ));
        assert!(matches!(
            Signature::from_base64(&BASE64.encode("<html>504 Gateway Time-out</html>")),
            Err(SignatureError::Malformed)
        ));
        assert!(matches!(
            PublicKey::from_tauri_base64("AAAA"),
            Err(SignatureError::Malformed)
        ));
    }

    /// The constant is a copy; the Tauri config stays the source until `src-tauri` is gone.
    #[test]
    fn baked_key_matches_tauri_config() {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../src-tauri/tauri.conf.json");
        let config: serde_json::Value =
            serde_json::from_slice(&std::fs::read(path).expect("src-tauri/tauri.conf.json is readable")).expect("json");
        assert_eq!(config["plugins"]["updater"]["pubkey"].as_str(), Some(LEGACY_0_8_PUBKEY));
        let _ = PublicKey::legacy_0_8();
    }

    /// The committed v2 key is a real minisign public key (key id `FED25848C41F2348`), not a placeholder and not
    /// the 0.8.x key. The release workflow runs this test before it builds anything.
    #[test]
    fn a_release_build_has_the_v2_key() {
        assert!(
            !UPDATER_V2_PUBKEY.contains("PLACEHOLDER"),
            "keys/updater-v2.pub is a placeholder"
        );
        assert_ne!(
            UPDATER_V2_PUBKEY.trim(),
            LEGACY_0_8_PUBKEY,
            "the native line has its own key"
        );
        let text = decode_text(UPDATER_V2_PUBKEY).expect("base64 of text");
        assert!(text.starts_with("untrusted comment: minisign public key: FED25848C41F2348\n"));
        let v2 = PublicKey::updater_v2().expect("keys/updater-v2.pub is a minisign public key");
        // A signature the 0.8.x key really made is not the v2 key's.
        let published: serde_json::Value =
            serde_json::from_slice(include_bytes!("../tests/fixtures/desktop-latest-0.8.22.json")).expect("json");
        let by_old_key = Signature::from_base64(
            published["platforms"]["windows-x86_64-nsis"]["signature"]
                .as_str()
                .expect("signature"),
        )
        .expect("decodes");
        assert_eq!(by_old_key.made_by(&PublicKey::legacy_0_8()), Ok(()));
        assert_eq!(by_old_key.made_by(&v2), Err(SignatureError::WrongKey));
    }

    /// A legacy (not pre-hashed) signature is valid minisign and our own key made it; it is refused all the same,
    /// before any byte is read: Tauri's signer never writes one, so none of ours exists.
    #[test]
    fn refuses_a_legacy_signature_by_our_own_key() {
        use ed25519_dalek::Signer as _;
        // An obviously-test key: the seed is 32 times the byte 7.
        let secret = ed25519_dalek::SigningKey::from_bytes(&[7u8; 32]);
        let key_id = [1u8, 2, 3, 4, 5, 6, 7, 8];
        let boxed = |algorithm: &[u8; 2], rest: &[u8]| BASE64.encode([&algorithm[..], &key_id[..], rest].concat());
        let public = format!(
            "untrusted comment: minisign public key\n{}\n",
            boxed(b"Ed", secret.verifying_key().as_bytes())
        );
        let key = PublicKey::from_tauri_base64(&BASE64.encode(public)).expect("a minisign public key");

        let bytes = b"installer bytes";
        let comment = "timestamp:1800000000\tfile:Beebeeb_0.9.0_x64-setup.exe";
        let over_bytes = secret.sign(bytes).to_bytes();
        let global = secret.sign(&[&over_bytes[..], comment.as_bytes()].concat()).to_bytes();
        let text = format!(
            "untrusted comment: legacy\n{}\ntrusted comment: {comment}\n{}\n",
            boxed(b"Ed", &over_bytes),
            BASE64.encode(global)
        );
        let signature = Signature::from_base64(&BASE64.encode(text)).expect("decodes");

        // The control: with legacy signatures allowed, this one verifies. So only the legacy rule refuses it.
        assert!(
            key.0.verify(bytes, &signature.inner, true).is_ok(),
            "a valid legacy signature"
        );
        assert_eq!(signature.made_by(&key), Err(SignatureError::Legacy));
        assert_eq!(signature.verify_bytes(&key, bytes), Err(SignatureError::Legacy));
    }
}
