//! The hand-off note: what the bridge release 0.8.25 writes just before it starts a native installer, and what the
//! native app reads on its first start.
//!
//! `%LOCALAPPDATA%\io.beebeeb.app\update-handoff.json` is `{"from":"0.8.25","to":"0.9.0","channel":"stable",
//! "at":<unix seconds>}`. It says that the Tauri app ran its pre-flight (engine stopped, sync root disconnected,
//! `state.db` checkpointed, credential owner tags written) before the native build replaced it. The installer
//! refuses an install below 0.8.25 by itself (`packaging/windows/installer.nsi`); this note is the app's own check,
//! for a state folder that reached a native build some other way (a copied profile, an installer run by hand).
//!
//! Contract for the shell: call [`arrival`] before the engine opens `state.db`. Only [`Arrival::Fresh`] and
//! [`Arrival::FromBridge`] may open it as it is; on [`Arrival::WithoutHandoff`] the shell runs the engine's own
//! upgrade checks first (or asks the person to sign in again), and never assumes the pre-flight ran.

use std::fs;
use std::path::Path;

use serde::Deserialize;

use crate::manifest::Channel;
use crate::version::Version;

pub const HANDOFF_FILE: &str = "update-handoff.json";
/// The first release that writes the note and the credential owner tags.
pub const BRIDGE: Version = Version::new(0, 8, 25);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Handoff {
    pub from: Version,
    pub to: Version,
    pub channel: Channel,
    pub at: u64,
}

/// How this native build came to its state folder.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Arrival {
    /// No earlier state: a first install.
    Fresh,
    /// The bridge (0.8.25 or a later 0.8.x) handed over to this version.
    FromBridge(Handoff),
    /// Earlier state exists and nothing vouches for it: no note, an unreadable one, one written by a version
    /// below 0.8.25, or one for another version than this build.
    WithoutHandoff,
}

#[derive(Deserialize)]
struct Raw {
    from: String,
    to: String,
    channel: String,
    at: u64,
}

fn read(state_dir: &Path) -> Option<Handoff> {
    let bytes = fs::read(state_dir.join(HANDOFF_FILE)).ok()?;
    let raw: Raw = serde_json::from_slice(&bytes).ok()?;
    Some(Handoff {
        from: Version::parse(&raw.from)?,
        to: Version::parse(&raw.to)?,
        channel: Channel::parse(&raw.channel)?,
        at: raw.at,
    })
}

/// Classifies the first start of the native build `running`. `has_earlier_state` is whether `state.db` exists and
/// no native build has opened it yet (the shell keeps its own marker for "a native build was here").
pub fn arrival(state_dir: &Path, running: Version, has_earlier_state: bool) -> Arrival {
    if !has_earlier_state {
        return Arrival::Fresh;
    }
    match read(state_dir) {
        Some(note) if note.from >= BRIDGE && note.from < crate::policy::NATIVE_FLOOR && note.to == running => {
            Arrival::FromBridge(note)
        }
        _ => Arrival::WithoutHandoff,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const RUNNING: Version = Version::new(0, 9, 0);

    fn with_note(text: &str) -> Arrival {
        let dir = tempfile::tempdir().expect("temp dir");
        fs::write(dir.path().join(HANDOFF_FILE), text).expect("write");
        arrival(dir.path(), RUNNING, true)
    }

    #[test]
    fn only_the_bridge_vouches_for_earlier_state() {
        let empty = tempfile::tempdir().expect("temp dir");
        assert_eq!(arrival(empty.path(), RUNNING, false), Arrival::Fresh);
        assert_eq!(arrival(empty.path(), RUNNING, true), Arrival::WithoutHandoff);

        let note =
            |from: &str, to: &str| format!(r#"{{"from":"{from}","to":"{to}","channel":"stable","at":1800000000}}"#);
        assert_eq!(
            with_note(&note("0.8.25", "0.9.0")),
            Arrival::FromBridge(Handoff {
                from: BRIDGE,
                to: RUNNING,
                channel: Channel::Stable,
                at: 1_800_000_000
            })
        );
        assert!(matches!(with_note(&note("0.8.31", "0.9.0")), Arrival::FromBridge(_)));
        for (from, to, why) in [
            ("0.8.24", "0.9.0", "below the bridge"),
            ("0.8.25", "0.9.1", "for another version"),
            ("0.9.0", "0.9.0", "not from the 0.8.x line"),
            ("0.8.25-beta", "0.9.0", "a version that does not parse"),
        ] {
            assert_eq!(with_note(&note(from, to)), Arrival::WithoutHandoff, "{why}");
        }
        assert_eq!(with_note("{ not json"), Arrival::WithoutHandoff);
        assert_eq!(with_note(r#"{"from":"0.8.25","to":"0.9.0"}"#), Arrival::WithoutHandoff);
    }
}
