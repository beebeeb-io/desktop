//! The rules an update must pass before this build offers or installs it (tasks 2449 and 2451). Pure: text and bytes
//! in, a verdict or a refusal out; no network, no Tauri.
//!
//! Owns: the updater keys this build trusts (the one baked into `tauri.conf.json` and the next one, in
//! `updater-v2.pub`); what a Tauri-format signature says (its key id, its signed file name); the rule that an
//! artefact's signature names the file the address ends in and the version the manifest claims; and the check of a
//! signed `desktop/v2/<channel>.json` manifest: signed by a trusted key for that channel's file, for this product and
//! channel, never below the first native release, its artefact on our own host.
//! Must not: fetch anything, read the clock, or fall back to an unsigned feed (the caller decides what a missing
//! manifest means; a refusal here is final).
//! Tests: `trust_tests`.

use base64::Engine as _;

/// The product name a v2 manifest must carry.
pub(crate) const PRODUCT: &str = "desktop";
/// The one host an artefact of a v2 manifest may be served from.
pub(crate) const ALLOWED_HOST: &str = "releases.beebeeb.io";
/// The first native release. A v2 manifest never offers anything older, and an unsigned feed never anything newer.
pub(crate) const NATIVE_FLOOR: (u64, u64, u64) = (0, 9, 0);
/// The largest artefact a v2 manifest may name.
pub(crate) const MAX_ARTIFACT_BYTES: u64 = 512 * 1024 * 1024;
/// The next updater public key (task 2451), in the form `tauri signer generate` writes its `.pub` file. Until the
/// real key is in the file it holds a line that starts with [`PLACEHOLDER_PREFIX`], and `build.rs` refuses a release
/// build.
pub(crate) const NEXT_PUBKEY_FILE: &str = include_str!("../../updater-v2.pub");
/// How the placeholder in `updater-v2.pub` starts. `build.rs` holds the same text.
pub(crate) const PLACEHOLDER_PREFIX: &str = "PLACEHOLDER";

/// Why an update was refused. The code is what the caller's error leads with; nothing here carries network text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Refusal {
    /// The manifest's signature is missing, malformed, by a key this build does not trust, or not for these bytes.
    ManifestSignature,
    /// The signature is for another manifest file, or the manifest names another product or channel.
    WrongChannel,
    ManifestUnreadable,
    /// The offered version is older than the first native release.
    BelowFloor,
    /// The update is not the one the signed manifest offers.
    NotInManifest,
    /// The artefact is not on our host over https, or its address is not a plain path.
    UntrustedAddress,
    /// The artefact has no length, or one out of range.
    BadSize,
    /// The artefact's signature is malformed or by a key this build does not trust.
    ArtifactSignature,
    /// The artefact's signature signs another file name, or a name without the manifest's version.
    SignatureForOtherFile,
    /// The offered version is not newer than the installed one.
    NotNewer,
    /// A native release offered by a feed that is not signed.
    NativeFromUnsignedFeed,
}

impl Refusal {
    pub(crate) fn code(self) -> &'static str {
        match self {
            Refusal::ManifestSignature => "update_manifest_not_signed",
            Refusal::WrongChannel => "update_manifest_wrong_channel",
            Refusal::ManifestUnreadable => "update_manifest_unreadable",
            Refusal::BelowFloor => "update_below_native_floor",
            Refusal::NotInManifest => "update_not_in_signed_manifest",
            Refusal::UntrustedAddress => "update_untrusted_address",
            Refusal::BadSize => "update_bad_size",
            Refusal::ArtifactSignature => "update_signature_untrusted_key",
            Refusal::SignatureForOtherFile => "update_signature_other_file",
            Refusal::NotNewer => "update_not_newer",
            Refusal::NativeFromUnsignedFeed => "update_native_needs_signed_manifest",
        }
    }

    /// The error text a command returns: the code, then one plain sentence.
    pub(crate) fn message(self) -> String {
        format!("{}: this update was refused and nothing was installed.", self.code())
    }
}

