//! Beebeeb desktop updater: no Tauri, no UI, no network or clock of its own.
//!
//! Design and decisions: `docs/superpowers/specs/2026-11-11-native-installer-updater.md`.
//!
//! - [`manifest`]: the channel manifest, in the format 0.8.x reads, plus three additions old clients ignore.
//! - [`signature`]: minisign in the form Tauri's signer writes; the key 0.8.x bakes in verifies both.
//! - [`policy`]: the rules a manifest passes before a byte is downloaded (signed, right channel, not replayed,
//!   trusted address, a signature that names the file and the version, never below the first native release).
//! - [`machine`]: one [`State`] and one `step`: check, resumable download, verify, stage, hand to the installer.
//! - [`host`]: the small real pieces: the system clock, the JSON record on disk, the NSIS launch (Windows).
//! - `net` (feature `net`): the https client.

pub mod host;
pub mod machine;
pub mod manifest;
#[cfg(feature = "net")]
pub mod net;
pub mod policy;
pub mod signature;
pub mod version;

#[cfg(test)]
mod machine_tests;
#[cfg(test)]
mod policy_tests;
#[cfg(test)]
mod real_release_tests;
#[cfg(test)]
mod testkit;

pub use machine::{
    Apply, Clock, Config, Event, Failure, Http, Intent, Persisted, Response, State, Store, Unreachable, Updater,
};
pub use manifest::{Channel, Manifest, ManifestError, Platform};
pub use policy::{HighWater, NATIVE_FLOOR, Offer, Refusal, Trust, Verdict};
pub use signature::{BEEBEEB_DESKTOP_PUBKEY, PublicKey, Signature, SignatureError};
pub use version::Version;
