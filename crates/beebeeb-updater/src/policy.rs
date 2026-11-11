//! The rules a manifest must pass before anything is downloaded. Pure: bytes in, a verdict or a refusal out.
//!
//! In order: the manifest is signed by a key this build trusts, for this channel's manifest file; it parses, with
//! no field we do not know; it is for this product and channel; its key may sign that version; its signed time is
//! not in the future, not older than one this install already accepted on the channel (replay), and a manifest
//! older than this build itself never installs anything; the artefact for this PC lives at an address we trust,
//! has a length, and carries a signature by a trusted key whose signed file name is the artefact's and names the
//! manifest's version. Only then is the version compared with the installed one.

use serde::{Deserialize, Serialize};

use crate::manifest::{Channel, Manifest};
use crate::signature::{PublicKey, Signature};
use crate::version::Version;

/// The first native release. The native updater never installs anything older: a 0.8.x build is the Tauri app,
/// which cannot read state a native build has written forward.
pub const NATIVE_FLOOR: Version = Version::new(0, 9, 0);

/// The largest artefact accepted. Today's NSIS installer is about 13 MiB.
pub const MAX_ARTIFACT_BYTES: u64 = 512 * 1024 * 1024;

/// A signed time may be this far ahead of this PC's clock. Beyond it the manifest is refused and never becomes a
/// replay mark: one signature made with a wrong clock must not block a channel for good.
pub const MAX_CLOCK_SKEW_SECS: u64 = 24 * 60 * 60;

/// The release versions a key may sign: `from..before`. A signature by a trusted key for a version outside its
/// scope is refused, so a key kept for one release line cannot sign for another.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KeyScope {
    pub from: Version,
    /// Exclusive. `None`: no upper end.
    pub before: Option<Version>,
}

impl KeyScope {
    pub const fn from(from: Version) -> Self {
        Self { from, before: None }
    }

    pub fn covers(&self, version: Version) -> bool {
        version >= self.from && self.before.is_none_or(|before| version < before)
    }
}

/// One key a build trusts, and what for.
#[derive(Clone)]
pub struct TrustedKey {
    pub key: PublicKey,
    pub valid_for: KeyScope,
}

/// What this build trusts. Fixed at build time by the shell; nothing in it comes from the network.
#[derive(Clone)]
pub struct Trust {
    /// The native line trusts one key (`signature::UPDATER_V2_PUBKEY`). A list, so a rotation is one release that
    /// trusts the old and the new key, each for its own versions.
    pub keys: Vec<TrustedKey>,
    /// `desktop`.
    pub product: String,
    /// Hosts a manifest or artefact may come from, lower case, exact match (our own hosting in the EU).
    pub allowed_hosts: Vec<String>,
    /// Manifest platform keys for this PC, most specific first.
    pub targets: Vec<String>,
    /// Nothing older than this is ever installed.
    pub floor: Version,
    /// When this build was made (unix seconds, baked in by the release build). A manifest signed before it was
    /// published before this build existed: it can say nothing new to this build and never installs anything.
    pub not_before: u64,
}

impl Trust {
    /// The trusted key that made `signature` (by key id; the bytes are not verified here).
    pub fn signer(&self, signature: &Signature) -> Option<&TrustedKey> {
        self.keys.iter().find(|trusted| signature.made_by(&trusted.key).is_ok())
    }

    /// The trusted key that made `signature` and may sign `version`.
    pub fn signer_for(&self, signature: &Signature, version: Version) -> Option<&PublicKey> {
        self.signer(signature)
            .filter(|trusted| trusted.valid_for.covers(version))
            .map(|trusted| &trusted.key)
    }
}

/// The newest manifest this install accepted on one channel. Persisted, so a replayed older manifest is refused
/// after a restart too.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct HighWater {
    /// The signed time of that manifest's signature (unix seconds).
    pub manifest_time: u64,
}

/// An artefact the policy accepted for download. Everything in it was covered by a verified signature.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Offer {
    pub version: Version,
    pub channel: Channel,
    pub notes: String,
    pub url: String,
    /// The artefact's file name: the last segment of `url`, and the name its signature signs.
    pub file_name: String,
    /// The artefact's `.sig` content (base64 minisign).
    pub signature: String,
    pub size: u64,
    /// True when `version` is older than the installed one: only a manual check reports it, and only a download
    /// that names the version starts it.
    pub downgrade: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verdict {
    UpToDate,
    Upgrade(Offer),
    Downgrade(Offer),
}

/// A verdict with the manifest's signed time, which the caller records as the channel's new high-water mark.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Checked {
    pub verdict: Verdict,
    pub manifest_time: u64,
}

/// Why a manifest was refused. Plain data for the UI; never carries text from the network.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Refusal {
    /// The manifest's signature is missing, malformed, by another key, or not for these bytes.
    ManifestSignature,
    /// The signature is for another manifest file, or the manifest names another product or channel.
    WrongChannel,
    ManifestUnreadable,
    /// Older than a manifest this install already accepted on the channel.
    StaleManifest,
    NoBuildForThisPc,
    /// The artefact is not on a host we trust, or not over https.
    UntrustedAddress,
    /// The artefact has no length, or one outside `1..=MAX_ARTIFACT_BYTES`.
    BadSize,
    /// The artefact's signature is malformed or by another key.
    ArtifactSignature,
    /// The artefact's signature signs another file name, or a name without the manifest's version.
    SignatureForOtherFile,
    /// The offered version is older than the first native release.
    BelowFloor,
    /// The key that signed the manifest is trusted, but not for this version.
    KeyNotValidForVersion,
    /// The manifest's signed time is more than [`MAX_CLOCK_SKEW_SECS`] ahead of this PC's clock.
    SignedInTheFuture,
    /// The manifest offers a newer version but was signed before this build was made: a replayed old manifest.
    OlderThanThisBuild,
}

