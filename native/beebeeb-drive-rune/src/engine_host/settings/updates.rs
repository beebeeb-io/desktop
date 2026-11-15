//! About › Updates (DW-0751) and the Release channel (DW-0789): this PC's updater (`beebeeb-updater`) behind
//! Check now and the channel menu.
//!
//! The updater is one state machine (`beebeeb_updater::Updater`). Here is its host and the part of Settings that
//! drives it:
//! - **The host.** The network is the crate's `HttpsClient` (feature `net`: https only, no redirect followed, a fixed
//!   user agent and nothing else about the person); the clock is the system's; the record is the crate's JSON file
//!   (`updater.json`) and the staging folder (`updates`) in this app's local data folder; the hand-off is
//!   [`EngineApply`]: the engine stops before the NSIS installer starts, and the installer is started only through the
//!   crate's `VerifiedInstaller` (verified again, and held, at that moment).
//! - **What is trusted is fixed in the build.** One key (`keys/updater-v2.pub`, through
//!   `host::windows_release_trust`), one host (`host::RELEASE_BASE_URL`), never below 0.9.0, nothing signed before the
//!   build itself. Nothing here reads a host, a key or a channel address from a setting, a flag or the environment.
//! - **One worker owns the updater.** Every request and every chunk of a download runs there, never on the window's
//!   thread; the window takes what it reported in [`UpdatesPart::pump`]. It checks once at launch and then whenever
//!   `Updater::background_check_due` says so (every 4 hours), which is what About's foot line says.
//! - **Nothing is installed by a check.** A check only asks; an older version on a channel is said (`About::older`)
//!   and never offered; a download and "Restart to update" are separate events that no press sends yet (see
//!   `UpdatesPart::fill`: General never says a download is waiting).
//! - **A build that is not a release takes no update and asks nothing.** Without a build time
//!   (`host::BUILD_NOT_BEFORE`) or a release version the updater would refuse every manifest (`NoBuildTime`); it is
//!   not even started, so not one request leaves this PC, and About says why.
//! - **Not per account.** The updater is this PC's: no request carries an account, an email, a file or a token, and
//!   its answers are not an account's, so they are not stamped with the session the way account reads are.
//!
//! Windows only: the release line of 0.9.0 is the Windows installer. On macOS and Linux no updater is built
//! ([`UpdatesPart::for_this_pc`] is `None`) and About draws the versions only.
//!
//! Must not: log a path, a URL, or a server's words; show a string from the network other than a release's own
//! version number and notes.

// On macOS and Linux the part is never built in a build that ships (no release line there yet): its code is
// compiled, and unused.
#![cfg_attr(not(any(windows, test)), allow(dead_code))]

use std::sync::{Arc, mpsc};
use std::time::{Duration, Instant};

#[cfg(any(windows, test))]
use beebeeb_engine::app::SyncStop;
use beebeeb_engine::app::{CredentialStores, Engine};
use beebeeb_engine::config::{DesktopConfig, ReleaseChannel};
use beebeeb_engine::identity::EngineIdentity;
#[cfg(any(windows, test))]
use beebeeb_updater::{Apply, EngineStopped, Offer, VerifiedInstaller};
use beebeeb_updater::{Clock, Event, Failure, Freshness, Http, Intent, Refusal, State, Store, Updater};
use chrono::{DateTime, Local};

use super::{STILL_SAVING, log};
use crate::frames::settings::source::{About, Channel, OlderChannel, ReleaseNotes, UpdateCheck};

/// How often the worker asks the updater whether a background check is due (the updater's own rule decides).
const LOOK_EVERY: Duration = Duration::from_secs(60);
/// Download progress is reported at most this often; the state it settles in always is.
const PROGRESS_EVERY: Duration = Duration::from_millis(250);

