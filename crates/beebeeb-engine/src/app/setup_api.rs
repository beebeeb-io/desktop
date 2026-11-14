//! The first run on this computer, for a host without a UI toolkit (the native first-run steps): whether it is
//! still to do, marking it done, and the one switch of its steps that lives in the settings file (notifications).
//!
//! "Still to do" is the settings file's own show-once flag (`known_folder_onboarding_seen`: "the first-run prompt
//! has been shown", written by the 0.8 app's first run too), so a computer that went through the first run before,
//! in either app, is not asked again. No new key is added to the settings file.
//!
//! The two writes are for the session a person pressed under and no other: a session that moved on (a lock, a
//! sign-out, another sign-in) writes nothing. They are this computer's settings, not the account's: the check is
//! that the person who pressed is still the one signed in. Each BLOCKS (it reads or writes the settings file): a
//! worker's, never the UI thread's. Must not: log a path.

use super::{CredentialStores, Engine};
use crate::account::SessionGeneration;
use crate::config::DesktopConfig;

/// Why a first-run read or write did not happen. Nothing was written in either case.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SetupRefused {
    /// The session of the press is not the one in memory any more (or a session change is under way).
    SessionMoved,
    /// The settings file could not be read or written.
    Settings,
}

impl SetupRefused {
    /// The refusal's name, for a log line.
    pub fn kind(self) -> &'static str {
        match self {
            Self::SessionMoved => "session_moved",
            Self::Settings => "settings",
        }
    }
}

impl<C: CredentialStores> Engine<C> {
    /// Whether this computer still has its first run to do. A settings file that cannot be read is an error, never
    /// a "yes": a first run is not shown to a person because a file did not read.
    pub fn first_run_due(&self) -> Result<bool, SetupRefused> {
        DesktopConfig::load_for(self.identity)
            .map(|config| !config.known_folder_onboarding_seen)
            .map_err(|_| SetupRefused::Settings)
    }

    /// WRITES the settings file: the first run on this computer is done. `session` is [`Engine::session_turn`] as
    /// the host read it AT THE PRESS.
    pub fn mark_first_run_done_for(&self, session: SessionGeneration) -> Result<(), SetupRefused> {
        self.setup_write(session, |config| {
            let changed = !config.known_folder_onboarding_seen;
            config.known_folder_onboarding_seen = true;
            changed
        })
    }

    /// WRITES the settings file: whether this computer shows Beebeeb's notifications at all (`notify_allow`, the
    /// switch Settings › Notifications reads). `session` as for [`Engine::mark_first_run_done_for`].
    pub fn set_notifications_allowed_for(&self, session: SessionGeneration, on: bool) -> Result<(), SetupRefused> {
        self.setup_write(session, |config| {
            let changed = config.notify_allow != on;
            config.notify_allow = on;
            changed
        })
    }

    /// The one path of this file's writes: only while the session in memory is still `session`, asked with no
    /// session change half done; then the settings file, under its own lock.
    fn setup_write(
        &self,
        session: SessionGeneration,
        change: impl FnOnce(&mut DesktopConfig) -> bool,
    ) -> Result<(), SetupRefused> {
        self.acct
            .while_session_is(&self.writes, session, |_| ())
            .ok_or(SetupRefused::SessionMoved)?;
        DesktopConfig::update_for(self.identity, |config| (change(config), ())).map_err(|_| SetupRefused::Settings)
    }
}
