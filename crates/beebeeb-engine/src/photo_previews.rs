//! Photo previews made on this PC (PREV-1): the account's preview setting, a JPEG preview of a photo whose original is
//! already here, sealed the way the iPhone app seals one, and the run that makes and uploads them.
//!
//! - [`seal`]: the wire recipe of a preview (`nonce(12) || ciphertext || tag(16)` under the file's derived key), so a
//!   preview made here opens on the phone and one made on the phone opens here.
//! - [`encode`]: an original on this PC to a JPEG preview, in memory only (never upscaled, EXIF orientation applied,
//!   no metadata in the output).
//! - [`plan`]: which photos get a preview: the owner's own photos whose bytes are all on this PC (never a placeholder,
//!   so nothing is ever downloaded for it), in a format this PC decodes.
//! - [`runner`]: one run at a time, stoppable, its state for a window to show.
//!
//! The account's setting is the one every device reads (`GET/PUT /api/v1/preferences/photo_previews`). This PC makes
//! JPEG previews only: with HEIC chosen it makes none (it has no HEIC encoder) and says so.
//!
//! Must not: write a preview's plaintext to disk (no temp file, no cache); send anything but the sealed blob and its
//! `format`/`px`; log a name, a path, an id, a key or a byte value.

pub mod encode;
pub mod plan;
pub mod runner;
pub mod seal;
pub mod thumbs_upload;

#[cfg(test)]
pub(crate) mod tests;

/// The sizes a preview can be made at: the long edge in pixels (the server takes these four only).
pub const PX_CHOICES: [u32; 4] = [1280, 1920, 2560, 3840];

/// The format of a stored preview.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PreviewFormat {
    Heic,
    Jpeg,
}

impl PreviewFormat {
    /// As the server and the preference name it.
    pub fn wire(self) -> &'static str {
        match self {
            Self::Heic => "heic",
            Self::Jpeg => "jpeg",
        }
    }

    pub fn from_wire(word: &str) -> Option<Self> {
        match word {
            "heic" => Some(Self::Heic),
            "jpeg" => Some(Self::Jpeg),
            _ => None,
        }
    }
}

/// The account's preview setting (`photo_previews`): on or off, the format and the long edge.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PreviewSetting {
    pub enabled: bool,
    pub format: PreviewFormat,
    pub px: u32,
}

impl Default for PreviewSetting {
    /// What the server answers for an account that never set it.
    fn default() -> Self {
        Self {
            enabled: false,
            format: PreviewFormat::Heic,
            px: 2560,
        }
    }
}

impl PreviewSetting {
    /// Read leniently: each field that is missing or not one of its values falls back to its default (the
    /// server validates what it stores; an older client may have stored less).
    pub fn from_value(value: &serde_json::Value) -> Self {
        let default = Self::default();
        Self {
            enabled: value
                .get("enabled")
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(default.enabled),
            format: value
                .get("format")
                .and_then(serde_json::Value::as_str)
                .and_then(PreviewFormat::from_wire)
                .unwrap_or(default.format),
            px: value
                .get("px")
                .and_then(serde_json::Value::as_u64)
                .and_then(|px| u32::try_from(px).ok())
                .filter(|px| PX_CHOICES.contains(px))
                .unwrap_or(default.px),
        }
    }

    pub fn to_value(self) -> serde_json::Value {
        serde_json::json!({ "enabled": self.enabled, "format": self.format.wire(), "px": self.px })
    }
}

/// One file's facts, as the server lists the owner's media: its version (when it last changed, its size) and whether
/// it has a stored preview, at which setting.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreviewState {
    pub id: String,
    pub has_preview: bool,
    pub format: Option<PreviewFormat>,
    pub px: Option<u32>,
    pub bytes: Option<u64>,
    /// When the server's version of the file last changed (unix seconds).
    pub updated_at: Option<i64>,
    /// The size of the server's version.
    pub size: Option<u64>,
}

impl PreviewState {
    /// The server's version is the one this PC holds: not changed after the version the local state last took
    /// (`remote_updated_at`, the base the engine's own conflict check reads), and the same size. Unknown is not.
    pub fn is_the_version_here(&self, remote_updated_at: i64, bytes: u64) -> bool {
        self.updated_at.is_some_and(|at| at <= remote_updated_at) && self.size == Some(bytes)
    }

    /// A preview at exactly this format and size is stored.
    pub fn matches(&self, format: PreviewFormat, px: u32) -> bool {
        self.has_preview && self.format == Some(format) && self.px == Some(px)
    }
}

/// What the account's previews take (`GET /api/v1/files/previews/summary`): how many and their bytes. The server's
/// count per setting is not read: an average per setting, and how many it was measured on, come from [`averages`].
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct PreviewsSummary {
    pub count: u64,
    pub total_bytes: u64,
}

/// The average bytes of a stored preview at each (format, px), and how many it was measured on, from the per-file
/// facts: only previews whose bytes are known count. A setting with none is absent (never a typical figure).
pub fn averages(states: &[PreviewState]) -> Vec<((PreviewFormat, u32), u64, u64)> {
    let mut sums: Vec<((PreviewFormat, u32), u64, u64)> = Vec::new();
    for state in states.iter().filter(|state| state.has_preview) {
        let (Some(format), Some(px), Some(bytes)) = (state.format, state.px, state.bytes) else {
            continue;
        };
        if bytes == 0 {
            continue;
        }
        match sums.iter_mut().find(|(at, _, _)| *at == (format, px)) {
            Some((_, sum, count)) => {
                *sum += bytes;
                *count += 1;
            }
            None => sums.push(((format, px), bytes, 1)),
        }
    }
    sums.into_iter()
        .map(|(at, sum, count)| (at, sum / count, count))
        .collect()
}