/// What About says, word for word. Fixed sentences: no text from the network is ever shown.
pub const NOT_A_RELEASE: &str = "This build is not a release, so it takes no updates.";
pub const NO_UPDATER: &str = "The updater did not start. Restart Beebeeb to check for updates.";
pub const NOT_CHECKED: &str = "Not checked yet.";
pub const CHANNEL_NOT_SAVED: &str = "Couldn't save the release channel on this PC.";
pub const NOT_VERIFIED: &str = "The update information could not be verified, so nothing was downloaded.";
pub const UNREADABLE: &str = "The update information could not be read, so nothing was downloaded.";
pub const OUT_OF_DATE: &str = "The release server sent out-of-date update information, so nothing was downloaded.";
pub const FUTURE: &str =
    "The update information is dated ahead of this PC's clock, so nothing was downloaded. Check the date and time.";
pub const OTHER_HOST: &str = "The update is not on Beebeeb's release server, so nothing was downloaded.";
pub const NO_BUILD: &str = "This channel has no build for this PC.";
pub const BELOW_FLOOR: &str = "This channel offers a version this app cannot go back to, so nothing was downloaded.";
pub const NOTHING_PUBLISHED: &str = "The release server has no update information for this channel.";
pub const NOT_SIGNED_FILE: &str =
    "The download was not the file Beebeeb signed. It was deleted and nothing was installed.";
pub const STOPPED_DOWNLOAD: &str = "The download stopped before it was complete. Nothing was installed.";
pub const OFFER_CHANGED: &str = "The version on offer changed. Nothing was installed.";
pub const NO_DISK: &str = "The update could not be saved on this PC. Nothing was installed.";
pub const SYNC_DID_NOT_STOP: &str = "Sync did not stop, so the update was not started.";
pub const INSTALLER_DID_NOT_START: &str = "The installer did not start. Nothing was installed.";
pub const VERIFYING: &str = "Checking the download's signature…";
pub const RESTARTING: &str = "Restarting to update…";

/// What the worker does with the updater. A trait so the worker runs the real updater and a test's alike; each is
/// built on the worker's thread, so it need not be `Send`.
pub(crate) trait Drive {
    fn step(&mut self, event: Event) -> State;
    fn background_check_due(&self) -> bool;
    fn freshness(&self) -> Freshness;
}

impl<H: Http, C: Clock, S: Store, A: beebeeb_updater::Apply> Drive for Updater<H, C, S, A> {
    fn step(&mut self, event: Event) -> State {
        Updater::step(self, event).clone()
    }
    fn background_check_due(&self) -> bool {
        Updater::background_check_due(self)
    }
    fn freshness(&self) -> Freshness {
        Updater::freshness(self)
    }
}

/// Builds the updater on the worker's thread.
pub(crate) type MakeDrive = Box<dyn FnOnce() -> Box<dyn Drive> + Send>;

/// One state the worker reached, for the order numbered `serial` (0: the launch check and every background check).
pub(crate) struct Report {
    serial: u64,
    state: State,
    freshness: Freshness,
    at: DateTime<Local>,
}

/// An order for the worker: an event for the machine, numbered.
struct Order {
    serial: u64,
    event: Event,
}

/// The window's end of the worker.
struct Worker {
    orders: mpsc::Sender<Order>,
    reports: mpsc::Receiver<Report>,
}

enum Mode {
    /// The updater runs on its worker.
    Live(Worker),
    /// Not a release build: no updater, no request.
    NotARelease,
    /// The worker could not be started.
    NoWorker,
}

/// The release channel the person chose was written to this PC's settings file, or not.
struct Saved {
    channel: Channel,
    saved: bool,
}

/// About's updates, on the updater. See the module's doc.
pub(crate) struct UpdatesPart {
    mode: Mode,
    identity: &'static EngineIdentity,
    /// The channel this PC's settings file holds (what the menu shows).
    channel: Channel,
    /// The channel the running build came from: `installed_release_channel`, else the channel chosen when this app
    /// started (where the running build was taken from, as far as this PC knows).
    from: Channel,
    /// This build's version, as About says it.
    installed: String,
    /// The worker's last report.
    last: Option<Report>,
    /// The number of the last order sent, and the one a check the person asked for (Check now, a channel) waits on.
    sent: u64,
    asked: Option<u64>,
    /// A channel on its way to the settings file: no second one starts until it is answered.
    saving: Option<Channel>,
    saves: mpsc::Sender<Saved>,
    saved: mpsc::Receiver<Saved>,
    toast_next: Option<&'static str>,
}

