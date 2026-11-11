//! The channel manifest: the JSON a release publishes per channel, in the format 0.8.x already reads.
//!
//! Today's shape (`release.yml`, "Generate desktop channel manifest") is `version`, `notes`, `pub_date` and
//! `platforms.<target> = { url, signature }`. The native updater reads the same shape plus three additions that an
//! older client ignores: `product`, `channel` (what the manifest is for) and `platforms.<target>.size` (the
//! artefact's length in bytes). [`Manifest::parse`] accepts both; the policy decides which additions it requires.

use std::collections::BTreeMap;
use std::fmt;

use serde::{Deserialize, Serialize};

use crate::version::Version;

/// The largest manifest read from the network. Today's are about 17 KiB (the notes are most of it).
pub const MAX_MANIFEST_BYTES: usize = 1024 * 1024;

/// A release channel. A build is made once and a channel is only the manifest that points at it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Channel {
    Alpha,
    Beta,
    Stable,
}

impl Channel {
    pub const ALL: [Channel; 3] = [Channel::Alpha, Channel::Beta, Channel::Stable];

    pub fn as_str(self) -> &'static str {
        match self {
            Channel::Alpha => "alpha",
            Channel::Beta => "beta",
            Channel::Stable => "stable",
        }
    }

    pub fn parse(text: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|channel| channel.as_str() == text)
    }

    /// The manifest 0.8.x reads, under `desktop/`: stable is `latest.json` there.
    pub fn legacy_manifest_file(self) -> &'static str {
        match self {
            Channel::Alpha => "alpha.json",
            Channel::Beta => "beta.json",
            Channel::Stable => "latest.json",
        }
    }

    /// The manifest the bridge release and the native app read, under `desktop/v2/`.
    pub fn manifest_file(self) -> &'static str {
        match self {
            Channel::Alpha => "alpha.json",
            Channel::Beta => "beta.json",
            Channel::Stable => "stable.json",
        }
    }
}

impl fmt::Display for Channel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// One platform's artefact.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Platform {
    pub url: String,
    /// The artefact's `.sig` file content, as published: base64 of the minisign signature text.
    pub signature: String,
    /// The artefact's length in bytes. Absent in the 0.8.x format.
    pub size: Option<u64>,
}

/// A parsed manifest. Parsing proves the shape only: nothing here is trusted until the policy has checked it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Manifest {
    pub version: Version,
    pub notes: String,
    pub pub_date: Option<String>,
    /// What the manifest is for (`desktop`). Absent in the 0.8.x format.
    pub product: Option<String>,
    /// The channel the manifest was published to. Absent in the 0.8.x format.
    pub channel: Option<Channel>,
    pub platforms: BTreeMap<String, Platform>,
}

/// Why a manifest could not be read. Never carries the manifest's text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ManifestError {
    TooLarge,
    NotJson,
    /// `version` is not plain `MAJOR.MINOR.PATCH`.
    BadVersion,
    /// `channel` is present and is not alpha, beta or stable.
    BadChannel,
    NoPlatforms,
}

#[derive(Deserialize)]
struct RawManifest {
    version: String,
    #[serde(default)]
    notes: Option<String>,
    #[serde(default)]
    pub_date: Option<String>,
    #[serde(default)]
    product: Option<String>,
    #[serde(default)]
    channel: Option<String>,
    platforms: BTreeMap<String, RawPlatform>,
}

#[derive(Deserialize)]
struct RawPlatform {
    url: String,
    signature: String,
    #[serde(default)]
    size: Option<u64>,
}

impl Manifest {
    pub fn parse(bytes: &[u8]) -> Result<Self, ManifestError> {
        if bytes.len() > MAX_MANIFEST_BYTES {
            return Err(ManifestError::TooLarge);
        }
        let raw: RawManifest = serde_json::from_slice(bytes).map_err(|_| ManifestError::NotJson)?;
        let version = Version::parse(&raw.version).ok_or(ManifestError::BadVersion)?;
        let channel = match raw.channel.as_deref() {
            None => None,
            Some(text) => Some(Channel::parse(text).ok_or(ManifestError::BadChannel)?),
        };
        if raw.platforms.is_empty() {
            return Err(ManifestError::NoPlatforms);
        }
        let platforms = raw
            .platforms
            .into_iter()
            .map(|(target, p)| {
                (
                    target,
                    Platform {
                        url: p.url,
                        signature: p.signature,
                        size: p.size,
                    },
                )
            })
            .collect();
        Ok(Self {
            version,
            notes: raw.notes.unwrap_or_default(),
            pub_date: raw.pub_date,
            product: raw.product,
            channel,
            platforms,
        })
    }

