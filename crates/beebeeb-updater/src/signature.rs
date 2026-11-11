//! Signatures: minisign, in the exact form Tauri's signer writes and 0.8.x verifies.
//!
//! A published `.sig` (and a manifest's `signature` field) is base64 of the minisign signature text. Its trusted
//! comment is `timestamp:<unix seconds><TAB>file:<file name>`, and minisign's global signature covers that comment,
//! so the time and the file name are signed too. Tauri's updater checks only the bytes; this module also hands out
//! the signed time and name, which the policy uses to tie a signature to one file and one moment.
//!
//! The public key is the one baked into every 0.8.x build (`src-tauri/tauri.conf.json`, `plugins.updater.pubkey`):
//! base64 of the minisign public key text. One key and one format verify old and new clients' updates.

use std::io::Read;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;

/// The updater public key every release since 0.3.0 bakes in (minisign key id `C6FADFD59D732197`), copied from
/// `src-tauri/tauri.conf.json`. A public key: it verifies and cannot sign. `tests::baked_key_matches_tauri_config`
/// fails if the two drift.
pub const BEEBEEB_DESKTOP_PUBKEY: &str = "dW50cnVzdGVkIGNvbW1lbnQ6IG1pbmlzaWduIHB1YmxpYyBrZXk6IEM2RkFERkQ1OUQ3MzIxOTcKUldTWElYT2QxZC82eG1ocjBUK0ZUZUZqU25JbmFNdmJoZ0w1dnhidDUxREZMd2wycUlDOGpVNloK";

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

    /// The key of the released desktop app.
    pub fn beebeeb_desktop() -> Self {
        Self::from_tauri_base64(BEEBEEB_DESKTOP_PUBKEY).expect("the baked updater public key decodes")
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
        assert_eq!(
            config["plugins"]["updater"]["pubkey"].as_str(),
            Some(BEEBEEB_DESKTOP_PUBKEY)
        );
        let _ = PublicKey::beebeeb_desktop();
    }
}