fn channel_of(channel: ReleaseChannel) -> Channel {
    match channel {
        ReleaseChannel::Stable => Channel::Stable,
        ReleaseChannel::Beta => Channel::Beta,
        ReleaseChannel::Alpha => Channel::Alpha,
    }
}

fn release_channel(channel: Channel) -> ReleaseChannel {
    match channel {
        Channel::Stable => ReleaseChannel::Stable,
        Channel::Beta => ReleaseChannel::Beta,
        Channel::Alpha => ReleaseChannel::Alpha,
    }
}

fn line(channel: Channel) -> beebeeb_updater::Channel {
    match channel {
        Channel::Stable => beebeeb_updater::Channel::Stable,
        Channel::Beta => beebeeb_updater::Channel::Beta,
        Channel::Alpha => beebeeb_updater::Channel::Alpha,
    }
}

fn busy(state: &State) -> bool {
    matches!(
        state,
        State::Checking { .. } | State::Rechecking { .. } | State::Downloading { .. } | State::Verifying { .. }
    )
}

/// The worker: the launch check, then each order, and a background check whenever one is due. Ends when the
/// window's end is dropped.
fn work(
    make: MakeDrive,
    channel: beebeeb_updater::Channel,
    orders: mpsc::Receiver<Order>,
    reports: mpsc::Sender<Report>,
) {
    let mut updater = make();
    let mut channel = channel;
    let background = |channel| Event::Check {
        channel,
        intent: Intent::Background,
    };
    if !run(&mut *updater, 0, background(channel), &reports) {
        return;
    }
    loop {
        let alive = match orders.recv_timeout(LOOK_EVERY) {
            Ok(Order { serial, event }) => {
                if let Event::Check { channel: asked, .. } = &event {
                    channel = *asked;
                }
                run(&mut *updater, serial, event, &reports)
            }
            Err(mpsc::RecvTimeoutError::Timeout) if updater.background_check_due() => {
                run(&mut *updater, 0, background(channel), &reports)
            }
            Err(mpsc::RecvTimeoutError::Timeout) => true,
            Err(mpsc::RecvTimeoutError::Disconnected) => false,
        };
        if !alive {
            return;
        }
    }
}

/// One event, then `Tick`s until the machine has no work left. `false`: the window's end is gone.
fn run(updater: &mut dyn Drive, serial: u64, event: Event, reports: &mpsc::Sender<Report>) -> bool {
    let report = |state: &State, updater: &dyn Drive| Report {
        serial,
        state: state.clone(),
        freshness: updater.freshness(),
        at: Local::now(),
    };
    let mut state = updater.step(event);
    if reports.send(report(&state, updater)).is_err() {
        return false;
    }
    let mut said = Instant::now();
    while busy(&state) {
        state = updater.step(Event::Tick);
        if !busy(&state) || said.elapsed() >= PROGRESS_EVERY {
            if reports.send(report(&state, updater)).is_err() {
                return false;
            }
            said = Instant::now();
        }
    }
    true
}