    /// The first of `targets` the manifest has, with its key. A Windows NSIS install asks for
    /// `windows-x86_64-nsis` and then `windows-x86_64`, as Tauri's updater does.
    pub fn platform<'a>(&'a self, targets: &[&str]) -> Option<(&'a str, &'a Platform)> {
        targets
            .iter()
            .find_map(|target| self.platforms.get_key_value(*target))
            .map(|(key, platform)| (key.as_str(), platform))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const LEGACY: &str = r#"{"version":"0.8.22","notes":"n","pub_date":"2026-11-06T10:57:51Z","platforms":{
        "windows-x86_64":{"url":"https://example.invalid/a.exe","signature":"c2ln"},
        "windows-x86_64-nsis":{"url":"https://example.invalid/b.exe","signature":"c2ln"}}}"#;

    #[test]
    fn reads_the_format_0_8_publishes() {
        let manifest = Manifest::parse(LEGACY.as_bytes()).expect("legacy manifest parses");
        assert_eq!(manifest.version, Version::new(0, 8, 22));
        assert_eq!((manifest.product.as_deref(), manifest.channel), (None, None));
        let (key, platform) = manifest
            .platform(&["windows-x86_64-nsis", "windows-x86_64"])
            .expect("a Windows entry");
        assert_eq!(
            (key, platform.url.as_str(), platform.size),
            ("windows-x86_64-nsis", "https://example.invalid/b.exe", None)
        );
        assert_eq!(manifest.platform(&["darwin-aarch64"]), None);
    }

    #[test]
    fn reads_the_three_additions() {
        let text = r#"{"version":"0.9.0","product":"desktop","channel":"beta","platforms":{
            "windows-x86_64-nsis":{"url":"https://example.invalid/b.exe","signature":"c2ln","size":42}}}"#;
        let manifest = Manifest::parse(text.as_bytes()).expect("parses");
        assert_eq!(
            (manifest.product.as_deref(), manifest.channel),
            (Some("desktop"), Some(Channel::Beta))
        );
        assert_eq!(manifest.platforms["windows-x86_64-nsis"].size, Some(42));
        assert_eq!(manifest.notes, "");
    }

    /// The bytes `scripts/native-channel-manifest.mjs` writes for a release; `tests/nativeChannelManifest.test.ts`
    /// holds the script to this same file, so the script and this reader cannot drift apart unnoticed.
    #[test]
    fn reads_what_the_release_script_writes() {
        let manifest = Manifest::parse(include_bytes!("../tests/fixtures/native-manifest-shape.json")).expect("parses");
        assert_eq!(
            (manifest.version, manifest.product.as_deref(), manifest.channel),
            (Version::new(0, 9, 0), Some("desktop"), Some(Channel::Beta))
        );
        let (target, platform) = manifest
            .platform(&["windows-x86_64-nsis", "windows-x86_64"])
            .expect("a Windows entry");
        assert_eq!((target, platform.size), ("windows-x86_64-nsis", Some(9_000_001)));
        let hosts = [crate::host::RELEASE_HOST.to_string()];
        assert_eq!(
            crate::policy::trusted_file_name(&platform.url, &hosts),
            Some("Beebeeb_0.9.0_x64-setup.exe")
        );
        assert_eq!(manifest.platforms.len(), 2);
    }

    #[test]
    fn refuses_what_is_not_a_release_manifest() {
        let parse = |text: &str| Manifest::parse(text.as_bytes());
        assert_eq!(parse("<html>504</html>"), Err(ManifestError::NotJson));
        assert_eq!(parse(r#"{"version":"0.9.0"}"#), Err(ManifestError::NotJson));
        assert_eq!(
            parse(r#"{"version":"0.9.0","platforms":{}}"#),
            Err(ManifestError::NoPlatforms)
        );
        assert_eq!(
            parse(&LEGACY.replace("0.8.22", "0.9.0-beta.1")),
            Err(ManifestError::BadVersion)
        );
        assert_eq!(
            parse(&LEGACY.replace(r#""notes":"n""#, r#""channel":"nightly""#)),
            Err(ManifestError::BadChannel)
        );
        assert_eq!(
            Manifest::parse(&vec![b' '; MAX_MANIFEST_BYTES + 1]),
            Err(ManifestError::TooLarge)
        );
    }

    #[test]
    fn channel_files_keep_the_legacy_stable_name_apart() {
        assert_eq!(Channel::Stable.legacy_manifest_file(), "latest.json");
        assert_eq!(Channel::Stable.manifest_file(), "stable.json");
        assert_eq!(Channel::parse("beta"), Some(Channel::Beta));
        assert_eq!(Channel::parse("Beta"), None);
    }
}
