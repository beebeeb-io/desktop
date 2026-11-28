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
        installer.start_nsis(stopped, crate::engine_host::launch::spawn)
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
        // The engine is read once more, through its slot, before the person is asked about. The stop held that slot
        // for seconds, and whatever waited on it behind the stop gets its turn here, ahead of this read (the slot is
        // served in order): a lock that closed on the stop's end has landed by the time presence is asked below.
        // Asked the moment the stop returned, presence still said "open" (dc-171). And a sync started again since the
        // stop makes the proof stale: no installer starts over a running runner, and nothing is asked back.
        if self.engine.is_syncing() {
            log("settings: sync runs again since the stop; the installer was not started");
            self.stopped_running = false;
            return false;
        }
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

/// A Mac only downloads updates: nothing stops, nothing is installed, no installer is started. The one thing it does
/// is open the release's download page in the browser (NSWorkspace, never `/usr/bin/open`).
#[cfg(any(target_os = "macos", test))]
pub(crate) struct MacApply {
    /// Opens an address in the browser and says whether it opened. A test hands in its own: no test opens a page.
    open: Box<dyn FnMut(&str) -> bool>,
}

#[cfg(any(target_os = "macos", test))]
impl MacApply {
    /// The release's hand-off: NSWorkspace.
    #[cfg_attr(test, allow(dead_code))]
    pub(crate) fn new() -> Self {
        Self::opening_with(Box::new(open_in_browser))
    }

    pub(crate) fn opening_with(open: Box<dyn FnMut(&str) -> bool>) -> Self {
        Self { open }
    }
}

#[cfg(any(target_os = "macos", test))]
impl Apply for MacApply {
    /// Never reached (the machine refuses an install on a Mac first): no proof is minted, so no installer starts.
    fn stop_engine(&mut self) -> Option<EngineStopped> {
        None
    }

    fn launch(&mut self, _installer: &VerifiedInstaller, _offer: &Offer, _stopped: EngineStopped) -> bool {
        false
    }

    fn resume_engine(&mut self) {}

    /// The address is the offer's own, which the policy only accepts on the release host over https.
    fn open_download_page(&mut self, offer: &Offer) -> bool {
        offer.url.starts_with("https://") && (self.open)(&offer.url)
    }
}

#[cfg(all(target_os = "macos", not(test)))]
fn open_in_browser(address: &str) -> bool {
    use objc2_app_kit::NSWorkspace;
    use objc2_foundation::{NSString, NSURL};
    let Some(url) = NSURL::URLWithString(&NSString::from_str(address)) else {
        return false;
    };
    NSWorkspace::sharedWorkspace().openURL(&url)
}

/// A test build opens nothing.
#[cfg(all(test, target_os = "macos"))]
#[allow(dead_code)]
fn open_in_browser(_address: &str) -> bool {
    false
}

#[cfg(all(test, not(target_os = "macos")))]
#[allow(dead_code)]
fn open_in_browser(_address: &str) -> bool {
    false
}