impl UpdatesPart {
    /// The updater of this PC, when this build has one: a release build of the Windows app. `None` on macOS and
    /// Linux, and in every test build of this crate (a test hands in its own with [`UpdatesPart::live`]): no test
    /// can reach the release server through it.
    pub(crate) fn for_this_pc<C: CredentialStores + 'static>(
        engine: &Arc<Engine<C>>,
        identity: &'static EngineIdentity,
        config: Option<&DesktopConfig>,
    ) -> Option<Self> {
        #[cfg(all(windows, not(test)))]
        {
            let (channel, from) = channels(config);
            let installed = super::version_words();
            Some(match release::updater(engine.clone(), identity) {
                Some(make) => Self::live(make, identity, channel, from, installed),
                None => Self::not_a_release(identity, channel, from, installed),
            })
        }
        #[cfg(not(all(windows, not(test))))]
        {
            let _ = (engine, identity, config);
            None
        }
    }

    /// The part on an updater `make` builds on the worker's thread.
    #[cfg(any(windows, test))]
    pub(crate) fn live(
        make: MakeDrive,
        identity: &'static EngineIdentity,
        channel: Channel,
        from: Channel,
        installed: String,
    ) -> Self {
        let (orders, inbox) = mpsc::channel();
        let (reports, taken) = mpsc::channel();
        let first = line(channel);
        let started = std::thread::Builder::new()
            .name("settings-updates".into())
            .spawn(move || work(make, first, inbox, reports));
        let mode = match started {
            Ok(_) => Mode::Live(Worker { orders, reports: taken }),
            Err(_) => {
                log("settings: could not start a thread for the updater");
                Mode::NoWorker
            }
        };
        Self::with_mode(mode, identity, channel, from, installed)
    }

    /// The part of a build that takes no update: nothing is started and nothing is asked.
    pub(crate) fn not_a_release(
        identity: &'static EngineIdentity,
        channel: Channel,
        from: Channel,
        installed: String,
    ) -> Self {
        Self::with_mode(Mode::NotARelease, identity, channel, from, installed)
    }

    fn with_mode(
        mode: Mode,
        identity: &'static EngineIdentity,
        channel: Channel,
        from: Channel,
        installed: String,
    ) -> Self {
        let (saves, saved) = mpsc::channel();
        Self {
            mode,
            identity,
            channel,
            from,
            installed,
            last: None,
            sent: 0,
            asked: None,
            saving: None,
            saves,
            saved,
            toast_next: None,
        }
    }

    /// Sends `event` to the worker. `false`: no worker takes it.
    fn send(&mut self, event: Event) -> bool {
        let Mode::Live(worker) = &self.mode else {
            return false;
        };
        self.sent += 1;
        let sent = worker
            .orders
            .send(Order {
                serial: self.sent,
                event,
            })
            .is_ok();
        if !sent {
            log("settings: the updater's worker has ended");
            self.mode = Mode::NoWorker;
        }
        sent
    }

    /// Check now: a check the person asked for (it also reports an older channel build, which is never offered).
    pub(crate) fn check_now(&mut self) {
        let event = Event::Check {
            channel: line(self.channel),
            intent: Intent::Manual,
        };
        if self.send(event) {
            self.asked = Some(self.sent);
        }
    }

    /// Writes the channel to this PC's settings file on a worker; the menu shows it, and the new channel is checked,
    /// only once the file took it. A refusal keeps the old channel and says so.
    pub(crate) fn set_channel(&mut self, channel: Channel) {
        if self.saving.is_some() {
            self.toast_next = Some(STILL_SAVING);
            return;
        }
        if channel == self.channel {
            return;
        }
        let (identity, saves) = (self.identity, self.saves.clone());
        let started = std::thread::Builder::new()
            .name("settings-channel".into())
            .spawn(move || {
                let saved = std::panic::catch_unwind(|| {
                    DesktopConfig::update_for(identity, |config| {
                        let wanted = release_channel(channel);
                        let changed = config.release_channel != wanted;
                        config.release_channel = wanted;
                        (changed, ())
                    })
                    .is_ok()
                })
                .unwrap_or(false);
                let _ = saves.send(Saved { channel, saved });
            });
        if started.is_ok() {
            self.saving = Some(channel);
        } else {
            log("settings: could not start a thread to save the release channel");
            self.toast_next = Some(CHANNEL_NOT_SAVED);
        }
    }

    /// Take what the worker and the channel's writer answered: (changed, a toast, busy).
    pub(crate) fn pump(&mut self) -> (bool, Option<&'static str>, bool) {
        let mut toast = self.toast_next.take();
        let mut changed = toast.is_some();
        while let Ok(Saved { channel, saved }) = self.saved.try_recv() {
            changed = true;
            self.saving = None;
            if saved {
                self.channel = channel;
                // Another channel: what the last check said is not about it. Asked as Check now asks, so that an
                // older build on it is said (and, as ever, not offered).
                self.last = None;
                self.check_now();
            } else {
                log("settings: the release channel was not saved");
                toast = toast.or(Some(CHANNEL_NOT_SAVED));
            }
        }
        if let Mode::Live(worker) = &self.mode {
            while let Ok(report) = worker.reports.try_recv() {
                changed = true;
                if self.asked.is_some_and(|asked| report.serial >= asked) && !busy(&report.state) {
                    self.asked = None;
                }
                self.last = Some(report);
            }
        }
        let working = self.last.as_ref().is_some_and(|report| busy(&report.state));
        (changed, toast, self.asked.is_some() || self.saving.is_some() || working)
    }

    /// About's update rows: the check's words, the channel the settings file holds, an older channel build (said,
    /// never offered), and the notes of a version on offer.
    pub(crate) fn fill(&self, about: &mut About) {
        about.channel = self.channel;
        about.check = self.check();
        about.older = self.older();
        if let Some(offer) = self.offer() {
            about.notes = ReleaseNotes {
                version: offer.version.to_string(),
                channel: self.channel,
                released: String::new(),
                lines: offer
                    .notes
                    .lines()
                    .map(str::trim)
                    .filter(|line| !line.is_empty())
                    .map(str::to_string)
                    .collect(),
            };
        }
    }

    /// The state About draws: what a background check keeps on screen while it runs.
    fn shown(&self) -> Option<&State> {
        let mut state = &self.last.as_ref()?.state;
        while let State::Rechecking { kept, .. } = state {
            state = kept;
        }
        Some(state)
    }

    fn offer(&self) -> Option<&beebeeb_updater::Offer> {
        match self.shown()? {
            State::Available { offer }
            | State::Downloading { offer, .. }
            | State::Verifying { offer }
            | State::ReadyToRestart { offer, .. }
            | State::Restarting { offer } => Some(offer),
            _ => None,
        }
    }

    fn older(&self) -> Option<OlderChannel> {
        match self.shown()? {
            State::Available { offer } if offer.downgrade => Some(OlderChannel {
                channel: self.channel,
                version: offer.version.to_string(),
                runs: self.installed.clone(),
                from: self.from,
            }),
            _ => None,
        }
    }

    fn check(&self) -> UpdateCheck {
        match &self.mode {
            Mode::NotARelease => return UpdateCheck::Checked(NOT_A_RELEASE.into()),
            Mode::NoWorker => return UpdateCheck::Checked(NO_UPDATER.into()),
            Mode::Live(_) => {}
        }
        if self.asked.is_some() || self.saving.is_some() {
            return UpdateCheck::Checking;
        }
        let (Some(report), Some(state)) = (self.last.as_ref(), self.shown()) else {
            // The launch check is on its way.
            return UpdateCheck::Checking;
        };
        let when = when(report.at, Local::now());
        let checked = |words: String| UpdateCheck::Checked(words);
        match state {
            State::Idle => checked(NOT_CHECKED.into()),
            State::Checking { .. } | State::Rechecking { .. } => UpdateCheck::Checking,
            State::UpToDate { installed, .. } => checked(format!("{installed} is the newest version · checked {when}")),
            State::Available { offer } if offer.downgrade => checked(format!(
                "No newer version on the {} channel · checked {when}",
                self.channel.label()
            )),
            State::Available { offer } => checked(format!("Version {} is available · checked {when}", offer.version)),
            State::Downloading {
                offer, received, total, ..
            } => checked(format!(
                "Downloading version {}: {} %",
                offer.version,
                received.saturating_mul(100) / (*total).max(1)
            )),
            State::Verifying { .. } => checked(VERIFYING.into()),
            State::ReadyToRestart { offer, .. } => {
                checked(format!("Version {} is downloaded and verified.", offer.version))
            }
            State::Restarting { .. } => checked(RESTARTING.into()),
            State::Failed { failure, .. } => failed(*failure, report.freshness),
        }
    }
}