/// A minisign text (a public key or a signature) from the form Tauri stores: base64 of the text. The text itself
/// is taken too, so a plain minisign `.pub` or `.sig` file works.
fn minisign_text(stored: &str) -> Option<String> {
    let stored = stored.trim();
    if stored.starts_with("untrusted comment:") {
        return Some(stored.to_string());
    }
    let bytes = base64::engine::general_purpose::STANDARD.decode(stored).ok()?;
    let text = String::from_utf8(bytes).ok()?;
    text.starts_with("untrusted comment:").then_some(text)
}

/// The 8-byte key id in the base64 line of a minisign key or signature (after the 2-byte algorithm).
fn key_id_of_line(line: &str) -> Option<[u8; 8]> {
    let bytes = base64::engine::general_purpose::STANDARD.decode(line.trim()).ok()?;
    bytes.get(2..10)?.try_into().ok()
}

/// One updater public key this build trusts.
pub(crate) struct TrustedKey {
    /// The key as the Tauri updater's `pubkey` setting takes it (base64 of the minisign text).
    pub(crate) tauri_pubkey: String,
    key_id: [u8; 8],
    key: minisign_verify::PublicKey,
}

impl TrustedKey {
    fn parse(stored: &str) -> Option<Self> {
        let text = minisign_text(stored)?;
        let key = minisign_verify::PublicKey::decode(&text).ok()?;
        let key_id = key_id_of_line(text.lines().nth(1)?)?;
        Some(Self {
            tauri_pubkey: base64::engine::general_purpose::STANDARD.encode(text.as_bytes()),
            key_id,
            key,
        })
    }
}

/// The keys an update may be signed with: the baked one first, then the next one when its file holds a key.
pub(crate) struct TrustedKeys(Vec<TrustedKey>);

impl TrustedKeys {
    /// `baked` is `plugins.updater.pubkey`; `next_file` is the text of `updater-v2.pub`. The baked key must parse.
    /// The placeholder is no key; anything else in the file must parse, so a damaged key file is an error, never a
    /// silently shorter list.
    pub(crate) fn new(baked: &str, next_file: &str) -> Result<Self, String> {
        let mut keys = vec![TrustedKey::parse(baked).ok_or("the baked updater public key does not parse")?];
        if !is_placeholder(next_file) {
            let next = TrustedKey::parse(next_file).ok_or("updater-v2.pub does not hold a public key")?;
            if next.key_id != keys[0].key_id {
                keys.push(next);
            }
        }
        Ok(Self(keys))
    }

    pub(crate) fn len(&self) -> usize {
        self.0.len()
    }

    /// The baked key: the one the Tauri updater uses when it is given no other.
    pub(crate) fn baked(&self) -> &TrustedKey {
        &self.0[0]
    }

    /// The trusted key whose id a signature carries, before any byte is checked: which key to verify with.
    pub(crate) fn key_for_signature(&self, signature: &str) -> Option<&TrustedKey> {
        let text = minisign_text(signature)?;
        let id = key_id_of_line(text.lines().nth(1)?)?;
        self.0.iter().find(|key| key.key_id == id)
    }

    /// Verifies `bytes` against `signature` with whichever trusted key made it (the trusted comment included), and
    /// returns the file name that signature signs.
    fn verify(&self, bytes: &[u8], signature: &str) -> Option<String> {
        let key = self.key_for_signature(signature)?;
        let text = minisign_text(signature)?;
        let parsed = minisign_verify::Signature::decode(&text).ok()?;
        key.key.verify(bytes, &parsed, true).ok()?;
        signed_file_name(signature)
    }
}

/// Whether the text of `updater-v2.pub` is still the placeholder.
pub(crate) fn is_placeholder(next_file: &str) -> bool {
    next_file.trim_start().starts_with(PLACEHOLDER_PREFIX)
}