/// An update while sync runs (uploads and a hydration are tasks of the engine's one runner): the installer starts
/// only after the runner's stop is confirmed, and when it does not start, the sync that was stopped is asked back.
#[cfg(test)]
mod active_work_tests {
    use std::cell::Cell;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Arc, Mutex};

    use beebeeb_engine::identity::TEST;
    use beebeeb_engine::signin::memory_store::MemoryCredentials;
    use beebeeb_updater::testkit::{self, FakeClock, FakeHttp, MemStore, Publish, TestKey};
    use beebeeb_updater::{Channel, Config, Event, Failure, Intent, State, Updater};

    use beebeeb_engine::app::EngineConfig;
    use beebeeb_engine::host::ChannelHost;
    use beebeeb_engine::runner::EngineRunner;
    use beebeeb_engine::sync_view::Start;

    use super::*;

    fn engine() -> Arc<Engine<MemoryCredentials>> {
        let (host, _messages) = ChannelHost::new(|| {});
        let config = EngineConfig {
            account_id: "updates-apply-active-work".into(),
            state_dir: crate::engine_host::tests::process_state_dir(),
            api_base: "http://127.0.0.1:9".to_string(),
        };
        let engine = Engine::open(&TEST, config, host, MemoryCredentials::default()).expect("the engine opens");
        engine.restore();
        Arc::new(engine)
    }

    /// A running runner whose task ends when aborted (the slot's task stands for uploads and a hydration).
    fn hold_running_stoppable(engine: &Engine<MemoryCredentials>) -> tokio::runtime::Runtime {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(1)
            .enable_all()
            .build()
            .expect("a runtime for the slot's task");
        let task = runtime.spawn(std::future::pending::<()>());
        let account = engine.accounts().active_account().expect("the account");
        *account.engine.try_lock().expect("the engine slot is free") = Some(EngineRunner::for_test_with_task(task));
        engine.set_view_for_test(|view| view.start = Start::Running);
        runtime
    }

    fn pressed() -> Arc<Presence> {
        let presence = Presence::open();
        assert!(presence.press());
        presence
    }

    type Machine = Updater<FakeHttp, FakeClock, MemStore, EngineApply<MemoryCredentials>>;

    fn settle(updater: &mut Machine, event: Event) -> State {
        let mut state = updater.step(event).clone();
        for _ in 0..100_000 {
            if !matches!(
                state,
                State::Checking { .. } | State::Rechecking { .. } | State::Downloading { .. } | State::Verifying { .. }
            ) {
                return state;
            }
            state = updater.step(Event::Tick).clone();
        }
        panic!("the updater did not settle");
    }

    fn ready(
        dir: &tempfile::TempDir,
        apply: EngineApply<MemoryCredentials>,
    ) -> Machine {
        let key = TestKey::generate();
        let mut http = FakeHttp::default();
        Publish::new(Channel::Stable, "0.9.1", testkit::artifact_bytes(300_000)).serve(&mut http, &key);
        let config = Config {
            trust: testkit::trust(&key),
            base_url: testkit::BASE.to_string(),
            installed: testkit::version("0.9.0"),
            staging_dir: dir.path().to_path_buf(),
        };
        let mut updater = Updater::new(config, http, FakeClock(Cell::new(testkit::NOW)), MemStore::default(), apply);
        let check = Event::Check {
            channel: Channel::Stable,
            intent: Intent::Background,
        };
        assert!(matches!(settle(&mut updater, check), State::Available { .. }));
        assert!(matches!(
            settle(&mut updater, Event::Download),
            State::ReadyToRestart { .. }
        ));
        updater
    }

    /// The installer's start sees the runner stopped (its uploads and hydration ended with it, the queue in
    /// `state.db` stays); a start that fails asks sync back, once.
    #[test]
    fn the_installer_never_starts_over_a_running_runner_and_a_failed_start_asks_sync_back() {
        for starts in [true, false] {
            let engine = engine();
            let _running = hold_running_stoppable(&engine);
            let sync_back = SyncBack::default();
            let syncing_at_start = Arc::new(Mutex::new(Vec::new()));
            let (seen, watched) = (syncing_at_start.clone(), engine.clone());
            let start: StartInstaller = Box::new(move |_, _| {
                seen.lock().unwrap().push(watched.is_syncing());
                starts
            });
            let apply = EngineApply::starting_with(engine.clone(), &TEST, sync_back.clone(), pressed(), start);
            let dir = tempfile::tempdir().unwrap();
            let mut updater = ready(&dir, apply);
            assert!(engine.is_syncing());
            let after = updater.step(Event::Apply).clone();
            assert_eq!(*syncing_at_start.lock().unwrap(), [false], "the runner had stopped");
            assert!(!engine.is_syncing());
            if starts {
                assert!(matches!(after, State::Restarting { .. }));
                assert!(!sync_back.take(), "the installer replaces this program: no sync back");
            } else {
                assert!(matches!(
                    after,
                    State::Failed {
                        failure: Failure::InstallerDidNotStart,
                        ..
                    }
                ));
                assert!(sync_back.take(), "sync that the hand-off stopped is asked back");
            }
        }
    }

    /// The person locked Beebeeb during the stop (seconds with uploads running): no installer, the stopped sync
    /// is asked back, the download is kept.
    #[test]
    fn a_lock_during_the_stop_starts_no_installer_and_asks_sync_back() {
        let engine = engine();
        let _running = hold_running_stoppable(&engine);
        let still_here = Arc::new(AtomicBool::new(true));
        let presence = Presence::from_fns(
            {
                let here = still_here.clone();
                move || (Some(beebeeb_engine::account::SessionGeneration::for_test(1)), here.load(Ordering::SeqCst))
            },
            || 0,
            || false,
        );
        assert!(presence.press());
        let sync_back = SyncBack::default();
        let flip = still_here.clone();
        let watched = engine.clone();
        // The lock closes when the runner has stopped.
        let start: StartInstaller = Box::new(|_, _| panic!("no installer starts after a lock"));
        let apply = EngineApply::starting_with(engine.clone(), &TEST, sync_back.clone(), presence, start);
        let dir = tempfile::tempdir().unwrap();
        let mut updater = ready(&dir, apply);
        let closer = std::thread::spawn(move || {
            while watched.is_syncing() {
                std::thread::sleep(std::time::Duration::from_millis(1));
            }
            flip.store(false, Ordering::SeqCst);
        });
        let after = updater.step(Event::Apply).clone();
        closer.join().unwrap();
        assert!(matches!(after, State::Failed { failure: Failure::InstallerDidNotStart, .. }));
        assert!(sync_back.take());
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1, "the download is kept");
    }
}
