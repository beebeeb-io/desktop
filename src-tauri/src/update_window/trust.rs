//! The rules an update must pass before this build offers or installs it (tasks 2449 and 2451). Pure: text and bytes
//! in, a verdict or a refusal out; no network, no Tauri.
//!
//! Owns: the two updater keys this build trusts, each for its own release line and never the other's (the key baked
//! into `tauri.conf.json`: the legacy feed, versions below 0.9.0; the key in `updater-v2.pub`: the v2 manifest,
//! 0.9.0 and later); what a Tauri-format signature says (its key id, its signed file name, its signed time); the
//! rule that an artefact's signature names the file the address ends in and the version the manifest claims; and
//! the check of a signed `desktop/v2/<channel>.json` manifest: signed by the v2 key for that channel's file, for
//! this product and channel, never below the first native release, signed inside the time bounds, its artefact on
//! our own host and signed by the v2 key too.
//! Must not: fetch anything, read the clock (the caller passes `now`), or fall back to an unsigned feed (the caller
//! decides what a missing manifest means; a refusal here is final).
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
/// The v2 key was made on 2026-11-11: `2026-11-11T00:00:00Z` in unix seconds. No manifest of the native line can
/// honestly carry an earlier signed time, so an earlier one is refused. One fixed value for the whole 0.8.x bridge
/// line, on purpose (lead ruling 2026-11-11 12:10 CET, after the security review): a per-build time would strand
/// installs (0.9.0 published first and 0.8.25 built after it would make 0.8.25 refuse 0.9.0; a later 0.8.26 hotfix
/// would refuse an already published 0.9.1).
pub(crate) const V2_KEY_CREATED: u64 = 1_794_355_200;
/// A signed time may be this far ahead of this PC's clock. Beyond it the manifest is refused and never becomes a
/// replay mark: one signature made with a wrong clock must not block a channel for good.
pub(crate) const MAX_CLOCK_SKEW_SECS: u64 = 24 * 60 * 60;
/// The v2 updater public key (task 2451), in the form `tauri signer generate` writes its `.pub` file. Until the
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
    /// A native release (0.9.0 or later), or a version this build cannot read, offered by the legacy feed.
    NativeFromUnsignedFeed,
    /// Signed by a key this build trusts, but not for this release line: the legacy key on the v2 line, or the v2
    /// key on the legacy line.
    KeyScope,
    /// The manifest's signed time is earlier than the v2 key itself ([`V2_KEY_CREATED`]).
    OlderThanKey,
    /// The manifest's signed time is more than [`MAX_CLOCK_SKEW_SECS`] ahead of this PC's clock.
    SignedInTheFuture,
    /// Older than a manifest this install already accepted on the channel (a replay).
    StaleManifest,
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
            Refusal::KeyScope => "update_key_not_for_this_line",
            Refusal::OlderThanKey => "update_manifest_older_than_key",
            Refusal::SignedInTheFuture => "update_manifest_signed_in_future",
            Refusal::StaleManifest => "update_manifest_stale",
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

/// The release line an address serves: which key signs it and which versions it may offer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Line {
    /// `desktop/<latest|beta|alpha>.json`: the key in `tauri.conf.json`, versions below 0.9.0 only.
    Legacy,
    /// `desktop/v2/<channel>.json`: the v2 key, 0.9.0 and later only.
    Native,
}

/// The two keys this build trusts, each for its own line. A signature by one of them on the other's line is
/// refused: the legacy key has no password and was a repository secret (see task 2451), so it must never be able
/// to sign a native release; and nothing signed by the v2 key belongs on the legacy feed.
pub(crate) struct TrustedKeys {
    legacy: TrustedKey,
    native: Option<TrustedKey>,
}

impl TrustedKeys {
    /// `baked` is `plugins.updater.pubkey`; `next_file` is the text of `updater-v2.pub`. The baked key must parse.
    /// The placeholder is no key (this build then takes nothing from the native line); anything else in the file
    /// must parse and must be another key than the baked one, so a damaged key file is an error, never a build
    /// that silently trusts one key for both lines.
    pub(crate) fn new(baked: &str, next_file: &str) -> Result<Self, String> {
        let legacy = TrustedKey::parse(baked).ok_or("the baked updater public key does not parse")?;
        let native = if is_placeholder(next_file) {
            None
        } else {
            let next = TrustedKey::parse(next_file).ok_or("updater-v2.pub does not hold a public key")?;
            if next.key_id == legacy.key_id {
                return Err("updater-v2.pub holds the baked key: the two lines need two keys".into());
            }
            Some(next)
        };
        Ok(Self { legacy, native })
    }