/// The file name a Tauri signature signs: `trusted comment: timestamp:<n>\tfile:<name>`, the third line of its text.
/// It is covered by the signature, so it is ours once the bytes verified with that signature.
pub(crate) fn signed_file_name(signature: &str) -> Option<String> {
    let text = minisign_text(signature)?;
    let comment = text.lines().nth(2)?.strip_prefix("trusted comment: ")?;
    comment
        .split('\t')
        .find_map(|field| field.strip_prefix("file:"))
        .map(str::to_string)
}

/// A plain `major.minor.patch` version, nothing else.
pub(crate) fn plain_version(version: &str) -> Option<(u64, u64, u64)> {
    let mut parts = version.split('.');
    let mut next = || {
        let part = parts.next()?;
        (!part.is_empty() && part.len() <= 9 && part.bytes().all(|b| b.is_ascii_digit())).then(|| part.parse().ok())?
    };
    let version = (next()?, next()?, next()?);
    parts.next().is_none().then_some(version)
}

/// Whether `version` is a native release (0.9.0 or later). A version this build cannot read counts as native, so
/// an unsigned feed cannot offer it.
pub(crate) fn is_native_line(version: &str) -> bool {
    plain_version(version).is_none_or(|version| version >= NATIVE_FLOOR)
}

/// True when the signature signs the file the address ends in, and that name carries the version as one of its
/// `_`-separated parts (`Beebeeb_0.8.26_x64-setup.exe`). An older release's installer, honestly signed, has that
/// release's name, so it cannot stand in for another version.
pub(crate) fn artifact_names_version(version: &str, download_url: &str, signature: &str) -> bool {
    let Some(signed) = signed_file_name(signature) else {
        return false;
    };
    let url_name = download_url.rsplit('/').next().unwrap_or_default();
    !version.is_empty() && signed == url_name && signed.split('_').any(|part| part == version)
}

/// The last path segment of `url` when it is `https://releases.beebeeb.io/<plain path>`: no port, user, query,
/// fragment or dot segment, nothing outside `A-Z a-z 0-9 . _ -` in a segment.
fn trusted_file_name(url: &str) -> Option<&str> {
    let (host, path) = url.strip_prefix("https://")?.split_once('/')?;
    let plain = |segment: &str| {
        !segment.is_empty()
            && segment != "."
            && segment != ".."
            && segment
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'))
    };
    (host == ALLOWED_HOST && path.split('/').all(plain)).then(|| path.rsplit('/').next())?
}

/// The file name of a channel's v2 manifest, which its detached signature must sign.
pub(crate) fn v2_manifest_file(channel: &str) -> Option<&'static str> {
    match channel {
        "stable" => Some("stable.json"),
        "beta" => Some("beta.json"),
        "alpha" => Some("alpha.json"),
        _ => None,
    }
}

#[derive(serde::Deserialize)]
struct RawManifest {
    version: String,
    product: Option<String>,
    channel: Option<String>,
    #[serde(default)]
    platforms: std::collections::BTreeMap<String, RawPlatform>,
}

#[derive(serde::Deserialize)]
struct RawPlatform {
    url: String,
    signature: String,
    size: Option<u64>,
}

/// A v2 manifest whose detached signature verified for this channel. Everything in it is ours.
pub(crate) struct SignedManifest {
    pub(crate) version: String,
    platforms: Vec<RawPlatform>,
}

/// Checks a `desktop/v2/<channel>.json` manifest and its detached signature. In order: signed by a trusted key
/// for these bytes; the signature signs this channel's manifest file name (another channel's manifest, validly
/// signed, stops here); it parses; it is for this product and channel; its version is a plain one at or above the
/// first native release.
pub(crate) fn verify_v2_manifest(
    keys: &TrustedKeys,
    channel: &str,
    manifest: &[u8],
    detached_signature: &str,
) -> Result<SignedManifest, Refusal> {
    let signed_name = keys
        .verify(manifest, detached_signature)
        .ok_or(Refusal::ManifestSignature)?;
    if Some(signed_name.as_str()) != v2_manifest_file(channel) {
        return Err(Refusal::WrongChannel);
    }
    let raw: RawManifest = serde_json::from_slice(manifest).map_err(|_| Refusal::ManifestUnreadable)?;
    if raw.product.as_deref() != Some(PRODUCT) || raw.channel.as_deref() != Some(channel) {
        return Err(Refusal::WrongChannel);
    }
    let version = plain_version(&raw.version).ok_or(Refusal::ManifestUnreadable)?;
    if version < NATIVE_FLOOR {
        return Err(Refusal::BelowFloor);
    }
    Ok(SignedManifest {
        version: raw.version,
        platforms: raw.platforms.into_values().collect(),
    })
}

