//! The rules a manifest must pass before anything is downloaded. Pure: bytes in, a verdict or a refusal out.
//!
//! In order: the manifest is signed by our key, for this channel's manifest file; it parses; it is for this product
//! and channel; it is not older than one this install already accepted on the channel (replay); the artefact for
//! this PC lives at an address we trust, has a length, and carries a signature by our key whose signed file name is
//! the artefact's and names the manifest's version. Only then is the version compared with the installed one.

use serde::{Deserialize, Serialize};

use crate::manifest::{Channel, Manifest};
use crate::signature::{PublicKey, Signature};
use crate::version::Version;

/// The first native release. The native updater never installs anything older: a 0.8.x build is the Tauri app,
/// which cannot read state a native build has written forward.
pub const NATIVE_FLOOR: Version = Version::new(0, 9, 0);

/// The largest artefact accepted. Today's NSIS installer is about 13 MiB.
pub const MAX_ARTIFACT_BYTES: u64 = 512 * 1024 * 1024;

/// What this build trusts. Fixed at build time by the shell; nothing in it comes from the network.
#[derive(Clone)]
pub struct Trust {
    pub key: PublicKey,
    /// `desktop`.
    pub product: String,
    /// Hosts a manifest or artefact may come from, lower case, exact match (our own hosting in the EU).
    pub allowed_hosts: Vec<String>,
    /// Manifest platform keys for this PC, most specific first.
    pub targets: Vec<String>,
    /// Nothing older than this is ever installed.
    pub floor: Version,
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
}

/// Checks a channel manifest and its detached signature against what is installed.
pub fn check(
    trust: &Trust,
    channel: Channel,
    installed: Version,
    seen: Option<HighWater>,
    manifest_bytes: &[u8],
    manifest_signature: &str,
) -> Result<Checked, Refusal> {
    let signature = Signature::from_base64(manifest_signature).map_err(|_| Refusal::ManifestSignature)?;
    signature
        .verify_bytes(&trust.key, manifest_bytes)
        .map_err(|_| Refusal::ManifestSignature)?;
    // From here the signature's time and file name are ours. Each channel's manifest has its own file name, so
    // another channel's manifest with its own valid signature stops here.
    if signature.file != channel.manifest_file() {
        return Err(Refusal::WrongChannel);
    }
    let manifest = Manifest::parse(manifest_bytes).map_err(|_| Refusal::ManifestUnreadable)?;
    if manifest.product.as_deref() != Some(trust.product.as_str()) || manifest.channel != Some(channel) {
        return Err(Refusal::WrongChannel);
    }
    let manifest_time = signature.timestamp;
    if seen.is_some_and(|seen| manifest_time < seen.manifest_time) {
        return Err(Refusal::StaleManifest);
    }
    if manifest.version == installed {
        return Ok(Checked {
            verdict: Verdict::UpToDate,
            manifest_time,
        });
    }

    let targets: Vec<&str> = trust.targets.iter().map(String::as_str).collect();
    let (_, platform) = manifest.platform(&targets).ok_or(Refusal::NoBuildForThisPc)?;
    let file_name = trusted_file_name(&platform.url, &trust.allowed_hosts).ok_or(Refusal::UntrustedAddress)?;
    let size = platform
        .size
        .filter(|size| (1..=MAX_ARTIFACT_BYTES).contains(size))
        .ok_or(Refusal::BadSize)?;
    let artifact = Signature::from_base64(&platform.signature).map_err(|_| Refusal::ArtifactSignature)?;
    artifact.made_by(&trust.key).map_err(|_| Refusal::ArtifactSignature)?;
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

/// True when one `_`-separated part of the file name is exactly the version (`Beebeeb_0.9.0_x64-setup.exe`).
fn names_version(file_name: &str, version: Version) -> bool {
    let version = version.to_string();
    file_name.split('_').any(|part| part == version)
}