/// Checks a channel manifest and its detached signature against what is installed.
pub fn check(
    trust: &Trust,
    channel: Channel,
    installed: Version,
    seen: Option<HighWater>,
    now: u64,
    manifest_bytes: &[u8],
    manifest_signature: &str,
) -> Result<Checked, Refusal> {
    let signature = Signature::from_base64(manifest_signature).map_err(|_| Refusal::ManifestSignature)?;
    let signer = trust.signer(&signature).ok_or(Refusal::ManifestSignature)?;
    signature
        .verify_bytes(&signer.key, manifest_bytes)
        .map_err(|_| Refusal::ManifestSignature)?;
    // From here the signature's time and file name are ours. Each channel's manifest has its own file name, so
    // another channel's manifest with its own valid signature stops here.
    if signature.file != channel.manifest_file() {
        return Err(Refusal::WrongChannel);
    }
    let manifest = Manifest::parse_signed(manifest_bytes).map_err(|_| Refusal::ManifestUnreadable)?;
    if manifest.product.as_deref() != Some(trust.product.as_str()) || manifest.channel != Some(channel) {
        return Err(Refusal::WrongChannel);
    }
    if !signer.valid_for.covers(manifest.version) {
        return Err(Refusal::KeyNotValidForVersion);
    }
    let manifest_time = signature.timestamp;
    let latest_believable = now.saturating_add(MAX_CLOCK_SKEW_SECS);
    if manifest_time > latest_believable {
        return Err(Refusal::SignedInTheFuture);
    }
    // A mark beyond what this rule lets in was written under a clock that has since been set back: it would refuse
    // every honest manifest, so it is not believed. `not_before` still holds, and the next manifest replaces it.
    let seen = seen.filter(|seen| seen.manifest_time <= latest_believable);
    if seen.is_some_and(|seen| manifest_time < seen.manifest_time) {
        return Err(Refusal::StaleManifest);
    }
    let up_to_date = Ok(Checked {
        verdict: Verdict::UpToDate,
        manifest_time,
    });
    if manifest.version == installed {
        return up_to_date;
    }
    // Signed before this build existed. Without this rule a fresh install (no mark yet) would take any old signed
    // manifest that names a version above its own, a withdrawn one included. Such a manifest never installs
    // anything: a newer version in it is a replay, an older one is a channel that has nothing for this build yet.
    if manifest_time < trust.not_before {
        return if manifest.version > installed {
            Err(Refusal::OlderThanThisBuild)
        } else {
            up_to_date
        };
    }

    let targets: Vec<&str> = trust.targets.iter().map(String::as_str).collect();
    let (_, platform) = manifest.platform(&targets).ok_or(Refusal::NoBuildForThisPc)?;
    let file_name = trusted_file_name(&platform.url, &trust.allowed_hosts).ok_or(Refusal::UntrustedAddress)?;
    let size = platform
        .size
        .filter(|size| (1..=MAX_ARTIFACT_BYTES).contains(size))
        .ok_or(Refusal::BadSize)?;
    let artifact = Signature::from_base64(&platform.signature).map_err(|_| Refusal::ArtifactSignature)?;
    trust
        .signer_for(&artifact, manifest.version)
        .ok_or(Refusal::ArtifactSignature)?;
    // The name is covered by the artefact's own signature once the bytes verify. An older release's artefact with
    // its honest signature has that release's name, so it cannot stand in for this version.
    if artifact.file != file_name || !names_version(file_name, manifest.version) {
        return Err(Refusal::SignatureForOtherFile);
    }
    if manifest.version < trust.floor {
        return Err(Refusal::BelowFloor);
    }

    let downgrade = manifest.version < installed;
    let offer = Offer {
        version: manifest.version,
        channel,
        notes: manifest.notes.clone(),
        url: platform.url.clone(),
        file_name: file_name.to_string(),
        signature: platform.signature.clone(),
        size,
        downgrade,
    };
    let verdict = if downgrade {
        Verdict::Downgrade(offer)
    } else {
        Verdict::Upgrade(offer)
    };
    Ok(Checked { verdict, manifest_time })
}

/// The last path segment of `url` when it is `https://<allowed host>/<plain path>`: no port, no user, no query,
/// no fragment, no dot segment, nothing outside `A-Z a-z 0-9 . _ - /` in the path.
pub fn trusted_file_name<'a>(url: &'a str, allowed_hosts: &[String]) -> Option<&'a str> {
    let rest = url.strip_prefix("https://")?;
    let (host, path) = rest.split_once('/')?;
    if !allowed_hosts.iter().any(|allowed| allowed == host) {
        return None;
    }
    let plain = |segment: &str| {
        !segment.is_empty()
            && segment != "."
            && segment != ".."
            && segment
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'))
    };
    if !path.split('/').all(plain) {
        return None;
    }
    path.rsplit('/').next()
}

/// True for `Beebeeb_<version>_<rest>.exe` and nothing else (`Beebeeb_0.9.0_x64-setup.exe`). A manifest's own
/// signature signs `<channel>.json`, so it can never pass as an artefact's, nor the reverse.
fn names_version(file_name: &str, version: Version) -> bool {
    file_name
        .strip_prefix("Beebeeb_")
        .and_then(|rest| rest.strip_suffix(".exe"))
        .and_then(|rest| rest.split_once('_'))
        .is_some_and(|(named, rest)| named == version.to_string() && !rest.is_empty())
}