impl SignedManifest {
    /// Whether the update the Tauri updater read (its version, address and signature) is one this signed manifest
    /// offers, at an address we trust, with a length, signed by a trusted key for that file and this version.
    /// Returns the key the artefact's bytes must verify with.
    pub(crate) fn artifact<'k>(
        &self,
        keys: &'k TrustedKeys,
        version: &str,
        download_url: &str,
        signature: &str,
    ) -> Result<&'k TrustedKey, Refusal> {
        if version != self.version {
            return Err(Refusal::NotInManifest);
        }
        let platform = self
            .platforms
            .iter()
            .find(|platform| platform.url == download_url && platform.signature == signature)
            .ok_or(Refusal::NotInManifest)?;
        trusted_file_name(&platform.url).ok_or(Refusal::UntrustedAddress)?;
        if !platform
            .size
            .is_some_and(|size| (1..=MAX_ARTIFACT_BYTES).contains(&size))
        {
            return Err(Refusal::BadSize);
        }
        let key = keys
            .key_for_signature(&platform.signature)
            .ok_or(Refusal::ArtifactSignature)?;
        if !artifact_names_version(&self.version, &platform.url, &platform.signature) {
            return Err(Refusal::SignatureForOtherFile);
        }
        Ok(key)
    }
}

/// Where an update came from.
pub(crate) enum Feed<'a> {
    /// Today's unsigned `desktop/<channel>.json`.
    Unsigned,
    /// A verified `desktop/v2/<channel>.json`.
    Signed(&'a SignedManifest),
}

/// What kind of install is asked for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Direction {
    /// The background check, the banner and "Restart to update": strictly newer only.
    NewerOnly,
    /// The manual check in Settings and its channel downgrade, which names the version it installs: an equal or
    /// older version of the unsigned feed is reported, never one of a signed manifest.
    AnyVersion,
}

/// The host facts the rules depend on.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Host {
    /// A Mac's artefact (`Beebeeb.app.tar.gz`) carries no version, and a Mac never installs in place (task 2176).
    pub(crate) macos: bool,
    /// Windows reads the v2 manifest first; there a native release comes from a signed manifest only.
    pub(crate) windows: bool,
}

/// The one decision before an update is offered or installed. Returns the key its bytes must verify with.
pub(crate) fn accept<'k>(
    keys: &'k TrustedKeys,
    feed: &Feed<'_>,
    host: Host,
    direction: Direction,
    installed: &str,
    version: &str,
    download_url: &str,
    signature: &str,
    newer: impl Fn(&str, &str) -> Option<bool>,
) -> Result<&'k TrustedKey, Refusal> {
    // `newer(installed, offered)`: `None` when either version cannot be read, which is never "newer".
    if direction == Direction::NewerOnly && newer(installed, version) != Some(true) {
        return Err(Refusal::NotNewer);
    }
    match feed {
        Feed::Signed(manifest) => {
            // A signed manifest only ever moves an install forward, whatever was asked for.
            if newer(installed, version) != Some(true) {
                return Err(Refusal::NotNewer);
            }
            manifest.artifact(keys, version, download_url, signature)
        }
        Feed::Unsigned => {
            if host.windows && is_native_line(version) {
                return Err(Refusal::NativeFromUnsignedFeed);
            }
            let key = keys.key_for_signature(signature).ok_or(Refusal::ArtifactSignature)?;
            if !host.macos && !artifact_names_version(version, download_url, signature) {
                return Err(Refusal::SignatureForOtherFile);
            }
            Ok(key)
        }
    }
}