/// When a check was answered, as the row says it: "today, 09:12", or "13 Nov, 09:12".
fn when(at: DateTime<Local>, now: DateTime<Local>) -> String {
    if at.date_naive() == now.date_naive() {
        format!("today, {}", at.format("%H:%M"))
    } else {
        at.format("%-d %b, %H:%M").to_string()
    }
}

/// Why a check, a download or the hand-off did not go through, in fixed words. "Try again in a moment" only where a
/// moment can help (no answer, or the server's own trouble); a refusal says what was refused and that nothing was
/// downloaded or installed. With nothing verified for 14 days, it says that too.
pub(crate) fn failed(failure: Failure, freshness: Freshness) -> UpdateCheck {
    let words: &str = match failure {
        Failure::Offline => "",
        Failure::ServerStatus(404) => NOTHING_PUBLISHED,
        Failure::ServerStatus(_) => "",
        Failure::Refused(refusal) => refused(refusal),
        Failure::WrongSize | Failure::BadSignature => NOT_SIGNED_FILE,
        Failure::DownloadInterrupted { .. } => STOPPED_DOWNLOAD,
        Failure::DowngradeChanged => OFFER_CHANGED,
        Failure::Disk => NO_DISK,
        Failure::EngineStillRunning => SYNC_DID_NOT_STOP,
        Failure::InstallerDidNotStart => INSTALLER_DID_NOT_START,
    };
    match (words, freshness) {
        ("", Freshness::Verified) => UpdateCheck::Failed,
        ("", Freshness::Unverified { days }) => UpdateCheck::Checked(format!(
            "Couldn’t reach the release server. No update has been verified for {days} days."
        )),
        (words, Freshness::Verified) => UpdateCheck::Checked(words.into()),
        (words, Freshness::Unverified { days }) => {
            UpdateCheck::Checked(format!("{words} No update has been verified for {days} days."))
        }
    }
}

