//! About's updates: the hand-off ([`EngineApply`]), the machine's `Apply` on this app's engine (see the parent
//! module's doc). The engine stops before the installer starts; the installer is the crate's `VerifiedInstaller`.

use super::*;

/// Starts the verified installer and says whether it started. A release on Windows: the passive NSIS run
/// ([`start_installer`]). A test hands in its own: no test ever starts an installer.
#[cfg(any(windows, test))]
pub(crate) type StartInstaller = Box<dyn FnMut(&VerifiedInstaller, EngineStopped) -> bool>;

/// Windows: the crate's passive NSIS start (`VerifiedInstaller::start_nsis`, the only way to start one). Elsewhere
/// nothing is started.
#[cfg(any(windows, test))]
// A Windows test build never calls it: no test there may start a real installer.
#[cfg_attr(all(windows, test), allow(dead_code))]
fn start_installer(installer: &VerifiedInstaller, stopped: EngineStopped) -> bool {
    #[cfg(windows)]
    {
        installer.start_nsis(stopped, super::super::launch::spawn)
    }
    #[cfg(not(windows))]
    {
        let _ = (installer, stopped);
        false
    }
}

/// "Restart to update": the engine stops before the installer starts, and the installer is the crate's
/// `VerifiedInstaller`, verified again and held at this moment by the machine.
#[cfg(any(windows, test))]
pub(crate) struct EngineApply<C: CredentialStores> {
    engine: Arc<Engine<C>>,
    identity: &'static EngineIdentity,
    /// Where a failed start asks for sync back.
    sync_back: SyncBack,
    /// This hand-off's stop ended a running sync and no installer has started since: a failed start asks for it
    /// back, and so does the drop (the worker ended in the middle of the hand-off). A sync that was not running is
    /// not started by an update that did not happen.
    stopped_running: bool,
    /// The person and the session the press was taken in: asked again right before the installer starts.
    presence: Arc<Presence>,
    start: StartInstaller,
}

#[cfg(any(windows, test))]
impl<C: CredentialStores> EngineApply<C> {
    /// The release's hand-off, with the real installer start ([`start_installer`]).
    #[cfg_attr(all(windows, test), allow(dead_code))]
    pub(crate) fn new(
        engine: Arc<Engine<C>>,
        identity: &'static EngineIdentity,
        sync_back: SyncBack,
        presence: Arc<Presence>,
    ) -> Self {
        Self::starting_with(engine, identity, sync_back, presence, Box::new(start_installer))
    }

    pub(crate) fn starting_with(
        engine: Arc<Engine<C>>,
        identity: &'static EngineIdentity,
        sync_back: SyncBack,
        presence: Arc<Presence>,
        start: StartInstaller,
    ) -> Self {
        Self {
            engine,
            identity,
            sync_back,
            stopped_running: false,
            presence,
            start,
        }
    }

    /// A stop of sync was not confirmed in this process (`presence::any_stop_unconfirmed`).
    fn stop_unconfirmed(&self) -> bool {
        super::presence::any_stop_unconfirmed(&self.engine)
    }
}

/// The worker ended in the middle of the hand-off (a panic after the stop, before the installer started or the
/// machine resumed sync): the sync this hand-off stopped is asked back, so no "sync is off" is left behind by an
/// update that did not happen. The window takes it in `pump` with the worker's end. (The release profile unwinds:
/// no crate here sets `panic = "abort"`.)
#[cfg(any(windows, test))]
impl<C: CredentialStores> Drop for EngineApply<C> {
    fn drop(&mut self) {
        if std::mem::take(&mut self.stopped_running) {
            log("settings: the updater ended during the hand-off; sync is started again");
            self.sync_back.ask();
        }
    }
}

#[cfg(any(windows, test))]
impl<C: CredentialStores> Apply for EngineApply<C> {
    /// The quit path's stop (`EngineActions::stop_for_quit` calls the same `Engine::stop`): the runner stops before
    /// its next operation and the queue in `state.db` stays as it is, so an upload that was cut is sent again by
    /// the next start. A stop the engine did not confirm mints no proof: the installer is not started; nor after
    /// ANY stop in this process that was not confirmed (`stop_unconfirmed`, asked before this stop and after it).
    /// (On Windows the engine also says `Unconfirmed` when its task ended and File Explorer's connection did not let
    /// go; the two are one answer here, so the words say only that the stop was not confirmed: `SYNC_UNCONFIRMED`.)
    fn stop_engine(&mut self) -> Option<EngineStopped> {
        if self.stop_unconfirmed() {
            log("settings: an earlier stop of sync was not confirmed; the update waits for a restart of Beebeeb");
            return None;
        }
        let stopped = match self.engine.stop() {
            SyncStop::Stopped => true,
            SyncStop::WasNotRunning => false,
            SyncStop::Unconfirmed => {
                log("settings: sync did not confirm it stopped; the update was not started");
                return None;
            }
        };
        if self.stop_unconfirmed() {
            log("settings: a stop of sync was not confirmed; the update waits for a restart of Beebeeb");
            // This stop was confirmed (another account's was not): the sync it ended is asked back at once, and the
            // engine's own start decides (it refuses where a stop of that account is not confirmed).
            if stopped {
                self.sync_back.ask();
            }
            return None;
        }
        self.stopped_running = stopped;
        Some(EngineStopped::engine_stopped_and_uploads_parked())
    }

    /// The installer, then the channel that served it is recorded (spec §4: the shell writes
    /// `installed_release_channel` when the installer started). Once the installer started, nothing asks sync back
    /// (the installer is replacing this program), and the record cannot end the worker: a panic in it is caught, as
    /// `set_channel`'s is, so the window still hears `Restarting` and quits.
    fn launch(&mut self, installer: &VerifiedInstaller, offer: &Offer, stopped: EngineStopped) -> bool {
        // The stop can take seconds: Beebeeb may have been locked, or the session changed, meanwhile (or a step that
        // ends it, such as a sign-out, was asked and waits behind this stop). Then no installer starts; the machine
        // resumes the sync this stop ended and keeps the verified file, and the row says the update waits.
        if !self.presence.holds() {
            log("settings: Beebeeb was locked or the session changed during the stop; the installer was not started");
            self.presence.note_refused_at_launch();
            return false;
        }
        if !(self.start)(installer, stopped) {
            return false;
        }
        self.stopped_running = false;
        let served = match offer.channel {
            beebeeb_updater::Channel::Stable => ReleaseChannel::Stable,
            beebeeb_updater::Channel::Beta => ReleaseChannel::Beta,
            beebeeb_updater::Channel::Alpha => ReleaseChannel::Alpha,
        };
        let identity = self.identity;
        let recorded = std::panic::catch_unwind(|| {
            DesktopConfig::update_for(identity, |config| {
                config.installed_release_channel = Some(served);
                (true, ())
            })
            .is_ok()
        })
        .unwrap_or(false);
        if !recorded {
            log("settings: the installer started; the channel it came from was not recorded");
        }
        true
    }

    /// The installer did not start after sync was stopped for it: the shell starts sync again with its own start
    /// (the folder and the first-run state are the shell's), through `SyncBack` and the window's poll. Only a sync
    /// this hand-off stopped.
    fn resume_engine(&mut self) {
        if std::mem::take(&mut self.stopped_running) {
            log("settings: the installer did not start; sync is started again");
            self.sync_back.ask();
        }
    }
}