    /// The one key a line's signatures must be made with. `None`: this build has no key for that line.
    pub(crate) fn for_line(&self, line: Line) -> Option<&TrustedKey> {
        match line {
            Line::Legacy => Some(&self.legacy),
            Line::Native => self.native.as_ref(),
        }
    }

    /// The line whose key made `signature`, by the key id it carries, before any byte is checked.
    fn line_of(&self, signature: &str) -> Option<Line> {
        let text = minisign_text(signature)?;
        let id = key_id_of_line(text.lines().nth(1)?)?;
        if self.legacy.key_id == id {
            Some(Line::Legacy)
        } else {
            self.native
                .as_ref()
                .is_some_and(|key| key.key_id == id)
                .then_some(Line::Native)
        }
    }

    /// The key of `line` when `signature` carries its id. A signature by the other line's key is
    /// [`Refusal::KeyScope`]; one by no key of ours is `unknown`.
    fn signer_on(&self, line: Line, signature: &str, unknown: Refusal) -> Result<&TrustedKey, Refusal> {
        match self.line_of(signature) {
            Some(made_by) if made_by == line => self.for_line(line).ok_or(unknown),
            Some(_) => Err(Refusal::KeyScope),
            None => Err(unknown),
        }
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

/// The time a Tauri signature was made (`timestamp:<n>` in its trusted comment, unix seconds). Covered by the
/// signature like the file name. Digits only, at most 12 of them.
pub(crate) fn signed_time(signature: &str) -> Option<u64> {
    let text = minisign_text(signature)?;
    let comment = text.lines().nth(2)?.strip_prefix("trusted comment: ")?;
    let digits = comment.split('\t').find_map(|field| field.strip_prefix("timestamp:"))?;
    (!digits.is_empty() && digits.len() <= 12 && digits.bytes().all(|b| b.is_ascii_digit()))
        .then(|| digits.parse().ok())?
}

/// The time bounds of a v2 manifest, in unix seconds (no time zone anywhere). `signed_at` is the manifest
/// signature's own time, `now` this PC's clock, `seen` the newest signed time this install accepted on the channel.
/// A mark further ahead than the future bound lets in was written under a clock that has since been set back: it
/// would refuse every honest manifest, so it is not believed here (and never lowered where it is stored).
pub(crate) fn check_signed_time(signed_at: u64, now: u64, seen: Option<u64>) -> Result<(), Refusal> {
    if signed_at < V2_KEY_CREATED {
        return Err(Refusal::OlderThanKey);
    }
    let latest_believable = now.saturating_add(MAX_CLOCK_SKEW_SECS);
    if signed_at > latest_believable {
        return Err(Refusal::SignedInTheFuture);
    }
    if seen.is_some_and(|seen| seen <= latest_believable && signed_at < seen) {
        return Err(Refusal::StaleManifest);
    }
    Ok(())
}

/// The replay mark to store after a manifest signed at `accepted` passed: never lower than the stored one, so a
/// clock set back cannot lower it.
pub(crate) fn next_replay_mark(stored: Option<u64>, accepted: u64) -> u64 {
    stored.map_or(accepted, |stored| stored.max(accepted))
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
/// the legacy feed cannot offer it, on any OS.
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
    /// The signed time of the manifest's signature: the channel's next replay mark.
    pub(crate) signed_at: u64,
    platforms: std::collections::BTreeMap<String, RawPlatform>,
}

/// Checks a `desktop/v2/<channel>.json` manifest and its detached signature. In order: signed by the v2 key (the
/// legacy key is [`Refusal::KeyScope`], any other key or other bytes [`Refusal::ManifestSignature`]); the signature
/// signs this channel's manifest file name (another channel's manifest, validly signed, stops here); it parses; it
/// is for this product and channel; its version is a plain one at or above the first native release; its signed
/// time is inside the bounds of [`check_signed_time`].
pub(crate) fn verify_v2_manifest(
    keys: &TrustedKeys,
    channel: &str,
    manifest: &[u8],
    detached_signature: &str,
    now: u64,
    seen: Option<u64>,
) -> Result<SignedManifest, Refusal> {
    let key = keys.signer_on(Line::Native, detached_signature, Refusal::ManifestSignature)?;
    let parsed = minisign_text(detached_signature)
        .and_then(|text| minisign_verify::Signature::decode(&text).ok())
        .ok_or(Refusal::ManifestSignature)?;
    // The trusted comment (file name and time) is verified with the bytes.
    key.key
        .verify(manifest, &parsed, true)
        .map_err(|_| Refusal::ManifestSignature)?;
    if signed_file_name(detached_signature).as_deref() != v2_manifest_file(channel) {
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
    // A signature without a readable time has no place inside the bounds.
    let signed_at = signed_time(detached_signature).ok_or(Refusal::ManifestUnreadable)?;
    check_signed_time(signed_at, now, seen)?;
    Ok(SignedManifest {
        version: raw.version,
        signed_at,
        platforms: raw.platforms,
    })
}

impl SignedManifest {
    /// Whether the update the Tauri updater read (its version, address and signature) is one this signed manifest
    /// offers for this PC (`target` is the updater's own, `windows-x86_64`: the entry's key is that target or
    /// `<target>-<installer kind>`, so another architecture's entry cannot stand in), at an address we trust, with a length, signed by the v2 key (the same line as the manifest; the
    /// legacy key is [`Refusal::KeyScope`]) for that file and this version. Returns the key the artefact's bytes
    /// must verify with.
    pub(crate) fn artifact<'k>(
        &self,
        keys: &'k TrustedKeys,
        target: &str,
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
            .filter(|(key, _)| {
                !target.is_empty()
                    && key.strip_prefix(target).is_some_and(|rest| {
                        rest.is_empty()
                            || rest
                                .strip_prefix('-')
                                .is_some_and(|kind| !kind.is_empty() && !kind.contains('-'))
                    })
            })
            .map(|(_, platform)| platform)
            .find(|platform| platform.url == download_url && platform.signature == signature)
            .ok_or(Refusal::NotInManifest)?;
        trusted_file_name(&platform.url).ok_or(Refusal::UntrustedAddress)?;
        if !platform
            .size
            .is_some_and(|size| (1..=MAX_ARTIFACT_BYTES).contains(&size))
        {
            return Err(Refusal::BadSize);
        }
        let key = keys.signer_on(Line::Native, &platform.signature, Refusal::ArtifactSignature)?;
        if !artifact_names_version(&self.version, &platform.url, &platform.signature) {
            return Err(Refusal::SignatureForOtherFile);
        }
        Ok(key)
    }
}

/// Where an update came from.
pub(crate) enum Feed<'a> {
    /// Today's unsigned `desktop/<channel>.json`: the legacy line.
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
pub(crate) struct Host<'a> {
    /// A Mac's artefact (`Beebeeb.app.tar.gz`) carries no version, and a Mac never installs in place (task 2176).
    pub(crate) macos: bool,
    /// The updater's target for this PC (`windows-x86_64`): which entries of a signed manifest are this PC's.
    pub(crate) target: &'a str,
}

/// The one decision before an update is offered or installed. Returns the key its bytes must verify with: the v2
/// key for an update of a signed manifest, the legacy key for one of the legacy feed, never the other way round.
/// The legacy feed offers only versions below 0.9.0, on every OS; the signed manifest only 0.9.0 and later.
pub(crate) fn accept<'k>(
    keys: &'k TrustedKeys,
    feed: &Feed<'_>,
    host: Host<'_>,
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
            manifest.artifact(keys, host.target, version, download_url, signature)
        }
        Feed::Unsigned => {
            if is_native_line(version) {
                return Err(Refusal::NativeFromUnsignedFeed);
            }
            let key = keys.signer_on(Line::Legacy, signature, Refusal::ArtifactSignature)?;
            if !host.macos && !artifact_names_version(version, download_url, signature) {
                return Err(Refusal::SignatureForOtherFile);
            }
            Ok(key)
        }
    }
}