pub(crate) fn refused(refusal: Refusal) -> &'static str {
    match refusal {
        Refusal::ManifestSignature
        | Refusal::ArtifactSignature
        | Refusal::SignatureForOtherFile
        | Refusal::KeyNotValidForVersion
        | Refusal::WrongChannel => NOT_VERIFIED,
        Refusal::ManifestUnreadable | Refusal::BadSize => UNREADABLE,
        Refusal::StaleManifest | Refusal::OlderThanThisBuild => OUT_OF_DATE,
        Refusal::SignedInTheFuture => FUTURE,
        Refusal::UntrustedAddress => OTHER_HOST,
        Refusal::NoBuildForThisPc => NO_BUILD,
        Refusal::BelowFloor => BELOW_FLOOR,
        Refusal::NoBuildTime => NOT_A_RELEASE,
    }
}

/// The channel the settings file holds, and the one the running build came from.
fn channels(config: Option<&DesktopConfig>) -> (Channel, Channel) {
    let chosen = config.map_or(ReleaseChannel::Stable, |config| config.release_channel);
    let from = config
        .and_then(|config| config.installed_release_channel)
        .unwrap_or(chosen);
    (channel_of(chosen), channel_of(from))
}

/// "Restart to update": the engine stops before the installer starts, and the installer is the crate's
/// `VerifiedInstaller`, verified again and held at this moment by the machine.
#[cfg(any(windows, test))]
pub(crate) struct EngineApply<C: CredentialStores> {
    engine: Arc<Engine<C>>,
    identity: &'static EngineIdentity,
}

#[cfg(any(windows, test))]
impl<C: CredentialStores> EngineApply<C> {
    pub(crate) fn new(engine: Arc<Engine<C>>, identity: &'static EngineIdentity) -> Self {
        Self { engine, identity }
    }
}

#[cfg(any(windows, test))]
impl<C: CredentialStores> Apply for EngineApply<C> {
    /// The quit path's stop (`EngineActions::stop_for_quit` calls the same `Engine::stop`): the runner stops before
    /// its next operation and the queue in `state.db` stays as it is, so an upload that was cut is sent again by
    /// the next start. A stop the engine did not confirm mints no proof: the installer is not started.
    fn stop_engine(&mut self) -> Option<EngineStopped> {
        match self.engine.stop() {
            SyncStop::Stopped | SyncStop::WasNotRunning => Some(EngineStopped::engine_stopped_and_uploads_parked()),
            SyncStop::Unconfirmed => {
                log("settings: sync did not confirm it stopped; the update was not started");
                None
            }
        }
    }

    /// Windows: the passive NSIS installer, then the channel that served it is recorded (spec §4: the shell writes
    /// `installed_release_channel` when the installer started). Elsewhere nothing is started.
    fn launch(&mut self, installer: &VerifiedInstaller, offer: &Offer, stopped: EngineStopped) -> bool {
        #[cfg(windows)]
        {
            if !installer.start_nsis(stopped) {
                return false;
            }
            let served = offer.channel;
            let recorded = DesktopConfig::update_for(self.identity, |config| {
                let channel = release_channel(match served {
                    beebeeb_updater::Channel::Stable => Channel::Stable,
                    beebeeb_updater::Channel::Beta => Channel::Beta,
                    beebeeb_updater::Channel::Alpha => Channel::Alpha,
                });
                config.installed_release_channel = Some(channel);
                (true, ())
            });
            if recorded.is_err() {
                log("settings: the installer started; the channel it came from was not recorded");
            }
            true
        }
        #[cfg(not(windows))]
        {
            let _ = (installer, offer, stopped, self.identity);
            false
        }
    }

    /// The installer did not start after sync was stopped. Starting sync again needs the shell's start (the sync
    /// folder and the first-run state), which this part does not hold: the same gap as the quit path's
    /// `resume_after_cancelled_quit`. Sync starts again with the app's next start of sync.
    fn resume_engine(&mut self) {
        log("settings: the installer did not start; sync stays stopped until it is started again");
    }
}

/// The release build's updater: the trust baked into this build, the one host, this PC's record and folder.
#[cfg(all(windows, not(test)))]
mod release {
    use std::sync::Arc;

    use beebeeb_engine::app::{CredentialStores, Engine};
    use beebeeb_engine::identity::EngineIdentity;
    use beebeeb_updater::host::{FileStore, RELEASE_BASE_URL, SystemClock, windows_release_trust};
    use beebeeb_updater::net::HttpsClient;
    use beebeeb_updater::{Config, Http, Response, Unreachable, Updater, Version};

    use super::{Drive, EngineApply, MakeDrive};
    use crate::engine_host::log;

    /// The https client, or none when it could not be built: every check then ends offline, in sight.
    enum Client {
        Https(HttpsClient),
        None,
    }

    impl Http for Client {
        fn get(&mut self, url: &str, from: u64) -> Result<Response, Unreachable> {
            match self {
                Client::Https(client) => client.get(url, from),
                Client::None => Err(Unreachable),
            }
        }
    }

    /// `None`: not a release build (no build time, or no release version), so no updater runs.
    pub(super) fn updater<C: CredentialStores + 'static>(
        engine: Arc<Engine<C>>,
        identity: &'static EngineIdentity,
    ) -> Option<MakeDrive> {
        beebeeb_updater::host::BUILD_NOT_BEFORE?;
        let installed = Version::parse(option_env!("BEEBEEB_RELEASE_VERSION")?)?;
        let trust = windows_release_trust().ok()?;
        let dirs = rune::dirs::AppDirs::new(crate::shell::identity::AppIdentity::CURRENT.state_dir_id())?;
        let folder = crate::shell::local_data_of(&dirs, "windows");
        Some(Box::new(move || {
            let config = Config {
                trust,
                base_url: RELEASE_BASE_URL.to_string(),
                installed,
                staging_dir: folder.join("updates"),
            };
            let client = HttpsClient::new().map_or_else(
                |_| {
                    log("settings: the updater's https client could not be built");
                    Client::None
                },
                Client::Https,
            );
            let store = FileStore::new(folder.join("updater.json"));
            let apply = EngineApply::new(engine, identity);
            Box::new(Updater::new(config, client, SystemClock, store, apply)) as Box<dyn Drive>
        }))
    }
}

#[cfg(test)]
impl UpdatesPart {
    /// Sends any event to the worker, as a press would (the download and the hand-off have no press yet).
    pub(crate) fn send_for_test(&mut self, event: Event) {
        if self.send(event) {
            self.asked = Some(self.sent);
        }
    }

    pub(crate) fn state_for_test(&self) -> Option<&State> {
        self.last.as_ref().map(|report| &report.state)
    }
}

#[cfg(test)]
#[path = "updates_tests.rs"]
mod tests;
